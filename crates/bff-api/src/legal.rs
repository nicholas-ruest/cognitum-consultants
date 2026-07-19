//! `GET /api/legal/clauses?proposal_id={id}` / `?topic={topic}` (PROMPT-41,
//! ADR-009 permission gate, ADR-016 resilience stack,
//! `../../.plans/ddd/anti-corruption-layers.md` §9).
//!
//! One session-gated route over [`nexus_client::LegalGateway`], following
//! [`crate::customer`]'s exact handler pattern (see that module's docs —
//! this one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own `Vec<`[`nexus_client::ApprovedLegalSnippet`]`>` on success,
//! `502` on gateway failure.
//!
//! # `proposal_id` xor `topic`, never neither
//! `anti-corruption-layers.md` §9 names exactly one outbound query shape,
//! `RequestApprovedClausesQuery { context: proposal_id | topic }` — an
//! either/or (see `nexus_client::legal`'s module docs for why
//! [`nexus_client::ClauseContext`] models that as a two-variant enum, not
//! two independent optional fields). This handler resolves the incoming
//! `?proposal_id=`/`?topic=` query params into that enum, `400`ing if
//! neither is present (mirroring `crate::commit::create_proposal`'s "either
//! `origin_workflow_session_id` or `origin_reference` is required" `400`
//! convention) — it does not silently prefer one over the other or invent a
//! third combined shape with no worked example to match.
//!
//! # No two-gateway split
//! Unlike Sales/Commit/Capacity/Execution/Landscape, Legal has exactly one
//! outbound call ([`nexus_client::LegalGateway::request_approved_clauses`])
//! and no side-effecting command — see `nexus_client::legal`'s module docs
//! for why [`session::AppState`] therefore carries a single
//! [`session::AppState::legal_gateway`] field rather than a query/command
//! pair.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use nexus_client::ClauseContext;
use serde::Deserialize;
use serde_json::json;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating the route below (PROMPT-15/ADR-009).
const LEGAL_CAPABILITY: &str = "legal";

/// `GET /api/legal/clauses` query params — `proposal_id` and `topic` are
/// mutually exclusive (see the module docs); at least one must be present
/// and non-blank.
#[derive(Debug, Deserialize)]
pub struct ClausesQuery {
    #[serde(default)]
    pub proposal_id: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the legal capability")
}

/// `502`: the gateway call to Legal (via Nexus) failed — never coerced into
/// a synthetic success, same convention as `crate::customer::customer_unavailable`.
fn legal_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "legal service unavailable")
}

/// `resolve_clause_context`'s failure case — kept as a small marker type
/// (not the eventual `Response` itself) so its `Result` doesn't trip
/// clippy's `result_large_err` lint the way returning a whole `Response`
/// inline here would; the caller builds the actual `400` response.
struct MissingClauseContext;

/// Resolves `query` into a [`ClauseContext`] per the module docs' either/or
/// rule. `proposal_id` takes precedence when both happen to be present
/// (matching `crate::commit::resolve_origin_reference`'s "first non-blank
/// field wins" convention) — [`MissingClauseContext`] when neither is
/// present/non-blank.
fn resolve_clause_context(query: &ClausesQuery) -> Result<ClauseContext<'_>, MissingClauseContext> {
    match (query.proposal_id.as_deref().map(str::trim), query.topic.as_deref().map(str::trim)) {
        (Some(proposal_id), _) if !proposal_id.is_empty() => Ok(ClauseContext::ProposalId(proposal_id)),
        (_, Some(topic)) if !topic.is_empty() => Ok(ClauseContext::Topic(topic)),
        _ => Err(MissingClauseContext),
    }
}

/// `GET /api/legal/clauses`: checks permission, resolves the query context
/// (module docs), then calls
/// [`nexus_client::LegalGateway::request_approved_clauses`] via
/// [`AppState::legal_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::ApprovedLegalSnippet`]`>` **verbatim**.
pub async fn get_approved_clauses(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Query(query): Query<ClausesQuery>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, LEGAL_CAPABILITY).await {
        return forbidden();
    }

    let context = match resolve_clause_context(&query) {
        Ok(context) => context,
        Err(MissingClauseContext) => {
            return error_response(StatusCode::BAD_REQUEST, "either proposal_id or topic query parameter is required");
        }
    };

    match state.legal_gateway.request_approved_clauses(context).await {
        Ok(clauses) => Json(clauses).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "legal approved clauses fetch failed");
            legal_unavailable()
        }
    }
}

