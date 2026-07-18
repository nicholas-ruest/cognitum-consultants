//! `GET /api/notifications`, `PATCH /api/notifications/:id/read`,
//! `GET /api/action-queue`, `POST /api/action-queue/:id/start` (PROMPT-33,
//! ADR-011's "BFF → browser: SSE for push, ordinary REST for consultant
//! actions" split).
//!
//! [`notifications_sse`](crate::notifications_sse)'s module docs called out
//! that, as of PROMPT-31, no write-side REST endpoints existed yet for
//! marking a notification read or starting an action-queue entry. This
//! module is that missing half: four session-gated routes over the same
//! [`bff_core::NotificationRepository`]/[`bff_core::ActionQueueRepository`]
//! instances already shared via [`crate::session::AppState`].
//!
//! # Ownership enforcement
//! Every route is scoped to the authenticated consultant
//! ([`auth::Session::consultant_id`]). The two list routes filter at the
//! query (`find_by_consultant_id`). The two mutating routes
//! (`mark-read`/`start`) additionally load the target row first
//! (`find_by_id`) and compare its `consultant_id` against the session's
//! before mutating anything — an id that doesn't exist, or that belongs to
//! a *different* consultant, is rejected `404 Not Found` in both cases
//! (never `403`, so a caller can't distinguish "not yours" from "doesn't
//! exist" and enumerate other consultants' ids). This is the concrete
//! mechanism behind "a consultant can only read/mark-read/start their own
//! items".
//!
//! # No `POST /api/action-queue/:id/complete` — intentional, not an
//! oversight
//! [`bff_core::ActionQueueEntry::complete`] requires a non-empty
//! `confirmation_event_id`, and that value only ever originates from a real
//! confirmation event flowing back through Nexus into PROMPT-30's
//! ingestion pipeline (`crate::event_ingestion`) — never from a REST caller,
//! which has no such id to supply. [`action_queue_start`] below therefore
//! calls only [`bff_core::ActionQueueRepository::mark_started`]
//! (`Pending -> InProgress`, invariant 3's "bare consultant click" —
//! `bff_core`'s doc comment on `ActionQueueEntry::start`), and this module
//! does not, and must not, expose any route that maps an arbitrary
//! `ActionQueueEntry` back to a specific Nexus command to "complete" it —
//! that mapping is capability-specific and doesn't exist generically at
//! this layer. Completion is reflected in this UI only once it arrives via
//! the ingestion pipeline and is pushed out over
//! `GET /api/notifications/stream` (`crate::notifications_sse`), or picked
//! up on the next `GET /api/action-queue` re-fetch.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use bff_core::{ActionQueueEntry, NotificationItem};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::session::{self, AppState};
use auth::Session;

/// Wire shape for one notification, returned by `GET /api/notifications`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotificationDto {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    pub read_state: String,
    pub created_at: DateTime<Utc>,
}

impl From<&NotificationItem> for NotificationDto {
    fn from(item: &NotificationItem) -> Self {
        Self {
            id: item.id(),
            title: item.title().to_owned(),
            body: item.body().to_owned(),
            deep_link: item.deep_link().map(str::to_owned),
            read_state: item.read_state().as_str().to_owned(),
            created_at: item.created_at(),
        }
    }
}

/// Wire shape for one action-queue entry, returned by
/// `GET /api/action-queue`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionQueueEntryDto {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    pub action_state: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl From<&ActionQueueEntry> for ActionQueueEntryDto {
    fn from(entry: &ActionQueueEntry) -> Self {
        Self {
            id: entry.id(),
            title: entry.title().to_owned(),
            body: entry.body().to_owned(),
            deep_link: entry.deep_link().map(str::to_owned),
            action_state: entry.action_state().as_str().to_owned(),
            expires_at: entry.expires_at(),
            created_at: entry.created_at(),
        }
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

/// `GET /api/notifications`: the authenticated consultant's notifications,
/// newest first (`NotificationRepository::find_by_consultant_id`'s own
/// ordering).
pub async fn list_notifications(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    match state.notification_repository.find_by_consultant_id(&session.consultant_id).await {
        Ok(items) => Json(items.iter().map(NotificationDto::from).collect::<Vec<_>>()).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "notification list failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load notifications")
        }
    }
}

