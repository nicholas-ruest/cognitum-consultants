//! Session state + session-lookup middleware (ADR-008, PROMPT-11).
//!
//! Holds the shared [`AppState`] used across `bff-api`'s router (DB pool,
//! session provider, Prometheus handle), the session-cookie name constant,
//! the `POST /api/login/dev` and `GET /api/session` handlers, and the
//! session-lookup middleware ([`require_session`]) applied uniformly to the
//! `/api/*` protected sub-router ([`protected_router`]) so future protected
//! routes (U15/U20+) only need a `.route(...)` call added there, not
//! repeated auth logic per-handler.
//!
//! **Dev-only login, documented rather than feature-gated**:
//! `POST /api/login/dev` calls `DevStubSessionProvider::create_dev_session`
//! directly. There is no real Armor-backed provider yet (ADR-008 "Interim
//! dev-stub"), so this route is registered unconditionally rather than
//! behind a `bff-api`-level Cargo feature — the actual safety valve is
//! `DevStubSessionProvider::new`'s runtime panic outside a `dev`
//! environment (`crates/auth/src/dev_stub.rs`), which already makes this
//! route a no-op (a panic at startup, in fact) in any non-dev deployment.
//! Once a real provider lands, this route and [`AppState::dev_session_provider`]
//! should become conditional on which provider `main.rs` constructs (see the
//! `TODO` on the `auth` dependency in `Cargo.toml`).

use std::sync::Arc;

