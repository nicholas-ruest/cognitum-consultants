//! Wiremock-backed tests for the Customer ACL gateway (`CustomerGateway`,
//! `NexusCustomerGateway`) — PROMPT-37.
//!
//! Mirrors `edu_gateway.rs`'s structure: a request-shape assertion for the
//! one outbound call (including the optional `customer_id` narrowing
//! param), several `CustomerContextCard` fixture scenarios (proving the
//! gateway against more than one `health_status`), and a malformed-response
//! error-not-panic case.

use std::sync::Arc;

use nexus_client::{CustomerGateway, CustomerGatewayError, NexusCustomerGateway};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusCustomerGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusCustomerGateway::new(transport)
}

#[tokio::test]
async fn request_assigned_customer_context_sends_consultant_id_as_a_query_param_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customer/v1/context"))
        .and(query_param("consultant_id", "consultant-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "contexts": [
                {
                    "customer_id": "customer-1",
                    "name": "Acme Corp",
                    "health_status": "green",
                    "relationship_summary": "Healthy, quarterly business review scheduled.",
                    "deep_link": "https://customer.cognitum.one/customers/customer-1"
                },
                {
                    "customer_id": "customer-2",
                    "name": "Beta LLC",
                    "health_status": "red",
                    "relationship_summary": "At risk — escalation in progress.",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_customer_context("consultant-1", None).await.expect("context fetch succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].customer_id, "customer-1");
    assert_eq!(result[0].health_status, "green");
    assert_eq!(result[0].deep_link.as_deref(), Some("https://customer.cognitum.one/customers/customer-1"));
    assert_eq!(result[1].health_status, "red");
    assert_eq!(result[1].deep_link, None);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_assigned_customer_context_sends_an_optional_customer_id_query_param_when_narrowing() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customer/v1/context"))
        .and(query_param("consultant_id", "consultant-1"))
        .and(query_param("customer_id", "customer-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "contexts": [
                {
                    "customer_id": "customer-2",
                    "name": "Beta LLC",
                    "health_status": "red",
                    "relationship_summary": "At risk — escalation in progress.",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway
        .request_assigned_customer_context("consultant-1", Some("customer-2"))
        .await
        .expect("context fetch succeeds");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].customer_id, "customer-2");
}

#[tokio::test]
async fn request_assigned_customer_context_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customer/v1/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "contexts": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_customer_context("consultant-empty", None).await.expect("context fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"contexts": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/customer/v1/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_customer_context("consultant-1", None).await;

    match result {
        Err(CustomerGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/customer/v1/context"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_assigned_customer_context("consultant-1", None).await;

    match result {
        Err(CustomerGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