/// `PATCH /api/notifications/:id/read`: transitions one notification
/// `Unread -> Read` (one-way — see [`bff_core::NotificationItem`]'s
/// invariant 3). `404` if `id` doesn't exist or belongs to a different
/// consultant (see the module docs' ownership section); otherwise `200`,
/// even if the notification was already read (the repository-level
/// `mark_read` is deliberately lenient — re-clicking an already-read item
/// is not a failure).
pub async fn mark_notification_read(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<Uuid>,
) -> Response {
    let existing = match state.notification_repository.find_by_id(id).await {
        Ok(existing) => existing,
        Err(err) => {
            tracing::error!(error = %err, notification_id = %id, "notification lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load notification");
        }
    };

    match existing {
        Some(item) if item.consultant_id() == session.consultant_id => {
            if let Err(err) = state.notification_repository.mark_read(id).await {
                tracing::error!(error = %err, notification_id = %id, "mark_read failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to mark notification read");
            }
            StatusCode::OK.into_response()
        }
        _ => not_found(),
    }
}

/// `GET /api/action-queue`: the authenticated consultant's action-queue
/// entries, newest first.
pub async fn list_action_queue(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    match state.action_queue_repository.find_by_consultant_id(&session.consultant_id).await {
        Ok(entries) => Json(entries.iter().map(ActionQueueEntryDto::from).collect::<Vec<_>>()).into_response(),
        Err(err) => {
            tracing::error!(error = %err, consultant_id = %session.consultant_id, "action queue list failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load action queue")
        }
    }
}

/// `POST /api/action-queue/:id/start`: transitions one entry
/// `Pending -> InProgress` — a bare consultant click, per
/// [`bff_core::ActionQueueEntry::start`]'s doc comment. **Does not, and
/// must not, trigger any Nexus command** — see the module docs for why
/// there is no generic capability-specific mapping to do so at this layer,
/// and no `.../complete` route exists for the same reason. `404` if `id`
/// doesn't exist or belongs to a different consultant; otherwise `200`
/// (the repository-level `mark_started` is lenient about the entry's
/// current state, same rationale as `mark_notification_read` above).
pub async fn action_queue_start(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<Uuid>,
) -> Response {
    let existing = match state.action_queue_repository.find_by_id(id).await {
        Ok(existing) => existing,
        Err(err) => {
            tracing::error!(error = %err, entry_id = %id, "action queue entry lookup failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to load action queue entry");
        }
    };

    match existing {
        Some(entry) if entry.consultant_id() == session.consultant_id => {
            if let Err(err) = state.action_queue_repository.mark_started(id).await {
                tracing::error!(error = %err, entry_id = %id, "mark_started failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to start action queue entry");
            }
            StatusCode::OK.into_response()
        }
        _ => not_found(),
    }
}

