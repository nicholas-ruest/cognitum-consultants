//! `POST /api/sales/*` (PROMPT-25, ADR-009 permission gate, ADR-016
//! resilience stack, `../../.plans/ddd/anti-corruption-layers.md` §1).
//!
//! Three session-gated routes over [`nexus_client::SalesGateway`]
//! (PROMPT-24): `lead-conflict-check` (query), `request-collaboration` and
//! `submit-referral` (commands). All three apply the same permission gate
//! as [`crate::permissions::RequirePermission`]'s underlying check —
//! `state.permission_cache.is_permitted(consultant_id, "sales")` — and
//! short-circuit with `403 Forbidden` *before* the gateway is ever called
//! (PROMPT-15's established pattern; see the tests for an explicit
//! call-count proof this short-circuit actually happens).
//!
//! # Critical invariant: no re-adjudication of `creation_allowed`
//! Per `anti-corruption-layers.md` §1 step 5 ("the BFF relays this
//! verbatim... no `AccountClaimResult` invariant in this repo re-derives or
//! overrides `creation_allowed`"), [`lead_conflict_check`] returns Sales'
//! [`nexus_client::AccountClaimResult`] to the frontend **unchanged** — it
//! is serialized directly, not copied field-by-field into a parallel BFF
//! DTO, so there is no code path here that could inspect or branch on
//! `creation_allowed`/`permitted_actions` even by accident. A gateway
//! *error* (Nexus/Sales unreachable, timed out, or returned an unexpected
//! shape) is mapped to `502 Bad Gateway`, never treated as — or coerced
//! into — a synthetic "no conflict" / `creation_allowed: true` result;
//! "Sales is unavailable" and "Sales says this is fine" are different
//! facts, and only the second one may ever produce a `200` here.
//!
//! # Gateway-construction decision: two `NexusSalesGateway` instances
//! [`nexus_client::sales`]'s module docs (PROMPT-24) spell out the
//! constraint: `check_account_claim` is a read with no side effect, so
//! ADR-016 allows (and expects) it to run over a
//! [`nexus_client::RetryingTransport`]-wrapped stack; `request_collaboration`
//! and `submit_referral` each create a record in Sales as a side effect and
//! must **never** be auto-retried, per ADR-016's "retry only idempotent
//! reads" contract. Because `NexusSalesGateway` holds one shared `transport`
//! field used by all three trait methods, one instance cannot safely serve
//! both retry profiles at once.
//!
//! [`session::AppState`] therefore carries **two** separate
//! `Arc<dyn SalesGateway>` fields —
//! [`session::AppState::sales_query_gateway`] (`TimeoutTransport` + write
//! budget, wrapped in `RetryingTransport`) for [`lead_conflict_check`], and
//! [`session::AppState::sales_command_gateway`] (`TimeoutTransport` +
//! write budget, no retry wrapper) for [`request_collaboration`] and
//! [`submit_referral`] — constructed once in `main` over the same base
//! `NexusTransport`, mirroring `main`'s existing Armor-gateway assembly
//! convention. This was chosen over the "one instance, no retries at all"
//! simpler alternative because: (1) `NexusSalesGateway`'s own doc comment
//! explicitly names "construct two `NexusSalesGateway` instances, one per
//! timeout/retry profile" as the correct fix for exactly this situation;
//! (2) `check_account_claim` is the *user-blocking, synchronous,
//! consultant-is-actively-waiting* call this repo's ADR-016 write-timeout
//! carve-out exists for in the first place — silently forgoing its retry
//! benefit would leave every transient Sales blip surfaced straight to the
//! consultant as "Sales unavailable" instead of self-healing within the
//! bounded retry budget; and (3) each handler already calls a specific
//! trait method, so picking the matching gateway field per handler costs
//! nothing beyond the two extra `AppState` fields — there is no risk of a
//! future call site accidentally reusing the retrying instance for a
//! command method, because each handler only ever has one gateway field in
//! scope for the method it calls.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating all three routes below (PROMPT-15/ADR-009).
const SALES_CAPABILITY: &str = "sales";

