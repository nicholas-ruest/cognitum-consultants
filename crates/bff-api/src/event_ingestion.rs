//! Nexus event-ingestion polling loop (PROMPT-30, ADR-011's "Nexus → BFF
//! ingestion... via polling" decision).
//!
//! `bff_core::event_ingestion` owns everything capability-agnostic — the
//! `CapabilityEventReceived` envelope, the classify-and-route decision, the
//! idempotent-ingestion service, and the `EventBus` it publishes into. This
//! module owns the one part that belongs in `bff-api` instead (ADR-004):
//! actually calling Nexus over `nexus_client::NexusTransport` and running
//! the interval loop as a background tokio task.
//!
//! # Provisional endpoint (ADR-007 framing: Nexus's real contract is unknown)
//! `GET events/v1/poll[?since=<cursor>]`, expected to return a bare JSON
//! array of [`CapabilityEventReceived`] — no wrapping envelope, matching the
//! "a `Vec<CapabilityEventReceived>` per poll" shape this unit's own prompt
//! describes. This is a guess, not a confirmed contract (same disclaimer as
//! `nexus_client::sales`'s provisional `sales/v1/...` paths) — update
//! [`EVENTS_POLL_PATH`]/[`fetch_events`] once Nexus's actual events-poll
//! contract is known.
//!
//! # Two dedup layers — do not confuse them
//! 1. **Cursor/watermark (this module, primary/efficiency mechanism)**:
//!    [`run_polling_loop`] tracks the maximum `received_at` seen across all
//!    events returned by the most recent poll and passes it as `?since=` on
//!    the *next* poll, so a well-behaved Nexus does not re-return
//!    already-processed events. This is purely an optimization — it reduces
//!    redundant network/DB work on the happy path — not a correctness
//!    guarantee: Nexus is free to ignore `since`, deliver at-least-once, or
//!    this loop may restart with the cursor reset to `None` (first poll
//!    after a restart re-fetches everything Nexus is willing to return).
//! 2. **Idempotent save (`bff_core::event_ingestion::ingest_events`, the
//!    actual correctness guarantee)**: the `(origin_capability,
//!    origin_event_id)` unique constraint (ADR-010, PROMPT-29). Even when
//!    the cursor above fails to prevent a re-fetch — for any of the reasons
//!    listed — `ingest_events` still only ever inserts a row, and publishes
//!    to the `EventBus`, once per distinct event. [`poll_once`]'s
//!    integration tests exercise this layer directly (calling it twice
//!    against the same mocked Nexus response) rather than relying on the
//!    cursor to prevent the duplicate fetch in the first place.

use std::sync::Arc;
use std::time::Duration;

use bff_core::{
    ingest_events, ActionQueueRepository, CapabilityEventReceived, EventBus, IngestionResult,
    NotificationRepository,
};
use chrono::{DateTime, Utc};
use nexus_client::{NexusRequest, NexusTransport, NexusTransportError};
use reqwest::Method;
use reqwest::header::HeaderMap;

