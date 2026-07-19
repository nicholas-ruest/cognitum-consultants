//! Wiremock-backed tests for the Products ACL gateway (`ProductsGateway`,
//! `NexusProductsGateway`) — PROMPT-39, ADR-029.
//!
//! Post-ADR-029 every call is `POST capabilities/products.catalog` carrying
//! a `CapabilityRequest` envelope; the gateway unwraps
//! `CapabilityResponse.payload`, still expecting a `{"cards": [...]}` object.
//! Filters, when present, travel in the payload (`{"filters": [...]}`) rather
//! than as repeated query params.

use std::sync::Arc;

use nexus_client::{NexusProductsGateway, ProductsGateway, ProductsGatewayError};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusProductsGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusProductsGateway::new(transport)
}

fn ok(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({ "request_id": "req-test", "success": true, "payload": payload }))
}

#[tokio::test]
async fn request_product_catalog_sends_an_empty_payload_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "products.catalog",
            "target_repo": "cognitum-products",
            "payload": {}
        })))
        .respond_with(ok(serde_json::json!({
            "cards": [
                {
                    "product_id": "product-1",
                    "name": "Cloud Migration Accelerator",
                    "packaging_summary": "4-week fixed-scope engagement",
                    "pricing_guidance": "Starting at $50,000",
                    "demo_assets": ["https://products.cognitum.one/demos/product-1.mp4"]
                },
                {
                    "product_id": "product-2",
                    "name": "Security Posture Review",
                    "packaging_summary": "2-week assessment",
                    "pricing_guidance": "Starting at $20,000",
                    "demo_assets": []
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await.expect("catalog fetch succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].product_id, "product-1");
    assert_eq!(result[0].name, "Cloud Migration Accelerator");
    assert_eq!(result[0].demo_assets, vec!["https://products.cognitum.one/demos/product-1.mp4".to_string()]);
    assert_eq!(result[1].demo_assets, Vec::<String>::new());

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_product_catalog_sends_filters_in_the_payload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .and(body_partial_json(serde_json::json!({ "payload": { "filters": ["cloud"] } })))
        .respond_with(ok(serde_json::json!({ "cards": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let filters = vec!["cloud".to_owned()];
    let result = gateway.request_product_catalog(Some(&filters)).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn request_product_catalog_defaults_missing_demo_assets_to_an_empty_list() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .respond_with(ok(serde_json::json!({
            "cards": [
                {
                    "product_id": "product-3",
                    "name": "Data Platform Modernization",
                    "packaging_summary": "8-week phased rollout",
                    "pricing_guidance": "Contact for quote"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await.expect("catalog fetch succeeds");

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].demo_assets, Vec::<String>::new());
}

#[tokio::test]
async fn request_product_catalog_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .respond_with(ok(serde_json::json!({ "cards": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_payload() {
    let mock_server = MockServer::start().await;
    // Well-formed envelope, but its `payload` is a bare array instead of the
    // expected `{"cards": [...]}` object.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await;

    match result {
        Err(ProductsGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn returns_transport_error_on_non_success_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/products.catalog"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await;

    match result {
        Err(ProductsGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}
