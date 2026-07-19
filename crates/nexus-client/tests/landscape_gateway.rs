//! Wiremock-backed tests for the Landscape ACL gateway (`LandscapeGateway`,
//! `NexusLandscapeGateway`) — PROMPT-40, ADR-029.
//!
//! Post-ADR-029 the read is `POST capabilities/landscape.intelligence` and
//! the write `POST capabilities/landscape.observations`, each carrying a
//! `CapabilityRequest` envelope; the gateway unwraps
//! `CapabilityResponse.payload`.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use nexus_client::{FieldObservationSubmission, LandscapeGateway, LandscapeGatewayError, NexusLandscapeGateway};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn gateway_for(mock_server: &MockServer) -> NexusLandscapeGateway {
    let transport =
        Arc::new(nexus_client::ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"));
    NexusLandscapeGateway::new(transport)
}

fn ok(payload: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(serde_json::json!({ "request_id": "req-test", "success": true, "payload": payload }))
}

#[tokio::test]
async fn request_intelligence_digest_parses_a_multi_item_fixture() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.intelligence"))
        .respond_with(ok(serde_json::json!({
            "items": [
                {
                    "intel_id": "intel-1",
                    "topic": "Cloud Migration Trends",
                    "summary": "Enterprises are accelerating multi-cloud adoption.",
                    "published_at": "2026-01-01T00:00:00Z",
                    "deep_link": "https://landscape.cognitum.one/intel/intel-1"
                },
                {
                    "intel_id": "intel-2",
                    "topic": "Regulatory Shifts",
                    "summary": "New data residency requirements in EMEA.",
                    "published_at": "2026-01-02T00:00:00Z",
                    "deep_link": null
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_intelligence_digest().await.expect("digest fetch succeeds");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].intel_id, "intel-1");
    assert_eq!(result[0].topic, "Cloud Migration Trends");
    assert_eq!(result[0].published_at, Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
    assert_eq!(result[0].deep_link.as_deref(), Some("https://landscape.cognitum.one/intel/intel-1"));
    assert_eq!(result[1].intel_id, "intel-2");
    assert_eq!(result[1].deep_link, None);

    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn request_intelligence_digest_handles_an_empty_result() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.intelligence"))
        .respond_with(ok(serde_json::json!({ "items": [] })))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_intelligence_digest().await.expect("digest fetch succeeds");

    assert!(result.is_empty());
}

#[tokio::test]
async fn returns_gateway_error_not_panic_on_malformed_intelligence_digest_payload() {
    let mock_server = MockServer::start().await;
    // Well-formed envelope, but its `payload` is a bare array instead of the
    // expected `{"items": [...]}` object.
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.intelligence"))
        .respond_with(ok(serde_json::json!([])))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let result = gateway.request_intelligence_digest().await;

    match result {
        Err(LandscapeGatewayError::UnexpectedResponseShape(_)) => {}
        other => panic!("expected UnexpectedResponseShape error, got {other:?}"),
    }
}

#[tokio::test]
async fn submit_field_observation_sends_the_submission_in_the_payload() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.observations"))
        .and(body_partial_json(serde_json::json!({
            "capability_id": "landscape.observations",
            "target_repo": "cognitum-landscape",
            "payload": {
                "observation_text": "Client mentioned a competitor's new offering.",
                "related_company_reference": "acme-corp",
                "submitted_by": "consultant-1"
            }
        })))
        .respond_with(ok(serde_json::json!({})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let submission = FieldObservationSubmission {
        observation_text: "Client mentioned a competitor's new offering.".to_owned(),
        related_company_reference: Some("acme-corp".to_owned()),
        submitted_by: "consultant-1".to_owned(),
    };
    let result = gateway.submit_field_observation(submission).await;

    assert!(result.is_ok());
    let received = mock_server.received_requests().await.expect("recording enabled");
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn submit_field_observation_omits_related_company_reference_when_none() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.observations"))
        .and(body_partial_json(serde_json::json!({
            "payload": {
                "observation_text": "General market shift noted, no specific client tied to it.",
                "submitted_by": "consultant-2"
            }
        })))
        .respond_with(ok(serde_json::json!({})))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let submission = FieldObservationSubmission {
        observation_text: "General market shift noted, no specific client tied to it.".to_owned(),
        related_company_reference: None,
        submitted_by: "consultant-2".to_owned(),
    };
    let result = gateway.submit_field_observation(submission).await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn returns_transport_error_on_non_success_status() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/capabilities/landscape.observations"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let gateway = gateway_for(&mock_server);
    let submission = FieldObservationSubmission {
        observation_text: "Some observation.".to_owned(),
        related_company_reference: None,
        submitted_by: "consultant-1".to_owned(),
    };
    let result = gateway.submit_field_observation(submission).await;

    match result {
        Err(LandscapeGatewayError::Transport(_)) => {}
        other => panic!("expected Transport error, got {other:?}"),
    }
}
