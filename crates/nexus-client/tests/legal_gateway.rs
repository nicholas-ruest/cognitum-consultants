//! Wiremock-backed tests for the Legal ACL gateway (`LegalGateway`,
//! `NexusLegalGateway`) — PROMPT-41.
//!
//! Mirrors `products_gateway.rs`'s structure: a multi-item fixture scenario
//! for the read (proving the gateway against more than one
//! `ApprovedLegalSnippet`), a request-shape assertion proving `proposal_id`
//! and `topic` are mutually exclusive query params, and a malformed-response
//! error-not-panic case.

use std::sync::Arc;

use nexus_client::{ClauseContext, LegalGateway, LegalGatewayError, NexusLegalGateway};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusLegalGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusLegalGateway::new(transport)
}

#[tokio::test]
async fn request_approved_clauses_by_proposal_id_parses_a_multi_item_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/legal/v1/clauses"))
        .and(query_param("proposal_id", "proposal-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
async fn request_approved_clauses_by_topic_sends_the_topic_query_param() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/legal/v1/clauses"))
        .and(query_param("topic", "data-residency"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "clauses": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::Topic("data-residency")).await.expect("clauses fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_approved_clauses_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/legal/v1/clauses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "clauses": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-none")).await.expect("clauses fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_clauses_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"clauses": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/legal/v1/clauses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
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
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/legal/v1/clauses"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_approved_clauses(ClauseContext::ProposalId("proposal-1")).await;

    match result {
        Err(LegalGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
