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
//! will need this same ID to propagate on outbound Nexus calls. [`current`]
//! is the hook it should use — it reads the ID out of a `tokio` task-local
//! that is set for the lifetime of the request's async task, so it is valid
//! anywhere inside that task's call graph, including from a future
//! `nexus-client` HTTP call made while handling the request.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use uuid::Uuid;

/// Inbound/outbound header carrying the correlation ID.
pub static CORRELATION_ID_HEADER: HeaderName = HeaderName::from_static("x-correlation-id");

tokio::task_local! {
    static CORRELATION_ID: String;
}

/// Returns the correlation ID for the request currently being handled, if
/// called from within that request's async task.
///
/// U12's `nexus-client` calls this to attach the ID to outbound Nexus
/// requests as the same `x-correlation-id` header.
///
/// Unused today (no outbound caller exists yet) — allowed dead code until
/// U12 wires it up.
#[allow(dead_code)]
pub fn current() -> Option<String> {
    CORRELATION_ID.try_with(Clone::clone).ok()
}

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
/// task-local for later retrieval via [`current`].
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
    let mut response = CORRELATION_ID
        .scope(correlation_id, async move {
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
