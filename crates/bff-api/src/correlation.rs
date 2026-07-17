//! Correlation-ID middleware (ADR-012).
//!
//! Every inbound request gets a correlation ID: either accepted from the
//! `x-correlation-id` request header (if a caller — e.g. the SPA — already
//! has one), or generated fresh as a UUID v4. The ID is attached as a field
//! on the request's tracing span so every log line emitted while handling
//! the request is filterable/joinable by it, and it is echoed back on the
//! response so the caller can correlate its own logs too.
//!
//! Header name choice: ADR-012 mandates *a* correlation-ID header mechanism
//! but does not pin an exact header name. This module uses `x-correlation-id`
//! as that convention.
//!
//! Forward-compatibility (U12): `nexus-client`'s outbound `NexusTransport`
//! needs this same ID to propagate on outbound Nexus calls. It reads the ID
//! out of a `tokio` task-local that is set for the lifetime of the
//! request's async task via [`correlation_context::current`], so it is
//! valid anywhere inside that task's call graph, including from a
//! `nexus-client` HTTP call made while handling the request.
//!
//! The task-local itself, the `current()`/`scope()` accessors, and the
//! header-name constant live in the `correlation-context` crate so that
//! `bff-api` and `nexus-client` can share them without depending on each
//! other (ADR-004). This module keeps the axum-specific middleware and
//! inbound UUID-generation logic, which are inbound-request concerns local
//! to `bff-api`.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use uuid::Uuid;

/// Inbound/outbound header carrying the correlation ID.
pub static CORRELATION_ID_HEADER: HeaderName =
    HeaderName::from_static(correlation_context::CORRELATION_ID_HEADER_NAME);

/// Reads the correlation ID from the inbound header, or generates a new one.
fn extract_or_generate(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(&CORRELATION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// Axum middleware (tower layer via [`axum::middleware::from_fn`]) that
/// attaches a correlation ID to the request's tracing span and to a
/// task-local for later retrieval via [`correlation_context::current`].
pub async fn middleware(request: Request, next: Next) -> Response {
    let correlation_id = extract_or_generate(request.headers());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "http_request",
        correlation_id = %correlation_id,
        %method,
        %path,
    );

    let response_id = correlation_id.clone();
    let mut response = correlation_context::scope(correlation_id, async move {
        tracing::info!("request started");
        let response = next.run(request).await;
        tracing::info!(status = %response.status(), "request completed");
        response
    })
    .instrument(span)
    .await;

    if let Ok(header_value) = HeaderValue::from_str(&response_id) {
        response.headers_mut().insert(CORRELATION_ID_HEADER.clone(), header_value);
    }

    response
}
