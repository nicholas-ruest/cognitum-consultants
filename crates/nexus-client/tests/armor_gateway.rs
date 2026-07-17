//! Wiremock-backed tests for the Armor ACL gateway (`ArmorGateway`,
//! `NexusArmorGateway`) — PROMPT-14 / U14.
//!
//! Per the module docs on `nexus_client::armor`, the gateway expects Armor's
//! response body wrapped in an `{"assertions": [...]}` envelope.

use std::sync::Arc;

use nexus_client::{ArmorGateway, ArmorGatewayError, NexusArmorGateway, ReqwestNexusTransport};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusArmorGateway {
    let transport =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusArmorGateway::new(transport)
}

fn assertion_json(consultant_id: &str, capability: &str, scope: &str, expires_at: &str) -> serde_json::Value {
    serde_json::json!({
        "consultant_id": consultant_id,
        "capability": capability,
        "scope": scope,
        "expires_at": expires_at,
    })
}

#[tokio::test]
async fn parses_five_varied_permission_assertions() {
    let mock_server = MockServer::start().await;
    let assertions = vec![
        assertion_json("consultant-1", "dashboard.view", "global", "2026-08-01T00:00:00Z"),
        assertion_json("consultant-1", "proposal.create", "region:emea", "2026-08-01T00:00:00Z"),
        assertion_json("consultant-1", "proposal.approve", "region:emea", "2026-09-15T12:30:00Z"),
        assertion_json("consultant-1", "nav.landscape", "global", "2026-07-20T08:00:00Z"),
        assertion_json("consultant-1", "capacity.request", "team:alpha", "2026-12-31T23:59:59Z"),
    ];
    Mock::given(method("GET"))
        .and(path("/armor/v1/assertions"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "assertions": assertions })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.fetch_assertions("consultant-1", "test-credential").await.expect("fetch succeeds");

    assert_eq!(result.len(), 5);
    assert_eq!(result[0].capability, "dashboard.view");
    assert_eq!(result[1].capability, "proposal.create");
    assert_eq!(result[1].scope, "region:emea");
    assert_eq!(result[4].capability, "capacity.request");
    assert!(result.iter().all(|a| a.consultant_id == "consultant-1"));
}

#[tokio::test]
async fn parses_exactly_one_permission_assertion() {
    let mock_server = MockServer::start().await;
    let assertions = vec![assertion_json("consultant-2", "dashboard.view", "global", "2026-08-01T00:00:00Z")];
    Mock::given(method("GET"))
        .and(path("/armor/v1/assertions"))
        .and(query_param("consultant_id", "consultant-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "assertions": assertions })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.fetch_assertions("consultant-2", "test-credential").await.expect("fetch succeeds");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].capability, "dashboard.view");
}

#[tokio::test]
async fn returns_empty_vec_for_zero_permission_assertions() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/armor/v1/assertions"))
        .and(query_param("consultant_id", "consultant-3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "assertions": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.fetch_assertions("consultant-3", "test-credential").await.expect("fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_response_shape() {
    let mock_server = MockServer::start().await;
    // Missing the "assertions" envelope field entirely, and the wrong shape
    // (bare array) besides — this must surface as an error, not a panic.
    Mock::given(method("GET"))
        .and(path("/armor/v1/assertions"))
        .and(query_param("consultant_id", "consultant-4"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "unexpected": "shape" }
        ])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.fetch_assertions("consultant-4", "test-credential").await;

    match result {
        Err(ArmorGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn attaches_authorization_header_with_correct_credential() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/armor/v1/assertions"))
        .and(query_param("consultant_id", "consultant-5"))
        .and(header("authorization", "Bearer secret-session-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "assertions": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    gateway.fetch_assertions("consultant-5", "secret-session-token").await.expect("fetch succeeds");

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].headers.get("authorization").expect("authorization header present"),
        "Bearer secret-session-token"
    );
}
