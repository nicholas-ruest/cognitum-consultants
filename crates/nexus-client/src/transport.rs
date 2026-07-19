use std::sync::Arc;
use std::time::Duration;

use reqwest::{Method, StatusCode, header::HeaderMap};

#[derive(Clone)]
pub struct NexusRequest {
    pub method: Method,
    /// Relative to the configured Nexus base URL. MUST NOT have a leading
    /// `/` — e.g. `"capabilities/sales.account_claims"`.
    pub path: String,
    /// Caller-supplied headers. MUST NOT set `x-correlation-id` or
    /// `traceparent` — the transport overwrites both unconditionally.
    pub headers: HeaderMap,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct NexusResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum NexusTransportError {
    #[error("invalid Nexus request path {path:?}: {reason}")]
    InvalidUrl { path: String, reason: String },
    #[error("Nexus request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("failed to decode Nexus response body as JSON: {0}")]
    DecodeResponseBytes(#[source] reqwest::Error),
    #[error("failed to parse Nexus response body as JSON: {0}")]
    ParseResponseJson(#[source] serde_json::Error),
    /// Raised by [`crate::timeout::TimeoutTransport`] (ADR-016) when the
    /// inner `send` call did not complete within the configured budget.
    #[error("Nexus request timed out after {after:?}")]
    Timeout { after: Duration },
    /// Raised by [`crate::circuit_breaker::CircuitBreakingTransport`]
    /// (ADR-016) when the breaker for this gateway is open and the call was
    /// short-circuited without reaching the network.
    #[error("circuit breaker open for this Nexus gateway; call short-circuited")]
    CircuitOpen,
    /// Nexus answered a capability call with a non-success HTTP status
    /// (e.g. `404` "capability not declared", or a `5xx` the resilience
    /// stack already exhausted its retries on). Raised by
    /// [`CapabilityCaller::call`].
    #[error("Nexus capability call returned a non-success status {status}")]
    UnexpectedStatus { status: StatusCode },
    /// Nexus answered a capability call with HTTP success but
    /// `CapabilityResponse.success == false` — a gateway-level failure
    /// carrying the server's `error` string. Raised by
    /// [`CapabilityCaller::call`] rather than handing a "successful" HTTP
    /// response with an embedded business failure back to the gateway.
    #[error("Nexus capability {capability_id} failed: {}", .message.as_deref().unwrap_or("no error message"))]
    CapabilityFailure { capability_id: String, message: Option<String> },
}

#[async_trait::async_trait]
pub trait NexusTransport: Send + Sync {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError>;
}

/// The one real synchronous route `nexus-server` exposes
/// (`POST /api/v1/capabilities/:capability_id`, ADR-029). Every gateway
/// call is this same POST; the `capability_id` is the only path variable.
/// Kept here so the `capabilities/{id}` join convention lives in one place.
const CAPABILITIES_PATH_PREFIX: &str = "capabilities/";

/// Placeholder caller organization id. `cognitum-consultants`'s session
/// (`auth::Session`, ADR-008 interim dev-stub) carries only a
/// `consultant_id` — there is no per-session organization id or role in
/// this repo yet — so a documented placeholder is used, mirroring the same
/// "interim placeholder pending real Armor/OIDC integration" precedent
/// `crate::permissions`'s `credential` already establishes on the
/// `cognitum-consultants` side. Replace once real session org/role lands.
const PLACEHOLDER_ORGANIZATION_ID: &str = "cognitum-consultants-dev-org";

/// Placeholder actor role. See [`PLACEHOLDER_ORGANIZATION_ID`].
const PLACEHOLDER_ROLE: &str = "consultant";

/// The authenticated caller on whose behalf a [`CapabilityCall`] is made —
/// the `caller`/`organization_id`/`actor` fields of the outbound
/// `nexus_contracts::CapabilityRequest` envelope (ADR-029). Held once on a
/// [`CapabilityCaller`] rather than threaded per-call, because today every
/// value is a process-level placeholder (see [`PLACEHOLDER_ORGANIZATION_ID`]);
/// swap [`Self::default`] for real per-session construction when session
/// org/role exists.
#[derive(Clone, Debug)]
pub struct CallerIdentity {
    pub caller: String,
    pub organization_id: String,
    pub actor: Actor,
}

impl Default for CallerIdentity {
    fn default() -> Self {
        Self {
            caller: "cognitum-consultants".to_owned(),
            organization_id: PLACEHOLDER_ORGANIZATION_ID.to_owned(),
            actor: Actor { user_id: None, service_account: None, role: PLACEHOLDER_ROLE.to_owned() },
        }
    }
}

/// The `actor` sub-object of the `CapabilityRequest` envelope. Mirrors
/// `nexus_contracts`'s wire shape exactly (all bare JSON strings); a local
/// plain-serde struct, never a cross-repo Rust dependency (ADR-007: the
/// repos are independent, JSON-contract-compatible, not type-shared).
#[derive(Clone, Debug, serde::Serialize)]
pub struct Actor {
    pub user_id: Option<String>,
    pub service_account: Option<String>,
    pub role: String,
}

/// A single capability invocation a gateway hands to [`CapabilityCaller`].
/// Carries only what varies per call — the `capability_id`, its
/// `target_repo`, and the per-call `payload` (the exact command/query JSON
/// the gateway already built). The envelope's identity/bookkeeping fields
/// (`request_id`, `caller`, `organization_id`, `actor`, `correlation_id`,
/// `metadata`) are filled by the caller, so the envelope shape lives in one
/// place rather than being rebuilt in all ten gateways (ADR-029).
#[derive(Clone, Debug)]
pub struct CapabilityCall {
    pub capability_id: String,
    pub target_repo: String,
    pub payload: serde_json::Value,
}

/// Outbound `nexus_contracts::CapabilityRequest`, mirrored locally as a
/// plain-serde struct (ADR-007/ADR-029 — no cross-repo type dependency).
#[derive(serde::Serialize)]
struct CapabilityRequest<'a> {
    request_id: String,
    capability_id: &'a str,
    caller: &'a str,
    target_repo: &'a str,
    organization_id: &'a str,
    actor: &'a Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<String>,
    payload: serde_json::Value,
    metadata: serde_json::Value,
}

/// Inbound `nexus_contracts::CapabilityResponse`, mirrored locally.
#[derive(serde::Deserialize)]
struct CapabilityResponse {
    #[allow(dead_code)]
    request_id: Option<String>,
    success: bool,
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    error: Option<String>,
}

/// Wraps a fully-decorated [`NexusTransport`] stack and turns a
/// [`CapabilityCall`] into the one real POST `nexus-server` accepts
/// (`capabilities/{capability_id}`), building the full `CapabilityRequest`
/// envelope on the way out and unwrapping `CapabilityResponse.payload` on
/// the way back (ADR-029).
///
/// This is where the ADR-029 capability-envelope pattern is established
/// **once**, above the resilience decorators (timeout/retry/circuit-breaker,
/// which stay pure `NexusTransport` decorators underneath) and below the
/// gateways (which build only their per-call `payload`). Each gateway holds
/// its own `CapabilityCaller` over its own decorated transport, so the
/// existing per-gateway retry-safety split (a retry-wrapped query stack vs.
/// a no-retry command stack) is preserved unchanged.
pub struct CapabilityCaller {
    transport: Arc<dyn NexusTransport>,
    identity: CallerIdentity,
}

impl CapabilityCaller {
    /// Uses the placeholder [`CallerIdentity::default`] — see that type's
    /// docs for why per-session identity is a placeholder today.
    pub fn new(transport: Arc<dyn NexusTransport>) -> Self {
        Self { transport, identity: CallerIdentity::default() }
    }

    /// Uses an explicit caller identity (for when real per-session
    /// org/role/actor context becomes available).
    pub fn with_identity(transport: Arc<dyn NexusTransport>, identity: CallerIdentity) -> Self {
        Self { transport, identity }
    }

    /// Issues `call` as a `POST capabilities/{capability_id}` against the
    /// underlying transport, returning the unwrapped
    /// `CapabilityResponse.payload` on success. Surfaces a non-2xx HTTP
    /// status as [`NexusTransportError::UnexpectedStatus`] and a
    /// `success: false` envelope as [`NexusTransportError::CapabilityFailure`]
    /// (carrying `.error`), rather than handing either back as a "success".
    pub async fn call(&self, call: CapabilityCall) -> Result<serde_json::Value, NexusTransportError> {
        // capability_ids are `[a-z0-9_.]` (RFC 3986 unreserved plus `.`),
        // so they are already safe as a single path segment — no
        // percent-encoding needed.
        let path = format!("{CAPABILITIES_PATH_PREFIX}{}", call.capability_id);

        let envelope = CapabilityRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            capability_id: &call.capability_id,
            caller: &self.identity.caller,
            target_repo: &call.target_repo,
            organization_id: &self.identity.organization_id,
            actor: &self.identity.actor,
            // Propagated exactly as the transport propagates `traceparent`:
            // from the inbound correlation scope, when one is active.
            correlation_id: correlation_context::current(),
            payload: call.payload,
            metadata: serde_json::json!({}),
        };
        let body = serde_json::to_value(&envelope).expect("capability request always serializes");

        let request = NexusRequest { method: Method::POST, path, headers: HeaderMap::new(), body: Some(body) };
        let response = self.transport.send(request).await?;

        if !response.status.is_success() {
            return Err(NexusTransportError::UnexpectedStatus { status: response.status });
        }

        let envelope: CapabilityResponse =
            serde_json::from_value(response.body).map_err(NexusTransportError::ParseResponseJson)?;

        if !envelope.success {
            return Err(NexusTransportError::CapabilityFailure {
                capability_id: call.capability_id,
                message: envelope.error,
            });
        }
        Ok(envelope.payload)
    }
}
