//! `GET /api/notifications/stream` (PROMPT-31, ADR-011's "BFF → browser:
//! SSE" decision).
//!
//! The browser-facing half of the notification-delivery pipeline whose
//! Nexus-facing half (`crate::event_ingestion`'s polling loop → `bff_core`'s
//! `ingest_events` → `bff_core::EventBus`) PROMPT-30 already built. This
//! module subscribes to the *same* shared [`bff_core::EventBus`] instance
//! (`AppState::event_bus` — one bus, constructed once in `main`, handed to
//! both the polling task and every SSE connection; see `main.rs`) and, for
//! each [`bff_core::IngestedEvent`] published to it, forwards the event to
//! the connected browser **only if it belongs to that browser's own
//! consultant** — the "no cross-consultant bleed" requirement.
//!
//! # Unidirectional push only (ADR-011)
//! This endpoint never accepts input beyond the initial `GET`. Consultant
//! actions — marking a notification read, starting/completing an
//! action-queue entry — are explicitly **not** part of this channel; ADR-011
//! calls them out as ordinary `POST`/`PATCH` request/response calls instead.
//! **As of this unit, no such write-side REST endpoints exist yet** in this
//! crate (`grep` the crate for `mark_read`/`mark_started`/`mark_completed`
//! call sites — the only callers are the repository trait definitions in
//! `bff-core` and their Postgres implementations in `persistence`; nothing
//! in `bff-api` invokes them). That is expected and out of scope here: this
//! unit's acceptance criteria list only the stream endpoint. This module
//! does not implement, assume, or depend on those endpoints existing.
//!
//! # Wire format
//! Each SSE `data:` line is a single JSON object, tagged by a `"kind"`
//! discriminator so a consultant's single stream can carry both aggregate
//! types (`consultant-experience-context.md` §2 — one notification centre,
//! one task list, one channel):
//!
//! ```json
//! {"kind":"notification","notification_id":"<uuid>","title":"...","body":"...","deep_link":"...","created_at":"2026-01-01T00:00:00Z"}
//! {"kind":"action_queue_entry","entry_id":"<uuid>","title":"...","body":"...","deep_link":"...","action_state":"pending","expires_at":"2026-01-04T00:00:00Z","created_at":"2026-01-01T00:00:00Z"}
//! ```
//!
//! See [`NotificationStreamEvent`] for the authoritative shape. `deep_link`
//! serializes as JSON `null` when absent (never an omitted field), matching
//! this repo's existing DTO convention (`dashboard::DashboardCardDto` et
//! al.).
//!
//! # Keep-alive
//! [`axum::response::sse::Sse::keep_alive`] (built into Axum, not
//! hand-rolled) sends a periodic SSE comment ping so idle-timeout proxies
//! between the browser and this service don't close the connection during
//! quiet periods — see [`notifications_stream`].

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Extension, Router};
use bff_core::IngestedEvent;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use uuid::Uuid;

use crate::session::{self, AppState};
use auth::Session;

/// How often [`Sse::keep_alive`] sends an SSE comment ping on an otherwise
/// idle connection. Within ADR-011's "every 15-30s" guidance — chosen at
/// the low end so a proxy with a 30s idle timeout still sees traffic well
/// before it would consider the connection dead.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(20);

/// Wire shape for one event pushed over `GET /api/notifications/stream` —
/// see the module docs for worked JSON examples. `#[serde(tag = "kind")]`
/// makes the discriminator a real field on the wire (`"kind":
/// "notification"` / `"kind": "action_queue_entry"`), not something the
/// client has to infer from which fields happen to be present.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationStreamEvent {
    Notification {
        notification_id: Uuid,
        title: String,
        body: String,
        deep_link: Option<String>,
        created_at: DateTime<Utc>,
    },
    ActionQueueEntry {
        entry_id: Uuid,
        title: String,
        body: String,
        deep_link: Option<String>,
        action_state: String,
        expires_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    },
}