use auth::dev_stub::DevStubSessionProvider;
use auth::{Session, SessionProvider};
use axum::extract::{FromRef, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{json, Value};
use uuid::Uuid;

/// Name of the cookie carrying the opaque session id (ADR-008).
pub const SESSION_COOKIE_NAME: &str = "cognitum_session";

/// Shared application state, constructed once at startup in `main` and
/// threaded through the whole router via `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    /// ADR-010 Postgres pool (PROMPT-09). Not read by any handler yet.
    pub db_pool: persistence::Pool,
    /// The session provider used for session *lookups* ([`require_session`]
    /// below), typed as `dyn SessionProvider` so this field doesn't need to
    /// change shape once a real, Armor-backed provider replaces the
    /// dev-stub.
    pub session_provider: Arc<dyn SessionProvider>,
    /// The concrete dev-stub provider — the same underlying instance as
    /// `session_provider` (both `Arc`s point at it) — kept separately so
    /// `POST /api/login/dev` can call its stub-specific
    /// `create_dev_session`, which isn't (and shouldn't be) part of the
    /// general `SessionProvider` trait.
    pub dev_session_provider: Arc<DevStubSessionProvider>,
    /// Whether to set the `Secure` cookie flag (ADR-008: `Secure` in
    /// non-local environments), driven by `Config::is_dev()` at startup.
    pub secure_cookies: bool,
    pub prometheus_handle: PrometheusHandle,
    /// Per-consultant cache of Armor `PermissionAssertion`s (ADR-009,
    /// PROMPT-15) — see `crate::permissions` module docs for the caching/
    /// TTL semantics and keying decision. `Arc`-wrapped (rather than
    /// cloned per-request like the other fields, which are already cheap
    /// `Arc`/`Copy`) since `PermissionCache` holds its own interior
    /// `RwLock`-guarded state that must be shared, not duplicated, across
    /// every handler invocation.
    pub permission_cache: Arc<crate::permissions::PermissionCache>,
    /// Repository for [`bff_core::DashboardConfiguration`] (PROMPT-21/23,
    /// ADR-010), backing `GET`/`PUT /api/dashboard`
    /// (`crate::dashboard`). `Arc<dyn ...>`, matching `permission_cache`'s
    /// convention, so `bff-api` depends only on the `bff_core` trait
    /// interface (ADR-004) — the concrete Postgres implementation
    /// (`persistence::PgDashboardConfigurationRepository`) is constructed
    /// once in `main` and shared read-only across every handler
    /// invocation.
    pub dashboard_repository: Arc<dyn bff_core::DashboardConfigurationRepository>,
    /// Sales ACL gateway (ADR-016, PROMPT-24/25) used for
    /// `SalesGateway::check_account_claim` — the user-blocking,
    /// idempotent-read call. `Arc<dyn ...>`, matching `permission_cache`'s
    /// and `dashboard_repository`'s convention. See `crate::sales` module
    /// docs for why this is a *separate* `NexusSalesGateway` instance from
    /// [`Self::sales_command_gateway`] rather than one shared field.
    pub sales_query_gateway: Arc<dyn nexus_client::SalesGateway>,
    /// Sales ACL gateway (ADR-016, PROMPT-24/25) used for
    /// `SalesGateway::request_collaboration` / `SalesGateway::submit_referral`
    /// — non-idempotent side-effecting commands that must never be
    /// auto-retried. Deliberately a different gateway *instance* than
    /// [`Self::sales_query_gateway`] even though both implement the same
    /// [`nexus_client::SalesGateway`] trait — see `crate::sales` module
    /// docs.
    pub sales_command_gateway: Arc<dyn nexus_client::SalesGateway>,
    /// Commit ACL gateway (ADR-016, PROMPT-34) used for
    /// `CommitGateway::list_proposals` — the idempotent-read call. Mirrors
    /// [`Self::sales_query_gateway`]'s split rationale exactly: see
    /// `crate::commit`/`nexus_client::commit` module docs for why this is a
    /// *separate* `NexusCommitGateway` instance from
    /// [`Self::commit_command_gateway`] rather than one shared field.
    pub commit_query_gateway: Arc<dyn nexus_client::CommitGateway>,
    /// Commit ACL gateway (ADR-016, PROMPT-34) used for
    /// `CommitGateway::create_proposal` / `CommitGateway::request_proposal_action`
    /// — non-idempotent side-effecting commands that must never be
    /// auto-retried. Deliberately a different gateway *instance* than
    /// [`Self::commit_query_gateway`] even though both implement the same
    /// [`nexus_client::CommitGateway`] trait — see `crate::commit` module
    /// docs.
    pub commit_command_gateway: Arc<dyn nexus_client::CommitGateway>,
    /// Edu ACL gateway (ADR-016, PROMPT-35) used for
    /// `EduGateway::request_learning_catalog`. Unlike
    /// [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`], there
    /// is no matching `edu_command_gateway` — Edu has no side-effecting
    /// outbound command (`anti-corruption-layers.md` §3), so a single
    /// `Arc<dyn ...>` instance, retry-wrapped over the ADR-016 extended-read
    /// timeout budget, safely serves the whole trait. See
    /// `crate::edu`/`nexus_client::edu` module docs.
    pub edu_gateway: Arc<dyn nexus_client::EduGateway>,
    /// Capacity ACL gateway (ADR-016, PROMPT-36) used for
    /// `CapacityGateway::get_own_profile` — the idempotent-read call.
    /// Mirrors [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`]'s
    /// split rationale exactly: see `crate::capacity`/`nexus_client::capacity`
    /// module docs for why this is a *separate* `NexusCapacityGateway`
    /// instance from [`Self::capacity_command_gateway`] rather than one
    /// shared field.
    pub capacity_query_gateway: Arc<dyn nexus_client::CapacityGateway>,
    /// Capacity ACL gateway (ADR-016, PROMPT-36) used for
    /// `CapacityGateway::update_own_profile` — a non-idempotent
    /// side-effecting command that must never be auto-retried. Deliberately
    /// a different gateway *instance* than [`Self::capacity_query_gateway`]
    /// even though both implement the same [`nexus_client::CapacityGateway`]
    /// trait — see `crate::capacity` module docs.
    pub capacity_command_gateway: Arc<dyn nexus_client::CapacityGateway>,
    /// Customer ACL gateway (ADR-016, PROMPT-37) used for
    /// `CustomerGateway::request_assigned_customer_context`. Unlike
    /// [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`]/
    /// [`Self::capacity_query_gateway`], there is no matching
    /// `customer_command_gateway` — Customer has no side-effecting outbound
    /// command (`anti-corruption-layers.md` §5), so a single `Arc<dyn ...>`
    /// instance, retry-wrapped over the ADR-016 read timeout budget, safely
    /// serves the whole trait — the same shape as
    /// [`Self::edu_gateway`]. See `crate::customer`/`nexus_client::customer`
    /// module docs.
    pub customer_gateway: Arc<dyn nexus_client::CustomerGateway>,
    /// Execution ACL gateway (ADR-016, PROMPT-38) used for
    /// `ExecutionGateway::request_assigned_engagements` — the idempotent-read
    /// call. Mirrors [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`]/
    /// [`Self::capacity_query_gateway`]'s split rationale exactly: see
    /// `crate::execution`/`nexus_client::execution` module docs for why this
    /// is a *separate* `NexusExecutionGateway` instance from
    /// [`Self::execution_command_gateway`] rather than one shared field.
    pub execution_query_gateway: Arc<dyn nexus_client::ExecutionGateway>,
    /// Execution ACL gateway (ADR-016, PROMPT-38) used for
    /// `ExecutionGateway::confirm_task_completion` — a non-idempotent
    /// side-effecting command that must never be auto-retried. Deliberately
    /// a different gateway *instance* than [`Self::execution_query_gateway`]
    /// even though both implement the same [`nexus_client::ExecutionGateway`]
    /// trait — see `crate::execution` module docs.
    pub execution_command_gateway: Arc<dyn nexus_client::ExecutionGateway>,
    /// Products ACL gateway (ADR-016, PROMPT-39) used for
    /// `ProductsGateway::request_product_catalog`. Unlike
    /// [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`]/
    /// [`Self::capacity_query_gateway`]/[`Self::execution_query_gateway`],
    /// there is no matching `products_command_gateway` — Products has no
    /// side-effecting outbound command (`anti-corruption-layers.md` §7), so
    /// a single `Arc<dyn ...>` instance, retry-wrapped over the ADR-016
    /// longest-read-timeout + most-aggressive-retry budget (this repo's
    /// most cacheable, least latency-sensitive gateway), safely serves the
    /// whole trait — the same shape as [`Self::edu_gateway`]/
    /// [`Self::customer_gateway`]. See
    /// `crate::products`/`nexus_client::products` module docs.
    pub products_gateway: Arc<dyn nexus_client::ProductsGateway>,
    /// Landscape ACL gateway (ADR-016, PROMPT-40) used for
    /// `LandscapeGateway::request_intelligence_digest` — the idempotent-read
    /// call. Mirrors [`Self::execution_query_gateway`]'s split rationale
    /// exactly: see `crate::landscape`/`nexus_client::landscape` module docs
    /// for why this is a *separate* `NexusLandscapeGateway` instance from
    /// [`Self::landscape_command_gateway`] rather than one shared field.
    pub landscape_query_gateway: Arc<dyn nexus_client::LandscapeGateway>,
    /// Landscape ACL gateway (ADR-016, PROMPT-40) used for
    /// `LandscapeGateway::submit_field_observation` — a non-idempotent
    /// side-effecting command that must never be auto-retried. Deliberately
    /// a different gateway *instance* than [`Self::landscape_query_gateway`]
    /// even though both implement the same [`nexus_client::LandscapeGateway`]
    /// trait — see `crate::landscape` module docs.
    pub landscape_command_gateway: Arc<dyn nexus_client::LandscapeGateway>,
    /// Legal ACL gateway (ADR-007, PROMPT-41) used for
    /// `LegalGateway::request_approved_clauses`. Unlike
    /// [`Self::sales_query_gateway`]/[`Self::commit_query_gateway`]/
    /// [`Self::landscape_query_gateway`], there is no matching
    /// `legal_command_gateway` — Legal has no side-effecting outbound
    /// command (`anti-corruption-layers.md` §9: "pure read-only, conformist
    /// relationship"), so a single `Arc<dyn ...>` instance, retry-wrapped
    /// over the ADR-016 read timeout budget, safely serves the whole trait —
    /// the same shape as [`Self::customer_gateway`]/[`Self::products_gateway`].
    /// See `crate::legal`/`nexus_client::legal` module docs.
    pub legal_gateway: Arc<dyn nexus_client::LegalGateway>,
    /// Repository for [`bff_core::CrossCapabilityWorkflowSession`]
    /// (PROMPT-22/34, ADR-010). PROMPT-22 only built the aggregate +
    /// repository; PROMPT-34 is the first real BFF consumer
    /// (`crate::workflow_sessions`'s `POST /api/workflow-sessions`, and
    /// `crate::commit::create_proposal`'s `origin_workflow_session_id`
    /// hand-off lookup). `Arc<dyn ...>`, matching every other repository
    /// field's convention.
    pub workflow_session_repository: Arc<dyn bff_core::WorkflowSessionRepository>,
    /// Repository for [`bff_core::NotificationItem`] (PROMPT-29/30,
    /// ADR-010). Not yet read by any handler ([`crate::event_ingestion`]'s
    /// polling loop is the only current writer, via `main`'s spawned
    /// background task) — kept on `AppState` (rather than only inside the
    /// polling task) so PROMPT-31's `GET /api/notifications/stream` SSE
    /// endpoint and any future `POST /api/notifications/*` action routes
    /// can share the same repository instance.
    pub notification_repository: Arc<dyn bff_core::NotificationRepository>,
    /// Repository for [`bff_core::ActionQueueEntry`] (PROMPT-29/30,
    /// ADR-010). Same "not yet read by a handler, shared for PROMPT-31"
    /// rationale as [`Self::notification_repository`].
    pub action_queue_repository: Arc<dyn bff_core::ActionQueueRepository>,
    /// In-process pub/sub bus (PROMPT-30, ADR-011) that
    /// [`crate::event_ingestion::run_polling_loop`] publishes freshly-
    /// ingested notifications/action-queue entries into. `Arc`-wrapped so
    /// the same bus instance is shared between the background polling task
    /// and (in PROMPT-31) every `GET /api/notifications/stream` SSE
    /// handler's `subscribe()` call.
    pub event_bus: Arc<bff_core::EventBus>,
}

