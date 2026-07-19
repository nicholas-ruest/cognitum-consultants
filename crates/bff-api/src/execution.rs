//! `GET /api/execution/engagements`, `POST /api/execution/tasks/{id}/complete`
//! (PROMPT-38, ADR-009 permission gate, ADR-016 resilience stack,
//! `../../.plans/ddd/anti-corruption-layers.md` §6).
//!
//! Two session-gated routes over [`nexus_client::ExecutionGateway`],
//! following [`crate::commit`]'s exact handler pattern (see that module's
//! docs — this one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own DTO on success, `502` on gateway failure.
//!
//! # `complete_task` never touches `ActionQueueEntry` — the critical invariant
//! `TaskAssigned`/`DeliveryRiskRaised` events are ingested by
//! `bff_core::event_ingestion` into `ActionQueueEntry` rows
//! (`crate::event_ingestion`'s polling loop, PROMPT-30/38). Per
//! `consultant-experience-context.md` §2.2 invariant 3, this repo can never
//! locally decide the underlying business action is done — only a
//! confirmation event routed back through Nexus's ingestion pipeline may
//! call [`bff_core::ActionQueueEntry::complete`]/
//! [`bff_core::ActionQueueRepository::mark_completed`] — concretely,
//! `bff_core::event_ingestion::ingest_confirmation`, reached only via a
//! `task_completed`-classified event (see that module's
//! `CONFIRMATION_EVENT_TYPES` doc comment); it is the **only** call site of
//! `mark_completed` in this entire repo.
//! [`complete_task`] below is Execution's capability-specific half of
//! PROMPT-38's "route completion through the BFF back to Execution"
//! requirement: it forwards a *completion request* to Execution via
//! [`nexus_client::ExecutionGateway::confirm_task_completion`] and returns an
//! ack — it does **not**, and must never, call into
//! [`bff_core::ActionQueueRepository`] at all. A task's `ActionQueueEntry`
//! remains `pending`/`in_progress` in this repo's own view until Execution's
//! own confirmation event later arrives through the standard ingestion
//! pipeline (same "no `.../complete` route flips local state" convention
//! `crate::notifications`'s module docs establish generically — this route
//! adds the missing "ask the owning capability" half for Execution
//! specifically, which that generic module deliberately leaves capability-
//! agnostic and un-implemented).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating both routes below (PROMPT-15/ADR-009).
const EXECUTION_CAPABILITY: &str = "execution";

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the execution capability")
}

/// `502`: the gateway call to Execution (via Nexus) failed — never coerced
/// into a synthetic success, same convention as
/// `crate::commit::commit_unavailable`.
fn execution_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "execution service unavailable")
}

/// `GET /api/execution/engagements`: checks permission, then calls
/// [`nexus_client::ExecutionGateway::request_assigned_engagements`] via
/// [`AppState::execution_query_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::EngagementSnapshot`]`>` **verbatim**.
pub async fn list_assigned_engagements(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, EXECUTION_CAPABILITY).await {
        return forbidden();
    }

    match state.execution_query_gateway.request_assigned_engagements(&session.consultant_id).await {
        Ok(engagements) => Json(engagements).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "execution engagement fetch failed");
            execution_unavailable()
        }
    }
}

/// `POST /api/execution/tasks/{id}/complete`: checks permission, then calls
/// [`nexus_client::ExecutionGateway::confirm_task_completion`] via
/// [`AppState::execution_command_gateway`]. See the module docs — this
/// **never** mutates `ActionQueueEntry` state itself.
pub async fn complete_task(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(task_id): Path<String>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, EXECUTION_CAPABILITY).await {
        return forbidden();
    }

    match state.execution_command_gateway.confirm_task_completion(&task_id, &session.consultant_id).await {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                task_id = %task_id,
                "execution task completion request failed"
            );
            execution_unavailable()
        }
    }
}