impl From<&IngestedEvent> for NotificationStreamEvent {
    fn from(event: &IngestedEvent) -> Self {
        match event {
            IngestedEvent::Notification(item) => Self::Notification {
                notification_id: item.id(),
                title: item.title().to_owned(),
                body: item.body().to_owned(),
                deep_link: item.deep_link().map(str::to_owned),
                created_at: item.created_at(),
            },
            IngestedEvent::Action(entry) => Self::ActionQueueEntry {
                entry_id: entry.id(),
                title: entry.title().to_owned(),
                body: entry.body().to_owned(),
                deep_link: entry.deep_link().map(str::to_owned),
                action_state: entry.action_state().as_str().to_owned(),
                expires_at: entry.expires_at(),
                created_at: entry.created_at(),
            },
        }
    }
}

/// Which consultant `event` belongs to — the field every published
/// [`IngestedEvent`] carries regardless of which aggregate it wraps (both
/// `NotificationItem` and `ActionQueueEntry` require a non-empty
/// `consultant_id`), and the basis for this module's consultant-scoping
/// filter.
fn event_consultant_id(event: &IngestedEvent) -> &str {
    match event {
        IngestedEvent::Notification(item) => item.consultant_id(),
        IngestedEvent::Action(entry) => entry.consultant_id(),
    }
}

/// `GET /api/notifications/stream`: subscribes to the shared
/// [`bff_core::EventBus`] (`AppState::event_bus`) and pushes every
/// [`IngestedEvent`] whose `consultant_id` matches the connected
/// consultant's own session — every other consultant's events are silently
/// skipped, never sent (the "no cross-consultant bleed" requirement).
/// Unreachable without a valid session cookie: this handler only runs
/// behind [`session::require_session`] (see [`notifications_router`]),
/// which short-circuits `401 Unauthorized` before subscribing to anything.
///
/// A lagged subscriber (the broadcast channel's bounded buffer overflowed
/// because this connection fell behind) skips the missed events and keeps
/// streaming — the same "not a correctness guarantee, best-effort delivery"
/// trade-off `bff_core::EventBus`'s own docs already accept, rather than
/// tearing down the whole SSE connection over it.
pub async fn notifications_stream(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let consultant_id = session.consultant_id;
    let receiver = state.event_bus.subscribe();

    let stream = BroadcastStream::new(receiver).filter_map(move |message| {
        let event = match message {
            Ok(event) => event,
            Err(BroadcastStreamRecvError::Lagged(_)) => return None,
        };

        if event_consultant_id(&event) != consultant_id.as_str() {
            return None;
        }

        let payload = NotificationStreamEvent::from(&event);
        let data =
            serde_json::to_string(&payload).expect("NotificationStreamEvent is always JSON-serializable");
        Some(Ok(Event::default().data(data)))
    });

    Sse::new(stream).keep_alive(KeepAlive::default().interval(KEEP_ALIVE_INTERVAL))
}

