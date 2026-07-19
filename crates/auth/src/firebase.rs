//! Real, Firebase-backed session provider (Google Sign-In), gated behind
//! the `firebase-auth` feature.
//!
//! Replaces the dev-stub for any environment where `Config::is_dev()` is
//! false: verifies a Firebase ID token's RS256 signature against Google's
//! own public certs (no Firebase Admin SDK / service-account credential
//! needed for verification — the certs are public), then checks the
//! token's `email` claim against the `approved_consultants` allowlist
//! (persistence) before issuing a session. An email not in that table is
//! rejected even with a perfectly valid Google identity — this is the
//! "admin approves specific non-cognitum.one addresses" access model, not
//! open sign-up.
//!
//! Sessions are persisted in Postgres (`sessions` table), unlike the
//! dev-stub's in-memory store — this provider is meant to run under Cloud
//! Run, which can scale an instance to zero between requests, so an
//! in-memory session would otherwise silently log everyone out.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{Session, SessionError, SessionProvider};

/// Google's public endpoint for the certs that sign Firebase ID tokens —
/// no credential needed to fetch this, it's meant to be public.
const GOOGLE_CERTS_URL: &str =
    "https://www.googleapis.com/robot/v1/metadata/x509/securetoken@system.gserviceaccount.com";

/// How long a cached cert set is trusted before re-fetching, independent of
/// any `kid`-not-found refetch below. Google rotates these keys
/// periodically; this just bounds worst-case staleness.
const CERTS_CACHE_TTL: Duration = Duration::from_secs(3600);

/// How long a real session is valid for. Longer than the dev-stub's,
/// since a real consultant re-authenticating with Google every day would
/// be poor UX for a login that actually has a credential check behind it.
const SESSION_TTL_DAYS: i64 = 14;

