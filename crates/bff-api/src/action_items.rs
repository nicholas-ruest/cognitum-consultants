//! `/api/action-items*` (ADR-020 part B).
//!
//! CRUD over [`bff_core::ConsultantActionItem`] — a consultant's own
//! freeform checklist ("L10 type action list"), entirely consultant-
//! authored and never Nexus-sourced, same "this repo owns it end-to-end"
//! shape as `crate::sales`'s `/sales/prospects*` routes. **Not** capability-
//! gated the way every Nexus-backed route is (`crate::sales::SALES_CAPABILITY`
//! et al.) — a personal action list has no natural Nexus capability family
//! to check permission against (it may or may not relate to a prospect at
//! all, per [`bff_core::ConsultantActionItem::linked_prospect_id`]'s own
//! doc comment); [`crate::session::require_session`] (any authenticated
//! consultant may manage their own list) is the only gate.
//!
//! Deliberately its own top-level path (`/api/action-items`), not nested
//! under `/api/action-queue` — see `bff_core::consultant_action_item`'s
//! module docs for why this is a *different* aggregate from
//! [`bff_core::ActionQueueEntry`], not a variant of it; keeping the API
//! surface visibly distinct matches that separation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use bff_core::ConsultantActionItem;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::session::{self, AppState};
use auth::Session;

/// Wire shape for one action item, returned by every route below.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionItemDto {
    pub id: Uuid,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub linked_prospect_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&ConsultantActionItem> for ActionItemDto {
    fn from(item: &ConsultantActionItem) -> Self {
        Self {
            id: item.id(),
            title: item.title().to_owned(),
            notes: item.notes().map(str::to_owned),
            done: item.done(),
            linked_prospect_id: item.linked_prospect_id(),
            created_at: item.created_at(),
            updated_at: item.updated_at(),
        }
    }
}

/// `POST /api/action-items` request body.
#[derive(Debug, Deserialize)]
pub struct CreateActionItemRequest {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub linked_prospect_id: Option<Uuid>,
}

/// `PATCH /api/action-items/{id}` request body — every field optional;
/// only fields present in the request are changed.
///
/// **Known limitation**, same as `crate::sales::PatchProspectRequest`:
/// `notes` cannot currently be cleared back to `None` once set (an omitted
/// field and an explicit `null` both deserialize to `None` here) — a
/// follow-up, not fixed in this pass.
#[derive(Debug, Deserialize)]
pub struct PatchActionItemRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub done: Option<bool>,
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

/// `GET /api/action-items`: the authenticated consultant's action items,
/// newest first.
pub async fn list_action_items(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    match state.action_item_repository.find_by_consultant_id(&session.consultant_id).await {
        Ok(items) => Json(items.iter().map(ActionItemDto::from).collect::<Vec<_>>()).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "action item list failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load action items")
        }
    }
}

/// Validates that `linked_prospect_id`, if present, names a prospect the
/// *current* consultant actually owns — rejected `400` (not a bare `500`
/// from the DB's FK constraint, and not a `404` that would leak whether an
/// id exists at all) if it names an unknown or someone-else's prospect.
/// `Ok(())` for `None` — linking is optional (module docs).
async fn validate_linked_prospect(
    state: &AppState,
    session: &Session,
    linked_prospect_id: Option<Uuid>,
) -> Result<(), Response> {
    let Some(prospect_id) = linked_prospect_id else {
        return Ok(());
    };

    let prospect = state.prospect_repository.find_by_id(prospect_id).await.map_err(|err| {
        tracing::error!(error = %err, prospect_id = %prospect_id, "prospect lookup failed");
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to validate linked prospect")
    })?;

    match prospect {
        Some(prospect) if prospect.consultant_id() == session.consultant_id => Ok(()),
        _ => Err(error_response(StatusCode::BAD_REQUEST, "linked_prospect_id does not name a prospect you own")),
    }
}

