//! `GET /api/landscape/intelligence`, `POST /api/landscape/observations`
//! (PROMPT-40, ADR-009 permission gate, ADR-016 resilience stack,
//! `../../.plans/ddd/anti-corruption-layers.md` §8).
//!
//! Two session-gated routes over [`nexus_client::LandscapeGateway`],
//! following [`crate::execution`]'s exact handler pattern (see that module's
//! docs — this one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own DTO on success, `502` on gateway failure.
//!
//! # `submitted_by` is always the caller's own session
//! `submit_observation` never accepts or forwards a `submitted_by`/
//! consultant id from the request body — it always constructs
//! [`nexus_client::FieldObservationSubmission::submitted_by`] from
//! `session.consultant_id`, the id `axum`'s `require_session` middleware
//! resolved from the caller's own session cookie. The same "own data only,
//! by construction" invariant [`crate::capacity`]'s module docs establish
//! for `ConsultantProfileIntake` — see `nexus_client::landscape`'s module
//! docs for the gateway-level half of this.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nexus_client::FieldObservationSubmission;
use serde::Deserialize;
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating both routes below (PROMPT-15/ADR-009).
const LANDSCAPE_CAPABILITY: &str = "landscape";

/// `POST /api/landscape/observations` request body — deliberately carries no
/// `submitted_by` field (see the module docs).
#[derive(Debug, Deserialize)]
pub struct SubmitObservationRequest {
    pub observation_text: String,
    #[serde(default)]
    pub related_company_reference: Option<String>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the landscape capability")
}

/// `502`: the gateway call to Landscape (via Nexus) failed — never coerced
/// into a synthetic success, same convention as
/// `crate::execution::execution_unavailable`.
fn landscape_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "landscape service unavailable")
}

/// `GET /api/landscape/intelligence`: checks permission, then calls
/// [`nexus_client::LandscapeGateway::request_intelligence_digest`] via
/// [`AppState::landscape_query_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::IntelligenceDigestItem`]`>` **verbatim**.
pub async fn get_intelligence_digest(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, LANDSCAPE_CAPABILITY).await {
        return forbidden();
    }

    match state.landscape_query_gateway.request_intelligence_digest().await {
        Ok(items) => Json(items).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "landscape intelligence digest fetch failed");
            landscape_unavailable()
        }
    }
}

/// `POST /api/landscape/observations`: checks permission, then calls
/// [`nexus_client::LandscapeGateway::submit_field_observation`] via
/// [`AppState::landscape_command_gateway`]. **Not idempotent-safe to
/// retry** (ADR-016) — a failed submission requires a conscious re-submit
/// from the frontend, never an automatic one.
pub async fn submit_observation(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<SubmitObservationRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, LANDSCAPE_CAPABILITY).await {
        return forbidden();
    }

    let submission = FieldObservationSubmission {
        observation_text: body.observation_text,
        related_company_reference: body.related_company_reference,
        submitted_by: session.consultant_id.clone(),
    };

    match state.landscape_command_gateway.submit_field_observation(submission).await {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                "landscape field observation submission failed"
            );
            landscape_unavailable()
        }
    }
}