impl FromRef<AppState> for PrometheusHandle {
    fn from_ref(state: &AppState) -> Self {
        state.prometheus_handle.clone()
    }
}

/// `POST /api/login/dev`: issues a fixed dev-consultant session
/// (`DevStubSessionProvider::create_dev_session`) and sets it as the
/// session cookie: `HttpOnly`, `SameSite=Strict`, and `Secure` when
/// `AppState::secure_cookies` (i.e. `!Config::is_dev()`).
pub async fn login_dev(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, Json<Value>), Response> {
    let session = state.dev_session_provider.create_dev_session().await.map_err(|err| {
        tracing::error!(error = %err, "dev session creation failed");
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "session creation failed" })))
            .into_response()
    })?;

    let cookie = Cookie::build((SESSION_COOKIE_NAME, session.session_id.to_string()))
        .http_only(true)
        .secure(state.secure_cookies)
        .same_site(SameSite::Strict)
        .path("/")
        .build();

    Ok((jar.add(cookie), Json(json!({ "consultant_id": session.consultant_id }))))
}

/// Response body for `GET /api/session`.
///
/// `permission_assertions` (ADR-009, PROMPT-19) is the consultant's current
/// Armor-granted [`PermissionAssertion`] set, sourced from
/// [`crate::permissions::PermissionCache`]. **This field is a UX/rendering
/// signal only, never an enforcement mechanism** — the frontend uses it to
/// conditionally render nav items etc. (ADR-009 layer 2), but every real
/// mutation is independently re-authorized server-side by the owning
/// capability (ADR-009 layer 3). A consultant whose client omits or
/// misrepresents this field gains nothing: the BFF's own `RequirePermission`
/// checks (layer 1) and the downstream capability's checks (layer 3) are
/// what actually gate access, not what this response does or doesn't list.
#[derive(serde::Serialize)]
pub struct SessionResponse {
    pub consultant_id: String,
    pub permission_assertions: Vec<nexus_client::PermissionAssertion>,
}

