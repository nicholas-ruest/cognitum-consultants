//! `GET /api/edu/catalog` (PROMPT-35, ADR-009 permission gate, ADR-016
//! resilience stack, `../../.plans/ddd/anti-corruption-layers.md` §3).
//!
//! One session-gated route over [`nexus_client::EduGateway`], following
//! [`crate::commit`]'s exact handler pattern (see that module's docs — this
//! one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own `Vec<`[`nexus_client::LearningSnapshot`]`>` on success,
//! `502` on gateway failure.
//!
//! # No two-gateway split
//! Unlike Sales/Commit, Edu has exactly one outbound call
//! ([`nexus_client::EduGateway::request_learning_catalog`]) and no
//! side-effecting command — see `nexus_client::edu`'s module docs for why
//! [`session::AppState`] therefore carries a single
//! [`session::AppState::edu_gateway`] field rather than a
//! query/command pair.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating the route below (PROMPT-15/ADR-009).
const EDU_CAPABILITY: &str = "edu";

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the edu capability")
}

/// `502`: the gateway call to Edu (via Nexus) failed — never coerced into a
/// synthetic success, same convention as `crate::sales::sales_unavailable`/
/// `crate::commit::commit_unavailable`.
fn edu_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "edu service unavailable")
}

/// `GET /api/edu/catalog`: checks permission, then calls
/// [`nexus_client::EduGateway::request_learning_catalog`] via
/// [`AppState::edu_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::LearningSnapshot`]`>` **verbatim**.
pub async fn get_catalog(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, EDU_CAPABILITY).await {
        return forbidden();
    }

    match state.edu_gateway.request_learning_catalog(&session.consultant_id, None).await {
        Ok(snapshots) => Json(snapshots).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "edu learning catalog fetch failed");
            edu_unavailable()
        }
    }
}

/// Builds the `/api/edu/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn edu_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/edu/catalog", get(get_catalog))
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
    use nexus_client::{ArmorGateway, ArmorGatewayError, EduGateway, EduGatewayError, LearningSnapshot, NexusTransportError, PermissionAssertion};
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
            unimplemented!("edu tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("edu tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("edu tests never call the sales gateway")
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
            unimplemented!("edu tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("edu tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("edu tests never call the commit gateway")
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
            unimplemented!("edu tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("edu tests never call the capacity gateway")
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
            unimplemented!("edu tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("edu tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("edu tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("edu tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("edu tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("edu tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;

    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("edu tests never call the legal gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `EduGateway`. Increments the shared `call_count`
    /// unconditionally so tests can assert the gateway was — or, per the
    /// permission-short-circuit test, was **never** — invoked.
    struct MockEduGateway {
        catalog_outcome: Outcome<Vec<LearningSnapshot>>,
        call_count: AtomicUsize,
    }

    impl MockEduGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> EduGatewayError {
            EduGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(10) })
        }
    }

    #[async_trait::async_trait]
    impl EduGateway for MockEduGateway {
        async fn request_learning_catalog(
            &self,
            _consultant_id: &str,
            _filters: Option<&[String]>,
        ) -> Result<Vec<LearningSnapshot>, EduGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.catalog_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn snapshot_fixture() -> LearningSnapshot {
        LearningSnapshot {
            course_id: "course-1".to_owned(),
            title: "Cloud Security Fundamentals".to_owned(),
            progress_status: "completed".to_owned(),
            certification_status: Some("issued".to_owned()),
            deep_link: Some("https://edu.cognitum.one/courses/course-1".to_owned()),
        }
    }

    fn default_mock_edu_gateway() -> MockEduGateway {
        MockEduGateway { catalog_outcome: Outcome::Ok(vec![snapshot_fixture()]), call_count: AtomicUsize::new(0) }
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
            firebase_project_id: None,
        }
    }

    async fn test_app(
        capabilities: Vec<&'static str>,
        mock_edu_gateway: Arc<MockEduGateway>,
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
            dev_session_provider: Some(dev_session_provider),
            firebase_session_provider: None,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache,
            dashboard_repository,
            sales_query_gateway: Arc::new(UnusedSalesGateway),
            sales_command_gateway: Arc::new(UnusedSalesGateway),
            commit_query_gateway: Arc::new(UnusedCommitGateway),
            commit_command_gateway: Arc::new(UnusedCommitGateway),
            edu_gateway: mock_edu_gateway,
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            execution_query_gateway: Arc::new(UnusedExecutionGateway),
            execution_command_gateway: Arc::new(UnusedExecutionGateway),
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway: Arc::new(UnusedLandscapeGateway),
            landscape_command_gateway: Arc::new(UnusedLandscapeGateway),
            legal_gateway: Arc::new(UnusedLegalGateway),
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", edu_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn get_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("GET").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn get_catalog_relays_the_learning_snapshots_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_edu_gateway());
        let (router, cookie, _container) = test_app(vec!["edu"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/edu/catalog")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body,
            json!([{
                "course_id": "course-1",
                "title": "Cloud Security Fundamentals",
                "progress_status": "completed",
                "certification_status": "issued",
                "deep_link": "https://edu.cognitum.one/courses/course-1",
            }])
        );
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn get_catalog_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_edu_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/edu/catalog")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn get_catalog_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockEduGateway { catalog_outcome: Outcome::Err, call_count: AtomicUsize::new(0) });
        let (router, cookie, _container) = test_app(vec!["edu"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/edu/catalog")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