#[derive(Debug, Deserialize)]
struct FirebaseClaims {
    aud: String,
    iss: String,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum FirebaseAuthError {
    #[error("failed to fetch Google's public certs: {0}")]
    FetchCerts(#[source] reqwest::Error),
    #[error("failed to parse Google's public certs response")]
    ParseCerts,
    #[error("token is malformed or its signature is invalid: {0}")]
    InvalidToken(#[source] jsonwebtoken::errors::Error),
    #[error("token has no `kid` header, or it doesn't match any known Google cert")]
    UnknownKey,
    #[error("token audience/issuer does not match this Firebase project")]
    WrongAudienceOrIssuer,
    #[error("token has no email claim, or the email is unverified")]
    NoVerifiedEmail,
    #[error("{0} is not an approved consultant email")]
    NotApproved(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// [`SessionProvider`] backed by Postgres, plus the Firebase-specific
/// login entrypoint ([`FirebaseSessionProvider::login_with_id_token`]) —
/// not part of the general [`SessionProvider`] trait, same "provider-
/// specific constructor lives outside the trait" convention
/// `DevStubSessionProvider::create_dev_session` already establishes.
pub struct FirebaseSessionProvider {
    pool: persistence::Pool,
    http_client: reqwest::Client,
    project_id: String,
    certs_cache: RwLock<Option<(HashMap<String, String>, Instant)>>,
}

impl FirebaseSessionProvider {
    pub fn new(pool: persistence::Pool, project_id: String) -> Self {
        Self { pool, http_client: reqwest::Client::new(), project_id, certs_cache: RwLock::new(None) }
    }

    /// Verifies `id_token`, checks its email against the allowlist, and —
    /// if approved — issues and persists a new [`Session`]. Every failure
    /// mode (bad signature, wrong project, unapproved email, db error) is
    /// distinguished so the BFF handler can return an honest error instead
    /// of a generic 401 for all of them.
    ///
    /// **`@cognitum.one` addresses are always allowed, with no allowlist
    /// row needed** — these are internal/admin accounts, not the
    /// non-cognitum.one consultant addresses `approved_consultants` exists
    /// to gate. Anyone else must have an explicit row (added via
    /// `scripts/manage-consultants.sh`, gated by the caller's own `gcloud`
    /// identity, not a shared password).
    pub async fn login_with_id_token(&self, id_token: &str) -> Result<Session, FirebaseAuthError> {
        let claims = self.verify(id_token).await?;

        let email = claims.email.filter(|_| claims.email_verified).ok_or(FirebaseAuthError::NoVerifiedEmail)?;

        if !email.ends_with("@cognitum.one") {
            let approved = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM approved_consultants WHERE email = $1")
                .bind(&email)
                .fetch_one(&self.pool)
                .await?;
            if approved == 0 {
                return Err(FirebaseAuthError::NotApproved(email));
            }
        }

        let session = Session {
            session_id: Uuid::new_v4(),
            consultant_id: email,
            expires_at: Utc::now() + ChronoDuration::days(SESSION_TTL_DAYS),
        };

        sqlx::query("INSERT INTO sessions (session_id, consultant_id, expires_at) VALUES ($1, $2, $3)")
            .bind(session.session_id)
            .bind(&session.consultant_id)
            .bind(session.expires_at)
            .execute(&self.pool)
            .await?;

        Ok(session)
    }

    async fn verify(&self, id_token: &str) -> Result<FirebaseClaims, FirebaseAuthError> {
        let header = decode_header(id_token).map_err(FirebaseAuthError::InvalidToken)?;
        let kid = header.kid.ok_or(FirebaseAuthError::UnknownKey)?;

        let pem = self.cert_for_kid(&kid).await?;
        let decoding_key = DecodingKey::from_rsa_pem(pem.as_bytes()).map_err(FirebaseAuthError::InvalidToken)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.project_id]);
        validation.set_issuer(&[format!("https://securetoken.google.com/{}", self.project_id)]);

        let token_data = decode::<FirebaseClaims>(id_token, &decoding_key, &validation)
            .map_err(|_| FirebaseAuthError::WrongAudienceOrIssuer)?;

        Ok(token_data.claims)
    }

    /// Returns the PEM cert for `kid`, refreshing the cache if it's stale
    /// or the key isn't found yet (handles Google rotating keys between
    /// our cache TTL windows).
    async fn cert_for_kid(&self, kid: &str) -> Result<String, FirebaseAuthError> {
        {
            let cache = self.certs_cache.read().await;
            if let Some((certs, fetched_at)) = cache.as_ref() {
                if fetched_at.elapsed() < CERTS_CACHE_TTL {
                    if let Some(pem) = certs.get(kid) {
                        return Ok(pem.clone());
                    }
                }
            }
        }

        let certs: HashMap<String, String> = self
            .http_client
            .get(GOOGLE_CERTS_URL)
            .send()
            .await
            .map_err(FirebaseAuthError::FetchCerts)?
            .json()
            .await
            .map_err(|_| FirebaseAuthError::ParseCerts)?;

        let pem = certs.get(kid).cloned().ok_or(FirebaseAuthError::UnknownKey)?;

        let mut cache = self.certs_cache.write().await;
        *cache = Some((certs, Instant::now()));

        Ok(pem)
    }
}

#[async_trait::async_trait]
impl SessionProvider for FirebaseSessionProvider {
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>, SessionError> {
        let row = sqlx::query_as::<_, (Uuid, String, chrono::DateTime<Utc>)>(
            "SELECT session_id, consultant_id, expires_at FROM sessions WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| SessionError::OperationFailed(err.to_string()))?;

        Ok(row.and_then(|(session_id, consultant_id, expires_at)| {
            (expires_at > Utc::now()).then_some(Session { session_id, consultant_id, expires_at })
        }))
    }

    async fn delete_session(&self, session_id: Uuid) -> Result<(), SessionError> {
        sqlx::query("DELETE FROM sessions WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|err| SessionError::OperationFailed(err.to_string()))?;
        Ok(())
    }
}
