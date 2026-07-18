//! Postgres `NOTIFY`/`LISTEN`-backed cross-instance SSE fan-out (PROMPT-32,
//! ADR-014's recommended option).
//!
//! `bff_core::event_ingestion`'s `EventBus` is purely in-process
//! (`tokio::sync::broadcast`): fine within one `bff-api` process, but each
//! horizontally-scaled instance has its own independent `EventBus`, so an
//! event ingested by instance A never reaches a browser whose SSE
//! connection (PROMPT-31) is held by instance B. This module closes that
//! gap by routing every fresh ingestion through Postgres instead of a
//! direct local publish:
//!
//! ```text
//! instance A ingests event
//!   -> PgNotifyPublisher::publish -> `SELECT pg_notify($1, $2)`
//!   -> Postgres fans the NOTIFY out to every LISTEN'ing connection
//!        -> instance A's own listener bridge -> instance A's local EventBus -> instance A's SSE subscribers
//!        -> instance B's listener bridge      -> instance B's local EventBus -> instance B's SSE subscribers
//!        -> ...every other instance, identically
//! ```
//!
//! This crate (`persistence`) owns the two Postgres-specific primitives:
//! [`PgNotifyPublisher`] (the producer side — implements
//! [`bff_core::EventPublisher`]) and [`listen`] (opens the dedicated
//! `PgListener` connection a consumer needs). The actual background-task
//! loop that drives a `PgListener` forever and republishes into a local
//! `EventBus` is `bff-api::event_notify_bridge` — running a `tokio::spawn`ed
//! background task is `bff-api`'s job (ADR-004), not this crate's.
//!
//! # Payload: a pointer, not the full event
//! See `bff_core::EventNotifyPointer`'s doc comment for the full
//! rationale — in short, Postgres caps a `NOTIFY` payload at 8000 bytes and
//! neither `NotificationItem` nor `ActionQueueEntry` bound `title`/`body`'s
//! length, so this repo NOTIFYs a `{kind, id}` pointer (always well under
//! the limit) and has every listener re-fetch the full aggregate by id.
//!
//! # A dedicated connection, not the pool
//! `LISTEN` is connection-scoped Postgres state — a `PgListener` opens and
//! holds its own persistent connection ([`sqlx::postgres::PgListener::connect`]),
//! separate from `create_pool`'s rotating `PgPool`. [`listen`] takes a raw
//! `database_url` for exactly this reason: handing it a `PgPool` connection
//! would only hold `LISTEN` state on whichever pooled connection happened to
//! serve that call, which `sqlx`'s pool could recycle out from under it.

use async_trait::async_trait;
use bff_core::{EventNotifyPointer, EventPublisher, IngestedEvent};
use sqlx::PgPool;

/// Re-exported so `bff-api`'s listener-bridge background task can name the
/// type without taking its own direct `sqlx` dependency — same convention
/// as [`crate::Pool`]'s re-export of `sqlx::postgres::PgPool`.
pub use sqlx::postgres::PgListener;

/// Opens a dedicated Postgres `LISTEN` connection subscribed to `channel`.
/// See the module docs for why this needs its own connection rather than
/// one borrowed from a [`PgPool`].
pub async fn listen(database_url: &str, channel: &str) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect(database_url).await?;
    listener.listen(channel).await?;
    Ok(listener)
}

/// [`bff_core::EventPublisher`] implemented as a Postgres `NOTIFY` — the
/// producer half of the cross-instance bridge (see the module docs).
/// Constructed once at `bff-api` startup (`main.rs`) and handed to the
/// polling loop (`bff-api::event_ingestion::run_polling_loop`) in place of
/// a direct `EventBus` reference, so a fresh ingestion NOTIFYs Postgres
/// instead of writing straight into this instance's own local `EventBus` —
/// avoiding double delivery (this instance's own SSE subscribers still get
/// the event, but via its own listener bridge's round-trip through
/// Postgres, exactly like every other instance).
pub struct PgNotifyPublisher {
    pool: PgPool,
    channel: String,
}

impl PgNotifyPublisher {
    pub fn new(pool: PgPool, channel: impl Into<String>) -> Self {
        Self { pool, channel: channel.into() }
    }
}

