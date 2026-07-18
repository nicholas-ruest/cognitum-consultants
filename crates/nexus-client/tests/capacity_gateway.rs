//! Wiremock-backed tests for the Capacity ACL gateway (`CapacityGateway`,
//! `NexusCapacityGateway`) — PROMPT-36.
//!
//! Mirrors `commit_gateway.rs`'s structure: a request-body-shape assertion
//! for `update_own_profile`, both a `ProfileUpdateAccepted` and a
//! `ProfileUpdateRejected { reason }` fixture (`anti-corruption-layers.md`
//! §4's two named inbound events), a `get_own_profile` query-param
//! assertion, and malformed-response error-not-panic cases for both calls.

use std::sync::Arc;

use nexus_client::{CapacityGateway, CapacityGatewayError, ConsultantProfileIntake, NexusCapacityGateway};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusCapacityGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusCapacityGateway::new(transport)
}

fn profile_fixture() -> ConsultantProfileIntake {
    ConsultantProfileIntake {
        skills: vec!["Rust".to_owned(), "Cloud Architecture".to_owned()],
        certifications: vec!["AWS Solutions Architect".to_owned()],
        languages: vec!["English".to_owned(), "French".to_owned()],
        availability_window: "2026-08-01/2026-12-31".to_owned(),
        geographic_coverage: vec!["EMEA".to_owned()],
    }
}

#[tokio::test]
async fn update_own_profile_sends_correct_command_body_and_parses_an_accepted_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capacity/v1/profile"))
        .and(body_json(serde_json::json!({
            "consultant_id": "consultant-1",
            "profile_fields": {
                "skills": ["Rust", "Cloud Architecture"],
                "certifications": ["AWS Solutions Architect"],
                "languages": ["English", "French"],
                "availability_window": "2026-08-01/2026-12-31",
                "geographic_coverage": ["EMEA"]
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "accepted": true
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.update_own_profile("consultant-1", profile_fixture()).await.expect("update succeeds");

    assert!(result.accepted);
    assert_eq!(result.reason, None);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn update_own_profile_parses_a_rejected_fixture_with_a_reason() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capacity/v1/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "accepted": false,
            "reason": "availability_window overlaps an existing commitment"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.update_own_profile("consultant-2", profile_fixture()).await.expect("update succeeds");

    assert!(!result.accepted);
    assert_eq!(result.reason.as_deref(), Some("availability_window overlaps an existing commitment"));
}

#[tokio::test]
async fn get_own_profile_sends_consultant_id_as_a_query_param_and_parses_the_profile() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/capacity/v1/profile"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "skills": ["Rust", "Cloud Architecture"],
            "certifications": ["AWS Solutions Architect"],
            "languages": ["English", "French"],
            "availability_window": "2026-08-01/2026-12-31",
            "geographic_coverage": ["EMEA"]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.get_own_profile("consultant-1").await.expect("get succeeds");

    assert_eq!(result, profile_fixture());
}

#[tokio::test]
async fn get_own_profile_handles_an_empty_profile() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/capacity/v1/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "skills": [],
            "certifications": [],
            "languages": [],
            "availability_window": "",
            "geographic_coverage": []
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.get_own_profile("consultant-empty").await.expect("get succeeds");

    assert!(result.skills.is_empty());
    assert!(result.certifications.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_update_own_profile_response() {
    let mock_server = MockServer::start().await;
    // Missing the required `accepted` field entirely — this must surface as
    // an error, not a panic.
    Mock::given(method("POST"))
        .and(path("/capacity/v1/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "unexpected": "shape"
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.update_own_profile("consultant-1", profile_fixture()).await;

    match result {
        Err(CapacityGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_get_own_profile_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `ConsultantProfileIntake` object.
    Mock::given(method("GET"))
        .and(path("/capacity/v1/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.get_own_profile("consultant-1").await;

    match result {
        Err(CapacityGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capacity/v1/profile"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.update_own_profile("consultant-1", profile_fixture()).await;

    match result {
        Err(CapacityGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
