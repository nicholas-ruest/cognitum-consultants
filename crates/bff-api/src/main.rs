use axum::{routing::get, Json, Router};
use serde_json::{json, Value};

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/healthz", get(healthz));

    // TODO(config): read from config crate, see ADR-004
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
