//! `GET /api/customer/assigned` (PROMPT-37, ADR-009 permission gate, ADR-016
//! resilience stack, `../../.plans/ddd/anti-corruption-layers.md` §5).
//!
//! One session-gated route over [`nexus_client::CustomerGateway`], following
//! [`crate::edu`]'s exact handler pattern (see that module's docs — this one
//! does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own `Vec<`[`nexus_client::CustomerContextCard`]`>` on success,
//! `502` on gateway failure.
//!
//! # No two-gateway split
//! Unlike Sales/Commit/Capacity, Customer has exactly one outbound call
//! ([`nexus_client::CustomerGateway::request_assigned_customer_context`])
//! and no side-effecting command — see `nexus_client::customer`'s module
//! docs for why [`session::AppState`] therefore carries a single
//! [`session::AppState::customer_gateway`] field rather than a
//! query/command pair.
//!
//! # Permission filtering at the query boundary, not post-fetch
//! This handler passes only `session.consultant_id` (never a caller-supplied
//! id) into [`nexus_client::CustomerGateway::request_assigned_customer_context`],
//! and always with `customer_id: None` — `GET /api/customer/assigned` lists
//! the *whole* assigned/permitted set, not one narrowed customer. Customer
//! (via Nexus) is the one that scopes the returned set to what this
//! consultant may see (`anti-corruption-layers.md` §5); this handler applies
//! no additional filtering of its own — there is nothing to filter, since it
//! never sees a broader set to begin with.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating the route below (PROMPT-15/ADR-009).
const CUSTOMER_CAPABILITY: &str = "customer";

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the customer capability")
}

/// `502`: the gateway call to Customer (via Nexus) failed — never coerced
/// into a synthetic success, same convention as
/// `crate::sales::sales_unavailable`/`crate::edu`'s `edu_unavailable`.
fn customer_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "customer service unavailable")
}

/// `GET /api/customer/assigned`: checks permission, then calls
/// [`nexus_client::CustomerGateway::request_assigned_customer_context`] via
/// [`AppState::customer_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::CustomerContextCard`]`>` **verbatim**.
pub async fn list_assigned_customers(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, CUSTOMER_CAPABILITY).await {
        return forbidden();
    }

    match state.customer_gateway.request_assigned_customer_context(&session.consultant_id, None).await {
        Ok(contexts) => Json(contexts).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "customer context fetch failed");
            customer_unavailable()
        }
    }
}

/// Builds the `/api/customer/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn customer_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/customer/assigned", get(list_assigned_customers))
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
        ArmorGateway, ArmorGatewayError, CustomerContextCard, CustomerGateway, CustomerGatewayError, NexusTransportError,
        PermissionAssertion,
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
            unimplemented!("customer tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("customer tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("customer tests never call the sales gateway")
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
            unimplemented!("customer tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("customer tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("customer tests never call the commit gateway")
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
            unimplemented!("customer tests never call the edu gateway")
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
            unimplemented!("customer tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("customer tests never call the capacity gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("customer tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("customer tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("customer tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("customer tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("customer tests never call the landscape gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `CustomerGateway`. Increments the shared `call_count`
    /// unconditionally so tests can assert the gateway was — or, per the
    /// permission-short-circuit test, was **never** — invoked.
    struct MockCustomerGateway {
        context_outcome: Outcome<Vec<CustomerContextCard>>,
        call_count: AtomicUsize,
    }

    impl MockCustomerGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> CustomerGatewayError {
            CustomerGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(5) })
        }
    }

    #[async_trait::async_trait]
    impl CustomerGateway for MockCustomerGateway {
        async fn request_assigned_customer_context(
            &self,
            _consultant_id: &str,
            _customer_id: Option<&str>,
        ) -> Result<Vec<CustomerContextCard>, CustomerGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.context_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn context_card_fixture() -> CustomerContextCard {
        CustomerContextCard {
            customer_id: "customer-1".to_owned(),
            name: "Acme Corp".to_owned(),
            health_status: "green".to_owned(),
            relationship_summary: "Healthy, quarterly business review scheduled.".to_owned(),
            deep_link: Some("https://customer.cognitum.one/customers/customer-1".to_owned()),
        }
    }

    fn default_mock_customer_gateway() -> MockCustomerGateway {
        MockCustomerGateway { context_outcome: Outcome::Ok(vec![context_card_fixture()]), call_count: AtomicUsize::new(0) }
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
        mock_customer_gateway: Arc<MockCustomerGateway>,
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
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: mock_customer_gateway,
            execution_query_gateway: Arc::new(UnusedExecutionGateway),
            execution_command_gateway: Arc::new(UnusedExecutionGateway),
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway: Arc::new(UnusedLandscapeGateway),
            landscape_command_gateway: Arc::new(UnusedLandscapeGateway),
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", customer_router(state.clone())).with_state(state);
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
    async fn list_assigned_customers_relays_the_context_cards_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_customer_gateway());
        let (router, cookie, _container) = test_app(vec!["customer"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/customer/assigned")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body,
            json!([{
                "customer_id": "customer-1",
                "name": "Acme Corp",
                "health_status": "green",
                "relationship_summary": "Healthy, quarterly business review scheduled.",
                "deep_link": "https://customer.cognitum.one/customers/customer-1",
            }])
        );
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn list_assigned_customers_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_customer_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/customer/assigned")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn list_assigned_customers_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockCustomerGateway { context_outcome: Outcome::Err, call_count: AtomicUsize::new(0) });
        let (router, cookie, _container) = test_app(vec!["customer"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/customer/assigned")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
