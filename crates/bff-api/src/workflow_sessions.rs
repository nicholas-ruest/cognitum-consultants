//! `POST /api/workflow-sessions` (PROMPT-34).
//!
//! [`bff_core::CrossCapabilityWorkflowSession`] (PROMPT-22) had no BFF route
//! at all until this unit — PROMPT-22 only built the aggregate and its
//! `WorkflowSessionRepository` port/Postgres implementation. This module is
//! the first real consumer: a single, minimal, session-gated route that
//! starts a new hand-off session, for the Sales -> Commit deep link
//! (`frontend/src/features/sales/LeadConflictCheck.tsx`'s "Start Proposal"
//! affordance) to call before navigating to the Commit feature module. See
//! `crate::commit`'s module docs for the other half of the hand-off —
//! `POST /api/commit/proposals`'s `origin_workflow_session_id` consumption.
//!
//! # Why gate on the *origin* capability's permission
//! [`create_workflow_session`] checks `is_permitted(consultant_id,
//! origin_capability)` — not the target capability's — because *starting* a
//! hand-off is an action taken from within the origin capability's own UI
//! (e.g. Sales' lead-conflict flow); a consultant needs to be permitted for
//! Sales to record "I'm starting a hand-off out of Sales", regardless of
//! whether they'll ultimately be permitted for the target. The target
//! capability's own permission is independently re-checked when the target
//! is actually engaged (e.g. `crate::commit::create_proposal`'s own
//! `is_permitted(consultant_id, "commit")` gate) — this route never
//! substitutes for that.
//!
//! # No `GET`/detail route yet
//! Only `POST` exists here: the one real consumer this unit ships
//! (`LeadConflictCheck.tsx`) only ever needs the freshly-created
//! `session_id` back from the `POST` response itself to build its
//! `?workflow_session_id=...` deep link — nothing in this codebase yet
//! reads a session back by id over HTTP. Add `GET
//! /api/workflow-sessions/{id}` when a real consumer needs it, rather than
//! speculatively building an unused route now.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use bff_core::CrossCapabilityWorkflowSession;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::{self, AppState};
use auth::Session;

/// `POST /api/workflow-sessions` request body.
#[derive(Debug, Deserialize)]
pub struct CreateWorkflowSessionRequest {
    pub origin_capability: String,
    pub origin_reference: String,
    pub target_capability: String,
}