/// `GET /api/session`: returns the authenticated consultant's identity plus
/// their current Permission Assertions (ADR-009, PROMPT-19). Only reachable
/// once [`require_session`] has attached a `Session` to the request
/// extensions — unauthenticated requests never reach this handler (see
/// [`protected_router`]).
pub async fn get_session_handler(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Json<SessionResponse> {
    let permission_assertions = state.permission_cache.assertions_for(&session.consultant_id).await;
    Json(SessionResponse { consultant_id: session.consultant_id, permission_assertions })
}

/// Session-lookup middleware (ADR-008): reads the session cookie, looks it
/// up via `SessionProvider::get_session`, and either attaches the resolved
/// `Session` to the request extensions and continues, or short-circuits
/// with `401 Unauthorized` if the cookie is missing, the session id is
/// malformed, the session is unknown, or it has expired (`get_session`
/// itself filters expired sessions, per the dev-stub's implementation).
pub async fn require_session(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    match resolve_session(&jar, state.session_provider.as_ref()).await {
        Some(session) => {
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        None => unauthorized(),
    }
}

/// Extracts and resolves the session cookie, if any. Returns `None` for a
/// missing cookie, a malformed session id, an unknown/expired session, or a
/// provider-level lookup failure (logged, then treated as unauthenticated
/// rather than a 5xx — a lookup failure should not look distinguishable
/// from "not logged in" to the caller).
async fn resolve_session(jar: &CookieJar, provider: &dyn SessionProvider) -> Option<Session> {
    let session_id =
        jar.get(SESSION_COOKIE_NAME).and_then(|cookie| Uuid::parse_str(cookie.value()).ok())?;

    match provider.get_session(session_id).await {
        Ok(session) => session,
        Err(err) => {
            tracing::error!(error = %err, "session lookup failed");
            None
        }
    }
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response()
}

/// Builds the `/api/*` protected sub-router: every route added here has
/// [`require_session`] applied uniformly, so future protected routes
/// (U15/U20+) just need a `.route(...)` call added to this function rather
/// than repeating auth logic per-handler.
pub fn protected_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/session", get(get_session_handler))
        .layer(axum::middleware::from_fn_with_state(state, require_session))
}
