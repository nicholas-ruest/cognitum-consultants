use crate::transport::{NexusRequest, NexusResponse, NexusTransport, NexusTransportError};
use opentelemetry::trace::TraceContextExt;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub struct ReqwestNexusTransport {
    client: reqwest::Client,
    base_url: reqwest::Url,
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
        Ok(Self { client, base_url: url })
    }
}

#[async_trait::async_trait]
impl NexusTransport for ReqwestNexusTransport {
    async fn send(&self, request: NexusRequest) -> Result<NexusResponse, NexusTransportError> {
        let url = self.base_url.join(&request.path)
            .map_err(|e| NexusTransportError::InvalidUrl { path: request.path.clone(), reason: e.to_string() })?;

        let mut builder = self.client.request(request.method, url).headers(request.headers);

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
