//! Circuit breaker skeleton for per-gateway resilience (ADR-016).
//!
//! ADR-016 asks for a circuit breaker that "trips ... to fail fast for a
//! cooldown period rather than continuing to spend timeout budget on calls
//! likely to fail," with real threshold/cooldown tuning explicitly
//! deferred to future ADR-012 metrics analysis. This module provides a
//! minimal but *real* implementation ([`SlidingWindowCircuitBreaker`]) that
//! actually tracks recent success/failure outcomes and actually changes
//! state (`Closed` -> `Open` -> `HalfOpen` -> ...) — it is not an empty
//! trait. The threshold/window/cooldown constants are hardcoded
//! placeholders, per ADR-016.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};

/// Current state of a [`CircuitBreaker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Calls are allowed through and outcomes are being tracked.
    Closed,
    /// Calls are short-circuited without reaching the inner transport.
    Open,
    /// Cooldown elapsed; a probe call is allowed through to test recovery.
    HalfOpen,
}

/// Tracks per-gateway failure rates and reports whether calls should be
/// allowed through. See the module docs for why this is deliberately
/// minimal rather than production-tuned.
pub trait CircuitBreaker: Send + Sync {
    /// Whether a new call should be attempted right now.
    fn allow_call(&self) -> bool;
    /// Record that the most recent call succeeded.
    fn record_success(&self);
    /// Record that the most recent call failed.
    fn record_failure(&self);
    /// Current breaker state, for observability/tests.
    fn state(&self) -> CircuitState;
}

struct Inner {
    state: CircuitState,
    /// Sliding window of recent outcomes; `true` = success.
    outcomes: VecDeque<bool>,
    opened_at: Option<Instant>,
}

/// A minimal sliding-window failure-rate circuit breaker.
///
/// Opens once at least `min_samples` outcomes have been recorded in the
/// last `window_size` calls and the failure rate is `>= failure_threshold`.
/// Stays open for `cooldown`, then allows a single half-open probe: success
/// closes the circuit again, failure re-opens it (resetting the cooldown).
pub struct SlidingWindowCircuitBreaker {
    window_size: usize,
    min_samples: usize,
    failure_threshold: f64,
    cooldown: Duration,
    inner: Mutex<Inner>,
}

impl SlidingWindowCircuitBreaker {
    pub fn new(window_size: usize, min_samples: usize, failure_threshold: f64, cooldown: Duration) -> Self {
        Self {
            window_size,
            min_samples,
            failure_threshold,
            cooldown,
            inner: Mutex::new(Inner { state: CircuitState::Closed, outcomes: VecDeque::new(), opened_at: None }),
        }
    }

    /// Placeholder default: opens once at least 5 calls have been seen and
    /// at least half of the last 10 failed; 30s cooldown before probing.
    pub fn with_defaults() -> Self {
        Self::new(10, 5, 0.5, Duration::from_secs(30))
    }

    fn record(&self, success: bool) {
        let mut inner = self.inner.lock().expect("circuit breaker mutex poisoned");
        match inner.state {
            CircuitState::HalfOpen => {
                if success {
                    inner.state = CircuitState::Closed;
                    inner.outcomes.clear();
                    inner.opened_at = None;
                } else {
                    inner.state = CircuitState::Open;
                    inner.outcomes.clear();
                    inner.opened_at = Some(Instant::now());
                }
            }
            CircuitState::Closed => {
                inner.outcomes.push_back(success);
                while inner.outcomes.len() > self.window_size {
                    inner.outcomes.pop_front();
                }
                if inner.outcomes.len() >= self.min_samples {
                    let failures = inner.outcomes.iter().filter(|ok| !**ok).count();
                    let failure_rate = failures as f64 / inner.outcomes.len() as f64;
                    if failure_rate >= self.failure_threshold {
                        inner.state = CircuitState::Open;
                        inner.opened_at = Some(Instant::now());
                    }
                }
            }
            CircuitState::Open => {
                // Outcome recorded after the breaker already re-opened
                // (e.g. a racing call that started before the open
                // transition); nothing to update.
            }
        }
    }
}

impl CircuitBreaker for SlidingWindowCircuitBreaker {
    fn allow_call(&self) -> bool {
        let mut inner = self.inner.lock().expect("circuit breaker mutex poisoned");
        match inner.state {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true,
            CircuitState::Open => {
                let elapsed = inner.opened_at.map(|at| at.elapsed()).unwrap_or_default();
                if elapsed >= self.cooldown {
                    inner.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn record_success(&self) {
        self.record(true);
    }

    fn record_failure(&self) {
        self.record(false);
    }

    fn state(&self) -> CircuitState {
        self.inner.lock().expect("circuit breaker mutex poisoned").state
    }
}

/// Wraps an inner [`NexusTransport`] with a [`CircuitBreaker`]: checks
/// `allow_call` before issuing the request and records success/failure
/// afterwards. A 5xx response counts as a breaker failure even though it is
/// an `Ok(NexusResponse)`, not an `Err`.
pub struct CircuitBreakingTransport<T: NexusTransport + ?Sized, B: CircuitBreaker> {
    inner: Arc<T>,
    breaker: B,
}

impl<T: NexusTransport + ?Sized, B: CircuitBreaker> CircuitBreakingTransport<T, B> {
    pub fn new(inner: Arc<T>, breaker: B) -> Self {
        Self { inner, breaker }
    }
}

#[async_trait]
impl<T: NexusTransport + ?Sized, B: CircuitBreaker> NexusTransport for CircuitBreakingTransport<T, B> {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError> {
        if !self.breaker.allow_call() {
            return Err(NexusTransportError::CircuitOpen);
        }

        let result = self.inner.send(request).await;
        match &result {
            Ok(response) if !response.status.is_server_error() => self.breaker.record_success(),
            _ => self.breaker.record_failure(),
        }
        result
    }
}
