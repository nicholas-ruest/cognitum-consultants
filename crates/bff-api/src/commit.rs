//! `POST /api/commit/proposals`, `GET /api/commit/proposals`,
//! `POST /api/commit/proposals/{id}/actions` (PROMPT-34, ADR-009 permission
//! gate, ADR-016 resilience stack, `../../.plans/ddd/anti-corruption-layers.md`
//! §2).
//!
//! Three session-gated routes over [`nexus_client::CommitGateway`], following
//! [`crate::sales`]'s exact handler pattern (see that module's docs — this
//! one does not repeat the full rationale, only what differs):
//! permission-short-circuit before any gateway call, verbatim relay of the
//! gateway's own DTO on success, `502` on gateway failure.
//!
//! # `create_proposal`'s `CrossCapabilityWorkflowSession` hand-off
//! `POST /api/commit/proposals` accepts an optional
//! `origin_workflow_session_id` (PROMPT-22's `CrossCapabilityWorkflowSession`,
//! PROMPT-34's first real BFF consumer of it — see
//! `crate::workflow_sessions` module docs for the other half, session
//! creation). When present:
//! 1. The session is looked up by id and its ownership verified
//!    (`session.consultant_id() == this request's authenticated consultant`)
//!    — an unknown id or one belonging to a different consultant is
//!    rejected `404`, the same "don't distinguish not-yours from
//!    doesn't-exist" convention `crate::notifications` already established.
//! 2. Its expiry is checked (`is_expired`) — an expired session is rejected
//!    `409 Conflict` (the session's own state, not this request's shape, is
//!    what's wrong) rather than silently falling back to
//!    `body.origin_reference`, so a consultant with a stale deep link gets
//!    an honest "this hand-off expired" signal instead of a confusing
//!    substitution.
//! 3. Its `origin_reference` becomes `CreateProposalCommand.origin_reference`
//!    — **never** `body.origin_reference`, even if both are present, per
//!    invariant 1 (`bff_core::CrossCapabilityWorkflowSession`'s doc
//!    comment): the session, once resolved, is the authoritative origin
//!    reference for this hand-off.
//! 4. On a successful `create_proposal`, the session is transitioned forward
//!    (`Started -> InProgress`, a no-op transition attempt is skipped if
//!    it's already `InProgress`) and its `target_reference` is set to the
//!    newly created `proposal_id`, then persisted. A failure to persist this
//!    bookkeeping is logged but does **not** fail the request — the
//!    proposal was already created successfully in Commit at that point
//!    (invariant 4: this aggregate's own state is never what gates whether
//!    the target capability's mutation succeeded).
//!
//! When `origin_workflow_session_id` is absent, `body.origin_reference` is
//! used directly (the "just create a proposal, no cross-capability hand-off"
//! path) — `400` if neither is present.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use bff_core::{CrossCapabilityWorkflowSession, WorkflowSessionStatus};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::session::{self, AppState};
use auth::Session;

/// Capability name gating all three routes below (PROMPT-15/ADR-009).
const COMMIT_CAPABILITY: &str = "commit";

/// `POST /api/commit/proposals` request body. See the module docs for the
/// `origin_workflow_session_id`-vs-`origin_reference` precedence rule.
#[derive(Debug, Deserialize)]
pub struct CreateProposalRequest {
    #[serde(default)]
    pub origin_workflow_session_id: Option<Uuid>,
    #[serde(default)]
    pub origin_reference: Option<String>,
}

/// `POST /api/commit/proposals/{id}/actions` request body.
#[derive(Debug, Deserialize)]
pub struct RequestProposalActionRequest {
    pub action: String,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "not permitted for the commit capability")
}

/// `502`: the gateway call to Commit (via Nexus) failed — never coerced
/// into a synthetic success, same convention as `crate::sales::sales_unavailable`.
fn commit_unavailable() -> Response {
    error_response(StatusCode::BAD_GATEWAY, "commit service unavailable")
}

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "workflow session not found")
}

fn session_expired() -> Response {
    error_response(StatusCode::CONFLICT, "workflow session has expired")
}

