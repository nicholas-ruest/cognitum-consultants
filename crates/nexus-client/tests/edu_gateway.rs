//! Wiremock-backed tests for the Edu ACL gateway (`EduGateway`,
//! `NexusEduGateway`) — PROMPT-35.
//!
//! Mirrors `commit_gateway.rs`'s structure: a request-shape assertion for
//! the one outbound call, several `LearningSnapshot` fixture scenarios (a
//! completed course, an in-progress one, and a certification-pending one —
//! proving the gateway against more than one shape of
//! `progress_status`/`certification_status`), and a malformed-response
//! error-not-panic case.

use std::sync::Arc;

use nexus_client::{EduGateway, EduGatewayError, NexusEduGateway};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusEduGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusEduGateway::new(transport)
}

#[tokio::test]
async fn request_learning_catalog_sends_consultant_id_as_a_query_param_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
async fn request_learning_catalog_sends_repeated_filter_query_params() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .and(query_param("consultant_id", "consultant-1"))
        .and(query_param("filter", "in_progress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "snapshots": [] })))
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
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "snapshots": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-empty", None).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"snapshots": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
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
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/edu/v1/catalog"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_learning_catalog("consultant-1", None).await;

    match result {
        Err(EduGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
