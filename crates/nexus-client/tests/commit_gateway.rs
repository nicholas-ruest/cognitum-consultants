//! Wiremock-backed tests for the Commit ACL gateway (`CommitGateway`,
//! `NexusCommitGateway`) — PROMPT-34, ADR-029.
//!
//! Post-ADR-029 `create_proposal` and `list_proposals` are both
//! `POST capabilities/commit.proposals` (distinguished by payload) and
//! `request_proposal_action` is `POST capabilities/commit.proposal_actions`;
//! each carries a `CapabilityRequest` envelope and the gateway unwraps
//! `CapabilityResponse.payload`.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use nexus_client::{CommitGateway, CommitGatewayError, NexusCommitGateway};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusCommitGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusCommitGateway::new(transport)
}

fn ok(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({ "request_id": "req-test", "success": true, "payload": payload }))
}

#[tokio::test]
async fn create_proposal_sends_correct_envelope_payload_and_parses_a_draft_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposals"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "commit.proposals",
            "target_repo": "cognitum-commit",
            "payload": { "origin_reference": "acme-corp", "consultant_id": "consultant-1" }
        })))
        .respond_with(ok(serde_json::json!({
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
        .and(path("/api/v1/capabilities/commit.proposals"))
        .respond_with(ok(serde_json::json!({
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
async fn list_proposals_sends_consultant_id_in_the_payload_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposals"))
        .and(body_partial_json(serde_json::json!({ "payload": { "consultant_id": "consultant-1" } })))
        .respond_with(ok(serde_json::json!({
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
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposals"))
        .respond_with(ok(serde_json::json!({ "proposals": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.list_proposals("consultant-empty").await.expect("list succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_proposal_action_sends_correct_envelope_payload_and_handles_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposal_actions"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "commit.proposal_actions",
            "payload": { "proposal_id": "proposal-1", "action": "request_revision" }
        })))
        .respond_with(ok(serde_json::json!({"acknowledged": true})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_proposal_action("proposal-1", "request_revision").await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_create_proposal_payload() {
    let mock_server = MockServer::start().await;
    // Well-formed envelope, but its `payload` is missing required fields.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposals"))
        .and(body_partial_json(serde_json::json!({ "payload": { "origin_reference": "acme-corp" } })))
        .respond_with(ok(serde_json::json!({ "unexpected": "shape" })))
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
async fn returns_gateway_error_not_panic_on_malformed_list_proposals_payload() {
    let mock_server = MockServer::start().await;
    // Well-formed envelope, but its `payload` is a bare array instead of the
    // expected `{"proposals": [...]}` object.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposals"))
        .and(body_partial_json(serde_json::json!({ "payload": { "consultant_id": "consultant-1" } })))
        .respond_with(ok(serde_json::json!([])))
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
async fn returns_transport_error_on_non_success_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/commit.proposal_actions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_proposal_action("proposal-1", "resend").await;

    match result {
        Err(CommitGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}
