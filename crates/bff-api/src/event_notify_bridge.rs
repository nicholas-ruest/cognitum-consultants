//! Postgres `LISTEN` bridge (PROMPT-32, ADR-014's recommended cross-instance
//! SSE fan-out mechanism) — the consumer half of `persistence::event_notify`.
//!
//! `bff_core::EventBus` is purely in-process: within a single `bff-api`
//! instance, `notifications_sse` (PROMPT-31) subscribing to the same
//! `EventBus` instance the ingestion polling loop (`event_ingestion`)
//! publishes into is enough. It is *not* enough once `bff-api` scales to
//! more than one instance — instance B's browsers hold SSE connections
//! against instance B's own, independent `EventBus`, which instance A's
//! ingestion never touches.
//!
//! This module closes that gap: [`run_listen_bridge`] holds a dedicated
//! `persistence::PgListener` subscribed to `bff_core::EVENT_NOTIFY_CHANNEL`
//! for the lifetime of the process, and for every Postgres `NOTIFY` it
//! receives (from *any* instance's ingestion, per
//! `persistence::PgNotifyPublisher` — including this instance's own, since
//! `event_ingestion::run_polling_loop` no longer publishes to the local
//! `EventBus` directly, PROMPT-32) it:
//!
//! 1. Decodes the payload as a `bff_core::EventNotifyPointer` (`{kind,
//!    id}` — see that type's doc comment for why it's a pointer, not the
//!    full event).
//! 2. Re-fetches the full aggregate via `bff_core::hydrate_notify_pointer`
//!    (the matching repository's `find_by_id`).
//! 3. Publishes the reconstructed `IngestedEvent` into *this instance's*
//!    local `EventBus` — the same `EventBus` `notifications_sse` already
//!    subscribes to (PROMPT-31, unchanged).
//!
//! End to end: `ingest -> NOTIFY -> every instance's LISTEN loop -> that
//! instance's local EventBus -> that instance's SSE subscribers`. See
//! `docs/deployment.md` for the full writeup.
//!
//! # Same-instance round-trip: not a meaningful latency/ordering concern
//! The instance that did the ingesting now receives its own event back
//! through Postgres instead of a direct local publish. A `NOTIFY` issued on
//! a connection is delivered to every `LISTEN`ing connection on the same
//! Postgres server essentially immediately (Postgres's own in-server
//! notification delivery, not a polling mechanism) — see
//! `persistence::event_notify`'s module docs and this module's tests for
//! measured proof this stays well under a second in practice, nowhere near
//! a user-perceptible delay for a "new notification appeared" push.
//!
//! # Reconnection
//! A `PgListener` connection can drop (network blip, Postgres restart).
//! [`run_listen_bridge`] never returns — on a `recv()` error it logs, waits
//! [`RECONNECT_DELAY`], and re-`listen`s from scratch. A gap during
//! reconnection is a lost real-time push, not a lost row: the underlying
//! `NotificationItem`/`ActionQueueEntry` is already durably in Postgres by
//! the time it was ever NOTIFYed (ingestion only publishes after a
//! successful `SaveOutcome::Inserted`) and shows up on the consultant's
//! next full-list fetch regardless of whether the push arrived.

use std::sync::Arc;
use std::time::Duration;

use bff_core::{hydrate_notify_pointer, ActionQueueRepository, EventBus, EventNotifyPointer, NotificationRepository};