/// Builds the `/api/landscape/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn landscape_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/landscape/intelligence", get(get_intelligence_digest))
        .route("/landscape/observations", post(submit_observation))
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
        ArmorGateway, ArmorGatewayError, FieldObservationSubmission, IntelligenceDigestItem, LandscapeGateway,
        LandscapeGatewayError, NexusTransportError, PermissionAssertion,
    };
    use persistence::PgDashboardConfigurationRepository;
    use serde_json::{json, Value};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;
    use crate::permissions::PermissionCache;

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

    struct UnusedSalesGateway;

    #[async_trait::async_trait]
    impl nexus_client::SalesGateway for UnusedSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::AccountClaimResult, nexus_client::SalesGatewayError> {
            unimplemented!("landscape tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("landscape tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("landscape tests never call the sales gateway")
        }
    }

    struct UnusedCommitGateway;

    #[async_trait::async_trait]
    impl nexus_client::CommitGateway for UnusedCommitGateway {
        async fn create_proposal(
            &self,
            _origin_reference: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::ProposalSummary, nexus_client::CommitGatewayError> {
            unimplemented!("landscape tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("landscape tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("landscape tests never call the commit gateway")
        }
    }

    struct UnusedEduGateway;

    #[async_trait::async_trait]
    impl nexus_client::EduGateway for UnusedEduGateway {
        async fn request_learning_catalog(
            &self,
            _consultant_id: &str,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::LearningSnapshot>, nexus_client::EduGatewayError> {
            unimplemented!("landscape tests never call the edu gateway")
        }
    }

    struct UnusedCapacityGateway;

    #[async_trait::async_trait]
    impl nexus_client::CapacityGateway for UnusedCapacityGateway {
        async fn update_own_profile(
            &self,
            _consultant_id: &str,
            _profile_fields: nexus_client::ConsultantProfileIntake,
        ) -> Result<nexus_client::ProfileUpdateResult, nexus_client::CapacityGatewayError> {
            unimplemented!("landscape tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("landscape tests never call the capacity gateway")
        }
    }

    struct UnusedCustomerGateway;

    #[async_trait::async_trait]
    impl nexus_client::CustomerGateway for UnusedCustomerGateway {
        async fn request_assigned_customer_context(
            &self,
            _consultant_id: &str,
            _customer_id: Option<&str>,
        ) -> Result<Vec<nexus_client::CustomerContextCard>, nexus_client::CustomerGatewayError> {
            unimplemented!("landscape tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("landscape tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("landscape tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("landscape tests never call the products gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `LandscapeGateway`. Each method increments the shared
    /// `call_count` unconditionally so tests can assert the gateway was —
    /// or, per the permission-short-circuit tests, was **never** — invoked.
    struct MockLandscapeGateway {
        digest_outcome: Outcome<Vec<IntelligenceDigestItem>>,
        submit_outcome: Outcome<()>,
        call_count: AtomicUsize,
        last_submission: std::sync::Mutex<Option<FieldObservationSubmission>>,
    }

    impl MockLandscapeGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> LandscapeGatewayError {
            LandscapeGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(3) })
        }
    }

    #[async_trait::async_trait]
    impl LandscapeGateway for MockLandscapeGateway {
        async fn request_intelligence_digest(&self) -> Result<Vec<IntelligenceDigestItem>, LandscapeGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.digest_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn submit_field_observation(
            &self,
            submission: FieldObservationSubmission,
        ) -> Result<(), LandscapeGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            *self.last_submission.lock().unwrap() = Some(submission);
            match &self.submit_outcome {
                Outcome::Ok(()) => Ok(()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn digest_fixture() -> Vec<IntelligenceDigestItem> {
        vec![IntelligenceDigestItem {
            intel_id: "intel-1".to_owned(),
            topic: "Cloud Migration Trends".to_owned(),
            summary: "Enterprises are accelerating multi-cloud adoption.".to_owned(),
            published_at: Utc::now(),
            deep_link: Some("https://landscape.cognitum.one/intel/intel-1".to_owned()),
        }]
    }

    fn default_mock_landscape_gateway() -> MockLandscapeGateway {
        MockLandscapeGateway {
            digest_outcome: Outcome::Ok(digest_fixture()),
            submit_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
            last_submission: std::sync::Mutex::new(None),
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

    async fn test_app(
        capabilities: Vec<&'static str>,
        mock_landscape_gateway: Arc<MockLandscapeGateway>,
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

        let landscape_query_gateway: Arc<dyn LandscapeGateway> = mock_landscape_gateway.clone();
        let landscape_command_gateway: Arc<dyn LandscapeGateway> = mock_landscape_gateway;

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
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            execution_query_gateway: Arc::new(UnusedExecutionGateway),
            execution_command_gateway: Arc::new(UnusedExecutionGateway),
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway,
            landscape_command_gateway,
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", landscape_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn get_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("GET").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
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
    async fn get_intelligence_digest_relays_the_digest_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_landscape_gateway());
        let (router, cookie, _container) = test_app(vec!["landscape"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/landscape/intelligence")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["intel_id"], json!("intel-1"));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn get_intelligence_digest_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_landscape_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/landscape/intelligence")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn get_intelligence_digest_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockLandscapeGateway {
            digest_outcome: Outcome::Err,
            submit_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
            last_submission: std::sync::Mutex::new(None),
        });
        let (router, cookie, _container) = test_app(vec!["landscape"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/landscape/intelligence")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn submit_observation_succeeds_when_permitted_and_uses_the_sessions_consultant_id() {
        let mock_gateway = Arc::new(default_mock_landscape_gateway());
        let (router, cookie, _container) = test_app(vec!["landscape"], mock_gateway.clone()).await;

        let request = post_request(
            &cookie,
            "/api/landscape/observations",
            json!({ "observation_text": "Client mentioned a competitor.", "related_company_reference": "acme-corp" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock_gateway.calls(), 1);

        let submission = mock_gateway.last_submission.lock().unwrap().clone().expect("submission recorded");
        assert_eq!(submission.observation_text, "Client mentioned a competitor.");
        assert_eq!(submission.related_company_reference.as_deref(), Some("acme-corp"));
        // Never client-supplied — always the session's own consultant id
        // (see the module docs).
        assert_eq!(submission.submitted_by, auth::dev_stub::DEV_CONSULTANT_ID);
    }

    #[tokio::test]
    async fn submit_observation_allows_an_omitted_related_company_reference() {
        let mock_gateway = Arc::new(default_mock_landscape_gateway());
        let (router, cookie, _container) = test_app(vec!["landscape"], mock_gateway.clone()).await;

        let request =
            post_request(&cookie, "/api/landscape/observations", json!({ "observation_text": "General market shift." }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let submission = mock_gateway.last_submission.lock().unwrap().clone().expect("submission recorded");
        assert_eq!(submission.related_company_reference, None);
    }

    #[tokio::test]
    async fn submit_observation_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_landscape_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/landscape/observations", json!({ "observation_text": "Something." }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn submit_observation_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockLandscapeGateway {
            digest_outcome: Outcome::Ok(vec![]),
            submit_outcome: Outcome::Err,
            call_count: AtomicUsize::new(0),
            last_submission: std::sync::Mutex::new(None),
        });
        let (router, cookie, _container) = test_app(vec!["landscape"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/landscape/observations", json!({ "observation_text": "Something." }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
