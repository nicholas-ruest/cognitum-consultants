//! Wiremock-backed smoke tests for `ReqwestNexusTransport` (ADR-012 / U12).
//!
//! These exercise the two cross-cutting concerns the transport is
//! responsible for on every outbound Nexus call: propagating the inbound
//! correlation ID (via `correlation-context`'s task-local) and stamping a
//! W3C `traceparent` header when a real OTel span context is active.

use nexus_client::{NexusRequest, NexusTransport, ReqwestNexusTransport};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;
use reqwest::Method;
use tracing::Instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn get_request(rel_path: &str) -> NexusRequest {
    NexusRequest {
        method: Method::GET,
        path: rel_path.to_owned(),
        headers: reqwest::header::HeaderMap::new(),
        body: None,
    }
}

#[tokio::test]
async fn attaches_correlation_id_when_scope_is_active() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/widgets"))
        .and(header("x-correlation-id", "test-correlation-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&mock_server)
        .await;

    let transport = ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri())
        .expect("valid base url");

    let response = correlation_context::scope("test-correlation-id".to_owned(), async {
        transport.send(get_request("widgets")).await
    })
    .await
    .expect("request succeeds");

    // The mock only matches when `x-correlation-id` carries the expected
    // value, so a 200 here proves the header was attached correctly.
    assert_eq!(response.status, reqwest::StatusCode::OK);
}

#[tokio::test]
async fn omits_correlation_id_when_no_scope_is_active() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/widgets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&mock_server)
        .await;

    let transport = ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri())
        .expect("valid base url");

    // No `correlation_context::scope` wrapper: this task has no correlation
    // ID bound.
    let response = transport.send(get_request("widgets")).await.expect("request succeeds");
    assert_eq!(response.status, reqwest::StatusCode::OK);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
    assert!(
        received[0].headers.get("x-correlation-id").is_none(),
        "expected no x-correlation-id header, got {:?}",
        received[0].headers.get("x-correlation-id")
    );
}

#[tokio::test]
async fn attaches_valid_traceparent_when_span_context_exists() {
    // Real OTel-backed subscriber, scoped to this test only via a
    // `DefaultGuard` (not the process-global `.init()`) so it doesn't leak
    // into other tests running in the same process.
    let tracer_provider = SdkTracerProvider::builder().build();
    let tracer = tracer_provider.tracer("nexus-client-test");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = Registry::default().with(otel_layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/widgets"))
        .and(header_exists("traceparent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&mock_server)
        .await;

    let transport = ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri())
        .expect("valid base url");

    transport
        .send(get_request("widgets"))
        .instrument(tracing::info_span!("test_call"))
        .await
        .expect("request succeeds");

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
    let traceparent = received[0]
        .headers
        .get("traceparent")
        .expect("traceparent header present")
        .to_str()
        .expect("valid header value");

    let re = regex_lite_check(traceparent);
    assert!(re, "traceparent {traceparent:?} did not match the expected W3C format");
}

/// Minimal hand-rolled check for the W3C `traceparent` format
/// `^00-[0-9a-f]{32}-[0-9a-f]{16}-[0-9a-f]{2}$`, avoiding a `regex`
/// dev-dependency for a single assertion.
fn regex_lite_check(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    let is_hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
    parts.len() == 4 && parts[0] == "00" && is_hex(parts[1], 32) && is_hex(parts[2], 16) && is_hex(parts[3], 2)
}
