//! nexus-client: one submodule per ACL gateway (sales, commit, edu, capacity,
//! customer, execution, products, landscape, legal, armor). Pure translation
//! boundary — no business policy logic (see ../ddd/anti-corruption-layers.md §11).
//! Per-capability submodules are added starting at U12; this is the empty stub.
//!
//! # Resilience decorators (ADR-016, PROMPT-13)
//! [`timeout`], [`retry`], and [`circuit_breaker`] each provide a
//! `NexusTransport`-wrapping decorator. They compose freely because they
//! all implement the same [`NexusTransport`] trait, e.g.:
//! - Idempotent read call: `CircuitBreakingTransport::new(RetryingTransport::new(TimeoutTransport::new(inner, ..), ..), ..)`
//! - Write/command call: `CircuitBreakingTransport::new(TimeoutTransport::new(inner, ..), ..)`
//!   — note the deliberate *absence* of `RetryingTransport` here; see
//!   `retry`'s module docs for why writes must never be auto-retried.

pub mod circuit_breaker;
pub mod reqwest_transport;
pub mod retry;
pub mod timeout;
pub mod transport;

pub use circuit_breaker::{CircuitBreaker, CircuitBreakingTransport, CircuitState, SlidingWindowCircuitBreaker};
pub use reqwest_transport::ReqwestNexusTransport;
pub use retry::{DEFAULT_MAX_RETRIES, RetryingTransport};
pub use timeout::{DEFAULT_READ_TIMEOUT, DEFAULT_WRITE_TIMEOUT, TimeoutTransport};
pub use transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};
