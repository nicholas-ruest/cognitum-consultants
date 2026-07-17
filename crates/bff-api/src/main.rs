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
    telemetry::init();

    let prometheus_handle = metrics::install_recorder();

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics::handler))
        .with_state(prometheus_handle)
        .layer(middleware::from_fn(metrics::track))
        .layer(middleware::from_fn(correlation::middleware));

    // TODO(config): read from config crate, see ADR-004
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    tracing::info!("bff-api listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.expect("server error");
}