#[async_trait]
impl EventPublisher for PgNotifyPublisher {
    /// Issues `SELECT pg_notify($1, $2)` with a JSON-encoded
    /// [`EventNotifyPointer`] payload. Best-effort, matching
    /// `bff_core::EventBus::publish`'s own "zero receivers is not a
    /// failure" stance: a NOTIFY that fails to send (e.g. a transient pool
    /// exhaustion) is logged, not propagated as an ingestion failure — the
    /// row is already durably saved in Postgres by this point
    /// (`ingest_events` only calls `publish` after a successful
    /// `SaveOutcome::Inserted`), so losing a NOTIFY only costs this
    /// particular real-time push, not correctness of the underlying data.
    async fn publish(&self, event: IngestedEvent) {
        let pointer = EventNotifyPointer::from(&event);
        let payload = match serde_json::to_string(&pointer) {
            Ok(payload) => payload,
            Err(err) => {
                tracing::error!(error = %err, "failed to serialize EventNotifyPointer");
                return;
            }
        };

        if let Err(err) = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&self.channel)
            .bind(&payload)
            .execute(&self.pool)
            .await
        {
            tracing::error!(error = %err, channel = %self.channel, "failed to pg_notify");
        }
    }
}

#[cfg(test)]
mod tests {
    use bff_core::{ActionQueueEntry, NotificationItem};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    async fn running_postgres(
    ) -> (String, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        (format!("postgres://postgres:postgres@{host}:{port}/postgres"), container)
    }

    fn t0() -> chrono::DateTime<chrono::Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    /// The unit-level payload round-trip proof (PROMPT-32's explicit ask):
    /// an `EventNotifyPointer` built from a real `IngestedEvent` is
    /// serialized, sent through a real Postgres `NOTIFY`, received by a
    /// real `PgListener`, and deserializes back to an equal value — proving
    /// the wire format survives the actual `pg_notify`/`LISTEN` round-trip,
    /// not just `serde_json::to_string`/`from_str` in isolation.
    #[tokio::test]
    async fn event_notify_pointer_round_trips_through_a_real_pg_notify_listen_cycle() {
        let (database_url, _container) = running_postgres().await;
        let channel = "test_event_notify_round_trip";

        let mut listener = listen(&database_url, channel).await.expect("listen failed");

        let notification = NotificationItem::new(
            "consultant-1",
            "sales",
            "evt-1",
            "Referral submitted",
            "A new referral was submitted for review.",
            None,
            t0(),
        )
        .unwrap();
        let original_pointer = EventNotifyPointer::from(&IngestedEvent::Notification(notification));

        let pool = crate::create_pool(&database_url).await.expect("create_pool failed");
        let payload = serde_json::to_string(&original_pointer).unwrap();
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(&payload)
            .execute(&pool)
            .await
            .expect("pg_notify failed");

        let notification = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv())
            .await
            .expect("timed out waiting for PgListener notification")
            .expect("PgListener recv failed");

        let decoded: EventNotifyPointer =
            serde_json::from_str(notification.payload()).expect("failed to decode NOTIFY payload");

        assert_eq!(decoded, original_pointer);
    }

    /// `PgNotifyPublisher::publish` (the actual production `EventPublisher`
    /// impl, not the raw `pg_notify` call above) end-to-end: publishing an
    /// `IngestedEvent::Action` results in a `{kind: "action_queue_entry",
    /// id: ...}` pointer arriving on a real `PgListener`.
    #[tokio::test]
    async fn pg_notify_publisher_publishes_a_decodable_pointer_for_an_action_queue_entry() {
        let (database_url, _container) = running_postgres().await;
        let channel = "test_pg_notify_publisher";

        let mut listener = listen(&database_url, channel).await.expect("listen failed");
        let pool = crate::create_pool(&database_url).await.expect("create_pool failed");
        let publisher = PgNotifyPublisher::new(pool, channel);

        let entry = ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "evt-2",
            "Collaboration request",
            "A collaboration request needs your response.",
            None,
            t0() + chrono::Duration::hours(72),
            t0(),
        )
        .unwrap();
        let expected_id = entry.id();

        publisher.publish(IngestedEvent::Action(entry)).await;

        let notification = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv())
            .await
            .expect("timed out waiting for PgListener notification")
            .expect("PgListener recv failed");

        let decoded: EventNotifyPointer =
            serde_json::from_str(notification.payload()).expect("failed to decode NOTIFY payload");

        assert_eq!(decoded, EventNotifyPointer::ActionQueueEntry { id: expected_id });
    }
}