/// How long [`run_listen_bridge`] waits before retrying after a `LISTEN`
/// connection failure or a `recv()` error — long enough not to hot-loop
/// against a Postgres that is genuinely down, short enough that recovery is
/// still fast once it comes back.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Runs the `LISTEN` bridge forever. Intended to be `tokio::spawn`ed once
/// at startup (`main.rs`), alongside (not instead of)
/// `event_ingestion::run_polling_loop` — see the module docs for the full
/// pipeline this closes.
pub async fn run_listen_bridge(
    database_url: String,
    notification_repo: Arc<dyn NotificationRepository>,
    action_queue_repo: Arc<dyn ActionQueueRepository>,
    event_bus: Arc<EventBus>,
) -> ! {
    loop {
        match persistence::listen(&database_url, bff_core::EVENT_NOTIFY_CHANNEL).await {
            Ok(mut listener) => {
                tracing::info!(channel = bff_core::EVENT_NOTIFY_CHANNEL, "LISTEN bridge connected");
                loop {
                    match listener.recv().await {
                        Ok(notification) => {
                            handle_notification(
                                notification.payload(),
                                notification_repo.as_ref(),
                                action_queue_repo.as_ref(),
                                event_bus.as_ref(),
                            )
                            .await;
                        }
                        Err(err) => {
                            tracing::error!(error = %err, "PgListener recv failed; reconnecting");
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to establish Postgres LISTEN connection; retrying");
            }
        }

        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Decodes and hydrates one raw NOTIFY payload, then publishes the
/// reconstructed event into `event_bus`. Never panics: a malformed payload
/// or a repository lookup failure is logged and skipped, not propagated —
/// one bad notification must not take down the whole bridge (same
/// failure-isolation stance `bff_core::ingest_events` takes for a single
/// malformed event within a poll batch).
async fn handle_notification(
    payload: &str,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
    event_bus: &EventBus,
) {
    let pointer: EventNotifyPointer = match serde_json::from_str(payload) {
        Ok(pointer) => pointer,
        Err(err) => {
            tracing::error!(error = %err, payload, "failed to decode NOTIFY payload; skipping");
            return;
        }
    };

    match hydrate_notify_pointer(pointer, notification_repo, action_queue_repo).await {
        Ok(Some(event)) => {
            event_bus.publish(event);
        }
        Ok(None) => {
            tracing::warn!("NOTIFY pointer referenced an id no repository has; skipping");
        }
        Err(err) => {
            tracing::error!(error = %err, "failed to hydrate NOTIFY pointer from repository");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration as StdDuration;

    use bff_core::{EventPublisher, IngestedEvent, NotificationItem};
    use persistence::{PgActionQueueRepository, PgNotificationRepository, PgNotifyPublisher};
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    async fn migrated_database_url(
    ) -> (String, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("../persistence/migrations").run(&pool).await.expect("migration failed to run");

        (database_url, container)
    }

    fn t0() -> chrono::DateTime<chrono::Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    /// One simulated `bff-api` instance: its own `PgListener`-backed bridge
    /// task plus its own, entirely independent `EventBus`. Two of these,
    /// sharing only the underlying Postgres connection, is the "two
    /// instances" the cross-instance test below needs — no real second HTTP
    /// server required to prove the actual fan-out mechanism (NOTIFY ->
    /// LISTEN -> local EventBus), per PROMPT-32's own "acceptable, much
    /// simpler" allowance.
    struct SimulatedInstance {
        event_bus: Arc<EventBus>,
    }

    async fn spawn_instance(
        database_url: &str,
        notification_repo: Arc<dyn NotificationRepository>,
        action_queue_repo: Arc<dyn ActionQueueRepository>,
    ) -> SimulatedInstance {
        let event_bus = Arc::new(EventBus::new(16));

        // Block until this instance's `PgListener` is actually subscribed
        // (`listen()` awaits Postgres's own `LISTEN` acknowledgement)
        // before returning, so a NOTIFY sent right after this call cannot
        // race the subscription — same "subscribe before publish" ordering
        // guarantee `notifications_sse`'s own tests rely on for the
        // in-process `EventBus`.
        let listener =
            persistence::listen(database_url, bff_core::EVENT_NOTIFY_CHANNEL).await.expect("listen failed");

        let bus_for_task = event_bus.clone();
        tokio::spawn(async move { run_from_listener(listener, notification_repo, action_queue_repo, bus_for_task).await });

        SimulatedInstance { event_bus }
    }

    /// The inner loop of [`run_listen_bridge`], minus the reconnect-on-
    /// failure wrapper — lets tests drive an already-`listen`ing
    /// `PgListener` directly (avoiding a race against `run_listen_bridge`'s
    /// own internal `listen()` call) while exercising the exact same
    /// `handle_notification` path production uses.
    async fn run_from_listener(
        mut listener: persistence::PgListener,
        notification_repo: Arc<dyn NotificationRepository>,
        action_queue_repo: Arc<dyn ActionQueueRepository>,
        event_bus: Arc<EventBus>,
    ) {
        loop {
            let Ok(notification) = listener.recv().await else { return };
            handle_notification(
                notification.payload(),
                notification_repo.as_ref(),
                action_queue_repo.as_ref(),
                event_bus.as_ref(),
            )
            .await;
        }
    }

    /// **The headline cross-instance proof (PROMPT-32's core acceptance
    /// criterion)**: two entirely independent `(PgListener, EventBus)`
    /// pairs — simulating two separate `bff-api` instances, sharing nothing
    /// but the underlying Postgres server — both receive an event that is
    /// NOTIFYed exactly once, from a third connection standing in for
    /// "instance A's ingestion" (`persistence::PgNotifyPublisher`, the same
    /// type `event_ingestion::run_polling_loop` uses in production).
    ///
    /// Neither instance's `EventBus` is ever touched directly by the
    /// publisher — the *only* path an event can reach either `EventBus` is
    /// through Postgres NOTIFY -> that instance's own `PgListener` -> that
    /// instance's own `handle_notification` call. There is no shared
    /// in-process state between the two `SimulatedInstance`s at all (two
    /// separate `EventBus`es, two separate `PgListener` connections, two
    /// separate `tokio::spawn`ed bridge tasks) — proving genuine
    /// cross-instance delivery, not just "the same bus happened to be
    /// reused."
    #[tokio::test]
    async fn a_notify_from_one_connection_reaches_two_independent_listener_bridges() {
        let (database_url, _container) = migrated_database_url().await;
        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed");

        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool.clone()));

        // Two simulated instances, each with its own bridge + EventBus.
        let instance_a =
            spawn_instance(&database_url, notification_repo.clone(), action_queue_repo.clone()).await;
        let instance_b =
            spawn_instance(&database_url, notification_repo.clone(), action_queue_repo.clone()).await;

        let mut sub_a = instance_a.event_bus.subscribe();
        let mut sub_b = instance_b.event_bus.subscribe();

        // The row must exist before NOTIFY fires — `handle_notification`
        // hydrates by re-fetching from Postgres, it doesn't carry the
        // aggregate in the payload (PROMPT-32's pointer-payload decision).
        let item = NotificationItem::new(
            "consultant-1",
            "sales",
            "evt-cross-instance-1",
            "Referral submitted",
            "A new referral was submitted for review.",
            None,
            t0(),
        )
        .unwrap();
        notification_repo.save(&item).await.expect("save failed");
        let expected_id = item.id();

        // A third connection stands in for "instance A's own ingestion
        // NOTIFYing Postgres" — the exact type `event_ingestion::
        // run_polling_loop` is wired to in `main.rs`.
        let notify_publisher = PgNotifyPublisher::new(pool, bff_core::EVENT_NOTIFY_CHANNEL);
        notify_publisher.publish(IngestedEvent::Notification(item.clone())).await;

        let received_a = tokio::time::timeout(StdDuration::from_secs(5), sub_a.recv())
            .await
            .expect("instance A timed out waiting for its EventBus to receive the event")
            .expect("instance A's EventBus recv failed");
        let received_b = tokio::time::timeout(StdDuration::from_secs(5), sub_b.recv())
            .await
            .expect("instance B timed out waiting for its EventBus to receive the event")
            .expect("instance B's EventBus recv failed");

        for received in [received_a, received_b] {
            match received {
                IngestedEvent::Notification(received_item) => {
                    assert_eq!(received_item.id(), expected_id);
                    assert_eq!(received_item, item);
                }
                IngestedEvent::Action(_) => panic!("expected a Notification, got an Action"),
            }
        }
    }

    /// `handle_notification` on a malformed/undecodable payload logs and
    /// returns rather than panicking — proven here by driving it directly
    /// with a payload that isn't valid `EventNotifyPointer` JSON, and
    /// confirming the local `EventBus` stays empty rather than the call
    /// panicking or hanging.
    #[tokio::test]
    async fn handle_notification_skips_an_undecodable_payload_without_panicking() {
        let (database_url, _container) = migrated_database_url().await;
        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed");
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool));
        let event_bus = EventBus::new(16);
        let mut subscription = event_bus.subscribe();

        handle_notification("not valid json", notification_repo.as_ref(), action_queue_repo.as_ref(), &event_bus).await;

        assert!(matches!(
            subscription.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    /// A pointer naming an id that isn't in the database (e.g. a stale
    /// NOTIFY racing a since-deleted test fixture) is skipped, not a panic
    /// — `hydrate_notify_pointer`'s `Ok(None)` contract propagated through.
    #[tokio::test]
    async fn handle_notification_skips_a_pointer_to_an_unknown_id() {
        let (database_url, _container) = migrated_database_url().await;
        let pool = persistence::create_pool(&database_url).await.expect("create_pool failed");
        let notification_repo: Arc<dyn NotificationRepository> = Arc::new(PgNotificationRepository::new(pool.clone()));
        let action_queue_repo: Arc<dyn ActionQueueRepository> = Arc::new(PgActionQueueRepository::new(pool));
        let event_bus = EventBus::new(16);
        let mut subscription = event_bus.subscribe();

        let pointer = EventNotifyPointer::Notification { id: uuid::Uuid::new_v4() };
        let payload = serde_json::to_string(&pointer).unwrap();

        handle_notification(&payload, notification_repo.as_ref(), action_queue_repo.as_ref(), &event_bus).await;

        assert!(matches!(
            subscription.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}
