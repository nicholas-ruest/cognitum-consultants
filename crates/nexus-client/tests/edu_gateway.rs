//! Wiremock-backed tests for the Edu ACL gateway (`EduGateway`,
//! `NexusEduGateway`) — PROMPT-35, ADR-029.
//!
//! Post-ADR-029 every call is `POST capabilities/edu.catalog` carrying a
//! `CapabilityRequest` envelope; filters travel in the payload
//! (`{"filters": [...]}`) rather than as repeated query params. The gateway
//! unwraps `CapabilityResponse.payload`, still expecting a
//! `{"snapshots": [...]}` object.

use std::sync::Arc;

use nexus_client::{EduGateway, EduGatewayError, NexusEduGateway};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusEduGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusEduGateway::new(transport)
}

fn ok(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({ "request_id": "req-test", "success": true, "payload": payload }))
}

#[tokio::test]
async fn request_learning_catalog_sends_consultant_id_in_the_payload_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "edu.catalog",
            "target_repo": "cognitum-edu",
            "payload": { "consultant_id": "consultant-1" }
        })))
        .respond_with(ok(serde_json::json!({
            "snapshots": [
                {
                    "course_id": "course-1",
                    "title": "Cloud Security Fundamentals",
                    "progress_status": "completed",
                    "certification_status": "issued",
                    "deep_link": "https://edu.cognitum.one/courses/course-1"
                },
                {
                    "course_id": "course-2",
                    "title": "Advanced Negotiation",
                    "progress_status": "in_progress",
                    "certification_status": null,
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-1", None).await.expect("catalog fetch succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].course_id, "course-1");
    assert_eq!(result[0].progress_status, "completed");
    assert_eq!(result[0].certification_status.as_deref(), Some("issued"));
    assert_eq!(result[0].deep_link.as_deref(), Some("https://edu.cognitum.one/courses/course-1"));
    assert_eq!(result[1].certification_status, None);
    assert_eq!(result[1].deep_link, None);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_learning_catalog_sends_filters_in_the_payload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .and(body_partial_json(serde_json::json!({
            "payload": { "consultant_id": "consultant-1", "filters": ["in_progress"] }
        })))
        .respond_with(ok(serde_json::json!({ "snapshots": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let filters = vec!["in_progress".to_owned()];
    let result = gateway.request_learning_catalog("consultant-1", Some(&filters)).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_learning_catalog_parses_a_training_due_fixture_with_no_certification() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .respond_with(ok(serde_json::json!({
            "snapshots": [
                {
                    "course_id": "course-3",
                    "title": "Annual Compliance Refresher",
                    "progress_status": "not_started",
                    "certification_status": "required",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-2", None).await.expect("catalog fetch succeeds");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].progress_status, "not_started");
    assert_eq!(result[0].certification_status.as_deref(), Some("required"));
}

#[tokio::test]
async fn request_learning_catalog_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .respond_with(ok(serde_json::json!({ "snapshots": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-empty", None).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_payload() {
    let mock_server = MockServer::start().await;
    // Well-formed envelope, but its `payload` is a bare array instead of the
    // expected `{"snapshots": [...]}` object.
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-1", None).await;

    match result {
        Err(EduGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_transport_error_on_non_success_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capabilities/edu.catalog"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-1", None).await;

    match result {
        Err(EduGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}
