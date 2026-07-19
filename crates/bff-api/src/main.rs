mod capacity;
mod commit;
mod correlation;
mod customer;
mod dashboard;
mod edu;
mod event_ingestion;
mod event_notify_bridge;
mod execution;
mod health;
mod landscape;
mod legal;
mod metrics;
mod notifications;
mod notifications_sse;
mod permissions;
mod products;
mod sales;
mod session;
mod telemetry;
mod workflow_sessions;

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
    // In dev, the in-memory dev-stub is the only provider (real login has
    // no credential to check anyway, per ADR-008). Outside dev,
    // `DevStubSessionProvider::new` panics if constructed at all, so the
    // real, Firebase-backed provider (`auth::firebase`) is used instead —
    // it persists sessions in Postgres (`db_pool`) rather than in-memory,
    // since a Cloud Run instance can scale to zero between requests.
    // The explicit tuple type is required (not just clarity) -- `session_provider`
    // must unify to the `Arc<dyn SessionProvider>` trait object across both
    // branches below, which needs a type hint since each branch's concrete
    // provider type differs. `#[allow]` rather than factoring into named
    // type aliases: this is the only call site, so an alias would just add
    // a layer of indirection for a tuple destructured three lines down.
    #[allow(clippy::type_complexity)]
    let (dev_session_provider, firebase_session_provider, session_provider): (
        Option<Arc<auth::dev_stub::DevStubSessionProvider>>,
        Option<Arc<auth::firebase::FirebaseSessionProvider>>,
        Arc<dyn auth::SessionProvider>,
    ) = if cfg.is_dev() {
        let dev_provider = Arc::new(auth::dev_stub::DevStubSessionProvider::new(&cfg));
        (Some(dev_provider.clone()), None, dev_provider)
    } else {
        let project_id = cfg
            .firebase_project_id
            .clone()
            .unwrap_or_else(|| panic!("FIREBASE_PROJECT_ID must be set outside a dev environment"));
        let firebase_provider = Arc::new(auth::firebase::FirebaseSessionProvider::new(db_pool.clone(), project_id));
        (None, Some(firebase_provider.clone()), firebase_provider)
    };

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

    // Commit ACL gateway (ADR-016, PROMPT-34): same shared-base-transport,
    // two-instances split as Sales above — see `commit`/`nexus_client::commit`
    // module docs for the full decision writeup. `list_proposals` gets the
    // read timeout + retry (idempotent, page-load-ish); `create_proposal`/
    // `request_proposal_action` get the write timeout, no retry
    // (non-idempotent commands).
    let commit_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let commit_base_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(commit_base_transport);

    let commit_query_timeout_transport =
        Arc::new(nexus_client::TimeoutTransport::new(commit_base_transport.clone(), nexus_client::DEFAULT_READ_TIMEOUT));
    let commit_query_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(commit_query_timeout_transport));
    let commit_query_gateway: Arc<dyn nexus_client::CommitGateway> =
        Arc::new(nexus_client::NexusCommitGateway::new(commit_query_transport));

    let commit_command_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::TimeoutTransport::new(commit_base_transport, nexus_client::DEFAULT_WRITE_TIMEOUT));
    let commit_command_gateway: Arc<dyn nexus_client::CommitGateway> =
        Arc::new(nexus_client::NexusCommitGateway::new(commit_command_transport));

    // Edu ACL gateway (ADR-016, PROMPT-35): a single instance, unlike
    // Sales/Commit's two-instance split — Edu has no side-effecting
    // outbound command to isolate a retry-safe read from (see
    // `edu`/`nexus_client::edu` module docs). `request_learning_catalog`
    // gets the extended read timeout (PROMPT-35's explicit "longer
    // timeout") wrapped in `RetryingTransport`, since it is an idempotent
    // query with no synchronous UI-blocking call sharing this gateway.
    let edu_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let edu_timeout_transport = nexus_client::TimeoutTransport::new(
        Arc::new(edu_base_transport),
        nexus_client::DEFAULT_EXTENDED_READ_TIMEOUT,
    );
    let edu_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(Arc::new(edu_timeout_transport)));
    let edu_gateway: Arc<dyn nexus_client::EduGateway> = Arc::new(nexus_client::NexusEduGateway::new(edu_transport));

    // Capacity ACL gateway (ADR-016, PROMPT-36): same shared-base-transport,
    // two-instances split as Sales/Commit above — see
    // `capacity`/`nexus_client::capacity` module docs for the full decision
    // writeup. `get_own_profile` gets the read timeout + retry (idempotent,
    // page-load-ish); `update_own_profile` gets the write timeout, no retry
    // (a non-idempotent command).
    let capacity_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let capacity_base_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(capacity_base_transport);

    let capacity_query_timeout_transport = Arc::new(nexus_client::TimeoutTransport::new(
        capacity_base_transport.clone(),
        nexus_client::DEFAULT_READ_TIMEOUT,
    ));
    let capacity_query_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(capacity_query_timeout_transport));
    let capacity_query_gateway: Arc<dyn nexus_client::CapacityGateway> =
        Arc::new(nexus_client::NexusCapacityGateway::new(capacity_query_transport));

    let capacity_command_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(
        nexus_client::TimeoutTransport::new(capacity_base_transport, nexus_client::DEFAULT_WRITE_TIMEOUT),
    );
    let capacity_command_gateway: Arc<dyn nexus_client::CapacityGateway> =
        Arc::new(nexus_client::NexusCapacityGateway::new(capacity_command_transport));

    // Customer ACL gateway (ADR-016, PROMPT-37): a single instance, unlike
    // Sales/Commit/Capacity's two-instance split — Customer has no
    // side-effecting outbound command to isolate a retry-safe read from
    // (see `customer`/`nexus_client::customer` module docs), the same shape
    // as Edu above. `request_assigned_customer_context` gets the read
    // timeout wrapped in `RetryingTransport`, since it is an idempotent
    // query with no synchronous UI-blocking call sharing this gateway.
    let customer_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let customer_timeout_transport =
        nexus_client::TimeoutTransport::new(Arc::new(customer_base_transport), nexus_client::DEFAULT_READ_TIMEOUT);
    let customer_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(Arc::new(customer_timeout_transport)));
    let customer_gateway: Arc<dyn nexus_client::CustomerGateway> =
        Arc::new(nexus_client::NexusCustomerGateway::new(customer_transport));

    // Execution ACL gateway (ADR-016, PROMPT-38): same shared-base-transport,
    // two-instances split as Sales/Commit/Capacity above — see
    // `execution`/`nexus_client::execution` module docs for the full
    // decision writeup. `request_assigned_engagements` gets the read timeout
    // + retry (idempotent, page-load-ish); `confirm_task_completion` gets
    // the write timeout, no retry (a non-idempotent command).
    let execution_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let execution_base_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(execution_base_transport);

    let execution_query_timeout_transport = Arc::new(nexus_client::TimeoutTransport::new(
        execution_base_transport.clone(),
        nexus_client::DEFAULT_READ_TIMEOUT,
    ));
    let execution_query_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(execution_query_timeout_transport));
    let execution_query_gateway: Arc<dyn nexus_client::ExecutionGateway> =
        Arc::new(nexus_client::NexusExecutionGateway::new(execution_query_transport));

    let execution_command_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(
        nexus_client::TimeoutTransport::new(execution_base_transport, nexus_client::DEFAULT_WRITE_TIMEOUT),
    );
    let execution_command_gateway: Arc<dyn nexus_client::ExecutionGateway> =
        Arc::new(nexus_client::NexusExecutionGateway::new(execution_command_transport));

    // Products ACL gateway (ADR-016, PROMPT-39): a single instance, unlike
    // Sales/Commit/Capacity/Execution's two-instance split — Products has no
    // side-effecting outbound command to isolate a retry-safe read from (see
    // `products`/`nexus_client::products` module docs), the same shape as
    // Edu/Customer above. Unlike every other gateway constructed so far,
    // `request_product_catalog` gets this repo's **longest** timeout
    // (`DEFAULT_MAX_READ_TIMEOUT`) and **most aggressive** retry budget
    // (`AGGRESSIVE_MAX_RETRIES`, not `with_default_retries`) — per this
    // unit's own acceptance criteria and `nexus_client::products`'s module
    // docs, Products is this repo's single most cacheable, least
    // latency-sensitive read.
    let products_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let products_timeout_transport = nexus_client::TimeoutTransport::new(
        Arc::new(products_base_transport),
        nexus_client::DEFAULT_MAX_READ_TIMEOUT,
    );
    let products_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(nexus_client::RetryingTransport::new(
        Arc::new(products_timeout_transport),
        nexus_client::AGGRESSIVE_MAX_RETRIES,
    ));
    let products_gateway: Arc<dyn nexus_client::ProductsGateway> =
        Arc::new(nexus_client::NexusProductsGateway::new(products_transport));

    // Landscape ACL gateway (ADR-016, PROMPT-40): same shared-base-transport,
    // two-instances split as Sales/Commit/Capacity/Execution above — see
    // `landscape`/`nexus_client::landscape` module docs for the full decision
    // writeup. `request_intelligence_digest` gets the read timeout + retry
    // (idempotent, page-load-ish, low-priority per this unit's own inbound-
    // event handling); `submit_field_observation` gets the write timeout, no
    // retry (a non-idempotent command).
    let landscape_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let landscape_base_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(landscape_base_transport);

    let landscape_query_timeout_transport = Arc::new(nexus_client::TimeoutTransport::new(
        landscape_base_transport.clone(),
        nexus_client::DEFAULT_READ_TIMEOUT,
    ));
    let landscape_query_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(landscape_query_timeout_transport));
    let landscape_query_gateway: Arc<dyn nexus_client::LandscapeGateway> =
        Arc::new(nexus_client::NexusLandscapeGateway::new(landscape_query_transport));

    let landscape_command_transport: Arc<dyn nexus_client::NexusTransport> = Arc::new(
        nexus_client::TimeoutTransport::new(landscape_base_transport, nexus_client::DEFAULT_WRITE_TIMEOUT),
    );
    let landscape_command_gateway: Arc<dyn nexus_client::LandscapeGateway> =
        Arc::new(nexus_client::NexusLandscapeGateway::new(landscape_command_transport));

    // Legal ACL gateway (ADR-007, PROMPT-41): a single instance, unlike
    // Sales/Commit/Capacity/Execution/Landscape's two-instance split — Legal
    // has no side-effecting outbound command to isolate a retry-safe read
    // from (see `legal`/`nexus_client::legal` module docs), the same shape
    // as Edu/Customer/Products above. `request_approved_clauses` gets the
    // read timeout wrapped in `RetryingTransport`, since it is an idempotent
    // query with no synchronous UI-blocking call sharing this gateway.
    let legal_base_transport = nexus_client::ReqwestNexusTransport::new(&cfg.nexus_endpoint_url)
        .unwrap_or_else(|err| panic!("invalid nexus_endpoint_url {:?}: {err}", cfg.nexus_endpoint_url));
    let legal_timeout_transport =
        nexus_client::TimeoutTransport::new(Arc::new(legal_base_transport), nexus_client::DEFAULT_READ_TIMEOUT);
    let legal_transport: Arc<dyn nexus_client::NexusTransport> =
        Arc::new(nexus_client::RetryingTransport::with_default_retries(Arc::new(legal_timeout_transport)));
    let legal_gateway: Arc<dyn nexus_client::LegalGateway> = Arc::new(nexus_client::NexusLegalGateway::new(legal_transport));

    // PROMPT-22/34, ADR-010: the Postgres-backed `CrossCapabilityWorkflowSession`
    // repository — PROMPT-22 built this, but no BFF route consumed it until
    // this unit (`workflow_sessions::workflow_sessions_router`,
    // `commit::create_proposal`'s hand-off lookup).
    let workflow_session_repository: Arc<dyn bff_core::WorkflowSessionRepository> =
        Arc::new(persistence::PgWorkflowSessionRepository::new(db_pool.clone()));

    // PROMPT-29/30, ADR-010: the Postgres-backed `NotificationItem`/
    // `ActionQueueEntry` repositories, and the in-process `EventBus`
    // PROMPT-31's SSE endpoint subscribes to. Constructed here (not inside
    // either background task below) so they can also be shared via
    // `AppState`.
    //
    // PROMPT-32/ADR-014: as of this unit, `event_bus` is no longer fed
    // directly by ingestion — it's fed by `event_notify_bridge`'s Postgres
    // `LISTEN` loop below, which is what makes this work across multiple
    // `bff-api` instances (see that module's docs for the full pipeline).
    let notification_repository: Arc<dyn bff_core::NotificationRepository> =
        Arc::new(persistence::PgNotificationRepository::new(db_pool.clone()));
    let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
        Arc::new(persistence::PgActionQueueRepository::new(db_pool.clone()));
    let event_bus = Arc::new(bff_core::EventBus::default());

    // PROMPT-32, ADR-014's recommended cross-instance SSE fan-out: the
    // ingestion polling loop below publishes through this Postgres `NOTIFY`
    // publisher instead of directly into `event_bus` — see
    // `event_notify_bridge`'s module docs for why (every instance,
    // including this one, learns about a fresh ingestion uniformly via its
    // own `LISTEN` loop rather than ingestion reaching only this process's
    // own subscribers directly).
    let event_notify_publisher: Arc<dyn bff_core::EventPublisher> = Arc::new(
        persistence::PgNotifyPublisher::new(db_pool.clone(), bff_core::EVENT_NOTIFY_CHANNEL),
    );

    // Events-poll transport (PROMPT-30, ADR-011): `api/v1/events/poll`
    // (ADR-030 §3) is a read (idempotent query, no side effect), so per
    // ADR-016 it gets the read-timeout budget wrapped in `RetryingTransport`,
    // same convention as `armor_transport` above and `sales_query_transport`'s
    // read-side counterpart.
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
        workflow_session_repository.clone(),
        event_notify_publisher,
        Duration::from_secs(cfg.event_poll_interval_seconds),
    ));

    // PROMPT-32, ADR-014: the other half of the bridge — a dedicated
    // Postgres `LISTEN` connection that republishes every NOTIFY (from any
    // instance's ingestion, including this one's) into this instance's own
    // `event_bus`, which the SSE endpoint below subscribes to unchanged
    // from PROMPT-31. See `event_notify_bridge`'s module docs for the full
    // two-hop delivery path.
    tokio::spawn(event_notify_bridge::run_listen_bridge(
        cfg.database_url.clone(),
        notification_repository.clone(),
        action_queue_repository.clone(),
        event_bus.clone(),
    ));

    let state = AppState {
        db_pool,
        session_provider,
        dev_session_provider,
        firebase_session_provider,
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
        commit_query_gateway,
        commit_command_gateway,
        edu_gateway,
        capacity_query_gateway,
        capacity_command_gateway,
        customer_gateway,
        execution_query_gateway,
        execution_command_gateway,
        products_gateway,
        landscape_query_gateway,
        landscape_command_gateway,
        legal_gateway,
        workflow_session_repository,
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
    // `event_bus` instance the LISTEN bridge task above republishes into
    // (PROMPT-32, ADR-014) — the polling task no longer publishes to it
    // directly, see the comments above both background tasks.
    // `notifications::notifications_write_router` adds the write-side REST
    // endpoints ADR-011 calls out as ordinary request/response calls
    // (PROMPT-33): `GET /api/notifications`, `PATCH
    // /api/notifications/:id/read`, `GET /api/action-queue`, `POST
    // /api/action-queue/:id/start` — see that module's docs for why there is
    // deliberately no `.../complete` route.
    // `edu::edu_router` adds `GET /api/edu/catalog` (PROMPT-35).
    // `capacity::capacity_router` adds `GET`/`PATCH /api/capacity/profile`
    // (PROMPT-36). `customer::customer_router` adds
    // `GET /api/customer/assigned` (PROMPT-37). `execution::execution_router`
    // adds `GET /api/execution/engagements` and
    // `POST /api/execution/tasks/{id}/complete` (PROMPT-38) — see that
    // module's docs for why the latter never touches `ActionQueueEntry`
    // state directly. `products::products_router` adds
    // `GET /api/products/catalog` (PROMPT-39) — see that module's docs for
    // why aggressive caching of this response lives client-side (TanStack
    // Query), not as an HTTP `Cache-Control` header here.
    // `landscape::landscape_router` adds `GET /api/landscape/intelligence`
    // and `POST /api/landscape/observations` (PROMPT-40). `legal::legal_router`
    // adds `GET /api/legal/clauses` (PROMPT-41) — see that module's docs for
    // the `proposal_id`/`topic` either/or query contract.
    let api_router = Router::new()
        .route("/login/dev", post(session::login_dev))
        .route("/login/firebase", post(session::login_firebase))
        .route("/logout", post(session::logout))
        .merge(session::protected_router(state.clone()))
        .merge(permissions::diagnostic_router(state.clone()))
        .merge(dashboard::dashboard_router(state.clone()))
        .merge(sales::sales_router(state.clone()))
        .merge(commit::commit_router(state.clone()))
        .merge(edu::edu_router(state.clone()))
        .merge(capacity::capacity_router(state.clone()))
        .merge(customer::customer_router(state.clone()))
        .merge(execution::execution_router(state.clone()))
        .merge(products::products_router(state.clone()))
        .merge(landscape::landscape_router(state.clone()))
        .merge(legal::legal_router(state.clone()))
        .merge(workflow_sessions::workflow_sessions_router(state.clone()))
        .merge(notifications_sse::notifications_router(state.clone()))
        .merge(notifications::notifications_write_router(state.clone()));

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
