//! Wiremock-backed tests for the Products ACL gateway (`ProductsGateway`,
//! `NexusProductsGateway`) — PROMPT-39.
//!
//! Mirrors `edu_gateway.rs`'s structure: a request-shape assertion for the
//! one outbound call (no `consultant_id` this time — see `products.rs`'s
//! module docs for why), several `ProductReferenceCard` fixture scenarios,
//! and a malformed-response error-not-panic case.

use std::sync::Arc;

use nexus_client::{NexusProductsGateway, ProductsGateway, ProductsGatewayError};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusProductsGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusProductsGateway::new(transport)
}

#[tokio::test]
async fn request_product_catalog_sends_no_query_params_and_parses_the_envelope() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
    assert_eq!(received[0].url.query(), None, "no filters means no query string at all");
}

#[tokio::test]
async fn request_product_catalog_sends_repeated_filter_query_params() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .and(query_param("filter", "cloud"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "cards": [] })))
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
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
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
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "cards": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await.expect("catalog fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_response() {
    let mock_server = MockServer::start().await;
    // A bare array instead of the expected `{"cards": [...]}` envelope.
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
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
async fn returns_unexpected_status_error_on_non_success_response() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/products/v1/catalog"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_product_catalog(None).await;

    match result {
        Err(ProductsGatewayError::UnexpectedStatus { .. }) => {}
        other => panic!("expected UnexpectedStatus error, got {other:?}"),
    }
}