/// Builds the `/notifications` + `/action-queue` sub-router, session-gated
/// the same way as every other protected route in this crate (see
/// `dashboard::dashboard_router`, `notifications_sse::notifications_router`).
pub fn notifications_write_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/notifications", get(list_notifications))
        .route("/notifications/{id}/read", patch(mark_notification_read))
        .route("/action-queue", get(list_action_queue))
        .route("/action-queue/{id}/start", post(action_queue_start))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::Body;
    use axum::http::Request;
    use axum_extra::extract::cookie::Cookie;
    use chrono::Duration as ChronoDuration;
    use persistence::{PgActionQueueRepository, PgNotificationRepository};
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tower::ServiceExt;

    use super::*;

    /// A `PgPool` that never actually connects (`connect_lazy`, mirroring
    /// `notifications_sse`'s test helper of the same name) — used only for
    /// `AppState::dashboard_repository`, which no route exercised by this
    /// module's tests ever reads.
    fn unconnected_pool() -> persistence::Pool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/postgres")
            .expect("connect_lazy should not eagerly connect")
    }

    struct UnusedSalesGateway;

    #[async_trait::async_trait]
    impl nexus_client::SalesGateway for UnusedSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::AccountClaimResult, nexus_client::SalesGatewayError> {
            unimplemented!("notifications tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("notifications tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("notifications tests never call the sales gateway")
        }
    }

    struct UnusedArmorGateway;

    #[async_trait::async_trait]
    impl nexus_client::ArmorGateway for UnusedArmorGateway {
        async fn fetch_assertions(
            &self,
            _consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<nexus_client::PermissionAssertion>, nexus_client::ArmorGatewayError> {
            unimplemented!("notifications tests never call the armor gateway")
        }
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

    async fn migrated_pool() -> (persistence::Pool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("../persistence/migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    async fn test_app(
        pool: persistence::Pool,
    ) -> (Router<()>, Cookie<'static>, AppState) {
        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();
        let session = dev_session_provider.create_dev_session().await.expect("create_dev_session failed");

        let notification_repository: Arc<dyn bff_core::NotificationRepository> =
            Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
            Arc::new(PgActionQueueRepository::new(pool.clone()));

        let state = AppState {
            db_pool: pool,
            session_provider,
            dev_session_provider,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache: Arc::new(crate::permissions::PermissionCache::new(Arc::new(UnusedArmorGateway))),
            dashboard_repository: Arc::new(persistence::PgDashboardConfigurationRepository::new(unconnected_pool())),
            sales_query_gateway: Arc::new(UnusedSalesGateway),
            sales_command_gateway: Arc::new(UnusedSalesGateway),
            notification_repository,
            action_queue_repository,
            event_bus: Arc::new(bff_core::EventBus::default()),
        };

        let router = Router::new().nest("/api", notifications_write_router(state.clone())).with_state(state.clone());
        let cookie = Cookie::new(session::SESSION_COOKIE_NAME, session.session_id.to_string());

        (router, cookie, state)
    }

    fn t0() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn notification_for(consultant_id: &str, origin_event_id: &str) -> NotificationItem {
        NotificationItem::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Referral submitted",
            "A new referral was submitted for review.",
            Some("https://app.example.com/sales/referrals/1".to_string()),
            t0(),
        )
        .unwrap()
    }

    fn action_entry_for(consultant_id: &str, origin_event_id: &str) -> ActionQueueEntry {
        ActionQueueEntry::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Collaboration request",
            "A collaboration request needs your response.",
            Some("https://app.example.com/sales/collab/1".to_string()),
            t0() + ChronoDuration::hours(72),
            t0(),
        )
        .unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn get_request(uri: &str, cookie: &Cookie<'static>) -> Request<Body> {
        Request::builder().method("GET").uri(uri).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    fn patch_request(uri: &str, cookie: &Cookie<'static>) -> Request<Body> {
        Request::builder().method("PATCH").uri(uri).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    fn post_request(uri: &str, cookie: &Cookie<'static>) -> Request<Body> {
        Request::builder().method("POST").uri(uri).header("cookie", cookie.to_string()).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn list_notifications_returns_only_the_authenticated_consultants_items() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;
        let consultant_id = auth::dev_stub::DEV_CONSULTANT_ID;

        state.notification_repository.save(&notification_for(consultant_id, "evt-1")).await.unwrap();
        state.notification_repository.save(&notification_for("someone-else", "evt-2")).await.unwrap();

        let response = router.oneshot(get_request("/api/notifications", &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        let items = body.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], serde_json::json!("Referral submitted"));
    }

    #[tokio::test]
    async fn unauthenticated_list_requests_get_401() {
        let (pool, _container) = migrated_pool().await;
        let (router, _cookie, _state) = test_app(pool).await;

        let request = Request::builder().method("GET").uri("/api/notifications").body(Body::empty()).unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn mark_notification_read_transitions_an_owned_notification() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;
        let consultant_id = auth::dev_stub::DEV_CONSULTANT_ID;

        let item = notification_for(consultant_id, "evt-1");
        state.notification_repository.save(&item).await.unwrap();

        let uri = format!("/api/notifications/{}/read", item.id());
        let response = router.oneshot(patch_request(&uri, &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let found = state.notification_repository.find_by_id(item.id()).await.unwrap().unwrap();
        assert_eq!(found.read_state(), bff_core::ReadState::Read);
    }

    /// The headline ownership-enforcement proof: a consultant cannot mark
    /// another consultant's notification read — the id belongs to
    /// "someone-else", not the dev-stub session's own consultant, so it
    /// must 404 and leave the row untouched.
    #[tokio::test]
    async fn mark_notification_read_on_another_consultants_item_is_rejected_with_404() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;

        let item = notification_for("someone-else", "evt-1");
        state.notification_repository.save(&item).await.unwrap();

        let uri = format!("/api/notifications/{}/read", item.id());
        let response = router.oneshot(patch_request(&uri, &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let found = state.notification_repository.find_by_id(item.id()).await.unwrap().unwrap();
        assert_eq!(found.read_state(), bff_core::ReadState::Unread, "must not have been mutated");
    }

    #[tokio::test]
    async fn mark_notification_read_on_unknown_id_is_404() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, _state) = test_app(pool).await;

        let uri = format!("/api/notifications/{}/read", Uuid::new_v4());
        let response = router.oneshot(patch_request(&uri, &cookie)).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_action_queue_returns_only_the_authenticated_consultants_entries() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;
        let consultant_id = auth::dev_stub::DEV_CONSULTANT_ID;

        state.action_queue_repository.save(&action_entry_for(consultant_id, "evt-1")).await.unwrap();
        state.action_queue_repository.save(&action_entry_for("someone-else", "evt-2")).await.unwrap();

        let response = router.oneshot(get_request("/api/action-queue", &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_json(response).await;
        let items = body.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["action_state"], serde_json::json!("pending"));
    }

    #[tokio::test]
    async fn action_queue_start_transitions_an_owned_pending_entry() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;
        let consultant_id = auth::dev_stub::DEV_CONSULTANT_ID;

        let entry = action_entry_for(consultant_id, "evt-1");
        state.action_queue_repository.save(&entry).await.unwrap();

        let uri = format!("/api/action-queue/{}/start", entry.id());
        let response = router.oneshot(post_request(&uri, &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let found = state.action_queue_repository.find_by_id(entry.id()).await.unwrap().unwrap();
        assert_eq!(found.action_state(), bff_core::ActionState::InProgress);
    }

    /// Ownership enforcement, action-queue side: cannot start another
    /// consultant's entry.
    #[tokio::test]
    async fn action_queue_start_on_another_consultants_entry_is_rejected_with_404() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, state) = test_app(pool).await;

        let entry = action_entry_for("someone-else", "evt-1");
        state.action_queue_repository.save(&entry).await.unwrap();

        let uri = format!("/api/action-queue/{}/start", entry.id());
        let response = router.oneshot(post_request(&uri, &cookie)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let found = state.action_queue_repository.find_by_id(entry.id()).await.unwrap().unwrap();
        assert_eq!(found.action_state(), bff_core::ActionState::Pending, "must not have been mutated");
    }

    #[tokio::test]
    async fn action_queue_start_on_unknown_id_is_404() {
        let (pool, _container) = migrated_pool().await;
        let (router, cookie, _state) = test_app(pool).await;

        let uri = format!("/api/action-queue/{}/start", Uuid::new_v4());
        let response = router.oneshot(post_request(&uri, &cookie)).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
