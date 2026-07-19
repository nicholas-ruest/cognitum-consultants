//! Verifies a Google-signed identity token presented by another Cloud Run
//! service calling *this* one directly (ADR-018's `POST
//! /api/reactions/:reaction_handler` receiver) — the mirror image of
//! `nexus_client::reqwest_transport`'s `fetch_identity_token` (which mints
//! one of these to call *out*), and structurally the same
//! RS256-against-Google's-public-certs verification shape
//! `crate::firebase` already implements for browser-originated Firebase ID
//! tokens. **Not the same cert source**, though: these are standard
//! GCE/Cloud-Run-minted service-account identity tokens
//! (`https://www.googleapis.com/oauth2/v3/certs`), signed by a different
//! key set than Firebase's `securetoken` tokens
//! (`crate::firebase::GOOGLE_CERTS_URL`) — the two verifiers are
//! independent despite the shared shape, and deliberately not unified into
//! one generic "verify any Google JWT" helper: that would blur two
//! distinct trust boundaries (a consultant's own Google identity vs. a
//! specific peer *service's* identity) behind one knob.
//!
//! Two checks beyond signature validity, both required:
//! 1. **Audience** — must equal the audience this endpoint expects
//!    (`ADR-018`'s own choice, symmetric to how ADR-029 has this repo
//!    request `aud: nexus-api` when calling *out* to nexus). The caller
//!    must have deliberately minted a token for calling this endpoint, not
//!    reused a token minted for some unrelated purpose.
//! 2. **Caller identity** — the token's `email` claim must exactly match
//!    the configured expected caller (`config::Config::nexus_caller_service_account_email`).
//!    A valid, correctly-audienced Google token proves *some* Google
//!    identity made the call; this second check is what actually restricts
//!    it to nexus-server's own runtime identity, not just anyone who can
//!    mint a token for the right audience (self-selected by the caller,
//!    not a real access-control boundary on its own).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

/// Google's public endpoint for the certs that sign standard OAuth2/GCE
/// service-account identity tokens — distinct from
/// [`crate::firebase::GOOGLE_CERTS_URL`], which serves Firebase-specific
/// `securetoken` certs signed by a different key set. **Returns real JWKS**
/// (`{"keys": [{"kid", "n", "e", ...}]}`), not the flat `{kid: pem}` map
/// `crate::firebase`'s `robot/v1/metadata/x509/...` endpoint returns — verified
/// live (`curl https://www.googleapis.com/oauth2/v3/certs`) before writing
/// [`JwkSet`]/[`Jwk`] below, after an earlier version of this module wrongly
/// assumed the same flat-map shape and would have silently rejected every
/// genuine caller (`ParseCerts` on every real token) — exactly the class of
/// "looks right, never verified against the real endpoint" bug ADR-018's own
/// investigation exists to avoid repeating.
const GOOGLE_OAUTH2_CERTS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// One entry in Google's JWKS response — only the fields needed to build an
/// RSA [`DecodingKey`] via [`DecodingKey::from_rsa_components`] (base64url-
/// encoded modulus/exponent, per RFC 7518 §6.3.1). Every other JWK field
/// (`alg`, `use`, `kty`) is ignored via serde's default unknown-field
/// tolerance — `kty`/`alg` are implicitly `RSA`/`RS256` for every key this
/// endpoint has ever published, and this module already hardcodes
/// `Algorithm::RS256` in [`GoogleIdentityTokenVerifier::verify`].
#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

/// Both issuer forms Google's own tokens are observed to use — accepting
/// either is standard OIDC-client leniency, not a weakened check (both name
/// the same issuer).
const GOOGLE_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

