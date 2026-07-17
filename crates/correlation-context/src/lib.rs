//! Transport-agnostic correlation-ID propagation for the current async task.
//! Owned here (not in bff-api or nexus-client) so both can share one
//! task-local without depending on each other (ADR-004 dependency direction).

pub const CORRELATION_ID_HEADER_NAME: &str = "x-correlation-id";

tokio::task_local! {
    static CORRELATION_ID: String;
}

/// Reads the correlation ID for the request currently being handled, if
/// called from within that request's async task. `None` outside any scope.
pub fn current() -> Option<String> {
    CORRELATION_ID.try_with(Clone::clone).ok()
}

/// Runs `fut` with `id` bound as the current correlation ID for its task.
pub async fn scope<F: std::future::Future>(id: String, fut: F) -> F::Output {
    CORRELATION_ID.scope(id, fut).await
}
