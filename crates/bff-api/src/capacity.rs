//! `GET /api/capacity/profile`, `PATCH /api/capacity/profile` (PROMPT-36,
//! ADR-009 permission gate, ADR-016 resilience stack,
//! `../../.plans/ddd/anti-corruption-layers.md` §4).
//!
//! Two session-gated routes over [`nexus_client::CapacityGateway`],
//! following [`crate::commit`]'s exact handler pattern (see that module's
//! docs — this one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own DTO on success, `502` on gateway failure.
//!
//! # Own-profile-only, by construction — not by a runtime check
//! Every call into [`nexus_client::CapacityGateway`] below passes
//! `session.consultant_id` — the id `axum`'s `require_session` middleware
//! resolved from the caller's own session cookie — as the *only* identifying
//! argument. Neither handler accepts, reads, or forwards any other
//! consultant id from the request (`GET /api/capacity/profile` takes no
//! query params at all; `PATCH /api/capacity/profile`'s body carries only
//! profile fields, no id). Combined with
//! [`nexus_client::CapacityGateway`]'s own trait shape (no method accepts a
//! second consultant id — see that module's docs), there is no code path
//! anywhere in this stack, from HTTP request to outbound Nexus call, that
//! could name a consultant other than the caller. PROMPT-36's "verify by
//! code review" acceptance criterion is satisfied by this file plus
//! `nexus_client::capacity`'s trait definition alone.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use nexus_client::ConsultantProfileIntake;
use serde::Deserialize;
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating both routes below (PROMPT-15/ADR-009).
const CAPACITY_CAPABILITY: &str = "capacity";

/// `PATCH /api/capacity/profile` request body — the restricted set of
/// fields Capacity's `ConsultantProfileIntake` accepts
/// (`anti-corruption-layers.md` §4). Deserialized into its own type (rather
/// than deserializing `ConsultantProfileIntake` directly) only so this
/// module owns its request-shape name independently of the gateway DTO —
/// the field set is otherwise identical, and [`From`] below performs a
/// direct, no-op-beyond-move conversion.
#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub skills: Vec<String>,
    pub certifications: Vec<String>,
    pub languages: Vec<String>,
    pub availability_window: String,
    pub geographic_coverage: Vec<String>,
}

impl From<UpdateProfileRequest> for ConsultantProfileIntake {
    fn from(request: UpdateProfileRequest) -> Self {
        ConsultantProfileIntake {
            skills: request.skills,
            certifications: request.certifications,
            languages: request.languages,
            availability_window: request.availability_window,
            geographic_coverage: request.geographic_coverage,
        }
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the capacity capability")
}

/// `502`: the gateway call to Capacity (via Nexus) failed — never coerced
/// into a synthetic success, same convention as
/// `crate::sales::sales_unavailable`/`crate::commit::commit_unavailable`.
fn capacity_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "capacity service unavailable")
}

/// `GET /api/capacity/profile`: checks permission, then calls
/// [`nexus_client::CapacityGateway::get_own_profile`] via
/// [`AppState::capacity_query_gateway`] and relays the resulting
/// [`nexus_client::ConsultantProfileIntake`] **verbatim**.
pub async fn get_profile(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, CAPACITY_CAPABILITY).await {
        return forbidden();
    }

    match state.capacity_query_gateway.get_own_profile(&session.consultant_id).await {
        Ok(profile) => Json(profile).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "capacity profile fetch failed");
            capacity_unavailable()
        }
    }
}

/// `PATCH /api/capacity/profile`: checks permission, then calls
/// [`nexus_client::CapacityGateway::update_own_profile`] via
/// [`AppState::capacity_command_gateway`] and relays the resulting
/// [`nexus_client::ProfileUpdateResult`] (accepted/rejected + reason)
/// **verbatim** — never re-adjudicated (see the module docs).
pub async fn update_profile(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<UpdateProfileRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, CAPACITY_CAPABILITY).await {
        return forbidden();
    }

    match state.capacity_command_gateway.update_own_profile(&session.consultant_id, body.into()).await {
        Ok(result) => Json(result).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "capacity profile update failed");
            capacity_unavailable()
        }
    }
}