/// How long a cached cert set is trusted before re-fetching. Matches
/// [`crate::firebase::CERTS_CACHE_TTL`]'s value/reasoning exactly (Google
/// rotates these keys periodically; this just bounds worst-case
/// staleness) — not shared as a constant across the two modules since they
/// cache genuinely different cert sets.
const CERTS_CACHE_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Deserialize)]
struct GoogleIdentityClaims {
    // Never read directly — `jsonwebtoken::decode`'s own `Validation`
    // (aud/iss set in `verify` below) checks these during decode itself.
    // Still must exist as fields for serde to deserialize them, or that
    // check silently becomes a no-op (same note `firebase::FirebaseClaims`
    // makes about its own `aud`/`iss` fields).
    #[allow(dead_code)]
    aud: String,
    #[allow(dead_code)]
    iss: String,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum GoogleIdentityTokenError {
    #[error("failed to fetch Google's public certs: {0}")]
    FetchCerts(#[source] reqwest::Error),
    #[error("failed to parse Google's public certs response")]
    ParseCerts,
    #[error("token is malformed")]
    MalformedToken(#[source] jsonwebtoken::errors::Error),
    #[error("token has no `kid` header, or it doesn't match any known Google cert")]
    UnknownKey,
    #[error("token signature, audience, or issuer did not validate")]
    InvalidToken,
    #[error("token has no verified email claim")]
    NoVerifiedEmail,
    #[error("{0} is not the expected caller for this endpoint")]
    UnexpectedCaller(String),
    #[error("no expected caller is configured — rejecting every inbound call (fail-closed)")]
    NoExpectedCallerConfigured,
}

/// Verifies inbound Google-signed identity tokens against one fixed
/// expected `(audience, caller_email)` pair. One instance is constructed
/// per configured expectation (`crate::bff-api`'s `main.rs` — one per
/// route/endpoint that needs this, mirroring `FirebaseSessionProvider`
/// being constructed once and shared via `AppState`), not per-request.
pub struct GoogleIdentityTokenVerifier {
    http_client: reqwest::Client,
    expected_audience: String,
    /// `None` means "reject every call" ([`GoogleIdentityTokenError::NoExpectedCallerConfigured`]),
    /// never "accept any caller" — see [`config::Config::nexus_caller_service_account_email`]'s
    /// own doc comment for why an unset expectation must fail closed.
    expected_caller_email: Option<String>,
    certs_cache: RwLock<Option<(HashMap<String, Jwk>, Instant)>>,
}

impl GoogleIdentityTokenVerifier {
    pub fn new(expected_audience: String, expected_caller_email: Option<String>) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            expected_audience,
            expected_caller_email,
            certs_cache: RwLock::new(None),
        }
    }

    /// Verifies `id_token`'s signature, audience, and issuer, then checks
    /// its `email` claim matches the configured expected caller exactly.
    /// `Ok(())` only when every check passes.
    pub async fn verify(&self, id_token: &str) -> Result<(), GoogleIdentityTokenError> {
        let Some(expected_caller_email) = self.expected_caller_email.as_deref() else {
            return Err(GoogleIdentityTokenError::NoExpectedCallerConfigured);
        };

        let header = decode_header(id_token).map_err(GoogleIdentityTokenError::MalformedToken)?;
        let kid = header.kid.ok_or(GoogleIdentityTokenError::UnknownKey)?;

        let jwk = self.jwk_for_kid(&kid).await?;
        let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
            .map_err(GoogleIdentityTokenError::MalformedToken)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.expected_audience]);
        validation.set_issuer(GOOGLE_ISSUERS);

        let token_data = decode::<GoogleIdentityClaims>(id_token, &decoding_key, &validation)
            .map_err(|_| GoogleIdentityTokenError::InvalidToken)?;

        let email = token_data
            .claims
            .email
            .filter(|_| token_data.claims.email_verified)
            .ok_or(GoogleIdentityTokenError::NoVerifiedEmail)?;

        if email != expected_caller_email {
            return Err(GoogleIdentityTokenError::UnexpectedCaller(email));
        }

