//! Armor ACL gateway (ADR-007, ADR-009, PROMPT-14).
//!
//! Armor owns authorization *policy*; this repo never computes or overrides
//! an authorization decision (`../../.plans/adr/ADR-009-authorization-permission-aware-presentation.md`).
//! [`ArmorGateway`] is a pure fetch-only ACL: it retrieves the consultant's
//! current [`PermissionAssertion`] set from Armor (via Nexus) and hands it
//! back untouched. There is no outbound business command here — see
//! `../../.plans/ddd/anti-corruption-layers.md` §10.
//!
//! # Response envelope: provisional, per ADR-007
//! Nexus's real Armor contract is not finalized (ADR-007). This gateway
//! assumes the response body is a JSON *object* with an `"assertions"` array
//! field (`{"assertions": [...]}`) rather than a bare top-level array. An
//! envelope was chosen over a bare array because it is the more common shape
//! for collection endpoints in practice and leaves room for the server to
//! add sibling metadata (e.g. pagination, a server-computed `as_of`
//! timestamp) later without a breaking shape change. This is a provisional
//! choice, not a confirmed contract — update [`AssertionsEnvelope`] once
//! Nexus's actual Armor response shape is known.
//!
//! # Transport-stack-assembly convention (read this before adding gateway #2)
//! [`NexusArmorGateway::new`] takes an already-fully-decorated
//! `Arc<dyn NexusTransport>` — it does **not** assemble the ADR-016
//! timeout/retry/circuit-breaker stack itself. Composition happens once, at
//! whatever call site wires up gateways for a running process (a future
//! composition root, not this module). This keeps the gateway itself a thin
//! request/response translator (consistent with `anti-corruption-layers.md`
//! §11's "pure translation boundary" rule), keeps decorator wiring in one
//! place instead of duplicated across ten gateway constructors, and lets
//! tests hand a bare mock transport straight to `new` without paying for
//! retry/timeout machinery they don't need. Every future gateway
//! (PROMPT-24 Sales, etc.) should follow this same convention: accept
//! `Arc<dyn NexusTransport>`, document the *expected* decoration in a
//! constructor doc comment, and leave assembly to the caller.
//!
//! Since `fetch_assertions` is a read (idempotent query), per ADR-016 the
//! caller is expected to pass a stack that includes
//! [`crate::retry::RetryingTransport`] — e.g.
//! `RetryingTransport::with_default_retries(Arc::new(TimeoutTransport::new(base, DEFAULT_READ_TIMEOUT)))`.
//! Never pass a retry-wrapped transport to a gateway method that issues a
//! non-idempotent command (none exist in this module, but future gateways
//! will have both).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::transport::{NexusRequest, NexusTransport, NexusTransportError};

/// A grant Armor currently asserts for a consultant. Never the underlying
/// authorization policy/rules themselves — those stay inside Armor
/// (`anti-corruption-layers.md` §10).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PermissionAssertion {
    pub consultant_id: String,
    pub capability: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

/// Envelope this gateway expects Armor's response body to match. See the
/// module docs for why an envelope (vs. a bare array) was chosen, and that
/// this is provisional pending Nexus's real contract.
#[derive(Debug, serde::Deserialize)]
struct AssertionsEnvelope {
    assertions: Vec<PermissionAssertion>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArmorGatewayError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("credential could not be encoded as an Authorization header value: {0}")]
    InvalidCredential(String),
    #[error("Armor returned a non-success status {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
    #[error("Armor returned a response body that did not match the expected assertions shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// Fetch-only ACL over Armor's Permission Assertions. No outbound business
/// command exists on this trait — see the module docs.
#[async_trait]
pub trait ArmorGateway: Send + Sync {
    /// Fetches the current Permission Assertions for `consultant_id`.
    ///
    /// `credential` is the consultant's session-derived token (ADR-008);
    /// this gateway does not know how to obtain one, only how to attach it
    /// to the outbound Nexus call.
    async fn fetch_assertions(
        &self,
        consultant_id: &str,
        credential: &str,
    ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError>;
}

/// [`ArmorGateway`] implementation backed by a [`NexusTransport`]. See the
/// module docs for the transport-stack-assembly convention.
pub struct NexusArmorGateway {
    transport: Arc<dyn NexusTransport>,
}

impl NexusArmorGateway {
    /// `transport` is expected to already be decorated per the ADR-016
    /// read-call convention (see module docs) — this constructor does not
    /// assemble timeout/retry/circuit-breaker layers itself.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ArmorGateway for NexusArmorGateway {
    async fn fetch_assertions(
        &self,
        consultant_id: &str,
        credential: &str,
    ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("consultant_id", consultant_id);
            format!("armor/v1/assertions?{}", query.finish())
        };

        let mut headers = HeaderMap::new();
        let auth_value = HeaderValue::from_str(&format!("Bearer {credential}"))
            .map_err(|e| ArmorGatewayError::InvalidCredential(e.to_string()))?;
        headers.insert(AUTHORIZATION, auth_value);

        let request = NexusRequest { method: Method::GET, path, headers, body: None };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(ArmorGatewayError::UnexpectedStatus { status: response.status });
        }

        let envelope: AssertionsEnvelope =
            serde_json::from_value(response.body).map_err(ArmorGatewayError::UnexpectedResponseShape)?;
        Ok(envelope.assertions)
    }
}
