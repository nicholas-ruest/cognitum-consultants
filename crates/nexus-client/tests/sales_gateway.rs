//! Wiremock-backed tests for the Sales ACL gateway (`SalesGateway`,
//! `NexusSalesGateway`) — PROMPT-24 / U24.
//!
//! Fixture 1 replicates the worked example in `anti-corruption-layers.md`
//! §1 exactly. Fixtures 2 and 3 cover other `match_status` values so the
//! gateway is proven against more than one shape of verdict.

use std::sync::Arc;

use nexus_client::{NexusSalesGateway, SalesGateway, SalesGatewayError};
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusSalesGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusSalesGateway::new(transport)
}

#[tokio::test]
async fn parses_active_owned_account_worked_example() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/account-claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "match_status": "active_owned_account",
            "creation_allowed": false,
            "display_message": "This company is already being worked.",
            "permitted_actions": ["request_collaboration", "submit_referral", "cancel"]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.check_account_claim("Acme Corp", "consultant-1").await.expect("check succeeds");

    assert_eq!(result.match_status, "active_owned_account");
    assert!(!result.creation_allowed);
    assert_eq!(result.display_message, "This company is already being worked.");
    assert_eq!(result.permitted_actions, vec!["request_collaboration", "submit_referral", "cancel"]);
}

#[tokio::test]
async fn parses_available_claim_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/account-claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "match_status": "available_claim",
            "creation_allowed": true,
            "display_message": "No existing owner found. You may create this lead.",
            "permitted_actions": ["create_lead", "cancel"]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.check_account_claim("Beta LLC", "consultant-2").await.expect("check succeeds");

    assert_eq!(result.match_status, "available_claim");
    assert!(result.creation_allowed);
    assert_eq!(result.permitted_actions, vec!["create_lead", "cancel"]);
}

#[tokio::test]
async fn parses_no_match_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/account-claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "match_status": "no_match",
            "creation_allowed": true,
            "display_message": "No matching company found in Sales.",
            "permitted_actions": []
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.check_account_claim("Nonexistent Inc", "consultant-3").await.expect("check succeeds");

    assert_eq!(result.match_status, "no_match");
    assert!(result.creation_allowed);
    assert!(result.permitted_actions.is_empty());
}

#[tokio::test]
async fn check_account_claim_sends_correct_command_body() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/account-claims"))
        .and(body_json(serde_json::json!({
            "company_name": "Acme Corp",
            "consultant_id": "consultant-1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "match_status": "no_match",
            "creation_allowed": true,
            "display_message": "No matching company found in Sales.",
            "permitted_actions": []
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    gateway.check_account_claim("Acme Corp", "consultant-1").await.expect("check succeeds");

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_collaboration_sends_correct_command_body_and_handles_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/collaboration-requests"))
        .and(body_json(serde_json::json!({
            "company_reference": "company-42",
            "consultant_id": "consultant-1",
            "message": "I'd like to collaborate on this account."
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"acknowledged": true})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway
        .request_collaboration("company-42", "consultant-1", Some("I'd like to collaborate on this account."))
        .await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn submit_referral_sends_correct_command_body_and_handles_success() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sales/v1/referrals"))
        .and(body_json(serde_json::json!({
            "company_reference": "company-99",
            "consultant_id": "consultant-2",
            "notes": "Referring to the EMEA team."
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"acknowledged": true})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.submit_referral("company-99", "consultant-2", Some("Referring to the EMEA team.")).await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_account_claim_response() {
    let mock_server = MockServer::start().await;
    // Missing required fields entirely (e.g. no `creation_allowed`, no
    // `permitted_actions`) — this must surface as an error, not a panic.
    Mock::given(method("POST"))
        .and(path("/sales/v1/account-claims"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "unexpected": "shape"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.check_account_claim("Acme Corp", "consultant-1").await;

    match result {
        Err(SalesGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}
