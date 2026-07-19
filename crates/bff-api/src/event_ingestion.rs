//! Nexus event-ingestion polling loop (PROMPT-30, ADR-011's "Nexus → BFF
//! ingestion... via polling" decision).
//!
//! `bff_core::event_ingestion` owns everything capability-agnostic — the
//! `CapabilityEventReceived` envelope, the classify-and-route decision, the
//! idempotent-ingestion service, and the `EventPublisher` trait it publishes
//! a freshly-inserted aggregate to. This module owns the one part that
//! belongs in `bff-api` instead (ADR-004): actually calling Nexus over
//! `nexus_client::NexusTransport` and running the interval loop as a
//! background tokio task.
//!
//! **PROMPT-32 (ADR-014) note**: production wiring (`main.rs`) hands
//! [`run_polling_loop`] a `persistence::PgNotifyPublisher`, not a raw
//! `bff_core::EventBus` — see [`run_polling_loop`]'s own doc comment and
//! `event_notify_bridge` (this crate) for the other half of the
//! cross-instance SSE fan-out bridge that makes this safe (every instance,
//! including this one, still ends up feeding its own local `EventBus` via
//! that bridge's `LISTEN` loop instead of losing delivery entirely).
//!
//! # Events-poll endpoint (ADR-030: nexus's real consumer-poll contract)
//! `GET api/v1/events/poll?consumer=<repo_id>[&since=<cursor>]`. ADR-030 §3
//! built this as a real, bounded, per-consumer feed (org-scoped per ADR-020)
//! to replace the earlier guessed `events/v1/poll` path, which never existed
//! on nexus-server and 404'd — the same class of guessed-path gap ADR-029
//! closed for capability calls. `consumer` is this repo's declared `repo_id`
//! ([`EVENTS_POLL_CONSUMER`]); nexus uses it to scope the feed to this
//! consumer's own org's events. The `api/v1/` prefix lives in
//! [`EVENTS_POLL_PATH`] itself (not the configured base URL, which is the
//! bare host), matching `nexus_client`'s `api/v1/capabilities/...` join
//! convention.
//!
//! ## Response shape: nexus `EventEnvelope`, mapped to `CapabilityEventReceived`
//! The route returns a bare JSON array of nexus's own `EventEnvelope`
//! objects (NOT this repo's [`CapabilityEventReceived`] directly — they share
//! only a few fields). [`fetch_events`] deserializes into the local
//! [`EventEnvelope`] mirror (independent-repo, no cross-Cargo-dep, same
//! pattern as ADR-029's capability structs) and maps each into a
//! [`CapabilityEventReceived`] via [`envelope_into_received`]. The
//! envelope-level fields map 1:1 (`producer_repo`→`origin_capability`,
//! `event_id`→`origin_event_id`, `event_type`→`event_type`,
//! `occurred_at`→`received_at`); the BFF-domain projection fields
//! (`summary`, `deep_link`, `consultant_id`, `related_origin_event_id`,
//! `related_proposal_id`) are read from the envelope's event-type-specific
//! `payload`. That payload mapping is **provisional** — nexus's real
//! per-`event_type` payload schema isn't declared yet (this repo currently
//! consumes zero event types in nexus's `consumers.json`, so the feed
//! returns `[]` today and no real payload exists to confirm against), so it
//! follows the same "read what's structurally required from the rough
//! payload and flag it pending nexus's real contract" convention
//! [`CapabilityEventReceived::consultant_id`]/`related_proposal_id` already
//! document — see [`envelope_into_received`] for the exact field rules and
//! defaults.
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
    filter_conservative_legal_events, ingest_events, ActionQueueRepository, CapabilityEventReceived, EventPublisher,
    IngestionResult, NotificationRepository, WorkflowSessionRepository,
};
use chrono::{DateTime, Utc};
use nexus_client::{NexusRequest, NexusTransport, NexusTransportError};
use reqwest::Method;
use reqwest::header::HeaderMap;