/// `POST /api/sales/lead-conflict-check` request body.
#[derive(Debug, Deserialize)]
pub struct LeadConflictCheckRequest {
    pub company_name: String,
}

/// `POST /api/sales/request-collaboration` request body.
#[derive(Debug, Deserialize)]
pub struct RequestCollaborationRequest {
    pub company_reference: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// `POST /api/sales/submit-referral` request body.
#[derive(Debug, Deserialize)]
pub struct SubmitReferralRequest {
    pub company_reference: String,
    #[serde(default)]
    pub notes: Option<String>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the sales capability")
}

/// `502`: the gateway call to Sales (via Nexus) failed — never coerced into
/// a synthetic success. See the module docs' "no re-adjudication" section.
fn sales_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "sales service unavailable")
}

/// `POST /api/sales/lead-conflict-check`: checks permission, then calls
/// [`nexus_client::SalesGateway::check_account_claim`] via
/// [`AppState::sales_query_gateway`] and relays the resulting
/// [`nexus_client::AccountClaimResult`] **verbatim** — see the module docs.
pub async fn lead_conflict_check(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<LeadConflictCheckRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, SALES_CAPABILITY).await {
        return forbidden();
    }

    match state.sales_query_gateway.check_account_claim(&body.company_name, &session.consultant_id).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                "sales account-claim check failed"
            );
            sales_unavailable()
        }
    }
}

/// `POST /api/sales/request-collaboration`: checks permission, then calls
/// [`nexus_client::SalesGateway::request_collaboration`] via
/// [`AppState::sales_command_gateway`].
pub async fn request_collaboration(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<RequestCollaborationRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, SALES_CAPABILITY).await {
        return forbidden();
    }

    match state
        .sales_command_gateway
        .request_collaboration(&body.company_reference, &session.consultant_id, body.message.as_deref())
        .await
    {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                "sales collaboration request failed"
            );
            sales_unavailable()
        }
    }
}

/// `POST /api/sales/submit-referral`: checks permission, then calls
/// [`nexus_client::SalesGateway::submit_referral`] via
/// [`AppState::sales_command_gateway`].
pub async fn submit_referral(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<SubmitReferralRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, SALES_CAPABILITY).await {
        return forbidden();
    }

    match state
        .sales_command_gateway
        .submit_referral(&body.company_reference, &session.consultant_id, body.notes.as_deref())
        .await
    {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                "sales referral submission failed"
            );
            sales_unavailable()
        }
    }
}