/// Builds the `/api/execution/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn execution_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/execution/engagements", get(list_assigned_engagements))
        .route("/execution/tasks/{id}/complete", post(complete_task))
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
        ArmorGateway, ArmorGatewayError, EngagementSnapshot, EngagementTaskSummary, ExecutionGateway, ExecutionGatewayError,
        NexusTransportError, PermissionAssertion,
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
            unimplemented!("execution tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("execution tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("execution tests never call the sales gateway")
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
            unimplemented!("execution tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("execution tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("execution tests never call the commit gateway")
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
            unimplemented!("execution tests never call the edu gateway")
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
            unimplemented!("execution tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("execution tests never call the capacity gateway")
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
            unimplemented!("execution tests never call the customer gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("execution tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("execution tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("execution tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;

    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("execution tests never call the legal gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `ExecutionGateway`. Each method increments the shared
    /// `call_count` unconditionally so tests can assert the gateway was —
    /// or, per the permission-short-circuit tests, was **never** — invoked.
    struct MockExecutionGateway {
        engagements_outcome: Outcome<Vec<EngagementSnapshot>>,
        completion_outcome: Outcome<()>,
        call_count: AtomicUsize,
    }

    impl MockExecutionGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> ExecutionGatewayError {
            ExecutionGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(5) })
        }
    }

    #[async_trait::async_trait]
    impl ExecutionGateway for MockExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<EngagementSnapshot>, ExecutionGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.engagements_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn confirm_task_completion(&self, _task_id: &str, _consultant_id: &str) -> Result<(), ExecutionGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.completion_outcome {
                Outcome::Ok(()) => Ok(()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn engagement_fixture() -> EngagementSnapshot {
        EngagementSnapshot {
            engagement_id: "engagement-1".to_owned(),
            workstreams: vec!["Discovery".to_owned(), "Delivery".to_owned()],
            milestones: vec!["Kickoff complete".to_owned()],
            tasks: vec![EngagementTaskSummary {
                task_id: "task-1".to_owned(),
                title: "Draft delivery plan".to_owned(),
                status: "assigned".to_owned(),
            }],
            delivery_status: "on_track".to_owned(),
            deep_link: Some("https://execution.cognitum.one/engagements/engagement-1".to_owned()),
        }
    }

    fn default_mock_execution_gateway() -> MockExecutionGateway {
        MockExecutionGateway {
            engagements_outcome: Outcome::Ok(vec![engagement_fixture()]),
            completion_outcome: Outcome::Ok(()),
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
            firebase_project_id: None,
                nexus_caller_service_account_email: None,
        }
    }

    async fn test_app(
        capabilities: Vec<&'static str>,
        mock_execution_gateway: Arc<MockExecutionGateway>,
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

        let execution_query_gateway: Arc<dyn ExecutionGateway> = mock_execution_gateway.clone();
        let execution_command_gateway: Arc<dyn ExecutionGateway> = mock_execution_gateway;

        let state = AppState {
            db_pool: pool.clone(),
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
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            execution_query_gateway,
            execution_command_gateway,
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway: Arc::new(UnusedLandscapeGateway),
            landscape_command_gateway: Arc::new(UnusedLandscapeGateway),
            legal_gateway: Arc::new(UnusedLegalGateway),
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
            event_notify_publisher: Arc::new(bff_core::EventBus::default()),
            google_identity_verifier: Arc::new(auth::google_identity_token::GoogleIdentityTokenVerifier::new(
                "test-audience".to_owned(),
                None,
            )),
            prospect_repository: Arc::new(persistence::PgProspectRepository::new(pool.clone())),
            action_item_repository: Arc::new(persistence::PgConsultantActionItemRepository::new(pool.clone())),
        };

        let router = Router::new().nest("/api", execution_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn get_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("GET").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    fn post_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("POST").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_assigned_engagements_relays_the_snapshots_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_execution_gateway());
        let (router, cookie, _container) = test_app(vec!["execution"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/execution/engagements")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body,
            json!([{
                "engagement_id": "engagement-1",
                "workstreams": ["Discovery", "Delivery"],
                "milestones": ["Kickoff complete"],
                "tasks": [{"task_id": "task-1", "title": "Draft delivery plan", "status": "assigned"}],
                "delivery_status": "on_track",
                "deep_link": "https://execution.cognitum.one/engagements/engagement-1",
            }])
        );
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn list_assigned_engagements_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_execution_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/execution/engagements")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn list_assigned_engagements_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockExecutionGateway {
            engagements_outcome: Outcome::Err,
            completion_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["execution"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/execution/engagements")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn complete_task_forwards_the_request_and_acks_when_permitted() {
        let mock_gateway = Arc::new(default_mock_execution_gateway());
        let (router, cookie, _container) = test_app(vec!["execution"], mock_gateway.clone()).await;

        let response = router.oneshot(post_request(&cookie, "/api/execution/tasks/task-1/complete")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body, json!({ "status": "ok" }));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn complete_task_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_execution_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(post_request(&cookie, "/api/execution/tasks/task-1/complete")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn complete_task_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockExecutionGateway {
            engagements_outcome: Outcome::Ok(vec![]),
            completion_outcome: Outcome::Err,
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["execution"], mock_gateway.clone()).await;

        let response = router.oneshot(post_request(&cookie, "/api/execution/tasks/task-1/complete")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