/// Builds the `/notifications/stream` route, session-gated the same way as
/// every other protected route in this crate (`dashboard::dashboard_router`,
/// `sales::sales_router`): a dedicated sub-router with
/// [`session::require_session`] layered on, merged into `main`'s `/api`
/// router.
pub fn notifications_router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/notifications/stream", get(notifications_stream))
        .layer(axum::middleware::from_fn_with_state(state, session::require_session))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use auth::dev_stub::DevStubSessionProvider;
    use axum::body::{Body, Bytes};
    use axum::http::{Request, StatusCode};
    use bff_core::{ActionQueueEntry, EventBus, NotificationItem};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::*;

    fn t0() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    /// A `PgPool` that never actually connects (`connect_lazy`, mirroring
    /// `health`/`persistence`'s own tests) — every test in this module
    /// exercises SSE delivery, never a database round-trip, so a real
    /// Postgres instance would only add setup cost for no coverage gain.
    fn unconnected_pool() -> persistence::Pool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/postgres")
            .expect("connect_lazy should not eagerly connect")
    }

    /// Stub `SalesGateway` — `AppState` requires the field regardless of
    /// what a given test exercises; mirrors `dashboard`'s
    /// `UnusedSalesGateway` test double.
    struct UnusedSalesGateway;

    #[async_trait::async_trait]
    impl nexus_client::SalesGateway for UnusedSalesGateway {
        async fn check_account_claim(
            &self,
            _company_name: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::AccountClaimResult, nexus_client::SalesGatewayError> {
            unimplemented!("notifications_sse tests never call the sales gateway")
        }

        async fn request_collaboration(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _message: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("notifications_sse tests never call the sales gateway")
        }

        async fn submit_referral(
            &self,
            _company_reference: &str,
            _consultant_id: &str,
            _notes: Option<&str>,
        ) -> Result<(), nexus_client::SalesGatewayError> {
            unimplemented!("notifications_sse tests never call the sales gateway")
        }
    }

    /// Stub `CommitGateway`, same rationale as `UnusedSalesGateway` above
    /// (PROMPT-34).
    struct UnusedCommitGateway;

    #[async_trait::async_trait]
    impl nexus_client::CommitGateway for UnusedCommitGateway {
        async fn create_proposal(
            &self,
            _origin_reference: &str,
            _consultant_id: &str,
        ) -> Result<nexus_client::ProposalSummary, nexus_client::CommitGatewayError> {
            unimplemented!("notifications_sse tests never call the commit gateway")
        }

        async fn list_proposals(
            &self,
            _consultant_id: &str,
        ) -> Result<Vec<nexus_client::ProposalSummary>, nexus_client::CommitGatewayError> {
            unimplemented!("notifications_sse tests never call the commit gateway")
        }

        async fn request_proposal_action(
            &self,
            _proposal_id: &str,
            _action: &str,
        ) -> Result<(), nexus_client::CommitGatewayError> {
            unimplemented!("notifications_sse tests never call the commit gateway")
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
            unimplemented!("notifications_sse tests never call the edu gateway")
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
            unimplemented!("notifications_sse tests never call the capacity gateway")
        }

        async fn get_own_profile(
            &self,
            _consultant_id: &str,
        ) -> Result<nexus_client::ConsultantProfileIntake, nexus_client::CapacityGatewayError> {
            unimplemented!("notifications_sse tests never call the capacity gateway")
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
            unimplemented!("notifications_sse tests never call the customer gateway")
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

    /// Builds a full `AppState` sharing `event_bus` with the caller — the
    /// same "one shared instance" wiring `main.rs` uses in production, so
    /// tests can publish into the exact bus the handler subscribes to.
    fn test_state(event_bus: Arc<EventBus>, dev_session_provider: Arc<DevStubSessionProvider>) -> AppState {
        let session_provider: Arc<dyn auth::SessionProvider> = dev_session_provider.clone();
        let pool = unconnected_pool();
        let notification_repository: Arc<dyn bff_core::NotificationRepository> =
            Arc::new(persistence::PgNotificationRepository::new(pool.clone()));
        let action_queue_repository: Arc<dyn bff_core::ActionQueueRepository> =
            Arc::new(persistence::PgActionQueueRepository::new(pool.clone()));

        AppState {
            db_pool: pool.clone(),
            session_provider,
            dev_session_provider,
            secure_cookies: false,
            prometheus_handle: crate::metrics::shared_test_handle(),
            permission_cache: Arc::new(crate::permissions::PermissionCache::new(Arc::new(
                UnusedArmorGateway,
            ))),
            dashboard_repository: Arc::new(persistence::PgDashboardConfigurationRepository::new(
                pool.clone(),
            )),
            sales_query_gateway: Arc::new(UnusedSalesGateway),
            sales_command_gateway: Arc::new(UnusedSalesGateway),
            commit_query_gateway: Arc::new(UnusedCommitGateway),
            commit_command_gateway: Arc::new(UnusedCommitGateway),
            edu_gateway: Arc::new(UnusedEduGateway),
            capacity_query_gateway: Arc::new(UnusedCapacityGateway),
            capacity_command_gateway: Arc::new(UnusedCapacityGateway),
            customer_gateway: Arc::new(UnusedCustomerGateway),
            workflow_session_repository: Arc::new(persistence::PgWorkflowSessionRepository::new(pool.clone())),
            notification_repository,
            action_queue_repository,
            event_bus,
        }
    }

    /// Builds an `AppState` plus a `Session` for `consultant_id`, for tests
    /// that call `notifications_stream` directly (bypassing the router/
    /// `require_session` middleware, exercised separately below).
    ///
    /// **Not routed through `DevStubSessionProvider::create_dev_session`**:
    /// that stub always issues sessions for the same fixed
    /// `DEV_CONSULTANT_ID` (ADR-008 "a fixed set of dev consultant
    /// identities"), so it cannot produce two *different* consultants'
    /// sessions — exactly what the cross-consultant-bleed test below
    /// needs. Building `Session` directly is safe here because
    /// `notifications_stream` takes `Extension<Session>` (already-resolved,
    /// per [`session::require_session`]'s contract) — it never itself
    /// looks the session up by id, so no real session-store round-trip is
    /// bypassed by constructing one in-memory for a unit test.
    fn direct_call_state_and_session(event_bus: Arc<EventBus>, consultant_id: &str) -> (AppState, auth::Session) {
        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session = auth::Session {
            session_id: uuid::Uuid::new_v4(),
            consultant_id: consultant_id.to_owned(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };

        (test_state(event_bus, dev_session_provider), session)
    }

    /// Builds an `AppState` plus a real, provider-registered `Session`
    /// (`DevStubSessionProvider::create_dev_session`) — for tests that go
    /// through the actual router/`require_session` middleware, which looks
    /// the session up by id rather than trusting a pre-attached
    /// `Extension`.
    async fn router_state_and_session(event_bus: Arc<EventBus>) -> (AppState, auth::Session) {
        let dev_session_provider = Arc::new(DevStubSessionProvider::new(&dev_config()));
        let session = dev_session_provider.create_dev_session().await.expect("create_dev_session failed");

        (test_state(event_bus, dev_session_provider), session)
    }

    struct UnusedArmorGateway;

    #[async_trait::async_trait]
    impl nexus_client::ArmorGateway for UnusedArmorGateway {
        async fn fetch_assertions(
            &self,
            _consultant_id: &str,
            _credential: &str,
        ) -> Result<Vec<nexus_client::PermissionAssertion>, nexus_client::ArmorGatewayError> {
            unimplemented!("notifications_sse tests never call the armor gateway")
        }
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
            t0() + chrono::Duration::hours(72),
            t0(),
        )
        .unwrap()
    }

    /// Drives `notifications_stream` directly (no router/middleware — that
    /// is covered separately by the 401 test below) for one session and
    /// returns the response body as a pollable `Stream` — subscription to
    /// `state.event_bus` happens synchronously inside the handler before
    /// this returns, so a caller that publishes an event right after this
    /// call cannot race the subscription.
    async fn body_data_stream(state: AppState, session: auth::Session) -> axum::body::BodyDataStream {
        use axum::response::IntoResponse;

        let response = notifications_stream(State(state), Extension(session)).await.into_response();
        response.into_body().into_data_stream()
    }

    /// Reads from `stream` until `count` SSE `data:` payloads have been
    /// collected (decoded to their JSON `Value`) or `timeout` elapses,
    /// whichever first — returns however many were collected either way,
    /// since an SSE body stream never naturally ends on its own.
    async fn collect_n(
        stream: &mut axum::body::BodyDataStream,
        count: usize,
        timeout: StdDuration,
    ) -> Vec<serde_json::Value> {
        let mut values = Vec::new();
        let collect = async {
            while values.len() < count {
                match stream.next().await {
                    Some(Ok(chunk)) => values.extend(parse_data_lines(&chunk)),
                    Some(Err(err)) => panic!("SSE body stream error: {err}"),
                    None => break,
                }
            }
        };

        let _ = tokio::time::timeout(timeout, collect).await;
        values
    }

    /// Parses zero or more `data: <json>` lines out of one raw SSE chunk
    /// (keep-alive comment lines, which start with `:`, are skipped).
    fn parse_data_lines(chunk: &Bytes) -> Vec<serde_json::Value> {
        String::from_utf8_lossy(chunk)
            .lines()
            .filter_map(|line| line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")))
            .filter_map(|json| serde_json::from_str(json).ok())
            .collect()
    }

    /// The headline "no cross-consultant bleed" proof (PROMPT-31's core
    /// acceptance criterion): consultant A and consultant B each hold their
    /// own subscription against the *same* shared bus; an event published
    /// for A only ever shows up on A's stream, never B's.
    #[tokio::test]
    async fn only_the_matching_consultants_stream_receives_their_event() {
        let event_bus = Arc::new(EventBus::new(16));
        let (state_a, session_a) = direct_call_state_and_session(event_bus.clone(), "consultant-a");
        let (state_b, session_b) = direct_call_state_and_session(event_bus.clone(), "consultant-b");

        // Subscribe both streams before publishing (broadcast semantics:
        // only events published after `subscribe()` are ever seen).
        let mut stream_a = body_data_stream(state_a, session_a).await;
        let mut stream_b = body_data_stream(state_b, session_b).await;

        let item = notification_for("consultant-a", "evt-a-1");
        event_bus.publish(IngestedEvent::Notification(item.clone()));

        let a_values = collect_n(&mut stream_a, 1, StdDuration::from_secs(2)).await;

        // Consultant B's stream must stay empty even after waiting past
        // the point A already received its event — proof this isn't just
        // "B would eventually get it too", but that B genuinely never does.
        let b_values = collect_n(&mut stream_b, 1, StdDuration::from_millis(300)).await;

        assert_eq!(a_values.len(), 1, "consultant A should receive exactly one event");
        assert_eq!(a_values[0]["notification_id"], serde_json::json!(item.id().to_string()));
        assert!(b_values.is_empty(), "consultant B must not receive consultant A's event");
    }

    /// The JSON payload shape matches the module docs: `kind` discriminator
    /// plus `notification_id`/`title`/`body`/`deep_link` for a
    /// notification, and `kind`/`entry_id`/... for an action-queue entry.
    #[tokio::test]
    async fn sse_payload_matches_the_documented_shape_for_both_event_kinds() {
        let event_bus = Arc::new(EventBus::new(16));
        let (state, session) = direct_call_state_and_session(event_bus.clone(), "consultant-1");

        let notification = notification_for("consultant-1", "evt-1");
        let entry = action_entry_for("consultant-1", "evt-2");
        let expected_notification_id = notification.id();
        let expected_entry_id = entry.id();

        let mut body_stream = body_data_stream(state, session).await;
        event_bus.publish(IngestedEvent::Notification(notification));
        event_bus.publish(IngestedEvent::Action(entry));
        let collected = collect_n(&mut body_stream, 2, StdDuration::from_secs(2)).await;

        assert_eq!(collected.len(), 2);

        let notification_payload = &collected[0];
        assert_eq!(notification_payload["kind"], serde_json::json!("notification"));
        assert_eq!(
            notification_payload["notification_id"],
            serde_json::json!(expected_notification_id.to_string())
        );
        assert_eq!(notification_payload["title"], serde_json::json!("Referral submitted"));
        assert_eq!(
            notification_payload["body"],
            serde_json::json!("A new referral was submitted for review.")
        );
        assert_eq!(
            notification_payload["deep_link"],
            serde_json::json!("https://app.example.com/sales/referrals/1")
        );

        let action_payload = &collected[1];
        assert_eq!(action_payload["kind"], serde_json::json!("action_queue_entry"));
        assert_eq!(action_payload["entry_id"], serde_json::json!(expected_entry_id.to_string()));
        assert_eq!(action_payload["title"], serde_json::json!("Collaboration request"));
        assert_eq!(action_payload["action_state"], serde_json::json!("pending"));
    }

    /// An unauthenticated connection attempt (no session cookie) is
    /// rejected `401` by `require_session` before the handler ever runs —
    /// in particular, before `event_bus.subscribe()` is ever called.
    #[tokio::test]
    async fn unauthenticated_request_is_rejected_before_subscribing() {
        let event_bus = Arc::new(EventBus::new(16));
        let (state, _session) = router_state_and_session(event_bus.clone()).await;

        let router = Router::new()
            .nest("/api", notifications_router(state.clone()))
            .with_state(state);

        let request = Request::builder()
            .method("GET")
            .uri("/api/notifications/stream")
            .body(Body::empty())
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        // Zero subscribers: nothing was ever handed a `Receiver`, proving
        // the 401 short-circuit happened strictly before any subscription.
        assert_eq!(event_bus.publish(IngestedEvent::Notification(notification_for("consultant-1", "evt-x"))), 0);
    }

    /// End-to-end proof over a real bound TCP listener + `axum::serve` +
    /// an HTTP client reading the response body incrementally
    /// (`reqwest::Response::chunk`, which — unlike `.bytes()`/`.text()` —
    /// awaits only the next chunk rather than the full (never-ending) SSE
    /// body). Exercises the actual `axum::response::sse::Sse` HTTP framing
    /// (status, `Content-Type: text/event-stream`, `data: ...\n\n` framing)
    /// rather than only the in-process `Sse::into_response()` body stream
    /// the tests above already cover — `tower::oneshot` was judged
    /// sufficient for those (it still drives a real `Response` body
    /// stream), but this test additionally proves the wiring holds across
    /// an actual socket, matching what a browser's `EventSource` sees.
    #[tokio::test]
    async fn end_to_end_over_a_real_listener_delivers_an_sse_data_frame() {
        let event_bus = Arc::new(EventBus::new(16));
        let (state, session) = router_state_and_session(event_bus.clone()).await;

        let router = Router::new()
            .nest("/api", notifications_router(state.clone()))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let client = reqwest::Client::new();
        let mut response = client
            .get(format!("http://{addr}/api/notifications/stream"))
            .header("Cookie", format!("{}={}", session::SESSION_COOKIE_NAME, session.session_id))
            .send()
            .await
            .expect("request failed");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").map(|v| v.to_str().unwrap()),
            Some("text/event-stream")
        );

        // By the time headers arrived above, the handler already ran to
        // completion (including `event_bus.subscribe()`) — `Sse::new`
        // builds the body synchronously and nothing awaits before the
        // response is returned — so publishing now cannot race the
        // subscription.
        let item = notification_for(&session.consultant_id, "evt-e2e-1");
        event_bus.publish(IngestedEvent::Notification(item.clone()));

        let found = tokio::time::timeout(StdDuration::from_secs(5), async {
            let mut buffer = String::new();
            loop {
                match response.chunk().await.expect("chunk read failed") {
                    Some(chunk) => {
                        buffer.push_str(&String::from_utf8_lossy(&chunk));
                        if buffer.contains(&item.id().to_string()) {
                            return true;
                        }
                    }
                    None => return false,
                }
            }
        })
        .await
        .unwrap_or(false);

        assert!(found, "expected the published notification's id to appear in the SSE response body");
    }
}