/// `POST /api/workflow-sessions` response body.
#[derive(Debug, Serialize)]
pub struct WorkflowSessionResponse {
    pub session_id: Uuid,
    pub status: String,
    pub expires_at: DateTime<Utc>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn forbidden(capability: &str) -> Response {
    error_response(StatusCode::FORBIDDEN, format!("not permitted for the {capability} capability"))
}

fn to_response(workflow_session: &CrossCapabilityWorkflowSession) -> WorkflowSessionResponse {
    WorkflowSessionResponse {
        session_id: workflow_session.session_id(),
        status: workflow_session.status().as_str().to_owned(),
        expires_at: workflow_session.expires_at(),
    }
}

/// `POST /api/workflow-sessions`: checks permission against
/// `body.origin_capability` (see the module docs), starts a fresh
/// [`CrossCapabilityWorkflowSession`] (`Started`, TTL from now), persists
/// it, and returns its id/status/expiry.
pub async fn create_workflow_session(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<CreateWorkflowSessionRequest>,
) -> Response {
    if !state.permission_cache.is_permitted(&session.consultant_id, &body.origin_capability).await {
        return forbidden(&body.origin_capability);
    }

    let workflow_session = match CrossCapabilityWorkflowSession::start(
        session.consultant_id.clone(),
        body.origin_capability,
        body.origin_reference,
        body.target_capability,
        Utc::now(),
    ) {
        Ok(workflow_session) => workflow_session,
        Err(err) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
    };

    if let Err(err) = state.workflow_session_repository.save(&workflow_session).await {
        tracing::error!(error = %err, consultant_id = %session.consultant_id, "workflow session save failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to save workflow session");
    }

    Json(to_response(&workflow_session)).into_response()
}

/// Builds the `/api/workflow-sessions` sub-router, with the same
/// [`session::require_session`] middleware every other protected route in
/// this crate applies.
pub fn workflow_sessions_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/workflow-sessions", post(create_workflow_session))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::Body;
    use axum::http::Request;
    use axum_extra::extract::cookie::Cookie;
    use bff_core::WorkflowSessionRepository;
    use chrono::Duration as ChronoDuration;
    use nexus_client::{ArmorGateway, ArmorGatewayError, PermissionAssertion};
    use persistence::PgWorkflowSessionRepository;
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
            unimplemented!("workflow_sessions tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("workflow_sessions tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("workflow_sessions tests never call the sales gateway")
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
            unimplemented!("workflow_sessions tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("workflow_sessions tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("workflow_sessions tests never call the commit gateway")
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
            unimplemented!("workflow_sessions tests never call the edu gateway")
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
            unimplemented!("workflow_sessions tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("workflow_sessions tests never call the capacity gateway")
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
            unimplemented!("workflow_sessions tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;

    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("workflow_sessions tests never call the execution gateway")
        }

        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("workflow_sessions tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;

    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("workflow_sessions tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;

    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("workflow_sessions tests never call the landscape gateway")
        }

        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("workflow_sessions tests never call the landscape gateway")
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
    ) -> (Router<()>, Cookie<'static>, AppState, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let (pool, container) = migrated_pool().await;

        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();
        let session = dev_session_provider.create_dev_session().await.expect("create_dev_session failed");

        let armor_gateway: Arc<dyn ArmorGateway> = Arc::new(MockArmorGateway { capabilities });
        let permission_cache = Arc::new(PermissionCache::new(armor_gateway));

        let workflow_session_repository: Arc<dyn WorkflowSessionRepository> =
            Arc::new(PgWorkflowSessionRepository::new(pool.clone()));

        let state = AppState {
            db_pool: pool.clone(),
            session_provider,
            dev_session_provider,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache,
            dashboard_repository: Arc::new(persistence::PgDashboardConfigurationRepository::new(pool.clone())),
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
            workflow_session_repository,
            notification_repository: Arc::new(persistence::PgNotificationRepository::new(pool.clone())),
            action_queue_repository: Arc::new(persistence::PgActionQueueRepository::new(pool.clone())),
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", workflow_sessions_router(state.clone())).with_state(state.clone());
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, state, container)
    }

    fn post_request(cookie: &Cookie<'static>, body: Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/workflow-sessions")
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
    async fn create_workflow_session_starts_and_persists_a_session_when_permitted() {
        let (router, cookie, state, _container) = test_app(vec!["sales"]).await;

        let request = post_request(
            &cookie,
            json!({ "origin_capability": "sales", "origin_reference": "acme-corp", "target_capability": "commit" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["status"], json!("started"));

        let session_id: Uuid = serde_json::from_value(body["session_id"].clone()).unwrap();
        let found = state.workflow_session_repository.find_by_id(session_id).await.unwrap().unwrap();
        assert_eq!(found.origin_capability(), "sales");
        assert_eq!(found.origin_reference(), "acme-corp");
        assert_eq!(found.target_capability(), "commit");
        assert_eq!(found.consultant_id(), auth::dev_stub::DEV_CONSULTANT_ID);
    }

    #[tokio::test]
    async fn create_workflow_session_returns_403_when_unpermitted_for_the_origin_capability() {
        let (router, cookie, _state, _container) = test_app(vec!["commit"]).await;

        let request = post_request(
            &cookie,
            json!({ "origin_capability": "sales", "origin_reference": "acme-corp", "target_capability": "commit" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_workflow_session_without_a_session_cookie_gets_401() {
        let (router, _cookie, _state, _container) = test_app(vec!["sales"]).await;

        let request = Request::builder()
            .method("POST")
            .uri("/api/workflow-sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "origin_capability": "sales", "origin_reference": "acme-corp", "target_capability": "commit" })
                    .to_string(),
            ))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_workflow_session_rejects_an_empty_origin_reference() {
        let (router, cookie, _state, _container) = test_app(vec!["sales"]).await;

        let request = post_request(
            &cookie,
            json!({ "origin_capability": "sales", "origin_reference": "", "target_capability": "commit" }),
        );
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
