mod correlation;
mod metrics;
mod telemetry;

use axum::{middleware, routing::get, Json, Router};
use serde_json::{json, Value};

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

#[tokio::main]
async fn main() {
    // Loaded before telemetry::init() — Config::load() is plain env-var
    // reads with no tracing dependency, so there's no ordering hazard.
    let cfg = config::Config::load();

    telemetry::init();

    // database_url may carry credentials in a real deployment (ADR-014);
    // log a redacted form rather than the raw value.
    tracing::info!(
        port = cfg.port,
        database_url = %cfg.redacted_database_url(),
        log_level = %cfg.log_level,
        nexus_endpoint_url = %cfg.nexus_endpoint_url,
        "config loaded"
    );

    let prometheus_handle = metrics::install_recorder();

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics::handler))
        .with_state(prometheus_handle)
        .layer(middleware::from_fn(metrics::track))
        .layer(middleware::from_fn(correlation::middleware));

    let addr = format!("127.0.0.1:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind to {addr}: {err}"));

    tracing::info!("bff-api listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.expect("server error");
}
