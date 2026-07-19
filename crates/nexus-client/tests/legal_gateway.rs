//! Wiremock-backed tests for the Legal ACL gateway (`LegalGateway`,
//! `NexusLegalGateway`) — PROMPT-41, ADR-029.
//!
//! Post-ADR-029 every call is the one real Nexus route,
//! `POST capabilities/legal.clauses`, carrying a `CapabilityRequest`
//! envelope; the gateway unwraps `CapabilityResponse.payload`. These tests
//! assert the envelope's `capability_id`/`target_repo`/`payload` and the
//! `proposal_id`-vs-`topic` either/or, then the parse of the returned
//! `{"clauses": [...]}` payload.

use std::sync::Arc;

use nexus_client::{ClauseContext, LegalGateway, LegalGatewayError, NexusLegalGateway};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusLegalGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusLegalGateway::new(transport)
}

/// A successful `CapabilityResponse` wrapping `payload`.
fn ok(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({ "request_id": "req-test", "success": true, "payload": payload }))
}

#[tokio::test]
async fn request_approved_clauses_by_proposal_id_parses_a_multi_item_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "legal.clauses",
            "target_repo": "cognitum-legal",
            "caller": "cognitum-consultants",
            "payload": { "proposal_id": "proposal-1" }
        })))
        .respond_with(ok(serde_json::json!({
            "clauses": [
                {
                    "clause_id": "clause-1",
                    "title": "Limitation of Liability",
                    "approved_text": "Neither party shall be liable for...",
                    "policy_reference": "policy-2026-01"
                },
                {
                    "clause_id": "clause-2",
                    "title": "Confidentiality",
                    "approved_text": "Each party agrees to keep confidential...",
                    "policy_reference": "policy-2025-11"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-1")).await.expect("clauses fetch succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].clause_id, "clause-1");
    assert_eq!(result[0].title, "Limitation of Liability");
    assert_eq!(result[0].policy_reference, "policy-2026-01");
    assert_eq!(result[1].clause_id, "clause-2");

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_approved_clauses_by_topic_sends_the_topic_in_the_payload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .and(body_partial_json(serde_json::json!({ "payload": { "topic": "data-residency" } })))
        .respond_with(ok(serde_json::json!({ "clauses": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::Topic("data-residency")).await.expect("clauses fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_approved_clauses_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .respond_with(ok(serde_json::json!({ "clauses": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-none")).await.expect("clauses fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_clauses_payload() {
    let mock_server = MockServer::start().await;
    // The envelope is well-formed, but its `payload` is a bare array instead
    // of the expected `{"clauses": [...]}` object.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::Topic("anything")).await;

    match result {
        Err(LegalGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_transport_error_on_non_success_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-1")).await;

    match result {
        Err(LegalGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_transport_error_when_capability_reports_failure() {
    let mock_server = MockServer::start().await;
    // HTTP 200 but the capability envelope reports a business-level failure.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/legal.clauses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "request_id": "req-test",
            "success": false,
            "error": "capability not declared"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-1")).await;

    match result {
        Err(LegalGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}