/// Builds the `/api/legal/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn legal_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/legal/clauses", get(get_approved_clauses))
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
        ApprovedLegalSnippet, ArmorGateway, ArmorGatewayError, ClauseContext, LegalGateway, LegalGatewayError,
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
            unimplemented!("legal tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("legal tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("legal tests never call the sales gateway")
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
            unimplemented!("legal tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("legal tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("legal tests never call the commit gateway")
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
            unimplemented!("legal tests never call the edu gateway")
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
            unimplemented!("legal tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("legal tests never call the capacity gateway")
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
            unimplemented!("legal tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("legal tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("legal tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("legal tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("legal tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("legal tests never call the landscape gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `LegalGateway`. Increments the shared `call_count`
    /// unconditionally so tests can assert the gateway was — or, per the
    /// permission-short-circuit test, was **never** — invoked. Records the
    /// last `ClauseContext` it was called with (as an owned copy) so tests
    /// can assert `proposal_id`/`topic` resolution.
    struct MockLegalGateway {
        clauses_outcome: Outcome<Vec<ApprovedLegalSnippet>>,
        call_count: AtomicUsize,
        last_context: std::sync::Mutex<Option<(bool, String)>>,
    }

    impl MockLegalGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> LegalGatewayError {
            LegalGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(5) })
        }
    }

    #[async_trait::async_trait]
    impl LegalGateway for MockLegalGateway {
        async fn request_approved_clauses(
            &self,
            context: ClauseContext<'_>,
        ) -> Result<Vec<ApprovedLegalSnippet>, LegalGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            *self.last_context.lock().unwrap() = Some(match context {
                ClauseContext::ProposalId(id) => (true, id.to_owned()),
                ClauseContext::Topic(topic) => (false, topic.to_owned()),
            });
            match &self.clauses_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn clause_fixture() -> ApprovedLegalSnippet {
        ApprovedLegalSnippet {
            clause_id: "clause-1".to_owned(),
            title: "Limitation of Liability".to_owned(),
            approved_text: "Neither party shall be liable for...".to_owned(),
            policy_reference: "policy-2026-01".to_owned(),
        }
    }

    fn default_mock_legal_gateway() -> MockLegalGateway {
        MockLegalGateway {
            clauses_outcome: Outcome::Ok(vec![clause_fixture()]),
            call_count: AtomicUsize::new(0),
            last_context: std::sync::Mutex::new(None),
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
        mock_legal_gateway: Arc<MockLegalGateway>,
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
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            execution_query_gateway: Arc::new(UnusedExecutionGateway),
            execution_command_gateway: Arc::new(UnusedExecutionGateway),
            products_gateway: Arc::new(UnusedProductsGateway),
            landscape_query_gateway: Arc::new(UnusedLandscapeGateway),
            landscape_command_gateway: Arc::new(UnusedLandscapeGateway),
            legal_gateway: mock_legal_gateway,
            workflow_session_repository,
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
            event_notify_publisher: Arc::new(bff_core::EventBus::default()),
            google_identity_verifier: Arc::new(auth::google_identity_token::GoogleIdentityTokenVerifier::new(
                "test-audience".to_owned(),
                None,
            )),
        };

        let router = Router::new().nest("/api", legal_router(state.clone())).with_state(state);
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
    async fn get_approved_clauses_relays_the_clauses_verbatim_when_permitted_by_proposal_id() {
        let mock_gateway = Arc::new(default_mock_legal_gateway());
        let (router, cookie, _container) = test_app(vec!["legal"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/legal/clauses?proposal_id=proposal-1")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(
            body,
            json!([{
                "clause_id": "clause-1",
                "title": "Limitation of Liability",
                "approved_text": "Neither party shall be liable for...",
                "policy_reference": "policy-2026-01",
            }])
        );
        assert_eq!(mock_gateway.calls(), 1);
        assert_eq!(*mock_gateway.last_context.lock().unwrap(), Some((true, "proposal-1".to_owned())));
    }

    #[tokio::test]
    async fn get_approved_clauses_resolves_a_topic_query_when_no_proposal_id_is_present() {
        let mock_gateway = Arc::new(default_mock_legal_gateway());
        let (router, cookie, _container) = test_app(vec!["legal"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/legal/clauses?topic=data-residency")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*mock_gateway.last_context.lock().unwrap(), Some((false, "data-residency".to_owned())));
    }

    #[tokio::test]
    async fn get_approved_clauses_requires_a_proposal_id_or_topic() {
        let mock_gateway = Arc::new(default_mock_legal_gateway());
        let (router, cookie, _container) = test_app(vec!["legal"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/legal/clauses")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn get_approved_clauses_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_legal_gateway());
        let (router, cookie, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/legal/clauses?proposal_id=proposal-1")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn get_approved_clauses_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockLegalGateway {
            clauses_outcome: Outcome::Err,
            call_count: AtomicUsize::new(0),
            last_context: std::sync::Mutex::new(None),
        });
        let (router, cookie, _container) = test_app(vec!["legal"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/legal/clauses?proposal_id=proposal-1")).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }
}
