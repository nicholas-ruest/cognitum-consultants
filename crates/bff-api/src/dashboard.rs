//! `GET`/`PUT /api/dashboard` (PROMPT-23, ADR-006 `/api/*` shape, ADR-009
//! permission filtering, ADR-010 persistence via U21's repository).
//!
//! Exposes `bff_core::DashboardConfiguration` (`consultant-experience-context.md`
//! §1.2) over HTTP. Both routes sit under [`crate::session::protected_router`]'s
//! `require_session` gate (via [`dashboard_router`]'s own
//! [`crate::session::require_session`] layer, same pattern as
//! [`crate::permissions::diagnostic_router`]) — an unauthenticated request
//! (missing/invalid/expired session cookie) is rejected `401 Unauthorized`
//! by that middleware before either handler body ever runs. The prompt text
//! says "403 if consultant isn't authenticated"; this is a deliberate,
//! documented correction, not a deviation: `401 Unauthorized` is the
//! correct status for "no valid credential presented at all", while `403
//! Forbidden` is reserved (elsewhere in this codebase — see
//! `crate::permissions::RequirePermission`) for "authenticated, but not
//! authorized for this specific resource". Every other protected route in
//! this repo already uses `401` for the missing-session case (PROMPT-11);
//! staying consistent with that beats a literal reading of the prompt's
//! imprecise wording.
//!
//! # Design decision: GET does not persist a freshly-constructed default
//! [`get_dashboard`] returns a fresh, in-memory-only
//! [`DashboardConfiguration::new`] when the repository has no saved
//! configuration for the consultant — it does **not** call
//! [`bff_core::DashboardConfigurationRepository::save`] first. `GET` is
//! expected to be side-effect-free/idempotent/safe (no data is created or
//! changed just by reading it), and a mere page load/refresh should not
//! silently create persisted state a consultant never explicitly chose
//! (e.g. concurrent GETs from two tabs both racing to "create" a row, or a
//! monitoring probe hitting this route and quietly seeding rows for
//! consultants who never open the dashboard). Persistence is explicit and
//! consultant-driven: it happens only via `PUT /api/dashboard`. A consultant
//! who never customizes their dashboard simply gets the same default set
//! recomputed (cheaply — three permission lookups already resolved this
//! request) on every `GET` until they `PUT` a real layout.
//!
//! # Design decision: filter, don't flag, stale permissions
//! Invariant 1 ("every card's `module_id` must be a capability the
//! consultant currently holds") is enforced at write time by
//! `DashboardConfiguration::add_card`, but permissions can be revoked
//! *after* a card was legitimately saved. [`get_dashboard`] re-checks every
//! card — including ones loaded from storage — against the consultant's
//! **current** resolved assertion set and silently drops (filters out) any
//! card for a capability no longer held, rather than returning it flagged
//! as `"unavailable"`. This matches ADR-009 layer 1/2's existing
//! established pattern elsewhere in this codebase (`PermissionCache`
//! short-circuiting with `403` before a downstream call; `Sidebar`'s
//! `navItemsFromAssertions` omitting nav items outright rather than
//! rendering disabled ones) — this repo's convention is "don't render what
//! you can't do", not "render everything, disabled". A flagged-unavailable
//! card would also leak the *former* `module_id` to a client that no
//! longer has any assertion for it, which the filter approach avoids for
//! free. The persisted row is left untouched either way — filtering only
//! affects what this response returns, never what's stored (a later
//! permission grant makes the same stored card reappear on the next `GET`
//! with no data-repair step needed).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use bff_core::{CardPlacement, DashboardConfiguration, DashboardConfigurationError};
use serde::{Deserialize, Serialize};

use crate::session::{self, AppState};
use auth::Session;

/// Wire shape for one dashboard card, shared by both the `GET` response and
/// the `PUT` request/response bodies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardCardDto {
    pub module_id: String,
    pub position: u32,
}

/// `GET`/`PUT /api/dashboard` response body.
#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub consultant_id: String,
    pub cards: Vec<DashboardCardDto>,
}

