use reqwest::{Method, StatusCode, header::HeaderMap};

pub struct NexusRequest {
    pub method: Method,
    /// Relative to the configured Nexus base URL. MUST NOT have a leading
    /// `/` — e.g. `"sales/v1/account-claims"`.
    pub path: String,
    /// Caller-supplied headers. MUST NOT set `x-correlation-id` or
    /// `traceparent` — the transport overwrites both unconditionally.
    pub headers: HeaderMap,
    pub body: Option<serde_json::Value>,
}

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
}

#[async_trait::async_trait]
pub trait NexusTransport: Send + Sync {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError>;
}
