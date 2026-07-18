//! Wiremock-backed tests for the Execution ACL gateway (`ExecutionGateway`,
//! `NexusExecutionGateway`) — PROMPT-38.
//!
//! Mirrors `commit_gateway.rs`'s structure: a request-body-shape assertion
//! per outbound call, several `EngagementSnapshot` fixture scenarios (an
//! on-track engagement and an at-risk one — proving the gateway against more
//! than one shape of `delivery_status`), and a malformed-response
//! error-not-panic case.

use std::sync::Arc;

use nexus_client::{EngagementTaskSummary, ExecutionGateway, ExecutionGatewayError, NexusExecutionGateway};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusExecutionGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusExecutionGateway::new(transport)
}

#[tokio::test]
async fn request_assigned_engagements_sends_consultant_id_as_a_query_param_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/execution/v1/engagements"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "engagements": [
                {
                    "engagement_id": "engagement-1",
                    "workstreams": ["Discovery", "Delivery"],
                    "milestones": ["Kickoff complete"],
                    "tasks": [
                        {"task_id": "task-1", "title": "Draft delivery plan", "status": "assigned"}
                    ],
                    "delivery_status": "on_track",
                    "deep_link": "https://execution.cognitum.one/engagements/engagement-1"
                },
                {
                    "engagement_id": "engagement-2",
                    "workstreams": ["Delivery"],
                    "milestones": [],
                    "tasks": [],
                    "delivery_status": "at_risk",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_engagements("consultant-1").await.expect("request succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].engagement_id, "engagement-1");
    assert_eq!(result[0].workstreams, vec!["Discovery".to_string(), "Delivery".to_string()]);
    assert_eq!(
        result[0].tasks,
        vec![EngagementTaskSummary {
            task_id: "task-1".to_string(),
            title: "Draft delivery plan".to_string(),
            status: "assigned".to_string(),
        }]
    );
    assert_eq!(result[0].delivery_status, "on_track");
    assert_eq!(result[0].deep_link.as_deref(), Some("https://execution.cognitum.one/engagements/engagement-1"));

    assert_eq!(result[1].delivery_status, "at_risk");
    assert!(result[1].tasks.is_empty());
    assert_eq!(result[1].deep_link, None);
}

#[tokio::test]
async fn request_assigned_engagements_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/execution/v1/engagements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "engagements": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_engagements("consultant-empty").await.expect("request succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn confirm_task_completion_sends_correct_command_body_and_handles_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/execution/v1/task-completions"))
        .and(body_json(serde_json::json!({
            "task_id": "task-1",
            "consultant_id": "consultant-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"accepted": true})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.confirm_task_completion("task-1", "consultant-1").await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_engagements_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"engagements": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/execution/v1/engagements"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_engagements("consultant-1").await;

    match result {
        Err(ExecutionGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/execution/v1/task-completions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.confirm_task_completion("task-1", "consultant-1").await;

    match result {
        Err(ExecutionGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
