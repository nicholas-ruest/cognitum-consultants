mod correlation;
mod dashboard;
mod event_ingestion;
mod health;
mod metrics;
mod notifications_sse;
mod permissions;
mod sales;
mod session;
mod telemetry;

use std::sync::Arc;
use std::time::Duration;

use axum::routing::{get, post};
use axum::{middleware, Router};
use session::AppState;
use tower_http::services::{ServeDir, ServeFile};

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

    // ADR-009/PROMPT-15's Armor ACL gateway, assembled per PROMPT-14's
    // read-call convention (nexus_client::armor module docs):
    // `RetryingTransport` wrapping a `TimeoutTransport` wrapping the base
    // `ReqwestNexusTransport` — `fetch_assertions` is an idempotent query,
    // so it is safe (and, per ADR-016, expected) to retry.
    let armor_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let armor_timeout_transport =
        nexus_client::TimeoutTransport::new(Arc::new(armor_base_transport), nexus_client::DEFAULT_READ_TIMEOUT);
    let armor_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(Arc::new(armor_timeout_transport)));
    let armor_gateway: Arc<dyn nexus_client::ArmorGateway> = Arc::new(nexus_client::NexusArmorGateway::new(armor_transport));
    let permission_cache = Arc::new(permissions::PermissionCache::new(armor_gateway));

    // PROMPT-21/23, ADR-010: the Postgres-backed `DashboardConfiguration`
    // repository, constructed alongside the other repositories/gateways
    // and shared via `AppState` the same way `permission_cache` is.
    let dashboard_repository: Arc<dyn bff_core::DashboardConfigurationRepository> =
        Arc::new(persistence::PgDashboardConfigurationRepository::new(db_pool.clone()));

    // Sales ACL gateway (ADR-016, PROMPT-24/25): one shared base transport,
    // decorated into *two* separate `NexusSalesGateway` instances — one
    // per differing retry-safety profile. See `sales` module docs for the
    // full decision writeup: `check_account_claim` is a retry-safe,
    // user-blocking read (write-timeout budget + `RetryingTransport`);
    // `request_collaboration`/`submit_referral` are non-idempotent
    // commands (write-timeout budget, no retry wrapper).
    let sales_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let sales_base_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(sales_base_transport);

    let sales_query_timeout_transport =
        Arc::new(nexus_client::TimeoutTransport::new(sales_base_transport.clone(), nexus_client::DEFAULT_WRITE_TIMEOUT));
    let sales_query_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(sales_query_timeout_transport));
    let sales_query_gateway: Arc<dyn nexus_client::SalesGateway> =
        Arc::new(nexus_client::NexusSalesGateway::new(sales_query_transport));

    let sales_command_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::TimeoutTransport::new(sales_base_transport, nexus_client::DEFAULT_WRITE_TIMEOUT));
    let sales_command_gateway: Arc<dyn nexus_client::SalesGateway> =
        Arc::new(nexus_client::NexusSalesGateway::new(sales_command_transport));

    // PROMPT-29/30, ADR-010: the Postgres-backed `NotificationItem`/
    // `ActionQueueEntry` repositories, and the in-process `EventBus` the
    // polling loop below publishes freshly-ingested aggregates into.
    // Constructed here (not inside the polling task) so they can also be
    // shared via `AppState` — PROMPT-31's SSE endpoint subscribes to the
    // same `event_bus` instance.
    let notification_repository: Arc<dyn bff_core::NotificationRepository> =
        Arc::new(persistence::PgNotificationRepository::new(db_pool.clone()));
    let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
        Arc::new(persistence::PgActionQueueRepository::new(db_pool.clone()));
    let event_bus = Arc::new(bff_core::EventBus::default());

    // Events-poll transport (PROMPT-30, ADR-011): `events/v1/poll` is a
    // read (idempotent query, no side effect), so per ADR-016 it gets the
    // read-timeout budget wrapped in `RetryingTransport`, same convention
    // as `armor_transport` above and `sales_query_transport`'s read-side
    // counterpart.
    let events_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let events_timeout_transport =
        nexus_client::TimeoutTransport::new(Arc::new(events_base_transport), nexus_client::DEFAULT_READ_TIMEOUT);
    let events_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(Arc::new(events_timeout_transport)));

    // Background polling task (PROMPT-30, ADR-011's "Nexus → BFF ingestion
    // via polling" decision): runs for the lifetime of the process, never
    // awaited here. Graceful shutdown (ADR-014) drains in-flight HTTP
    // requests via `with_graceful_shutdown` below; this task is simply
    // dropped when the process exits, which is acceptable since a poll
    // cycle has no partially-committed state of its own (`ingest_events`'s
    // per-event saves are already individually atomic).
    tokio::spawn(event_ingestion::run_polling_loop(
        events_transport,
        notification_repository.clone(),
        action_queue_repository.clone(),
        event_bus.clone(),
        Duration::from_secs(cfg.event_poll_interval_seconds),
    ));

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
        permission_cache,
        dashboard_repository,
        sales_query_gateway,
        sales_command_gateway,
        notification_repository,
        action_queue_repository,
        event_bus,
    };

    // `/api/login/dev` is dev-only in practice (see `session` module docs)
    // but not `#[cfg]`-gated; `/api/session` is the one protected route
    // that exists today, behind `session::protected_router`'s uniformly
    // applied `require_session` middleware. `permissions::diagnostic_router`
    // adds one more, temporary, protected route (see its doc comment) that
    // proves the ADR-009 `is_permitted` + `403` short-circuit mechanism.
    // `dashboard::dashboard_router` adds the real `GET`/`PUT /api/dashboard`
    // routes (PROMPT-23). `sales::sales_router` adds the real
    // `POST /api/sales/*` routes (PROMPT-25).
    // `notifications_sse::notifications_router` adds `GET
    // /api/notifications/stream` (PROMPT-31, ADR-011): the SSE push
    // endpoint consultants' browsers hold open, subscribed to the same
    // `event_bus` instance the polling task above publishes into.
    let api_router = Router::new()
        .route("/login/dev", post(session::login_dev))
        .merge(session::protected_router(state.clone()))
        .merge(permissions::diagnostic_router(state.clone()))
        .merge(dashboard::dashboard_router(state.clone()))
        .merge(sales::sales_router(state.clone()))
        .merge(notifications_sse::notifications_router(state.clone()));

    let mut app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .nest("/api", api_router)
        .route("/metrics", get(metrics::handler));

    // ADR-014: single image serves both `/api/*` and the SPA. Mounted as a
    // `fallback_service` — added *before* the `.layer(...)` calls below so
    // metrics/correlation middleware wrap it the same as every other
    // route — so explicit routes above (`/healthz`, `/readyz`, `/api/*`,
    // `/metrics`) always win; the static-file service only ever answers a
    // request nothing else matched. Only mounted when `STATIC_DIR` is set
    // *and* exists on disk: the existing test suite (and any environment
    // with no built frontend) never sets it, so this is skipped rather than
    // required, per this unit's acceptance criteria.
    match cfg.static_dir.as_deref().filter(|dir| dir.is_dir()) {
        Some(static_dir) => {
            tracing::info!(dir = %static_dir.display(), "serving SPA static assets");
            let index_html = static_dir.join("index.html");
            let serve_dir = ServeDir::new(static_dir).not_found_service(ServeFile::new(index_html));
            app = app.fallback_service(serve_dir);
        }
        None => {
            tracing::info!(
                static_dir = ?cfg.static_dir,
                "STATIC_DIR not configured or missing; skipping static file serving"
            );
        }
    }

    let app = app
        .layer(middleware::from_fn(metrics::track))
        .layer(middleware::from_fn(correlation::middleware))
        .with_state(state);

    // Bind on all interfaces, not just loopback: a containerized deployment
    // (ADR-014) needs this process reachable from outside its own network
    // namespace (`docker run -p`, an orchestrator's Service, etc.), and
    // `127.0.0.1` would silently make the container unreachable despite the
    // process itself running fine.
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind to {addr}: {err}"));

    tracing::info!("bff-api listening on {}", listener.local_addr().unwrap());

    // ADR-014 graceful shutdown: stop accepting new connections and let
    // in-flight requests (including open SSE connections, per ADR-011) drain
    // on `SIGTERM`/`SIGINT` instead of the process being killed mid-request.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    tracing::info!("bff-api shut down");
}

/// Resolves once `SIGTERM` or `SIGINT` (Ctrl+C, for local-dev parity) is
/// received, whichever comes first — passed to
/// `axum::serve(...).with_graceful_shutdown` so the server drains in-flight
/// requests instead of being killed mid-request (ADR-014).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received Ctrl+C, starting graceful shutdown"),
        _ = terminate => tracing::info!("received SIGTERM, starting graceful shutdown"),
    }
}