/// Nexus's real consumer events-poll route (ADR-030 §3). Carries the full
/// `api/v1/` prefix because the configured base URL is the bare host (same
/// convention as `nexus_client`'s `api/v1/capabilities/...` path).
const EVENTS_POLL_PATH: &str = "api/v1/events/poll";

/// This repo's declared `repo_id`, sent as the `consumer` query param so
/// nexus scopes the events feed to this consumer's own org (ADR-030 §3,
/// ADR-020). Matches the `cognitum-consultants` entry in nexus's
/// `config/registries/repos.json`.
const EVENTS_POLL_CONSUMER: &str = "cognitum-consultants";

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

/// Builds the poll path with the always-present `consumer` query param and,
/// once the cursor has advanced, the `since` param — percent-encoding both
/// (matching `nexus_client::armor`'s `url::form_urlencoded::Serializer`
/// convention). `consumer` is required by nexus on every poll (ADR-030 §3);
/// `since` is omitted on the very first poll after a restart (no cursor yet).
fn build_poll_path(since: Option<DateTime<Utc>>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("consumer", EVENTS_POLL_CONSUMER);
    if let Some(since) = since {
        query.append_pair("since", &since.to_rfc3339());
    }
    format!("{EVENTS_POLL_PATH}?{}", query.finish())
}

/// Local mirror of nexus's `EventEnvelope` wire shape (ADR-030 §3), just the
/// fields this consumer reads. Only the envelope-level fields mapped 1:1 plus
/// the event-type-specific `payload` are declared; every other envelope field
/// (`event_version`, `aggregate_id`/`aggregate_type`, `actor`,
/// `organization_id`, `causation_id`, `correlation_id`, `sequence_number`,
/// `metadata`) is intentionally ignored via serde's default unknown-field
/// tolerance — so this struct needs no `nexus_contracts` dependency and is
/// unaffected by e.g. `Semver`'s exact serde repr. A local plain-serde
/// struct, never a cross-repo Rust dependency (ADR-007/ADR-029).
#[derive(Debug, serde::Deserialize)]
struct EventEnvelope {
    event_id: String,
    event_type: String,
    occurred_at: DateTime<Utc>,
    producer_repo: String,
    #[serde(default)]
    payload: serde_json::Value,
}

/// Reads a string field from an `EventEnvelope`'s `payload`, if present.
fn payload_str(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload.get(key).and_then(serde_json::Value::as_str).map(str::to_owned)
}