/// Builds the `/api/capacity/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn capacity_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/capacity/profile", get(get_profile).patch(update_profile))
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
        ArmorGateway, ArmorGatewayError, CapacityGateway, CapacityGatewayError, ConsultantProfileIntake,
        NexusTransportError, PermissionAssertion, ProfileUpdateResult,
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
            unimplemented!("capacity tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("capacity tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("capacity tests never call the sales gateway")
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
            unimplemented!("capacity tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("capacity tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("capacity tests never call the commit gateway")
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
            unimplemented!("capacity tests never call the edu gateway")
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
            unimplemented!("capacity tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("capacity tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("capacity tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("capacity tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("capacity tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("capacity tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;

    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("capacity tests never call the legal gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `CapacityGateway`. Each method increments the shared
    /// `call_count` unconditionally so tests can assert the gateway was —
    /// or, per the permission-short-circuit tests, was **never** — invoked.
    struct MockCapacityGateway {
        update_outcome: Outcome<ProfileUpdateResult>,
        get_outcome: Outcome<ConsultantProfileIntake>,
        call_count: AtomicUsize,
    }

    impl MockCapacityGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> CapacityGatewayError {
            CapacityGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(3) })
        }
    }

    #[async_trait::async_trait]
    impl CapacityGateway for MockCapacityGateway {
        async fn update_own_profile(
            &self,
            _consultant_id: &str,
            _profile_fields: ConsultantProfileIntake,
        ) -> Result<ProfileUpdateResult, CapacityGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.update_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn get_own_profile(&self, _consultant_id: &str) -> Result<ConsultantProfileIntake, CapacityGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.get_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn profile_fixture() -> ConsultantProfileIntake {
        ConsultantProfileIntake {
            skills: vec!["Rust".to_owned()],
            certifications: vec!["AWS Solutions Architect".to_owned()],
            languages: vec!["English".to_owned()],
            availability_window: "2026-08-01/2026-12-31".to_owned(),
            geographic_coverage: vec!["EMEA".to_owned()],
        }
    }

    fn default_mock_capacity_gateway() -> MockCapacityGateway {
        MockCapacityGateway {
            update_outcome: Outcome::Ok(ProfileUpdateResult { accepted: true, reason: None }),
            get_outcome: Outcome::Ok(profile_fixture()),
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
        mock_capacity_gateway: Arc<MockCapacityGateway>,
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

        let capacity_query_gateway: Arc<dyn CapacityGateway> = mock_capacity_gateway.clone();
        let capacity_command_gateway: Arc<dyn CapacityGateway> = mock_capacity_gateway;

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
            capacity_query_gateway,
            capacity_command_gateway,
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
            event_notify_publisher: Arc::new(bff_core::EventBus::default()),
            google_identity_verifier: Arc::new(auth::google_identity_token::GoogleIdentityTokenVerifier::new(
                "test-audience".to_owned(),
                None,
            )),
            prospect_repository: Arc::new(persistence::PgProspectRepository::new(pool.clone())),
            action_item_repository: Arc::new(persistence::PgConsultantActionItemRepository::new(pool.clone())),
        };

        let router = Router::new().nest("/api", capacity_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, container)
    }

    fn get_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("GET").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    fn patch_request(cookie: &Cookie<'static>, path: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method("PATCH")
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

    fn update_body() -> Value {
        json!({
            "skills": ["Rust"],
            "certifications": ["AWS Solutions Architect"],
            "languages": ["English"],
            "availability_window": "2026-08-01/2026-12-31",
            "geographic_coverage": ["EMEA"]
        })
    }

    #[tokio::test]
    async fn get_profile_relays_the_profile_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_capacity_gateway());
        let (router, cookie, _container) = test_app(vec!["capacity"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/capacity/profile")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["skills"], json!(["Rust"]));
        assert_eq!(body["geographic_coverage"], json!(["EMEA"]));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn get_profile_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_capacity_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/capacity/profile")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn get_profile_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockCapacityGateway {
            update_outcome: Outcome::Ok(ProfileUpdateResult { accepted: true, reason: None }),
            get_outcome: Outcome::Err,
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["capacity"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/capacity/profile")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn update_profile_relays_an_accepted_result_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_capacity_gateway());
        let (router, cookie, _container) = test_app(vec!["capacity"], mock_gateway.clone()).await;

        let response = router.oneshot(patch_request(&cookie, "/api/capacity/profile", update_body())).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body, json!({ "accepted": true }));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn update_profile_relays_a_rejected_result_with_reason_verbatim() {
        let mock_gateway = Arc::new(MockCapacityGateway {
            update_outcome: Outcome::Ok(ProfileUpdateResult {
                accepted: false,
                reason: Some("availability_window overlaps an existing commitment".to_owned()),
            }),
            get_outcome: Outcome::Ok(profile_fixture()),
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["capacity"], mock_gateway.clone()).await;

        let response = router.oneshot(patch_request(&cookie, "/api/capacity/profile", update_body())).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body,
            json!({ "accepted": false, "reason": "availability_window overlaps an existing commitment" })
        );
    }

    #[tokio::test]
    async fn update_profile_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_capacity_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(patch_request(&cookie, "/api/capacity/profile", update_body())).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn update_profile_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockCapacityGateway {
            update_outcome: Outcome::Err,
            get_outcome: Outcome::Ok(profile_fixture()),
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _container) = test_app(vec!["capacity"], mock_gateway.clone()).await;

        let response = router.oneshot(patch_request(&cookie, "/api/capacity/profile", update_body())).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
