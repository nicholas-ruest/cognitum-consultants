//! Wiremock-backed tests for `CapabilityCaller` — the ADR-029
//! capability-envelope layer that turns a `CapabilityCall` into the one real
//! Nexus route (`POST capabilities/{capability_id}`), building the outbound
//! `CapabilityRequest` envelope and unwrapping `CapabilityResponse.payload`.

use std::sync::Arc;

use nexus_client::{CapabilityCall, CapabilityCaller, NexusTransportError, ReqwestNexusTransport};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn caller_for(mock_server: &MockServer) -> CapabilityCaller {
    let transport =
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    CapabilityCaller::new(transport)
}

fn call(capability_id: &str, payload: serde_json::Value) -> CapabilityCall {
    CapabilityCall { capability_id: capability_id.to_owned(), target_repo: "cognitum-sales".to_owned(), payload }
}

#[tokio::test]
async fn posts_to_the_capability_route_with_a_full_envelope_and_unwraps_the_payload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/sales.account_claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "resp-1",
            "success": true,
            "payload": { "match_status": "no_match" }
        })))
        .mount(&mock_server)
        .await;

    let caller = caller_for(&mock_server);
    let payload = caller
        .call(call("sales.account_claims", serde_json::json!({ "company_name": "Acme Corp" })))
        .await
        .expect("call succeeds");

    assert_eq!(payload, serde_json::json!({ "match_status": "no_match" }));

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).expect("body is json");

    assert_eq!(body["capability_id"], "sales.account_claims");
    assert_eq!(body["caller"], "cognitum-consultants");
    assert_eq!(body["target_repo"], "cognitum-sales");
    assert_eq!(body["payload"], serde_json::json!({ "company_name": "Acme Corp" }));
    assert_eq!(body["metadata"], serde_json::json!({}));
    // Identity travels in the envelope (placeholder values today).
    assert!(body["organization_id"].is_string());
    assert_eq!(body["actor"]["role"], "consultant");
    // A generated request id is always present.
    assert!(body["request_id"].as_str().is_some_and(|id| !id.is_empty()));
}

#[tokio::test]
async fn surfaces_a_capability_failure_with_its_error_message() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/sales.account_claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "resp-2",
            "success": false,
            "error": "capability not declared"
        })))
        .mount(&mock_server)
        .await;

    let caller = caller_for(&mock_server);
    let result = caller.call(call("sales.account_claims", serde_json::json!({}))).await;

    match result {
        Err(NexusTransportError::CapabilityFailure { capability_id, message }) => {
            assert_eq!(capability_id, "sales.account_claims");
            assert_eq!(message.as_deref(), Some("capability not declared"));
        }
        other => panic!("expected CapabilityFailure, got {other:?}"),
    }
}

#[tokio::test]
async fn surfaces_a_non_success_http_status_as_unexpected_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/sales.account_claims"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let caller = caller_for(&mock_server);
    let result = caller.call(call("sales.account_claims", serde_json::json!({}))).await;

    match result {
        Err(NexusTransportError::UnexpectedStatus { status }) => {
            assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
        }
        other => panic!("expected UnexpectedStatus, got {other:?}"),
    }
}

#[tokio::test]
async fn propagates_the_correlation_id_into_the_envelope_when_a_scope_is_active() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/sales.account_claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "resp-3",
            "success": true,
            "payload": {}
        })))
        .mount(&mock_server)
        .await;

    let caller = caller_for(&mock_server);
    correlation_context::scope("corr-123".to_owned(), async {
        caller.call(call("sales.account_claims", serde_json::json!({}))).await.expect("call succeeds");
    })
    .await;

    let received = mock_server.received_requests().await.expect("recording enabled");
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).expect("body is json");
    assert_eq!(body["correlation_id"], "corr-123");
}