/// Provisional Nexus events-poll endpoint path — see the module docs.
const EVENTS_POLL_PATH: &str = "events/v1/poll";

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    #[error(transparent)]
    Transport(#[from] NexusTransportError),
    #[error("Nexus events poll returned a non-success status {status}")]
    UnexpectedStatus { status: reqwest::StatusCode },
    #[error("Nexus events poll returned a response body that did not match the expected shape: {0}")]
    UnexpectedResponseShape(#[source] serde_json::Error),
}

/// Result of a single [`poll_once`] call, for logging/observability at
/// [`run_polling_loop`]'s call site.
#[derive(Debug)]
pub struct PollOutcome {
    pub events_fetched: usize,
    pub ingestion: IngestionResult,
    /// The new cursor value to pass as `since` on the *next* poll — the
    /// maximum `received_at` across this poll's events, or `since`
    /// unchanged if this poll returned no events. `None` only when this was
    /// the very first poll and it returned nothing yet.
    pub cursor: Option<DateTime<Utc>>,
}

/// Builds the `since`-qualified poll path, percent-encoding the timestamp
/// query parameter (matching `nexus_client::armor`'s
/// `url::form_urlencoded::Serializer` convention).
fn build_poll_path(since: Option<DateTime<Utc>>) -> String {
    match since {
        Some(since) => {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("since", &since.to_rfc3339());
            format!("{EVENTS_POLL_PATH}?{}", query.finish())
        }
        None => EVENTS_POLL_PATH.to_string(),
    }
}

async fn fetch_events(
    transport: &dyn NexusTransport,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<CapabilityEventReceived>, PollError> {
    let request =
        NexusRequest { method: Method::GET, path: build_poll_path(since), headers: HeaderMap::new(), body: None };

    let response = transport.send(request).await?;
    if !response.status.is_success() {
        return Err(PollError::UnexpectedStatus { status: response.status });
    }

    serde_json::from_value(response.body).map_err(PollError::UnexpectedResponseShape)
}

/// Runs exactly one poll-and-ingest cycle: fetches whatever batch of
/// [`CapabilityEventReceived`] envelopes Nexus returns for `since`, and
/// hands it to [`ingest_events`]. Exposed separately from
/// [`run_polling_loop`] so tests (and any future manual/one-shot trigger)
/// can drive a single cycle deterministically.
pub async fn poll_once(
    transport: &dyn NexusTransport,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
    event_bus: &EventBus,
    since: Option<DateTime<Utc>>,
) -> Result<PollOutcome, PollError> {
    let events = fetch_events(transport, since).await?;
    let events_fetched = events.len();
    let cursor = events.iter().map(|event| event.received_at).max().or(since);

    let ingestion = ingest_events(events, notification_repo, action_queue_repo, event_bus).await;

    Ok(PollOutcome { events_fetched, ingestion, cursor })
}

/// Runs [`poll_once`] forever, sleeping `interval` between cycles. Intended
/// to be `tokio::spawn`ed once at startup (`main.rs`) and never awaited
/// directly — it does not return under normal operation. A poll that fails
/// (Nexus unreachable, bad response shape, etc.) is logged and does not
/// crash the loop; the next cycle simply retries with the same `since`
/// cursor as before the failed attempt.
pub async fn run_polling_loop(
    transport: Arc<dyn NexusTransport>,
    notification_repo: Arc<dyn NotificationRepository>,
    action_queue_repo: Arc<dyn ActionQueueRepository>,
    event_bus: Arc<EventBus>,
    interval: Duration,
) -> ! {
    let mut cursor: Option<DateTime<Utc>> = None;

    loop {
        match poll_once(transport.as_ref(), notification_repo.as_ref(), action_queue_repo.as_ref(), event_bus.as_ref(), cursor)
            .await
        {
            Ok(outcome) => {
                cursor = outcome.cursor;
                tracing::info!(
                    events_fetched = outcome.events_fetched,
                    inserted = outcome.ingestion.inserted(),
                    duplicates = outcome.ingestion.duplicates(),
                    rejected = outcome.ingestion.rejected(),
                    "polled Nexus for capability events"
                );
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to poll Nexus for capability events");
            }
        }

        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bff_core::{ActionQueueRepository, NotificationRepository};
    use nexus_client::ReqwestNexusTransport;
    use persistence::{PgActionQueueRepository, PgNotificationRepository};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn migrated_pool() -> (persistence::Pool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("../persistence/migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    fn transport_for(mock_server: &MockServer) -> Arc<dyn NexusTransport> {
        Arc::new(ReqwestNexusTransport::with_client(reqwest::Client::new(), &mock_server.uri()).expect("valid url"))
    }

    fn event_batch_body() -> serde_json::Value {
        serde_json::json!([
            {
                "origin_capability": "sales",
                "origin_event_id": "cra-1",
                "event_type": "collaboration_request_acknowledged",
                "summary": "Sales acknowledged your collaboration request.",
                "deep_link": "https://app.example.com/sales/collab/1",
                "received_at": "2026-01-01T00:00:00Z",
                "consultant_id": "consultant-1"
            }
        ])
    }

    /// End-to-end proof (PROMPT-30 acceptance criterion 4): a wiremock-
    /// mocked Nexus events-poll endpoint returns a batch, `poll_once`
    /// ingests it into real Postgres tables, and a second, identical poll
    /// (simulating the loop running twice) does not create a duplicate row
    /// — the idempotent-save safety net (layer 2, module docs) holding even
    /// though this test does not rely on the cursor to prevent the re-fetch.
    #[tokio::test]
    async fn polling_twice_against_an_identical_batch_ingests_exactly_once() {
        let (pool, _container) = migrated_pool().await;
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool.clone()));
        let event_bus = EventBus::new(16);
        let mut subscription = event_bus.subscribe();

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/events/v1/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(event_batch_body()))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let first = poll_once(transport.as_ref(), notification_repo.as_ref(), action_queue_repo.as_ref(), &event_bus, None)
            .await
            .expect("first poll failed");

        assert_eq!(first.events_fetched, 1);
        assert_eq!(first.ingestion.inserted(), 1);
        assert_eq!(first.ingestion.duplicates(), 0);
        let cursor_after_first = first.cursor.expect("cursor should advance after a non-empty poll");

        let second = poll_once(
            transport.as_ref(),
            notification_repo.as_ref(),
            action_queue_repo.as_ref(),
            &event_bus,
            Some(cursor_after_first),
        )
        .await
        .expect("second poll failed");

        assert_eq!(second.events_fetched, 1, "the mock always returns the same fixed batch");
        assert_eq!(second.ingestion.inserted(), 0, "the event was already ingested by the first poll");
        assert_eq!(second.ingestion.duplicates(), 1);

        // Real Postgres round-trip: exactly one action-queue row exists,
        // for the correct consultant, classified as action-implying.
        let entries = action_queue_repo.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].origin_event_id(), "cra-1");

        let notifications = notification_repo.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert!(notifications.is_empty(), "collaboration_request_acknowledged is action-implying, not informational");

        // Exactly one publish reached the bus across both polls.
        subscription.try_recv().expect("expected exactly one publish");
        assert!(matches!(subscription.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));

        // The second request actually carried the cursor forward as `since`
        // — proof the watermark mechanism (layer 1) is wired, not just the
        // idempotent-save safety net (layer 2) papering over it never being
        // used.
        let requests = mock_server.received_requests().await.expect("request recording enabled by default");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].url.query().is_none(), "the first poll has no cursor yet");
        let second_query = requests[1].url.query().expect("second poll should carry a since= cursor");
        assert!(second_query.contains("since="), "expected a since= query param, got {second_query:?}");
    }

    /// An empty batch is a normal outcome (no events since `since`), not an
    /// error, and leaves the cursor unchanged rather than resetting it.
    #[tokio::test]
    async fn polling_an_empty_batch_ingests_nothing_and_preserves_the_cursor() {
        let (pool, _container) = migrated_pool().await;
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool));
        let event_bus = EventBus::new(16);

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/events/v1/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let since: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let outcome = poll_once(transport.as_ref(), notification_repo.as_ref(), action_queue_repo.as_ref(), &event_bus, Some(since))
            .await
            .expect("poll failed");

        assert_eq!(outcome.events_fetched, 0);
        assert_eq!(outcome.ingestion.inserted(), 0);
        assert_eq!(outcome.cursor, Some(since), "cursor should be preserved, not reset, on an empty batch");
    }

    /// A non-success status from the mocked Nexus is surfaced as an error,
    /// never coerced into an empty/successful batch.
    #[tokio::test]
    async fn polling_a_non_success_status_is_reported_as_an_error() {
        let (pool, _container) = migrated_pool().await;
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool));
        let event_bus = EventBus::new(16);

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/events/v1/poll"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let result =
            poll_once(transport.as_ref(), notification_repo.as_ref(), action_queue_repo.as_ref(), &event_bus, None).await;

        assert!(matches!(result, Err(PollError::UnexpectedStatus { .. })));
    }
}