        Ok(())
    }

    /// Returns the [`Jwk`] for `kid`, refreshing the cache if it's stale or
    /// the key isn't found yet — same refresh shape as
    /// [`crate::firebase::FirebaseSessionProvider::cert_for_kid`], but
    /// keyed on the real JWKS response shape (see [`JwkSet`]'s doc comment
    /// for why this isn't the flat `{kid: pem}` map that sibling method
    /// parses — a different Google endpoint, a different response shape).
    async fn jwk_for_kid(&self, kid: &str) -> Result<Jwk, GoogleIdentityTokenError> {
        {
            let cache = self.certs_cache.read().await;
            if let Some((keys, fetched_at)) = cache.as_ref()
                && fetched_at.elapsed() < CERTS_CACHE_TTL
                && let Some(jwk) = keys.get(kid)
            {
                return Ok(jwk.clone());
            }
        }

        let jwk_set: JwkSet = self
            .http_client
            .get(GOOGLE_OAUTH2_CERTS_URL)
            .send()
            .await
            .map_err(GoogleIdentityTokenError::FetchCerts)?
            .json()
            .await
            .map_err(|_| GoogleIdentityTokenError::ParseCerts)?;

        let keys: HashMap<String, Jwk> =
            jwk_set.keys.into_iter().map(|jwk| (jwk.kid.clone(), jwk)).collect();

        let jwk = keys.get(kid).cloned().ok_or(GoogleIdentityTokenError::UnknownKey)?;

        let mut cache = self.certs_cache.write().await;
        *cache = Some((keys, Instant::now()));

        Ok(jwk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A verifier with no expected caller configured must reject every
    /// call, before it even attempts to parse the token — the fail-closed
    /// default `config::Config::nexus_caller_service_account_email`'s doc
    /// comment promises.
    #[tokio::test]
    async fn rejects_every_call_when_no_expected_caller_is_configured() {
        let verifier = GoogleIdentityTokenVerifier::new("some-audience".to_owned(), None);

        let result = verifier.verify("not-even-a-real-token").await;

        assert!(matches!(result, Err(GoogleIdentityTokenError::NoExpectedCallerConfigured)));
    }

    /// A structurally invalid token (not even a parseable JWT) is rejected
    /// as malformed, not treated as a network/cert-fetch failure.
    #[tokio::test]
    async fn rejects_a_malformed_token() {
        let verifier = GoogleIdentityTokenVerifier::new(
            "some-audience".to_owned(),
            Some("expected@example.iam.gserviceaccount.com".to_owned()),
        );

        let result = verifier.verify("not-even-a-real-token").await;

        assert!(matches!(result, Err(GoogleIdentityTokenError::MalformedToken(_))));
    }

    /// Regression guard against the exact bug this module already had once:
    /// an earlier version assumed `oauth2/v3/certs` returns a flat
    /// `{kid: pem}` map (the shape `crate::firebase`'s *different* cert
    /// endpoint actually returns) and would have silently rejected every
    /// genuine caller — invisible to every other test here, since none of
    /// them can mint a real Google-signed token to exercise the happy path.
    /// This fixture is a representative excerpt of the real response,
    /// captured live (`curl https://www.googleapis.com/oauth2/v3/certs`)
    /// while fixing that bug.
    #[test]
    fn parses_googles_real_jwks_response_shape() {
        let body = r#"{
            "keys": [
                {
                    "n": "sjGSud3Gx-92yeucu7BIAhvzkybkL21eOCejL9t2JMpqy6cMThhS2Dtr-ByKdjtoD0GLP8LCT2yJfJH5YAbHVbvBU88eUsbGd7ZlaYqicTD5Pc6B_BO8LjSr3YH1kVoOcn8Lct31-EhloVAIxBLROsS2489N3bwWOLaOhnYCLvMWqVFqV5TJPMbIBzADXeJmAyF_K2uP5P7KDWYlGz2V6AH7aS6n3_K0vdb8SVeqCu8N0M5SifpSUMQidVp5Ku-wd0Yu6P9mZcAzS9GuzePJMNsbKjDkITlc1k-KZMO2RH23zAbMCNqVABQRFLCQhulYEAbd-sYbulsrHaw4MYo3Yw",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": "5896225329794346b0639e6f9d7bd8bce2954fd2",
                    "e": "AQAB",
                    "kty": "RSA"
                },
                {
                    "kid": "30fe0e23c4d6e36c52577b1e2fefd1abc38895de",
                    "e": "AQAB",
                    "use": "sig",
                    "kty": "RSA",
                    "n": "xyz-some-other-modulus-value",
                    "alg": "RS256"
                }
            ]
        }"#;

        let jwk_set: JwkSet = serde_json::from_str(body).expect("must parse Google's real JWKS shape");

        assert_eq!(jwk_set.keys.len(), 2);
        assert_eq!(jwk_set.keys[0].kid, "5896225329794346b0639e6f9d7bd8bce2954fd2");
        assert!(jwk_set.keys[0].n.starts_with("sjGSud3Gx"));
        assert_eq!(jwk_set.keys[0].e, "AQAB");
        assert_eq!(jwk_set.keys[1].kid, "30fe0e23c4d6e36c52577b1e2fefd1abc38895de");
    }
}
