//! Retry decorator for **idempotent, read-only** [`NexusTransport`] calls
//! only (ADR-016).
//!
//! # Contract: reads only, never commands
//! [`RetryingTransport`] must be used **only** to wrap idempotent
//! query/read calls (e.g. `RequestProductCatalogQuery`). Per ADR-016,
//! non-idempotent commands (e.g. `RequestCollaborationCommand`,
//! `CreateProposalCommand`) must **never** be auto-retried by this crate: a
//! retry against an unknown-outcome prior attempt risks a duplicate side
//! effect in the owning capability, which this repo has no way to detect
//! or undo. Rust's type system cannot enforce "only wrap GET-shaped calls
//! in this decorator" — that discipline is the responsibility of whoever
//! wires up a gateway on top of this crate, and is documented here as the
//! binding contract.
//!
//! **For write/command calls, do not wrap in `RetryingTransport` — use
//! [`crate::timeout::TimeoutTransport`] (or the base [`NexusTransport`])
//! directly.** That absence of a retry wrapper *is* the distinct
//! non-retrying API for commands; there is deliberately no second
//! "retry-with-a-flag-off" type.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};

/// Default bounded retry count (ADR-016: "e.g. 3 retries").
pub const DEFAULT_MAX_RETRIES: u32 = 3;

const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(2);

/// Wraps an inner [`NexusTransport`] with bounded exponential-backoff
/// retries. **Only wrap idempotent/read calls in this type** — see the
/// module docs.
pub struct RetryingTransport<T: NexusTransport + ?Sized> {
    inner: Arc<T>,
    max_retries: u32,
}

impl<T: NexusTransport + ?Sized> RetryingTransport<T> {
    pub fn new(inner: Arc<T>, max_retries: u32) -> Self {
        Self { inner, max_retries }
    }

    /// Convenience constructor using [`DEFAULT_MAX_RETRIES`].
    pub fn with_default_retries(inner: Arc<T>) -> Self {
        Self::new(inner, DEFAULT_MAX_RETRIES)
    }
}

#[async_trait]
impl<T: NexusTransport + ?Sized> NexusTransport for RetryingTransport<T> {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError> {
        let mut backoff = INITIAL_BACKOFF;
        let mut retries_left = self.max_retries;

        loop {
            let result = self.inner.send(request.clone()).await;
            if retries_left == 0 || !is_retriable(&result) {
                return result;
            }
            retries_left -= 1;
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

/// Retriable: transient network/timeout failures, and 5xx responses (the
/// upstream capability is likely struggling, not rejecting the request
/// shape). NOT retriable: malformed request/response shapes (`InvalidUrl`,
/// `ParseResponseJson`) and 4xx responses — retrying an unchanged request
/// against a client-side rejection cannot succeed.
fn is_retriable(result: &Result<NexusResponse, NexusTransportError>) -> bool {
    match result {
        Ok(response) => response.status.is_server_error(),
        Err(NexusTransportError::Timeout { .. }) => true,
        Err(NexusTransportError::Request(_)) => true,
        Err(NexusTransportError::DecodeResponseBytes(_)) => true,
        Err(NexusTransportError::CircuitOpen) => false,
        Err(NexusTransportError::InvalidUrl { .. }) => false,
        Err(NexusTransportError::ParseResponseJson(_)) => false,
    }
}