/// Resolves `body`'s origin reference per the module docs' precedence rule.
/// Returns the resolved reference plus the loaded, still-owned session (if
/// any) so the caller can transition it forward after a successful
/// `create_proposal` call. `Err(Response)` short-circuits the handler with
/// the appropriate status.
async fn resolve_origin_reference(
    state: &AppState,
    session: &Session,
    body: &CreateProposalRequest,
) -> Result<(String, Option<CrossCapabilityWorkflowSession>), Response> {
    let Some(workflow_session_id) = body.origin_workflow_session_id else {
        return match &body.origin_reference {
            Some(reference) if !reference.trim().is_empty() => Ok((reference.clone(), None)),
            _ => Err(error_response(
                StatusCode::BAD_REQUEST,
                "either origin_workflow_session_id or origin_reference is required",
            )),
        };
    };

    let found = match state.workflow_session_repository.find_by_id(workflow_session_id).await {
        Ok(found) => found,
        Err(err) => {
            tracing::error!(error = %err, workflow_session_id = %workflow_session_id, "workflow session lookup failed");
            return Err(error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load workflow session"));
        }
    };

    match found {
        Some(workflow_session) if workflow_session.consultant_id() == session.consultant_id => {
            if workflow_session.is_expired(Utc::now()) {
                return Err(session_expired());
            }
            let origin_reference = workflow_session.origin_reference().to_owned();
            Ok((origin_reference, Some(workflow_session)))
        }
        _ => Err(not_found()),
    }
}

/// Transitions `workflow_session` forward and records `proposal_id` as its
/// `target_reference`, per the module docs' step 4. Persistence/transition
/// failures are logged, never surfaced to the caller — the proposal already
/// exists in Commit by the time this runs.
async fn advance_workflow_session(state: &AppState, mut workflow_session: CrossCapabilityWorkflowSession, proposal_id: &str) {
    let now = Utc::now();

    if workflow_session.status() == WorkflowSessionStatus::Started
        && let Err(err) = workflow_session.transition_to(WorkflowSessionStatus::InProgress, now)
    {
        tracing::error!(error = %err, session_id = %workflow_session.session_id(), "workflow session transition failed");
        return;
    }

    if let Err(err) = workflow_session.set_target_reference(proposal_id, now) {
        tracing::error!(error = %err, session_id = %workflow_session.session_id(), "workflow session target_reference update failed");
        return;
    }

    if let Err(err) = state.workflow_session_repository.save(&workflow_session).await {
        tracing::error!(error = %err, session_id = %workflow_session.session_id(), "workflow session save failed");
    }
}

/// `POST /api/commit/proposals`: checks permission, resolves the origin
/// reference (module docs), then calls
/// [`nexus_client::CommitGateway::create_proposal`] via
/// [`AppState::commit_command_gateway`] and relays the resulting
/// [`nexus_client::ProposalSummary`] **verbatim**.
pub async fn create_proposal(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<CreateProposalRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, COMMIT_CAPABILITY).await {
        return forbidden();
    }

    let (origin_reference, workflow_session) = match resolve_origin_reference(&state, &session, &body).await {
        Ok(resolved) => resolved,
        Err(response) => return response,
    };

    match state.commit_command_gateway.create_proposal(&origin_reference, &session.consultant_id).await {
        Ok(summary) => {
            if let Some(workflow_session) = workflow_session {
                advance_workflow_session(&state, workflow_session, &summary.proposal_id).await;
            }
            Json(summary).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "commit proposal creation failed");
            commit_unavailable()
        }
    }
}

/// `GET /api/commit/proposals`: checks permission, then calls
/// [`nexus_client::CommitGateway::list_proposals`] via
/// [`AppState::commit_query_gateway`] and relays the resulting
/// `Vec<`[`nexus_client::ProposalSummary`]`>` **verbatim**.
pub async fn list_proposals(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, COMMIT_CAPABILITY).await {
        return forbidden();
    }

    match state.commit_query_gateway.list_proposals(&session.consultant_id).await {
        Ok(proposals) => Json(proposals).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "commit proposal list failed");
            commit_unavailable()
        }
    }
}

/// `POST /api/commit/proposals/{id}/actions`: checks permission, then calls
/// [`nexus_client::CommitGateway::request_proposal_action`] via
/// [`AppState::commit_command_gateway`].
pub async fn request_proposal_action(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(proposal_id): Path<String>,
    Json(body): Json<RequestProposalActionRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, COMMIT_CAPABILITY).await {
        return forbidden();
    }

    match state.commit_command_gateway.request_proposal_action(&proposal_id, &body.action).await {
        Ok(()) => Json(json!({ "status": "ok" })).into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                consultant_id = %session.consultant_id,
                proposal_id = %proposal_id,
                "commit proposal action request failed"
            );
            commit_unavailable()
        }
    }
}

