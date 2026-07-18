//! `GET /healthz` (liveness) and `GET /readyz` (readiness), per ADR-014's
//! "health-check endpoint(s) suitable for liveness/readiness probes"
//! requirement.
//!
//! **Liveness** (`/healthz`): "is the process up and able to answer HTTP at
//! all". Always `200` once the listener is bound — deliberately does not
//! touch Postgres or any other dependency, so an orchestrator's liveness
//! probe (which typically restarts the container on repeated failure) never
//! restarts a healthy process just because a downstream dependency is
//! temporarily degraded.
//!
//! **Readiness** (`/readyz`): "is this instance ready to receive traffic
//! right now". Runs [`persistence::check_connectivity`] (a cheap `SELECT 1`)
//! against the shared pool, bounded by [`READINESS_TIMEOUT`] so a hung
//! database doesn't hang the probe itself. `200` with
//! `{"status":"ok","checks":{"database":"ok"}}` when reachable; `503` with
//! an error detail otherwise — the shape an orchestrator's readiness probe
//! (which typically just stops routing traffic, not restarts) expects to
//! poll on a short interval.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use tokio::time::timeout;

use crate::session::AppState;

/// Worst-case time `GET /readyz` waits on the database check before
/// reporting not-ready. Short relative to typical orchestrator probe
/// intervals (seconds, not the default 30s `sqlx` connection-acquire
/// timeout) so a stalled database degrades the probe promptly rather than
/// piling up slow requests.
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// `GET /healthz`: liveness. Always `200` — see module docs.
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// `GET /readyz`: readiness. Delegates to [`readiness_status`], which is
/// unit-tested directly against a real pool without needing a full
/// [`AppState`].
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    readiness_status(&state.db_pool).await
}

/// Core readiness logic, factored out of [`readyz`] so it can be exercised
/// in tests against a bare `persistence::Pool` (success and unreachable
/// cases) without constructing the rest of [`AppState`]'s gateways/caches,
/// which readiness doesn't otherwise touch.
async fn readiness_status(pool: &persistence::Pool) -> (StatusCode, Json<Value>) {
    match timeout(READINESS_TIMEOUT, persistence::check_connectivity(pool)).await {
        Ok(Ok(())) => {
            (StatusCode::OK, Json(json!({ "status": "ok", "checks": { "database": "ok" } })))
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "readiness check: database unreachable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "status": "error",
                    "checks": { "database": format!("error: {err}") }
                })),
            )
        }
        Err(_) => {
            tracing::warn!(timeout_secs = READINESS_TIMEOUT.as_secs(), "readiness check: database check timed out");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "error", "checks": { "database": "timeout" } })),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::PgPoolOptions;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    #[tokio::test]
    async fn healthz_reports_ok() {
        let Json(body) = healthz().await;

        assert_eq!(body, json!({ "status": "ok" }));
    }

    #[tokio::test]
    async fn readyz_reports_ok_when_database_is_reachable() {
        let container =
            Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = persistence::create_pool(&database_url).await.expect("failed to connect");

        let (status, Json(body)) = readiness_status(&pool).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "status": "ok", "checks": { "database": "ok" } }));
    }

    #[tokio::test]
    async fn readyz_reports_service_unavailable_when_database_is_unreachable() {
        // `connect_lazy` against a loopback port nothing listens on: fast,
        // deterministic `ECONNREFUSED`, no live Postgres/testcontainer
        // teardown timing to race (mirrors `persistence::lib`'s own
        // `check_connectivity_fails_when_postgres_is_unreachable` test).
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/postgres")
            .expect("connect_lazy should not eagerly connect");

        let (status, Json(body)) = readiness_status(&pool).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], json!("error"));
        // Depending on the environment's network stack, an unreachable
        // loopback port surfaces either as an immediate connection error or
        // (e.g. some sandboxed/firewalled setups) as a stall that trips
        // `READINESS_TIMEOUT` first — both are valid "not ready" outcomes,
        // so accept either detail rather than assuming one specific error
        // shape.
        let database_detail = body["checks"]["database"].as_str().unwrap();
        assert!(
            database_detail.starts_with("error:") || database_detail == "timeout",
            "unexpected database check detail: {database_detail:?}"
        );
    }
}
