mod correlation;
mod metrics;
mod session;
mod telemetry;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::{middleware, Json, Router};
use serde_json::{json, Value};
use session::AppState;

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

    // Fail-fast startup (12-factor): if Postgres is unreachable, refuse to
    // come up rather than serve traffic a persistence-backed route would
    // later fail anyway (ADR-010 — this repo's own aggregates need
    // Postgres as the multi-instance source of truth). No route reads the
    // pool yet (U09 only stands up the connection), but the failure mode
    // for "can't reach the datastore we depend on" should be the same
    // whether or not a route is exercising it yet.
    // Not yet consumed by any route (no persistence-backed handlers exist
    // until U11+); kept alive for the lifetime of `main` so the pool isn't
    // torn down immediately after this fail-fast connectivity check.
    let db_pool = persistence::create_pool(&cfg.database_url).await.unwrap_or_else(|err| {
        panic!("failed to connect to database at {}: {err}", cfg.redacted_database_url())
    });
    tracing::info!(pool_size = db_pool.size(), "database pool created");

    let prometheus_handle = metrics::install_recorder();

    // ADR-008 interim dev-stub session provider (see the `TODO` on the
    // `auth` dependency in Cargo.toml): the only `SessionProvider` this
    // repo has until a real Armor-backed one lands (U11+). Constructed
    // once here — this is where `Config` gets passed to
    // `DevStubSessionProvider::new`, per PROMPT-11 — and shared via
    // `AppState`.
    let dev_session_provider = Arc::new(auth::dev_stub::DevStubSessionProvider::new(&cfg));
    let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();

    let state = AppState {
        db_pool,
        session_provider,
        dev_session_provider,
        // ADR-008: `Secure` in non-local environments. The dev-stub only
        // ever runs with `cfg.is_dev() == true` (it panics otherwise), so
        // this is `false` in practice today — implemented config-driven
        // regardless, since it will matter once a real provider exists.
        secure_cookies: !cfg.is_dev(),
        prometheus_handle,
    };

    // `/api/login/dev` is dev-only in practice (see `session` module docs)
    // but not `#[cfg]`-gated; `/api/session` is the one protected route
    // that exists today, behind `session::protected_router`'s uniformly
    // applied `require_session` middleware.
    let api_router =
        Router::new().route("/login/dev", post(session::login_dev)).merge(session::protected_router(state.clone()));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", api_router)
        .route("/metrics", get(metrics::handler))
        .layer(middleware::from_fn(metrics::track))
        .layer(middleware::from_fn(correlation::middleware))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind to {addr}: {err}"));

    tracing::info!("bff-api listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.expect("server error");
}