/// Builds the `/api/commit/*` sub-router, with the same
/// [`session::require_session`] middleware [`session::protected_router`]
/// applies to every other protected route in this repo.
pub fn commit_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/commit/proposals", post(create_proposal).get(list_proposals))
        .route("/commit/proposals/{id}/actions", post(request_proposal_action))
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
    use bff_core::{DashboardConfigurationRepository, WorkflowSessionRepository};
    use chrono::{Duration as ChronoDuration, Utc};
    use nexus_client::{ArmorGateway, ArmorGatewayError, CommitGateway, CommitGatewayError, NexusTransportError, PermissionAssertion, ProposalSummary};
    use persistence::{PgDashboardConfigurationRepository, PgWorkflowSessionRepository};
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
            unimplemented!("commit tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("commit tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("commit tests never call the sales gateway")
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
            unimplemented!("commit tests never call the edu gateway")
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
            unimplemented!("commit tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("commit tests never call the capacity gateway")
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
            unimplemented!("commit tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("commit tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("commit tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("commit tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("commit tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("commit tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;

    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("commit tests never call the legal gateway")
        }
    }

    enum Outcome<T> {
        Ok(T),
        Err,
    }

    /// Test-double `CommitGateway`. Each method increments the shared
    /// `call_count` unconditionally so tests can assert the gateway was —
    /// or, per the permission-short-circuit tests, was **never** — invoked.
    struct MockCommitGateway {
        create_outcome: Outcome<ProposalSummary>,
        list_outcome: Outcome<Vec<ProposalSummary>>,
        action_outcome: Outcome<()>,
        call_count: AtomicUsize,
    }

    impl MockCommitGateway {
        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn gateway_error() -> CommitGatewayError {
            CommitGatewayError::Transport(NexusTransportError::Timeout { after: Duration::from_secs(3) })
        }
    }

    #[async_trait::async_trait]
    impl CommitGateway for MockCommitGateway {
        async fn create_proposal(
            &self,
            _origin_reference: &str,
            _consultant_id: &str,
        ) -> Result<ProposalSummary, CommitGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.create_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn list_proposals(&self, _consultant_id: &str) -> Result<Vec<ProposalSummary>, CommitGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.list_outcome {
                Outcome::Ok(result) => Ok(result.clone()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }

        async fn request_proposal_action(&self, _proposal_id: &str, _action: &str) -> Result<(), CommitGatewayError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            match &self.action_outcome {
                Outcome::Ok(()) => Ok(()),
                Outcome::Err => Err(Self::gateway_error()),
            }
        }
    }

    fn proposal_fixture() -> ProposalSummary {
        ProposalSummary {
            proposal_id: "proposal-1".to_owned(),
            title: "Acme Corp Engagement Proposal".to_owned(),
            status: "draft".to_owned(),
            stage: "drafting".to_owned(),
            last_updated_at: Utc::now(),
            deep_link: Some("https://commit.cognitum.one/proposals/proposal-1".to_owned()),
        }
    }

    fn default_mock_commit_gateway() -> MockCommitGateway {
        MockCommitGateway {
            create_outcome: Outcome::Ok(proposal_fixture()),
            list_outcome: Outcome::Ok(vec![proposal_fixture()]),
            action_outcome: Outcome::Ok(()),
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

    async fn test_app(
        capabilities: Vec<&'static str>,
        mock_commit_gateway: Arc<MockCommitGateway>,
    ) -> (Router<()>, Cookie<'static>, AppState, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
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
        let workflow_session_repository: Arc<dyn WorkflowSessionRepository> =
            Arc::new(PgWorkflowSessionRepository::new(pool.clone()));

        let commit_query_gateway: Arc<dyn CommitGateway> = mock_commit_gateway.clone();
        let commit_command_gateway: Arc<dyn CommitGateway> = mock_commit_gateway;

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
            commit_query_gateway,
            commit_command_gateway,
            edu_gateway: Arc::new(UnusedEduGateway),
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

        let router = Router::new().nest("/api", commit_router(state.clone())).with_state(state.clone());
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, state, container)
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

    fn get_request(cookie: &Cookie<'static>, path: &str) -> Request<Body> {
        Request::builder().method("GET").uri(path).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_proposal_relays_the_proposal_summary_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals", json!({ "origin_reference": "acme-corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["proposal_id"], json!("proposal-1"));
        assert_eq!(body["status"], json!("draft"));
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn create_proposal_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec![], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals", json!({ "origin_reference": "acme-corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0, "the 403 short-circuit must happen before any gateway call");
    }

    #[tokio::test]
    async fn create_proposal_requires_an_origin_reference_or_workflow_session_id() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals", json!({}));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn create_proposal_never_returns_a_synthetic_success_when_the_gateway_errors() {
        let mock_gateway = Arc::new(MockCommitGateway {
            create_outcome: Outcome::Err,
            list_outcome: Outcome::Ok(vec![]),
            action_outcome: Outcome::Ok(()),
            call_count: AtomicUsize::new(0),
        });
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals", json!({ "origin_reference": "acme-corp" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn list_proposals_relays_the_list_verbatim_when_permitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/commit/proposals")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn list_proposals_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec![], mock_gateway.clone()).await;

        let response = router.oneshot(get_request(&cookie, "/api/commit/proposals")).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn request_proposal_action_succeeds_when_permitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals/proposal-1/actions", json!({ "action": "resend" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock_gateway.calls(), 1);
    }

    #[tokio::test]
    async fn request_proposal_action_returns_403_and_never_calls_the_gateway_when_unpermitted() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec![], mock_gateway.clone()).await;

        let request = post_request(&cookie, "/api/commit/proposals/proposal-1/actions", json!({ "action": "resend" }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(mock_gateway.calls(), 0);
    }

    // --- CrossCapabilityWorkflowSession hand-off (PROMPT-22/34) -----------

    #[tokio::test]
    async fn create_proposal_uses_the_workflow_sessions_origin_reference_and_advances_it() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let now = Utc::now();
        let workflow_session = CrossCapabilityWorkflowSession::start(
            auth::dev_stub::DEV_CONSULTANT_ID,
            "sales",
            "acme-corp-lead-42",
            "commit",
            now,
        )
        .unwrap();
        state.workflow_session_repository.save(&workflow_session).await.unwrap();

        let request = post_request(
            &cookie,
            "/api/commit/proposals",
            json!({ "origin_workflow_session_id": workflow_session.session_id() }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(mock_gateway.calls(), 1);

        let reloaded =
            state.workflow_session_repository.find_by_id(workflow_session.session_id()).await.unwrap().unwrap();
        assert_eq!(reloaded.status(), WorkflowSessionStatus::InProgress);
        assert_eq!(reloaded.target_reference(), Some("proposal-1"));
    }

    #[tokio::test]
    async fn create_proposal_with_an_unknown_workflow_session_id_is_rejected_with_404() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, _state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let request =
            post_request(&cookie, "/api/commit/proposals", json!({ "origin_workflow_session_id": Uuid::new_v4() }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn create_proposal_with_another_consultants_workflow_session_is_rejected_with_404() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let workflow_session =
            CrossCapabilityWorkflowSession::start("someone-else", "sales", "acme-corp-lead-42", "commit", Utc::now())
                .unwrap();
        state.workflow_session_repository.save(&workflow_session).await.unwrap();

        let request = post_request(
            &cookie,
            "/api/commit/proposals",
            json!({ "origin_workflow_session_id": workflow_session.session_id() }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(mock_gateway.calls(), 0);
    }

    #[tokio::test]
    async fn create_proposal_with_an_expired_workflow_session_is_rejected_with_409() {
        let mock_gateway = Arc::new(default_mock_commit_gateway());
        let (router, cookie, state, _container) = test_app(vec!["commit"], mock_gateway.clone()).await;

        let expired = CrossCapabilityWorkflowSession::from_parts(
            Uuid::new_v4(),
            auth::dev_stub::DEV_CONSULTANT_ID.to_owned(),
            "sales".to_owned(),
            "acme-corp-lead-42".to_owned(),
            "commit".to_owned(),
            None,
            WorkflowSessionStatus::Started,
            Utc::now() - ChronoDuration::minutes(1),
        )
        .unwrap();
        state.workflow_session_repository.save(&expired).await.unwrap();

        let request =
            post_request(&cookie, "/api/commit/proposals", json!({ "origin_workflow_session_id": expired.session_id() }));
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(mock_gateway.calls(), 0);
    }
}
