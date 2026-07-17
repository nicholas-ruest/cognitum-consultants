//! Wiremock-backed tests for the ADR-016 resilience decorators
//! (`TimeoutTransport`, `RetryingTransport`, `CircuitBreakingTransport`)
//! and their composability (PROMPT-13 / U13).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nexus_client::circuit_breaker::{CircuitBreakingTransport, SlidingWindowCircuitBreaker};
use nexus_client::retry::RetryingTransport;
use nexus_client::timeout::TimeoutTransport;
use nexus_client::{NexusRequest, NexusTransport, NexusTransportError, ReqwestNexusTransport};
use reqwest::Method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, Respond};
use wiremock::matchers::{method, path};

fn get_request(rel_path: &str) -> NexusRequest {
    NexusRequest {
        method: Method::GET,
        path: rel_path.to_owned(),
        headers: reqwest::header::HeaderMap::new(),
        body: None,
    }
}

/// Responds with `failure_status` for the first `fail_times` calls, then
/// `200 OK` forever after. Used to deterministically drive retry
/// scenarios without depending on wiremock's mount-order semantics.
struct FailNTimesThenSucceed {
    remaining_failures: AtomicU32,
    failure_status: u16,
}

impl Respond for FailNTimesThenSucceed {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let remaining = self.remaining_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            if n > 0 { Some(n - 1) } else { Some(0) }
        });
        let had_failures_left = remaining.map(|n| n > 0).unwrap_or(false);
        if had_failures_left {
            ResponseTemplate::new(self.failure_status)
        } else {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({}))
        }
    }
}

/// Always responds with `failure_status`.
struct AlwaysFail {
    failure_status: u16,
}

impl Respond for AlwaysFail {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        ResponseTemplate::new(self.failure_status)
    }
}

#[tokio::test]
async fn timeout_transport_returns_timeout_error_when_inner_call_is_slow() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(300)))
        .mount(&mock_server)
        .await;

    let inner =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    let transport = TimeoutTransport::new(inner, Duration::from_millis(50));

    let result = transport.send(get_request("slow")).await;

    match result {
        Err(NexusTransportError::Timeout { after }) => {
            assert_eq!(after, Duration::from_millis(50));
        }
        other => panic!("expected Timeout error, got {other:?}"),
    }
}

#[tokio::test]
async fn retrying_transport_succeeds_on_second_attempt_after_one_failure() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(FailNTimesThenSucceed { remaining_failures: AtomicU32::new(1), failure_status: 503 })
        .mount(&mock_server)
        .await;

    let inner =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    let transport = RetryingTransport::new(inner, 3);

    let response = transport.send(get_request("flaky")).await.expect("retry recovers");
    assert_eq!(response.status, reqwest::StatusCode::OK);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 2, "expected exactly 2 requests: 1 failure + 1 successful retry");
}

#[tokio::test]
async fn retrying_transport_gives_up_after_max_retries_exhausted() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/always-down"))
        .respond_with(AlwaysFail { failure_status: 503 })
        .mount(&mock_server)
        .await;

    let inner =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    let max_retries = 3;
    let transport = RetryingTransport::new(inner, max_retries);

    let response = transport.send(get_request("always-down")).await.expect("transport itself does not error on 5xx");
    assert_eq!(response.status, reqwest::StatusCode::SERVICE_UNAVAILABLE);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(
        received.len() as u32,
        max_retries + 1,
        "expected exactly max_retries + 1 requests (1 initial + {max_retries} retries)"
    );
}

#[tokio::test]
async fn circuit_breaking_transport_short_circuits_once_open() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/unstable"))
        .respond_with(AlwaysFail { failure_status: 500 })
        .mount(&mock_server)
        .await;

    let inner =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    // 2 samples minimum, 100% failure threshold, long cooldown so the
    // breaker stays open for the duration of this test.
    let breaker = SlidingWindowCircuitBreaker::new(5, 2, 1.0, Duration::from_secs(60));
    let transport = CircuitBreakingTransport::new(inner, breaker);

    // Drive enough failures to trip the breaker open.
    for _ in 0..2 {
        let _ = transport.send(get_request("unstable")).await;
    }

    let requests_before = mock_server.received_requests().await.expect("recording enabled").len();
    assert_eq!(requests_before, 2, "sanity check: both priming calls reached the mock server");

    let result = transport.send(get_request("unstable")).await;
    assert!(
        matches!(result, Err(NexusTransportError::CircuitOpen)),
        "expected the breaker to short-circuit, got {result:?}"
    );

    let requests_after = mock_server.received_requests().await.expect("recording enabled").len();
    assert_eq!(requests_after, requests_before, "short-circuited call must not reach the mock server");
}

#[tokio::test]
async fn resilience_decorators_compose_for_a_read_call() {
    // Mirrors the ADR-016 read-call stack: circuit breaker wrapping retry
    // wrapping timeout wrapping the base transport. Proves the three
    // decorators compose because they all implement `NexusTransport`.
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/composed"))
        .respond_with(FailNTimesThenSucceed { remaining_failures: AtomicU32::new(1), failure_status: 500 })
        .mount(&mock_server)
        .await;

    let base =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    let timeout = Arc::new(TimeoutTransport::new(base, Duration::from_secs(5)));
    let retrying = Arc::new(RetryingTransport::new(timeout, 3));
    let breaker = SlidingWindowCircuitBreaker::new(10, 5, 0.9, Duration::from_secs(30));
    let composed = CircuitBreakingTransport::new(retrying, breaker);

    let response = composed.send(get_request("composed")).await.expect("retry recovers through the full stack");
    assert_eq!(response.status, reqwest::StatusCode::OK);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 2, "1 failure absorbed by the retry layer + 1 successful retry");
}
