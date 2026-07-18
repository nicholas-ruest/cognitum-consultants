//! Per-call timeout decorator for [`NexusTransport`] (ADR-016).
//!
//! `NexusTransport` is a plain trait, not a literal `tower::Service`, so the
//! `tower::timeout` layer ADR-016 describes is implemented here as a
//! decorator ([`TimeoutTransport`]) that races the inner call against
//! `tokio::time::sleep` via `tokio::time::timeout`, rather than as a
//! `tower::Layer`.
//!
//! # Read vs. write timeout convention
//! ADR-016 sets placeholder default timeout budgets that future gateway
//! code (none exists yet — see ADR-007/PROMPT-13) should apply when
//! constructing a `TimeoutTransport` for a given capability call:
//! - **Reads** (idempotent queries, e.g. catalog lookups): **5 seconds**
//!   ([`DEFAULT_READ_TIMEOUT`]).
//! - **Writes** (user-blocking commands the consultant is actively waiting
//!   on): **3 seconds** ([`DEFAULT_WRITE_TIMEOUT`]).
//!
//! This unit has no gateways yet, so it cannot itself decide which value
//! applies to which call — `timeout` is simply a constructor parameter.
//! Real per-capability tuning is deferred to ADR-012 metrics, per ADR-016.
//!
//! # A third tier: extended read (PROMPT-35, ADR-016's Edu example)
//! ADR-016 names a distinction beyond the plain read/write split: "a
//! read-mostly `ProductsGateway`/`LegalGateway` query gets a longer
//! allowance than a synchronous, user-blocking `SalesGateway`
//! conflict-check" — [`DEFAULT_READ_TIMEOUT`] already covers a background/
//! page-load read like `CommitGateway::list_proposals`, but a capability
//! whose *only* outbound call is a read with no synchronous UI-blocking
//! call ever sharing its gateway (Edu's `RequestLearningCatalogQuery`,
//! ADR-016's own named example) can afford to wait out a slower upstream
//! before giving up, rather than sharing the same budget a page that also
//! renders a synchronous conflict-check needs to stay snappy for.
//! [`DEFAULT_EXTENDED_READ_TIMEOUT`] is that third, longer budget —
//! double [`DEFAULT_READ_TIMEOUT`], the same "generous but bounded"
//! reasoning this repo's other untuned placeholder defaults use (see e.g.
//! `bff_core::event_ingestion::DEFAULT_ACTION_QUEUE_ENTRY_TTL_HOURS`'s doc
//! comment) — not yet tuned against real ADR-012 latency data, same
//! caveat as every other constant in this module.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};

/// Placeholder default timeout for idempotent read/query calls (ADR-016).
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Placeholder default timeout for user-blocking write/command calls
/// (ADR-016).
pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(3);

/// Placeholder default timeout for read-mostly capabilities with no
/// synchronous, user-blocking call sharing their gateway (PROMPT-35). See
/// the module docs' "A third tier" section.
pub const DEFAULT_EXTENDED_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Wraps an inner [`NexusTransport`] and enforces a fixed timeout on every
/// `send` call, per ADR-016's per-gateway timeout budget requirement.
///
/// On expiry, returns [`NexusTransportError::Timeout`] instead of letting
/// the inner call run unbounded.
pub struct TimeoutTransport<T: NexusTransport + ?Sized> {
    inner: Arc<T>,
    timeout: Duration,
}

impl<T: NexusTransport + ?Sized> TimeoutTransport<T> {
    /// `timeout` is caller-supplied; see the module docs for the ADR-016
    /// read (5s) vs. write (3s) convention future gateway code should use.
    pub fn new(inner: Arc<T>, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}

#[async_trait]
impl<T: NexusTransport + ?Sized> NexusTransport for TimeoutTransport<T> {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError> {
        match tokio::time::timeout(self.timeout, self.inner.send(request)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(NexusTransportError::Timeout { after: self.timeout }),
        }
    }
}