/// Builds the `/api/sales/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo — an
/// unauthenticated request 401s before any handler body (including the
/// permission check) runs.
pub fn sales_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/sales/lead-conflict-check", post(lead_conflict_check))
        .route("/sales/request-collaboration", post(request_collaboration))
        .route("/sales/submit-referral", post(submit_referral))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum_extra::extract::cookie::Cookie;
    use bff_core::DashboardConfigurationRepository;
    use chrono::{Duration as ChronoDuration, Utc};
    use nexus_client::{
        AccountClaimResult, ArmorGateway, ArmorGatewayError, NexusTransportError, PermissionAssertion, SalesGateway,
        SalesGatewayError,
    };
    use persistence::PgDashboardConfigurationRepository;
    use serde_json::{json, Value};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;
    use crate::permissions::PermissionCache;

    /// Test-double `ArmorGateway`, matching `dashboard`'s pattern: returns a
    /// fixed, caller-supplied capability set rather than ever calling a
    /// live Armor/Nexus endpoint.
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

    /// Configurable outcome for one gateway method call.
    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `SalesGateway`. Each method increments the shared
    /// `call_count` unconditionally (before inspecting its configured
    /// outcome) so tests can assert the gateway was — or, per the
    /// permission-short-circuit tests, was **never** — invoked.
    struct MockSalesGateway {
        claim_outcome: Outcome<AccountClaimResult>,
        collaboration_outcome: Outcome<()>,
        referral_outcome: Outcome<()>,
        call_count: AtomicUsize,
    }

    impl MockSalesGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> SalesGatewayError {
            SalesGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(3) })
        }
    }

    #[async_trait::async_trait]
    impl SalesGateway for MockSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<AccountClaimResult, SalesGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.claim_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), SalesGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.collaboration_outcome {
                Outcome::Ok(()) => Ok(()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), SalesGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.referral_outcome {
                Outcome::Ok(()) => Ok(()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    /// `active_owned_account` fixture, matching `anti-corruption-layers.md`
    /// §1's worked example verbatim.
    fn active_owned_account_fixture() -> AccountClaimResult {
        AccountClaimResult {
            match_status: "active_owned_account".to_owned(),
            creation_allowed: false,
            display_message: "This company is already being worked.".to_owned(),
            permitted_actions: vec![
                "request_collaboration".to_owned(),
                "submit_referral".to_owned(),
                "cancel".to_owned(),
            ],
        }
    }

    fn default_mock_sales_gateway() -> MockSalesGateway {
        MockSalesGateway {
            claim_outcome: Outcome::Ok(active_owned_account_fixture()),
            collaboration_outcome: Outcome::Ok(()),
            referral_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
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
    /// granting exactly `capabilities`, and `mock_sales_gateway` wired as
    /// *both* `sales_query_gateway` and `sales_command_gateway` so a single
    /// shared call counter observes every gateway call regardless of which
    /// handler made it) plus a `Router` mounting `sales_router` under
    /// `/api`, and an authenticated session cookie for
    /// `DevStubSessionProvider`'s fixed dev consultant.
    async fn test_app(
        capabilities: Vec<&'static str>,
        mock_sales_gateway: Arc<MockSalesGateway>,
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

        let sales_query_gateway: Arc<dyn SalesGateway> = mock_sales_gateway.clone();
        let sales_command_gateway: Arc<dyn SalesGateway> = mock_sales_gateway;

        let state = AppState {
            db_pool: pool,
            session_provider,
            dev_session_provider,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache,
            dashboard_repository,
            sales_query_gateway,
            sales_command_gateway,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", sales_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn post_request(cookie: &Cookie<'static>, path: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("cookie", cookie.to_string())
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn lead_conflict_check_relays_the_account_claim_result_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_sales_gateway());
        let (router, cookie, _container) = test_app(vec!["sales"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/sales/lead-conflict-check", json!({ "company_name": "Acme Corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;

        // Exact-match proof of verbatim relay: no BFF re-shaping of the
        // gateway's `AccountClaimResult`, per the module docs.
        assert_eq!(
            body,
            json!({
                "match_status": "active_owned_account",
                "creation_allowed": false,
                "display_message": "This company is already being worked.",
                "permitted_actions": ["request_collaboration", "submit_referral", "cancel"],
            })
        );
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn lead_conflict_check_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_sales_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/sales/lead-conflict-check", json!({ "company_name": "Acme Corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn lead_conflict_check_never_returns_creation_allowed_true_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockSalesGateway {
            claim_outcome: Outcome::Err,
            collaboration_outcome: Outcome::Ok(()),
            referral_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["sales"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/sales/lead-conflict-check", json!({ "company_name": "Acme Corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_ne!(response.status(), StatusCode::OK);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = response_json(response).await;
        assert_ne!(body.get("creation_allowed"), Some(&Value::Bool(true)));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn request_collaboration_succeeds_when_permitted() {
        let mock_gateway = Arc::new(default_mock_sales_gateway());
        let (router, cookie, _container) = test_app(vec!["sales"], mock_gateway.clone()).await;

        let request = post_request(
            &cookie,
            "/api/sales/request-collaboration",
            json!({ "company_reference": "acme-corp", "message": "let's team up" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn submit_referral_succeeds_when_permitted() {
        let mock_gateway = Arc::new(default_mock_sales_gateway());
        let (router, cookie, _container) = test_app(vec!["sales"], mock_gateway.clone()).await;

        let request = post_request(
            &cookie,
            "/api/sales/submit-referral",
            json!({ "company_reference": "acme-corp", "notes": "not pursuing this one" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn request_collaboration_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_sales_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let request = post_request(
            &cookie,
            "/api/sales/request-collaboration",
            json!({ "company_reference": "acme-corp" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }
}