/// `PUT /api/dashboard` request body.
#[derive(Debug, Deserialize)]
pub struct PutDashboardRequest {
    pub cards: Vec<DashboardCardDto>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn to_response(config: &DashboardConfiguration) -> DashboardResponse {
    DashboardResponse {
        consultant_id: config.consultant_id().to_owned(),
        cards: config
            .cards()
            .iter()
            .map(|card| DashboardCardDto { module_id: card.module_id().to_owned(), position: card.position() })
            .collect(),
    }
}

/// `GET /api/dashboard`: returns the authenticated consultant's current
/// dashboard composition (defaults if none has been saved yet), with every
/// card re-checked against the consultant's *current* permissions. See the
/// module docs for the GET-does-not-persist and filter-not-flag design
/// decisions.
pub async fn get_dashboard(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    let assertions = state.permission_cache.assertions_for(&session.consultant_id).await;
    let is_permitted = |module_id: &str| assertions.iter().any(|assertion| assertion.capability == module_id);

    let stored = match state.dashboard_repository.find_by_consultant_id(&session.consultant_id).await {
        Ok(existing) => existing,
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "dashboard lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load dashboard configuration");
        }
    };

    let config = match stored {
        Some(existing) => existing,
        None => match DashboardConfiguration::new(&session.consultant_id, &is_permitted) {
            Ok(config) => config,
            Err(err) => {
                tracing::error!(error = %err, consultant_id = %session.consultant_id, "default dashboard construction failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to build default dashboard configuration");
            }
        },
    };

    // Permission-filtering invariant, re-checked on every read (module
    // docs): drop any card — even one loaded from storage — for a
    // capability the consultant no longer currently holds.
    let cards: Vec<DashboardCardDto> = config
        .cards()
        .iter()
        .filter(|card| is_permitted(card.module_id()))
        .map(|card| DashboardCardDto { module_id: card.module_id().to_owned(), position: card.position() })
        .collect();

    Json(DashboardResponse { consultant_id: session.consultant_id, cards }).into_response()
}

/// `PUT /api/dashboard`: accepts a full new card layout, validates it
/// against the consultant's current permissions and invariant 2 (unique
/// positions) via [`DashboardConfiguration::add_card`] for every entry,
/// and persists the result only if every card is valid. Rejects the whole
/// request (422, nothing persisted) if any card fails — never silently
/// drops an invalid card.
pub async fn put_dashboard(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<PutDashboardRequest>,
) -> Response {
    let assertions = state.permission_cache.assertions_for(&session.consultant_id).await;
    let is_permitted = |module_id: &str| assertions.iter().any(|assertion| assertion.capability == module_id);

    // Start from an empty aggregate: `DashboardConfiguration::new` would
    // otherwise inject its own default card set, which has no place in a
    // PUT — every card in the saved result must come from `body.cards`,
    // nothing implicit. Passing a reject-everything predicate here means
    // no default survives the filter, regardless of what the consultant is
    // actually permitted for; the real `is_permitted` check is applied
    // below, per requested card, via `add_card`.
    let mut config = match DashboardConfiguration::new(&session.consultant_id, &|_module_id| false) {
        Ok(config) => config,
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "dashboard construction failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to construct dashboard configuration");
        }
    };

    for card in body.cards {
        let placement = CardPlacement::new(card.module_id, card.position);
        if let Err(err) = config.add_card(placement, &is_permitted) {
            return error_response(StatusCode::UNPROCESSABLE_ENTITY, describe_validation_error(&err));
        }
    }

    if let Err(err) = state.dashboard_repository.save(&config).await {
        tracing::error!(error = %err, consultant_id = %session.consultant_id, "dashboard save failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to save dashboard configuration");
    }

    Json(to_response(&config)).into_response()
}

fn describe_validation_error(err: &DashboardConfigurationError) -> String {
    err.to_string()
}