/// Maps a nexus [`EventEnvelope`] into this repo's [`CapabilityEventReceived`]
/// projection (see the module docs' "Response shape" section).
///
/// Envelope-level fields map 1:1. The BFF-domain projection fields are read
/// from the event-type-specific `payload` by their `CapabilityEventReceived`
/// field name (`related_proposal_id` also accepts a bare `proposal_id`, the
/// key nexus's Legal events are expected to carry). This payload mapping is
/// **provisional** pending nexus's real per-`event_type` payload schema — the
/// feed is empty until this repo declares consumed event types in nexus's
/// `consumers.json`, so no real payload exists to confirm it against yet.
/// Defaults for the two required fields a payload might omit:
/// - `summary` falls back to `event_type` — an unrecognized/summary-less
///   event still surfaces something display-safe rather than an empty body.
/// - `consultant_id` falls back to `""` — an event that names no consultant
///   in its payload cannot be routed to one; the empty id is a documented,
///   inert placeholder (it simply won't match any real consultant's feed)
///   rather than a guess, and will be revisited once the real payload schema
///   is known. Matches this file's existing conservative-provisional
///   convention rather than dropping the event silently.
fn envelope_into_received(env: EventEnvelope) -> CapabilityEventReceived {
    let EventEnvelope { event_id, event_type, occurred_at, producer_repo, payload } = env;
    let summary = payload_str(&payload, "summary").unwrap_or_else(|| event_type.clone());
    CapabilityEventReceived {
        origin_capability: producer_repo,
        origin_event_id: event_id,
        event_type,
        summary,
        deep_link: payload_str(&payload, "deep_link"),
        received_at: occurred_at,
        consultant_id: payload_str(&payload, "consultant_id").unwrap_or_default(),
        related_origin_event_id: payload_str(&payload, "related_origin_event_id"),
        related_proposal_id: payload_str(&payload, "related_proposal_id")
            .or_else(|| payload_str(&payload, "proposal_id")),
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

    let envelopes: Vec<EventEnvelope> =
        serde_json::from_value(response.body).map_err(PollError::UnexpectedResponseShape)?;
    Ok(envelopes.into_iter().map(envelope_into_received).collect())
}

/// Runs exactly one poll-and-ingest cycle: fetches whatever batch of
/// [`CapabilityEventReceived`] envelopes Nexus returns for `since`, applies
/// [`filter_conservative_legal_events`] (PROMPT-41 — a no-op for every event
/// that isn't a `LegalClauseUpdated`, see that function's doc comment), and
/// hands the result to [`ingest_events`]. Exposed separately from
/// [`run_polling_loop`] so tests (and any future manual/one-shot trigger)
/// can drive a single cycle deterministically.
///
/// `events_fetched`/`cursor` are computed from the *raw*, pre-filter batch —
/// the filter only decides what gets surfaced as a notification, it must
/// not affect the polling cursor's "how far has this loop actually read
/// from Nexus" bookkeeping (module docs, dedup layer 1 vs. layer 2).
pub async fn poll_once(
    transport: &dyn NexusTransport,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
    workflow_session_repo: &dyn WorkflowSessionRepository,
    publisher: &dyn EventPublisher,
    since: Option<DateTime<Utc>>,
) -> Result<PollOutcome, PollError> {
    let events = fetch_events(transport, since).await?;
    let events_fetched = events.len();
    let cursor = events.iter().map(|event| event.received_at).max().or(since);

    let events = filter_conservative_legal_events(events, workflow_session_repo).await;
    let ingestion = ingest_events(events, notification_repo, action_queue_repo, publisher).await;

    Ok(PollOutcome { events_fetched, ingestion, cursor })
}

/// Runs [`poll_once`] forever, sleeping `interval` between cycles. Intended
/// to be `tokio::spawn`ed once at startup (`main.rs`) and never awaited
/// directly — it does not return under normal operation. A poll that fails
/// (Nexus unreachable, bad response shape, etc.) is logged and does not
/// crash the loop; the next cycle simply retries with the same `since`
/// cursor as before the failed attempt.
///
/// `publisher` is a [`bff_core::EventPublisher`], not a raw
/// [`bff_core::EventBus`] — in production (`main.rs`) this is a
/// `persistence::PgNotifyPublisher` (PROMPT-32, ADR-014's cross-instance
/// SSE fan-out): a fresh ingestion here NOTIFYs Postgres instead of writing
/// straight into this process's own local `EventBus`, so every `bff-api`
/// instance (including this one) learns about it uniformly via its own
/// listener bridge (`event_notify_bridge::run_listen_bridge`), rather than
/// this instance's ingestion reaching only its own in-process subscribers
/// directly.
pub async fn run_polling_loop(
    transport: Arc<dyn NexusTransport>,
    notification_repo: Arc<dyn NotificationRepository>,
    action_queue_repo: Arc<dyn ActionQueueRepository>,
    workflow_session_repo: Arc<dyn WorkflowSessionRepository>,
    publisher: Arc<dyn EventPublisher>,
    interval: Duration,
) -> ! {
    let mut cursor: Option<DateTime<Utc>> = None;

    loop {
        match poll_once(
            transport.as_ref(),
            notification_repo.as_ref(),
            action_queue_repo.as_ref(),
            workflow_session_repo.as_ref(),
            publisher.as_ref(),
            cursor,
        )
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

    use bff_core::{ActionQueueRepository, EventBus, NotificationRepository, WorkflowSessionRepository};
    use nexus_client::ReqwestNexusTransport;
    use persistence::{PgActionQueueRepository, PgNotificationRepository, PgWorkflowSessionRepository};
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

    /// A batch in nexus's real `EventEnvelope` wire shape (ADR-030 §3) — the
    /// envelope-level fields plus a full complement of the fields
    /// [`fetch_events`] deliberately ignores (`event_version`, `aggregate_*`,
    /// `actor`, `organization_id`, `correlation_id`, `metadata`), so these
    /// tests exercise [`envelope_into_received`] against the actual shape and
    /// prove the ignored fields don't trip deserialization. The BFF-domain
    /// projection fields live in `payload`, per the mapping.
    fn event_batch_body() -> serde_json::Value {
        serde_json::json!([
            {
                "event_id": "cra-1",
                "event_type": "collaboration_request_acknowledged",
                "event_version": {"major": 1, "minor": 0, "patch": 0},
                "occurred_at": "2026-01-01T00:00:00Z",
                "producer_repo": "sales",
                "aggregate_id": "collab-1",
                "aggregate_type": "collaboration_request",
                "organization_id": "org-1",
                "actor": {"user_id": "sales-user-9", "service_account": null, "role": "sales-rep"},
                "correlation_id": "corr-1",
                "payload": {
                    "summary": "Sales acknowledged your collaboration request.",
                    "deep_link": "https://app.example.com/sales/collab/1",
                    "consultant_id": "consultant-1"
                },
                "metadata": {}
            }
        ])
    }

    /// End-to-end proof (PROMPT-30 acceptance criterion 4): a wiremock-
    /// mocked Nexus events-poll endpoint returns a batch, `poll_once`
    /// ingests it into real Postgres tables, and a second, identical poll
    /// (simulating the loop running twice) does not create a duplicate row
    /// — the idempotent-save safety net (layer 2, module docs) holding even
    /// though this test does not rely on the cursor to prevent the re-fetch.
    ///
    /// Passes a raw `EventBus` as `poll_once`'s `&dyn EventPublisher`
    /// (`EventBus` implements that trait directly, PROMPT-32) — this test
    /// is about the poll/ingest/dedup plumbing, not about which
    /// `EventPublisher` production actually wires up (`persistence::
    /// PgNotifyPublisher`, see `event_notify_bridge`'s own tests for the
    /// real cross-instance NOTIFY/LISTEN proof).
    #[tokio::test]
    async fn polling_twice_against_an_identical_batch_ingests_exactly_once() {
        let (pool, _container) = migrated_pool().await;
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool.clone()));
        let workflow_session_repo: Arc<dyn WorkflowSessionRepository> = Arc::new(PgWorkflowSessionRepository::new(pool.clone()));
        let event_bus = EventBus::new(16);
        let mut subscription = event_bus.subscribe();

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(event_batch_body()))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let first = poll_once(
            transport.as_ref(),
            notification_repo.as_ref(),
            action_queue_repo.as_ref(),
            workflow_session_repo.as_ref(),
            &event_bus,
            None,
        )
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
            workflow_session_repo.as_ref(),
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
        // used. Both polls always carry the `consumer` param (ADR-030 §3);
        // only the second (post-cursor) poll carries `since`.
        let requests = mock_server.received_requests().await.expect("request recording enabled by default");
        assert_eq!(requests.len(), 2);
        let first_query = requests[0].url.query().expect("every poll carries the consumer param");
        assert!(first_query.contains("consumer=cognitum-consultants"), "expected consumer= param, got {first_query:?}");
        assert!(!first_query.contains("since="), "the first poll has no cursor yet, got {first_query:?}");
        let second_query = requests[1].url.query().expect("second poll should carry a since= cursor");
        assert!(second_query.contains("consumer=cognitum-consultants"), "expected consumer= param, got {second_query:?}");
        assert!(second_query.contains("since="), "expected a since= query param, got {second_query:?}");
    }

    /// An empty batch is a normal outcome (no events since `since`), not an
    /// error, and leaves the cursor unchanged rather than resetting it.
    #[tokio::test]
    async fn polling_an_empty_batch_ingests_nothing_and_preserves_the_cursor() {
        let (pool, _container) = migrated_pool().await;
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool.clone()));
        let workflow_session_repo: Arc<dyn WorkflowSessionRepository> = Arc::new(PgWorkflowSessionRepository::new(pool));
        let event_bus = EventBus::new(16);

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let since: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let outcome = poll_once(
            transport.as_ref(),
            notification_repo.as_ref(),
            action_queue_repo.as_ref(),
            workflow_session_repo.as_ref(),
            &event_bus,
            Some(since),
        )
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
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool.clone()));
        let workflow_session_repo: Arc<dyn WorkflowSessionRepository> = Arc::new(PgWorkflowSessionRepository::new(pool));
        let event_bus = EventBus::new(16);

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/poll"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;
        let transport = transport_for(&mock_server);

        let result = poll_once(
            transport.as_ref(),
            notification_repo.as_ref(),
            action_queue_repo.as_ref(),
            workflow_session_repo.as_ref(),
            &event_bus,
            None,
        )
        .await;

        assert!(matches!(result, Err(PollError::UnexpectedStatus { .. })));
    }

    // --- Pure unit tests for the EventEnvelope -> CapabilityEventReceived
    // mapping (no Postgres/container needed). ---

    fn envelope_from(value: serde_json::Value) -> EventEnvelope {
        serde_json::from_value(value).expect("valid EventEnvelope")
    }

    #[test]
    fn maps_envelope_level_fields_and_reads_projection_fields_from_payload() {
        // The full real EventEnvelope shape, including the fields the mapping
        // ignores — proves they neither break deserialization nor leak in.
        let batch = event_batch_body();
        let env = envelope_from(batch.as_array().unwrap()[0].clone());
        let received = envelope_into_received(env);

        assert_eq!(received.origin_capability, "sales"); // <- producer_repo
        assert_eq!(received.origin_event_id, "cra-1"); // <- event_id
        assert_eq!(received.event_type, "collaboration_request_acknowledged");
        assert_eq!(received.received_at, "2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap()); // <- occurred_at
        assert_eq!(received.summary, "Sales acknowledged your collaboration request."); // <- payload
        assert_eq!(received.deep_link.as_deref(), Some("https://app.example.com/sales/collab/1")); // <- payload
        assert_eq!(received.consultant_id, "consultant-1"); // <- payload
        assert_eq!(received.related_origin_event_id, None);
        assert_eq!(received.related_proposal_id, None);
    }

    #[test]
    fn summary_falls_back_to_event_type_when_payload_omits_it() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-1",
            "event_type": "task_assigned",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "execution",
            "payload": { "consultant_id": "consultant-1" }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.summary, "task_assigned", "summary should fall back to event_type");
        assert_eq!(received.consultant_id, "consultant-1");
    }

    #[test]
    fn consultant_id_defaults_to_empty_when_payload_omits_it() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-2",
            "event_type": "proposal_status_changed",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "commit",
            "payload": { "summary": "Status changed." }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.consultant_id, "", "a payload with no consultant_id yields the documented empty placeholder");
        assert_eq!(received.summary, "Status changed.");
    }

    #[test]
    fn related_proposal_id_reads_the_bare_proposal_id_alias_from_payload() {
        let env = envelope_from(serde_json::json!({
            "event_id": "e-3",
            "event_type": "legal_clause_updated",
            "occurred_at": "2026-01-01T00:00:00Z",
            "producer_repo": "legal",
            "payload": { "summary": "Clause updated.", "consultant_id": "consultant-1", "proposal_id": "prop-7" }
        }));
        let received = envelope_into_received(env);
        assert_eq!(received.related_proposal_id.as_deref(), Some("prop-7"));
    }
}
