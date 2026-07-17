//! Prometheus metrics endpoint (ADR-012).
//!
//! Uses the `metrics` crate facade with `metrics-exporter-prometheus`'s
//! Axum-compatible recorder/handle: [`install_recorder`] installs the global
//! recorder at startup, [`handler`] renders the current snapshot in
//! Prometheus text exposition format for `GET /metrics`, and [`track`] is a
//! middleware that records request count and latency per route so
//! `/metrics` carries real data from the moment the server starts (ADR-012's
//! minimum metric set: per-route request count/latency histograms).

use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Installs the global Prometheus metrics recorder and returns the handle
/// used to render `/metrics` responses.
pub fn install_recorder() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus metrics recorder")
}

/// `GET /metrics` handler: renders the current metrics snapshot in
/// Prometheus text exposition format.
pub async fn handler(State(handle): State<PrometheusHandle>) -> String {
    handle.render()
}

/// Middleware recording per-route request count and latency, keyed by
/// method, route template (via [`MatchedPath`], not the raw concrete path,
/// to avoid unbounded label cardinality once path params exist), and status.
pub async fn track(matched_path: Option<MatchedPath>, request: Request, next: Next) -> Response {
    let path = matched_path
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());
    let method = request.method().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let latency_seconds = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let labels = [("method", method), ("path", path), ("status", status)];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels).record(latency_seconds);

    response
}