/// `POST /api/action-items`: creates a new, not-done item for the
/// authenticated consultant.
pub async fn create_action_item(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(body): Json<CreateActionItemRequest>,
) -> Response {
    if let Err(response) = validate_linked_prospect(&state, &session, body.linked_prospect_id).await {
        return response;
    }

    let item = match ConsultantActionItem::new(
        &session.consultant_id,
        body.title,
        body.notes,
        body.linked_prospect_id,
        Utc::now(),
    ) {
        Ok(item) => item,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    if let Err(err) = state.action_item_repository.save(&item).await {
        tracing::error!(error = %err, consultant_id = %session.consultant_id, "action item save failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to save action item");
    }

    (StatusCode::CREATED, Json(ActionItemDto::from(&item))).into_response()
}

/// Loads the item for `id`, returning `404` (never `403`) if it doesn't
/// exist or belongs to a different consultant — same convention
/// `crate::sales::load_owned_prospect` uses.
async fn load_owned_item(state: &AppState, session: &Session, id: Uuid) -> Result<ConsultantActionItem, Response> {
    let existing = state.action_item_repository.find_by_id(id).await.map_err(|err| {
        tracing::error!(error = %err, item_id = %id, "action item lookup failed");
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load action item")
    })?;

    match existing {
        Some(item) if item.consultant_id() == session.consultant_id => Ok(item),
        _ => Err(not_found()),
    }
}

/// `PATCH /api/action-items/{id}`: updates whichever fields are present in
/// the request body — including toggling `done` in either direction (the
/// aggregate's own invariant 3: this is a freely-reversible checklist, not
/// a one-way transition).
pub async fn patch_action_item(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchActionItemRequest>,
) -> Response {
    let existing = match load_owned_item(&state, &session, id).await {
        Ok(item) => item,
        Err(response) => return response,
    };

    let title = body.title.unwrap_or_else(|| existing.title().to_owned());
    let notes = match body.notes {
        Some(notes) => Some(notes),
        None => existing.notes().map(str::to_owned),
    };
    let done = body.done.unwrap_or(existing.done());

    let updated = match ConsultantActionItem::from_parts(
        existing.id(),
        existing.consultant_id().to_owned(),
        title,
        notes,
        done,
        existing.linked_prospect_id(),
        existing.created_at(),
        Utc::now(),
    ) {
        Ok(item) => item,
        Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    if let Err(err) = state.action_item_repository.save(&updated).await {
        tracing::error!(error = %err, item_id = %id, "action item save failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to save action item");
    }

    Json(ActionItemDto::from(&updated)).into_response()
}

/// `POST /api/action-items/{id}/delete`: removes an item outright — a
/// dedicated `POST`, not a bare `DELETE /api/action-items/{id}`, matching
/// this repo's existing convention of dedicated action sub-paths for
/// mutations beyond plain field updates (`crate::sales`'s `/stage`/`/notes`
/// routes; `crate::notifications`'s `/read`/`/start`).
pub async fn delete_action_item(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(response) = load_owned_item(&state, &session, id).await {
        return response;
    }

    if let Err(err) = state.action_item_repository.delete(id).await {
        tracing::error!(error = %err, item_id = %id, "action item delete failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to delete action item");
    }

    StatusCode::OK.into_response()
}

/// Builds the `/api/action-items*` sub-router, session-gated only — see the
/// module docs for why there's no capability check here.
pub fn action_items_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/action-items", get(list_action_items).post(create_action_item))
        .route("/action-items/{id}", patch(patch_action_item))
        .route("/action-items/{id}/delete", post(delete_action_item))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::Body;
    use axum::http::Request;
    use axum_extra::extract::cookie::Cookie;
    use bff_core::{Prospect, ProspectRepository};
    use serde_json::{json, Value};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;
    use crate::permissions::PermissionCache;

    // --- Unused-gateway stubs (AppState needs every field regardless);
    // action-items tests never call any capability gateway. ---

    struct UnusedArmorGateway;
    #[async_trait::async_trait]
    impl nexus_client::ArmorGateway for UnusedArmorGateway {
        async fn fetch_assertions(
            &self,
            _consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<nexus_client::PermissionAssertion>, nexus_client::ArmorGatewayError> {
            unimplemented!("action-items tests never call the armor gateway")
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
            unimplemented!("action-items tests never call the sales gateway")
        }
        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("action-items tests never call the sales gateway")
        }
        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("action-items tests never call the sales gateway")
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
            unimplemented!("action-items tests never call the commit gateway")
        }
        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("action-items tests never call the commit gateway")
        }
        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("action-items tests never call the commit gateway")
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
            unimplemented!("action-items tests never call the edu gateway")
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
            unimplemented!("action-items tests never call the capacity gateway")
        }
        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("action-items tests never call the capacity gateway")
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
            unimplemented!("action-items tests never call the customer gateway")
        }
    }

    struct UnusedExecutionGateway;
    #[async_trait::async_trait]
    impl nexus_client::ExecutionGateway for UnusedExecutionGateway {
        async fn request_assigned_engagements(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::EngagementSnapshot>, nexus_client::ExecutionGatewayError> {
            unimplemented!("action-items tests never call the execution gateway")
        }
        async fn confirm_task_completion(
            &self,
            _task_id: &str,
            _consultant_id: &str,
        ) -> Result<(), nexus_client::ExecutionGatewayError> {
            unimplemented!("action-items tests never call the execution gateway")
        }
    }

    struct UnusedProductsGateway;
    #[async_trait::async_trait]
    impl nexus_client::ProductsGateway for UnusedProductsGateway {
        async fn request_product_catalog(
            &self,
            _filters: Option<&[String]>,
        ) -> Result<Vec<nexus_client::ProductReferenceCard>, nexus_client::ProductsGatewayError> {
            unimplemented!("action-items tests never call the products gateway")
        }
    }

    struct UnusedLandscapeGateway;
    #[async_trait::async_trait]
    impl nexus_client::LandscapeGateway for UnusedLandscapeGateway {
        async fn request_intelligence_digest(
            &self,
        ) -> Result<Vec<nexus_client::IntelligenceDigestItem>, nexus_client::LandscapeGatewayError> {
            unimplemented!("action-items tests never call the landscape gateway")
        }
        async fn submit_field_observation(
            &self,
            _submission: nexus_client::FieldObservationSubmission,
        ) -> Result<(), nexus_client::LandscapeGatewayError> {
            unimplemented!("action-items tests never call the landscape gateway")
        }
    }

    struct UnusedLegalGateway;
    #[async_trait::async_trait]
    impl nexus_client::LegalGateway for UnusedLegalGateway {
        async fn request_approved_clauses(
            &self,
            _context: nexus_client::ClauseContext<'_>,
        ) -> Result<Vec<nexus_client::ApprovedLegalSnippet>, nexus_client::LegalGatewayError> {
            unimplemented!("action-items tests never call the legal gateway")
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

    #[allow(clippy::type_complexity)]
    async fn test_app() -> (
        Router<()>,
        Cookie<'static>,
        persistence::Pool,
        testcontainers_modules::testcontainers::ContainerAsync<Postgres>,
    ) {
        let (pool, container) = migrated_pool().await;

        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();
        let session = dev_session_provider.create_dev_session().await.expect("create_dev_session failed");

        let armor_gateway: Arc<dyn nexus_client::ArmorGateway> = Arc::new(UnusedArmorGateway);
        let permission_cache = Arc::new(PermissionCache::new(armor_gateway));

        let state = AppState {
            db_pool: pool.clone(),
            session_provider,
            dev_session_provider: Some(dev_session_provider),
            firebase_session_provider: None,
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
            legal_gateway: Arc::new(UnusedLegalGateway),
            workflow_session_repository: Arc::new(persistence::PgWorkflowSessionRepository::new(pool.clone())),
            notification_repository: Arc::new(persistence::PgNotificationRepository::new(pool.clone())),
            action_queue_repository: Arc::new(persistence::PgActionQueueRepository::new(pool.clone())),
            event_bus: Arc::new(bff_core::EventBus::default()),
            event_notify_publisher: Arc::new(bff_core::EventBus::default()),
            google_identity_verifier: Arc::new(auth::google_identity_token::GoogleIdentityTokenVerifier::new(
                "test-audience".to_owned(),
                None,
            )),
            prospect_repository: Arc::new(persistence::PgProspectRepository::new(pool.clone())),
            action_item_repository: Arc::new(persistence::PgConsultantActionItemRepository::new(pool.clone())),
        };

        let router = Router::new().nest("/api", action_items_router(state.clone())).with_state(state);
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, pool, container)
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

    #[tokio::test]
    async fn list_without_a_session_cookie_gets_401() {
        let (router, _cookie, _pool, _container) = test_app().await;

        let request = Request::builder().method("GET").uri("/api/action-items").body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_and_list_round_trips_an_item() {
        let (router, cookie, _pool, _container) = test_app().await;

        let create_response =
            router.clone().oneshot(post_request(&cookie, "/api/action-items", json!({ "title": "Call Acme back" }))).await.unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let created = response_json(create_response).await;
        assert_eq!(created["title"], "Call Acme back");
        assert_eq!(created["done"], false);

        let list_response = router.oneshot(get_request(&cookie, "/api/action-items")).await.unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let list = response_json(list_response).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_rejects_an_empty_title() {
        let (router, cookie, _pool, _container) = test_app().await;

        let response = router.oneshot(post_request(&cookie, "/api/action-items", json!({ "title": "" }))).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_toggles_done_in_either_direction() {
        let (router, cookie, _pool, _container) = test_app().await;

        let create_response =
            router.clone().oneshot(post_request(&cookie, "/api/action-items", json!({ "title": "Call Acme" }))).await.unwrap();
        let created = response_json(create_response).await;
        let id = created["id"].as_str().unwrap();

        let done_response =
            router.clone().oneshot(patch_request(&cookie, &format!("/api/action-items/{id}"), json!({ "done": true }))).await.unwrap();
        assert_eq!(response_json(done_response).await["done"], true);

        let undone_response =
            router.oneshot(patch_request(&cookie, &format!("/api/action-items/{id}"), json!({ "done": false }))).await.unwrap();
        assert_eq!(response_json(undone_response).await["done"], false);
    }

    #[tokio::test]
    async fn patch_returns_404_for_an_unknown_id() {
        let (router, cookie, _pool, _container) = test_app().await;

        let response = router
            .oneshot(patch_request(&cookie, "/api/action-items/00000000-0000-0000-0000-000000000000", json!({ "done": true })))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_removes_the_item() {
        let (router, cookie, _pool, _container) = test_app().await;

        let create_response =
            router.clone().oneshot(post_request(&cookie, "/api/action-items", json!({ "title": "Call Acme" }))).await.unwrap();
        let created = response_json(create_response).await;
        let id = created["id"].as_str().unwrap();

        let delete_response = router.clone().oneshot(post_request(&cookie, &format!("/api/action-items/{id}/delete"), json!({}))).await.unwrap();
        assert_eq!(delete_response.status(), StatusCode::OK);

        let list_response = router.oneshot(get_request(&cookie, "/api/action-items")).await.unwrap();
        let list = response_json(list_response).await;
        assert!(list.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_rejects_a_linked_prospect_id_that_does_not_exist() {
        let (router, cookie, _pool, _container) = test_app().await;

        let prospect_id = Uuid::new_v4();
        let response = router
            .oneshot(post_request(
                &cookie,
                "/api/action-items",
                json!({ "title": "Follow up", "linked_prospect_id": prospect_id.to_string() }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_accepts_a_linked_prospect_id_the_consultant_owns() {
        let (router, cookie, pool, _container) = test_app().await;

        // `/api/sales/prospects` lives on `sales_router`, not
        // `action_items_router` -- this test only needs a real row in the
        // `prospects` table owned by the dev-stub's fixed consultant id, so
        // it inserts one directly via the repository rather than routing
        // through an endpoint this router doesn't mount.
        let prospect_repo = persistence::PgProspectRepository::new(pool);
        let prospect = Prospect::new(auth::dev_stub::DEV_CONSULTANT_ID, "Acme Corp", None, Utc::now()).unwrap();
        prospect_repo.save(&prospect).await.expect("seed prospect save failed");

        let response = router
            .oneshot(post_request(
                &cookie,
                "/api/action-items",
                json!({ "title": "Follow up", "linked_prospect_id": prospect.id().to_string() }),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_json(response).await;
        assert_eq!(body["linked_prospect_id"], prospect.id().to_string());
    }
}
