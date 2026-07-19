use std::time::Duration;

use crate::transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};
use opentelemetry::trace::TraceContextExt;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// nexus-server's own configured `NEXUS_AUTH_AUDIENCE`/`NEXUS_IAM_AUDIENCE`
/// (verified live against the deployed service, ADR-029): every capability
/// call must carry a Google-signed identity token with this exact `aud`
/// claim, or nexus-server rejects it — `{"error":"missing bearer token"}`
/// with none at all, `{"error":"malformed token: ..."}` for anything that
/// isn't a real Google-issued JWT for this audience.
const NEXUS_IDENTITY_TOKEN_AUDIENCE: &str = "nexus-api";

/// GCP's metadata server, reachable without any credential configuration
/// from *inside* a Compute Engine/Cloud Run/GKE workload — it hands back an
/// identity token for whatever audience is asked, signed for the
/// workload's attached service account. This is the standard Cloud
/// Run-to-Cloud-Run service auth pattern (no static secret to provision or
/// rotate), and exactly what nexus-server's own `NEXUS_IAM_AUDIENCE`
/// config expects a caller to present.
const METADATA_IDENTITY_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity";

/// Bounds the metadata-server round trip so a request off GCP (local dev,
/// the e2e suite, CI — none of which have a metadata server to reach)
/// fails this step fast rather than hanging on a DNS/connect timeout that
/// would otherwise eat into the outer call's own timeout budget.
const METADATA_TOKEN_TIMEOUT: Duration = Duration::from_millis(750);

pub struct ReqwestNexusTransport {
    client: reqwest::Client,
    base_url: reqwest::Url,
    /// A second, short-timeout client dedicated to the metadata-server
    /// identity-token fetch — kept separate from `client` (whose timeout is
    /// tuned per gateway by `TimeoutTransport`, not this transport) so this
    /// internal step never borrows a caller's own request budget.
    metadata_client: reqwest::Client,
}

impl ReqwestNexusTransport {
    pub fn new(base_url: &str) -> Result<Self, NexusTransportError> {
        Self::with_client(reqwest::Client::new(), base_url)
    }

    /// Exposed for tests: inject a client pointed at a `wiremock::MockServer`.
    pub fn with_client(client: reqwest::Client, base_url: &str) -> Result<Self, NexusTransportError> {
        let mut url = reqwest::Url::parse(base_url)
            .map_err(|e| NexusTransportError::InvalidUrl { path: base_url.to_owned(), reason: e.to_string() })?;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        let metadata_client = reqwest::Client::builder()
            .timeout(METADATA_TOKEN_TIMEOUT)
            .build()
            .expect("reqwest client with only a timeout configured always builds");
        Ok(Self { client, base_url: url, metadata_client })
    }

    /// Fetches a fresh Google-signed identity token (`aud: nexus-api`) from
    /// the GCP metadata server, attached as `Authorization: Bearer <token>`
    /// on every outbound Nexus call. Fetched per-call, not cached: the
    /// metadata server is a local, in-VM endpoint (not a real network
    /// hop) — measured well under the timeout above — so the simplicity of
    /// not tracking a token's `exp` claim/refresh window outweighs the
    /// saved round trip.
    ///
    /// Returns `Ok(None)`, not an error, when the metadata server can't be
    /// reached — the expected case everywhere this code runs *off* GCP
    /// (local dev, the e2e suite, CI), where `NEXUS_ENDPOINT_URL` points at
    /// the mock Nexus server, which never checks this header. Only a
    /// present-but-rejected token should ever surface as a real call
    /// failure, and that happens naturally: nexus-server itself returns
    /// `401` for a missing/invalid token, which `CapabilityCaller` already
    /// turns into `NexusTransportError::UnexpectedStatus`.
    async fn fetch_identity_token(&self) -> Option<String> {
        let response = self
            .metadata_client
            .get(METADATA_IDENTITY_TOKEN_URL)
            .header("Metadata-Flavor", "Google")
            .query(&[("audience", NEXUS_IDENTITY_TOKEN_AUDIENCE)])
            .send()
            .await
            .inspect_err(|err| tracing::debug!(error = %err, "GCP metadata server unreachable; not on GCP (expected in local dev/e2e/CI)"))
            .ok()?;

        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "GCP metadata server rejected identity-token request");
            return None;
        }

        response
            .text()
            .await
            .inspect_err(|err| tracing::warn!(error = %err, "failed to read identity token from metadata server response"))
            .ok()
    }
}

#[async_trait::async_trait]
impl NexusTransport for ReqwestNexusTransport {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError> {
        let url = self.base_url.join(&request.path)
            .map_err(|e| NexusTransportError::InvalidUrl { path: request.path.clone(), reason: e.to_string() })?;

        let mut builder = self.client.request(request.method, url).headers(request.headers);

        if let Some(token) = self.fetch_identity_token().await
            && let Ok(header_value) = HeaderValue::from_str(&format!("Bearer {token}"))
        {
            builder = builder.header(AUTHORIZATION, header_value);
        }

        if let Some(correlation_id) = correlation_context::current() {
            builder = builder.header(correlation_context::CORRELATION_ID_HEADER_NAME, correlation_id);
        }

        let otel_ctx = tracing::Span::current().context();
        let otel_span = otel_ctx.span();
        let span_ctx = otel_span.span_context();
        if span_ctx.is_valid() {
            let traceparent = format!(
                "00-{}-{}-{:02x}",
                span_ctx.trace_id(),
                span_ctx.span_id(),
                span_ctx.trace_flags().to_u8(),
            );
            builder = builder.header("traceparent", traceparent);
        }

        if let Some(body) = &request.body {
            builder = builder.json(body);
        }

        let response = builder.send().await.map_err(NexusTransportError::Request)?;
        let status = response.status();
        let headers = response.headers().clone();
        let body: serde_json::Value = if response.content_length() == Some(0) {
            serde_json::Value::Null
        } else {
            let bytes = response.bytes().await.map_err(NexusTransportError::DecodeResponseBytes)?;
            if bytes.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&bytes).map_err(NexusTransportError::ParseResponseJson)?
            }
        };

        Ok(NexusResponse { status, headers, body })
    }
}