/// Builds the `/api/dashboard` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies — see the module docs for why an unauthenticated request 401s
/// here rather than reaching either handler.
pub fn dashboard_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/dashboard", get(get_dashboard).put(put_dashboard))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum_extra::extract::cookie::Cookie;
    use bff_core::DashboardConfigurationRepository;
    use chrono::{Duration as ChronoDuration, Utc};
    use nexus_client::{ArmorGateway, ArmorGatewayError, PermissionAssertion};
    use persistence::PgDashboardConfigurationRepository;
    use serde_json::{json, Value};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;
    use crate::permissions::PermissionCache;

    /// Test-double `ArmorGateway`: returns a fixed, caller-supplied
    /// assertion set instead of ever calling a live Armor/Nexus endpoint —
    /// per the acceptance criteria, dashboard tests use a mock/test double
    /// for the permission-check dependency, not a live Armor call.
    struct MockArmorGateway {
        capabilities: Vec<&'static str>,
    }

    #[async_trait::async_trait]
    impl ArmorGateway for MockArmorGateway {
        async fn fetch_assertions(
            &self,
            consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<PermissionAssertion>, ArmorGatewayError> {
            Ok(self
                .capabilities
                .iter()
                .map(|capability| PermissionAssertion {
                    consultant_id: consultant_id.to_owned(),
                    capability: (*capability).to_owned(),
                    scope: "default".to_owned(),
                    expires_at: Utc::now() + ChronoDuration::minutes(5),
                })
                .collect())
        }
    }

    /// Stub `SalesGateway` for dashboard tests, which exercise no sales
    /// route — `AppState` requires the field regardless (PROMPT-25), so
    /// this satisfies the type without pretending to be a meaningful test
    /// double; any call panics, which would indicate a dashboard route
    /// unexpectedly reaching the sales gateway.
    struct UnusedSalesGateway;

    #[async_trait::async_trait]
    impl nexus_client::SalesGateway for UnusedSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::AccountClaimResult, nexus_client::SalesGatewayError> {
            unimplemented!("dashboard tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("dashboard tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("dashboard tests never call the sales gateway")
        }
    }

    /// Stub `CommitGateway` for dashboard tests, same rationale as
    /// `UnusedSalesGateway` above (PROMPT-34).
    struct UnusedCommitGateway;

    #[async_trait::async_trait]
    impl nexus_client::CommitGateway for UnusedCommitGateway {
        async fn create_proposal(
            &self,
            _origin_reference: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::ProposalSummary, nexus_client::CommitGatewayError> {
            unimplemented!("dashboard tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("dashboard tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("dashboard tests never call the commit gateway")
        }
    }

    async fn migrated_pool() -> (persistence::Pool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("../persistence/migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    fn dev_config() -> config::Config {
        config::Config {
            database_url: "postgres://localhost:5432/test".to_owned(),
            port: 3000,
            log_level: "info".to_owned(),
            nexus_endpoint_url: "http://localhost:8080".to_owned(),
            environment: config::DEV_ENVIRONMENT.to_owned(),
            static_dir: None,
            event_poll_interval_seconds: 5,
        }
    }

    /// Builds a full `AppState` (real Postgres pool, mock `ArmorGateway`
    /// granting exactly `capabilities`) plus a `Router` mounting
    /// `dashboard_router` under `/api`, and an authenticated session cookie
    /// for `DevStubSessionProvider`'s fixed dev consultant.
    async fn test_app(
        capabilities: Vec<&'static str>,
    ) -> (Router<()>, Cookie<'static>, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let (pool, container) = migrated_pool().await;

        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();
        let session = dev_session_provider.create_dev_session().await.expect("create_dev_session failed");

        let armor_gateway: Arc<dyn ArmorGateway> = Arc::new(MockArmorGateway { capabilities });
        let permission_cache = Arc::new(PermissionCache::new(armor_gateway));

        let dashboard_repository: Arc<dyn DashboardConfigurationRepository> =
            Arc::new(PgDashboardConfigurationRepository::new(pool.clone()));
        let notification_repository: Arc<dyn bff_core::NotificationRepository> =
            Arc::new(persistence::PgNotificationRepository::new(pool.clone()));
        let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
            Arc::new(persistence::PgActionQueueRepository::new(pool.clone()));
        let workflow_session_repository: Arc<dyn bff_core::WorkflowSessionRepository> =
            Arc::new(persistence::PgWorkflowSessionRepository::new(pool.clone()));

        let state = AppState {
            db_pool: pool,
            session_provider,
            dev_session_provider,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache,
            dashboard_repository,
            sales_query_gateway: Arc::new(UnusedSalesGateway),
            sales_command_gateway: Arc::new(UnusedSalesGateway),
            commit_query_gateway: Arc::new(UnusedCommitGateway),
            commit_command_gateway: Arc::new(UnusedCommitGateway),
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", dashboard_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn get_request(cookie: &Cookie<'static>) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri("/api/dashboard")
            .header("cookie", cookie.to_string())
            .body(Body::empty())
            .unwrap()
    }

    fn put_request(cookie: &Cookie<'static>, body: Value) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri("/api/dashboard")
            .header("cookie", cookie.to_string())
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Explicit proof of the 401-before-any-dashboard-logic gate (module
    /// docs' "deliberate correction" note): a request with no session
    /// cookie at all must never reach either handler.
    #[tokio::test]
    async fn unauthenticated_request_gets_401_before_any_dashboard_logic_runs() {
        let (router, _cookie, _container) = test_app(vec!["sales", "commit"]).await;

        let request = Request::builder().method("GET").uri("/api/dashboard").body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn put_without_a_session_cookie_also_gets_401() {
        let (router, _cookie, _container) = test_app(vec!["sales"]).await;

        let request = Request::builder()
            .method("PUT")
            .uri("/api/dashboard")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "cards": [] }).to_string()))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_with_no_existing_config_returns_permission_filtered_defaults() {
        let (router, cookie, _container) = test_app(vec!["sales", "commit"]).await;

        let response = router.oneshot(get_request(&cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        let module_ids: Vec<&str> =
            body["cards"].as_array().unwrap().iter().map(|card| card["module_id"].as_str().unwrap()).collect();

        // DEFAULT_CARD_MODULE_IDS is ["sales", "commit", "execution"]; only
        // the permitted two survive.
        assert_eq!(module_ids, vec!["sales", "commit"]);
        assert_eq!(body["consultant_id"], auth::dev_stub::DEV_CONSULTANT_ID);
    }

    #[tokio::test]
    async fn get_with_an_existing_config_returns_it_filtered_against_current_permissions() {
        let (router, cookie, container) = test_app(vec!["sales", "legal"]).await;

        // Seed a saved configuration granting *different* permissions than
        // the mock gateway currently returns above (the point of this
        // test): "commit" was permitted when saved, but the current mock
        // grants "sales"/"legal" only, so it must be filtered out of the
        // GET response even though it's still in storage.
        {
            let saved_capabilities = ["sales", "commit", "legal"];
            let is_permitted_at_save_time = |module_id: &str| saved_capabilities.contains(&module_id);

            let pool = persistence::create_pool(&connection_string(&container).await).await.unwrap();
            let repo = PgDashboardConfigurationRepository::new(pool);
            let mut config =
                DashboardConfiguration::new(auth::dev_stub::DEV_CONSULTANT_ID, &is_permitted_at_save_time).unwrap();
            config.add_card(CardPlacement::new("legal", 10), &is_permitted_at_save_time).unwrap();
            repo.save(&config).await.expect("seed save failed");
        }

        let response = router.oneshot(get_request(&cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        let module_ids: Vec<&str> =
            body["cards"].as_array().unwrap().iter().map(|card| card["module_id"].as_str().unwrap()).collect();

        // "commit" was in the saved set but is not in the current
        // permission set, so it must not appear; "sales"/"legal" (still
        // currently permitted) must.
        assert!(!module_ids.contains(&"commit"), "stale-permission card must be filtered, got {module_ids:?}");
        assert!(module_ids.contains(&"sales"));
        assert!(module_ids.contains(&"legal"));
    }

    async fn connection_string(container: &testcontainers_modules::testcontainers::ContainerAsync<Postgres>) -> String {
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        format!("postgres://postgres:postgres@{host}:{port}/postgres")
    }

    #[tokio::test]
    async fn put_with_a_valid_layout_persists_and_returns_200() {
        let (router, cookie, _container) = test_app(vec!["sales", "commit"]).await;

        let body = json!({ "cards": [
            { "module_id": "sales", "position": 0 },
            { "module_id": "commit", "position": 1 },
        ] });
        let response = router.clone().oneshot(put_request(&cookie, body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let saved = response_json(response).await;
        let module_ids: Vec<&str> =
            saved["cards"].as_array().unwrap().iter().map(|card| card["module_id"].as_str().unwrap()).collect();
        assert_eq!(module_ids, vec!["sales", "commit"]);

        // Confirm it actually persisted, not just returned in-memory.
        let get_response = router.oneshot(get_request(&cookie)).await.unwrap();
        let refetched = response_json(get_response).await;
        let refetched_module_ids: Vec<&str> =
            refetched["cards"].as_array().unwrap().iter().map(|card| card["module_id"].as_str().unwrap()).collect();
        assert_eq!(refetched_module_ids, vec!["sales", "commit"]);
    }

    #[tokio::test]
    async fn put_with_an_unpermitted_card_is_rejected_and_nothing_is_persisted() {
        let (router, cookie, _container) = test_app(vec!["sales"]).await;

        let body = json!({ "cards": [
            { "module_id": "sales", "position": 0 },
            { "module_id": "legal", "position": 1 },
        ] });
        let response = router.clone().oneshot(put_request(&cookie, body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let error_body = response_json(response).await;
        assert!(error_body["error"].as_str().unwrap().contains("legal"));

        // Nothing must have been persisted from the rejected request.
        let get_response = router.oneshot(get_request(&cookie)).await.unwrap();
        let refetched = response_json(get_response).await;
        let module_ids: Vec<&str> =
            refetched["cards"].as_array().unwrap().iter().map(|card| card["module_id"].as_str().unwrap()).collect();
        assert_eq!(module_ids, vec!["sales"], "a rejected PUT must not have persisted anything");
    }

    #[tokio::test]
    async fn put_with_duplicate_positions_is_rejected() {
        let (router, cookie, _container) = test_app(vec!["sales", "commit"]).await;

        let body = json!({ "cards": [
            { "module_id": "sales", "position": 0 },
            { "module_id": "commit", "position": 0 },
        ] });
        let response = router.oneshot(put_request(&cookie, body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
