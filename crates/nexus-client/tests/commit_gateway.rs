//! Wiremock-backed tests for the Commit ACL gateway (`CommitGateway`,
//! `NexusCommitGateway`) — PROMPT-34.
//!
//! Mirrors `sales_gateway.rs`'s structure: a request-body-shape assertion
//! per outbound call, several `ProposalSummary` fixture scenarios (a draft,
//! an in-review, and an accepted proposal — proving the gateway against more
//! than one shape of `status`/`stage`), and a malformed-response
//! error-not-panic case.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use nexus_client::{CommitGateway, CommitGatewayError, NexusCommitGateway};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusCommitGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusCommitGateway::new(transport)
}

#[tokio::test]
async fn create_proposal_sends_correct_command_body_and_parses_a_draft_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/commit/v1/proposals"))
        .and(body_json(serde_json::json!({
            "origin_reference": "acme-corp",
            "consultant_id": "consultant-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "proposal_id": "proposal-1",
            "title": "Acme Corp Engagement Proposal",
            "status": "draft",
            "stage": "drafting",
            "last_updated_at": "2026-01-01T00:00:00Z",
            "deep_link": "https://commit.cognitum.one/proposals/proposal-1"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.create_proposal("acme-corp", "consultant-1").await.expect("create succeeds");

    assert_eq!(result.proposal_id, "proposal-1");
    assert_eq!(result.title, "Acme Corp Engagement Proposal");
    assert_eq!(result.status, "draft");
    assert_eq!(result.stage, "drafting");
    assert_eq!(result.last_updated_at, Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
    assert_eq!(result.deep_link.as_deref(), Some("https://commit.cognitum.one/proposals/proposal-1"));

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn create_proposal_parses_a_fixture_with_no_deep_link() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/commit/v1/proposals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "proposal_id": "proposal-2",
            "title": "Beta LLC Engagement Proposal",
            "status": "draft",
            "stage": "drafting",
            "last_updated_at": "2026-01-02T00:00:00Z",
            "deep_link": null
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.create_proposal("beta-llc", "consultant-2").await.expect("create succeeds");

    assert_eq!(result.proposal_id, "proposal-2");
    assert_eq!(result.deep_link, None);
}

#[tokio::test]
async fn list_proposals_sends_consultant_id_as_a_query_param_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/commit/v1/proposals"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "proposals": [
                {
                    "proposal_id": "proposal-1",
                    "title": "Acme Corp Engagement Proposal",
                    "status": "in_review",
                    "stage": "internal_review",
                    "last_updated_at": "2026-01-03T00:00:00Z",
                    "deep_link": "https://commit.cognitum.one/proposals/proposal-1"
                },
                {
                    "proposal_id": "proposal-3",
                    "title": "Gamma Inc Engagement Proposal",
                    "status": "accepted",
                    "stage": "closed_won",
                    "last_updated_at": "2026-01-04T00:00:00Z",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.list_proposals("consultant-1").await.expect("list succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].status, "in_review");
    assert_eq!(result[1].status, "accepted");
    assert_eq!(result[1].deep_link, None);
}

#[tokio::test]
async fn list_proposals_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/commit/v1/proposals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "proposals": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.list_proposals("consultant-empty").await.expect("list succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_proposal_action_sends_correct_command_body_and_handles_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/commit/v1/proposal-actions"))
        .and(body_json(serde_json::json!({
            "proposal_id": "proposal-1",
            "action": "request_revision"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"acknowledged": true})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_proposal_action("proposal-1", "request_revision").await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_create_proposal_response() {
    let mock_server = MockServer::start().await;
    // Missing required fields entirely (e.g. no `status`, no `stage`) — this
    // must surface as an error, not a panic.
    Mock::given(method("POST"))
        .and(path("/commit/v1/proposals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "unexpected": "shape"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.create_proposal("acme-corp", "consultant-1").await;

    match result {
        Err(CommitGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_list_proposals_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"proposals": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/commit/v1/proposals"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.list_proposals("consultant-1").await;

    match result {
        Err(CommitGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/commit/v1/proposal-actions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_proposal_action("proposal-1", "resend").await;

    match result {
        Err(CommitGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
