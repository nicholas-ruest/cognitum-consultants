//! Nexus event ingestion → `NotificationItem`/`ActionQueueEntry` mapping
//! (PROMPT-30, ADR-011's "Nexus → BFF ingestion... via polling" decision).
//!
//! `bff-api` (PROMPT-30) owns the actual polling loop — it needs
//! `nexus-client`'s transport and needs to run as a background tokio task,
//! neither of which belongs in this crate (ADR-004). This module owns
//! everything capability-agnostic: the wire envelope
//! ([`CapabilityEventReceived`]), the classify-and-route decision
//! ([`classify`]), the idempotent-ingestion service ([`ingest_events`]), and
//! the internal pub/sub primitive ([`EventBus`]) ingestion publishes into.
//!
//! # `consultant_id`: a pragmatic addition beyond the DDD doc's envelope
//! `../ddd/domain-events.md` §2 lists `CapabilityEventReceived`'s "rough
//! payload" as `origin_capability, origin_event_id, event_type, summary,
//! deep_link, received_at` — explicitly called "rough", and explicitly
//! missing any field identifying *which consultant* the event is for. Both
//! [`crate::NotificationItem`] and [`crate::ActionQueueEntry`] require a
//! non-empty `consultant_id` (invariant 4 / structural requirement on each
//! aggregate), and this repo has no other way to derive one from Nexus's
//! actual (unknown, provisional per ADR-007) event contract. Rather than
//! leave per-consultant targeting unresolved, [`CapabilityEventReceived`]
//! here adds a `consultant_id: String` field beyond the DDD doc's sketch —
//! a pragmatic, flagged assumption Nexus's real contract will need to
//! confirm or correct, not a silent invention: every real event source this
//! repo integrates with (Sales, first) already carries `consultant_id` on
//! its own outbound commands (see `nexus_client::sales`), so it is
//! reasonable to expect Nexus's normalized envelope to carry it back too.
//!
//! # Two dedup layers — do not confuse them
//! 1. **Idempotent save (this module, correctness guarantee)**: the
//!    `(origin_capability, origin_event_id)` unique constraint (ADR-010,
//!    PROMPT-29) that [`NotificationRepository::save`]/[`ActionQueueRepository::save`]
//!    enforce. [`ingest_events`] relies on [`SaveOutcome`] to know whether a
//!    given event was actually new, and only publishes to the [`EventBus`]
//!    on [`SaveOutcome::Inserted`] — a duplicate delivery within, or across,
//!    calls to [`ingest_events`] never produces a second row *or* a second
//!    bus publish.
//! 2. **Cursor/watermark (`bff-api`'s polling loop, efficiency mechanism)**:
//!    a *separate*, best-effort optimization that avoids re-fetching
//!    already-seen events from Nexus in the first place. See
//!    `bff-api::event_ingestion`'s module docs for that half. Losing the
//!    cursor (e.g. a restart) is not a correctness problem — layer 1 above
//!    still holds — only a wasted round-trip.

use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

use crate::{
    ActionQueueEntry, ActionQueueEntryError, ActionQueueRepository, NotificationItem,
    NotificationItemError, NotificationRepository, RepoError, SaveOutcome,
};

/// Normalized envelope for any upstream capability event, prior to being
/// classified as a notification or action item
/// (`../ddd/domain-events.md` §2). Deserializable: this is the shape
/// Nexus's polling endpoint (`bff-api::event_ingestion`) returns a
/// `Vec<CapabilityEventReceived>` of, per poll.
///
/// See the module docs for why [`Self::consultant_id`] exists beyond the
/// DDD doc's "rough payload" list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct CapabilityEventReceived {
    /// Which of the ten external contexts (via Nexus) this event
    /// originated from, e.g. `"sales"`. Half of the idempotency key.
    pub origin_capability: String,
    /// The origin system's own event id. Half of the idempotency key.
    pub origin_event_id: String,
    /// The event's type/name as Nexus reports it, e.g.
    /// `"collaboration_request_acknowledged"` — see [`classify`] for how
    /// this drives the notification-vs-action decision.
    pub event_type: String,
    /// Short, display-safe summary — becomes the resulting aggregate's
    /// `body` verbatim (see [`crate::NotificationItem`] invariant 2).
    pub summary: String,
    /// Opaque deep-link reference, if any.
    pub deep_link: Option<String>,
    /// When the origin system raised this event. Doubles as both the
    /// resulting aggregate's `created_at` and the basis for `bff-api`'s
    /// polling cursor/watermark (see the module docs).
    pub received_at: DateTime<Utc>,
    /// **Provisional addition beyond `../ddd/domain-events.md` §2's rough
    /// payload sketch** — see the module docs for the full rationale.
    pub consultant_id: String,
}

/// Whether a [`CapabilityEventReceived`] implies a required consultant
/// action ([`ActionQueueEntry`]) or is purely informational
/// ([`NotificationItem`]) — see [`classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClassification {
    Notification,
    Action,
}

/// Known `event_type`s that imply a required consultant action, per
/// PROMPT-30's own examples (`task_assigned`,
/// `collaboration_request_acknowledged`). Matched case/separator-insensitive
/// (see [`normalize_event_type`]) since Nexus's real casing convention for
/// `event_type` is not yet confirmed (some source docs use `snake_case`,
/// others use the `PascalCase` event names from `../ddd/domain-events.md`).
///
/// **This list is expected to grow.** It is intentionally small and
/// explicit today (Sales is the only integrated capability, PROMPT-24/25);
/// each subsequent capability integrated in Phase 4 that has action-implying
/// events should add its normalized `event_type`(s) here. Unknown/future
/// `event_type`s are never silently dropped — [`classify`] defaults them to
/// [`EventClassification::Notification`], the conservative choice: an
/// unrecognized event still reaches the consultant as an informational
/// item, rather than being lost or (worse) incorrectly treated as
/// actionable when this repo doesn't yet know what action it implies.
const ACTION_EVENT_TYPES: &[&str] = &["task_assigned", "collaboration_request_acknowledged"];

/// Normalizes an `event_type` for matching against [`ACTION_EVENT_TYPES`]:
/// lowercases and strips non-alphanumeric separators, so `"task_assigned"`,
/// `"TaskAssigned"`, and `"Task Assigned"` all match the same known entry.
fn normalize_event_type(event_type: &str) -> String {
    event_type.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Classifies `event_type` into [`EventClassification::Action`] (a known
/// action-implying event type) or [`EventClassification::Notification`]
/// (everything else, including unknown/future types — see
/// [`ACTION_EVENT_TYPES`]'s doc comment for why that default is
/// deliberate).
pub fn classify(event_type: &str) -> EventClassification {
    let normalized = normalize_event_type(event_type);
    if ACTION_EVENT_TYPES.iter().any(|known| normalize_event_type(known) == normalized) {
        EventClassification::Action
    } else {
        EventClassification::Notification
    }
}

/// Default time-to-live applied to an [`ActionQueueEntry`] built from an
/// ingested event, when the origin event carries no TTL of its own.
/// **Assumption** (`../ddd/consultant-experience-context.md` §2.2 invariant
/// 4 only says `expires_at` is "mirrored from (or defaulted relative to) the
/// origin event", without a value): 72 hours, chosen as a generous-but-
/// bounded window for a consultant to act on a required response — the same
/// "generous but bounded" reasoning `CrossCapabilityWorkflowSession` uses
/// for its own TTL default, scaled up because an action-queue entry (e.g.
/// "respond to this collaboration request") is a slower-paced task than a
/// single in-session workflow hand-off.
pub const DEFAULT_ACTION_QUEUE_ENTRY_TTL_HOURS: i64 = 72;

/// Turns a raw `event_type` into a short, human-readable title: splits on
/// `_`/`-` and on internal case changes (so both `snake_case` and
/// `PascalCase` inputs work — see [`normalize_event_type`]'s doc comment for
/// why this crate can't assume one casing convention), then title-cases each
/// word. E.g. `"collaboration_request_acknowledged"` and
/// `"CollaborationRequestAcknowledged"` both become `"Collaboration Request
/// Acknowledged"`.
fn humanize_event_type(event_type: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in event_type.chars() {
        if c == '_' || c == '-' || c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if c.is_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    words
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_notification_item(
    event: &CapabilityEventReceived,
) -> Result<NotificationItem, NotificationItemError> {
    NotificationItem::new(
        event.consultant_id.clone(),
        event.origin_capability.clone(),
        event.origin_event_id.clone(),
        humanize_event_type(&event.event_type),
        event.summary.clone(),
        event.deep_link.clone(),
        event.received_at,
    )
}

fn build_action_queue_entry(
    event: &CapabilityEventReceived,
) -> Result<ActionQueueEntry, ActionQueueEntryError> {
    let expires_at = event.received_at + chrono::Duration::hours(DEFAULT_ACTION_QUEUE_ENTRY_TTL_HOURS);
    ActionQueueEntry::new(
        event.consultant_id.clone(),
        event.origin_capability.clone(),
        event.origin_event_id.clone(),
        humanize_event_type(&event.event_type),
        event.summary.clone(),
        event.deep_link.clone(),
        expires_at,
        event.received_at,
    )
}

/// Aggregate published to the [`EventBus`] on a fresh
/// ([`SaveOutcome::Inserted`]) ingestion — PROMPT-31's SSE endpoint is the
/// intended subscriber.
#[derive(Debug, Clone)]
pub enum IngestedEvent {
    Notification(NotificationItem),
    Action(ActionQueueEntry),
}

/// Per-event result of [`ingest_events`], for logging/observability at the
/// polling-loop call site (`bff-api`).
#[derive(Debug)]
pub enum IngestionOutcome {
    /// The event was classified, the resulting aggregate constructed, and
    /// `save` succeeded (whether that was a fresh insert or a no-op
    /// redelivery — see `save_outcome`).
    Saved {
        origin_capability: String,
        origin_event_id: String,
        classification: EventClassification,
        save_outcome: SaveOutcome,
    },
    /// The event could not be turned into a valid aggregate (e.g. a blank
    /// `consultant_id`) or the repository `save` call itself failed. Never
    /// panics or aborts the rest of the batch — one malformed/failed event
    /// must not block ingestion of the others in the same poll.
    Rejected { origin_capability: String, origin_event_id: String, reason: String },
}

/// Aggregated result of one [`ingest_events`] call.
#[derive(Debug, Default)]
pub struct IngestionResult {
    pub outcomes: Vec<IngestionOutcome>,
}

impl IngestionResult {
    /// Number of events that produced a brand-new row.
    pub fn inserted(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, IngestionOutcome::Saved { save_outcome: SaveOutcome::Inserted, .. }))
            .count()
    }

    /// Number of events that were redeliveries of an already-ingested event
    /// (idempotent no-op).
    pub fn duplicates(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, IngestionOutcome::Saved { save_outcome: SaveOutcome::AlreadyExists, .. }))
            .count()
    }

    /// Number of events rejected (invalid aggregate or repository failure).
    pub fn rejected(&self) -> usize {
        self.outcomes.iter().filter(|o| matches!(o, IngestionOutcome::Rejected { .. })).count()
    }
}

/// Minimal in-process pub/sub primitive [`ingest_events`] publishes freshly-
/// inserted aggregates into. Intentionally thin — a wrapper over
/// [`tokio::sync::broadcast`] with no filtering, no consultant-scoping, and
/// no persistence of its own; PROMPT-31's SSE endpoint is the intended
/// consumer via [`EventBus::subscribe`], and is expected to do its own
/// per-connection consultant-scoping (filtering the bus's events down to
/// one consultant's own) rather than this type doing it centrally — keeping
/// this a single shared broadcast channel of all ingested events.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<IngestedEvent>,
}

/// Default channel capacity: how many not-yet-received events a slow
/// subscriber can lag behind before [`broadcast`]'s lagged-receiver
/// behavior kicks in. Not tuned against real load (no deployed SSE
/// subscribers yet, PROMPT-31) — a conservative starting point, same
/// "no real traffic to tune against yet" reasoning `persistence`'s
/// `DEFAULT_MAX_CONNECTIONS` documents for its own untuned default.
pub const DEFAULT_EVENT_BUS_CAPACITY: usize = 256;

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity);
        Self { sender }
    }

    /// A new subscription, receiving every [`IngestedEvent`] published after
    /// this call (broadcast semantics: never events published before
    /// subscribing).
    pub fn subscribe(&self) -> broadcast::Receiver<IngestedEvent> {
        self.sender.subscribe()
    }

    /// Publishes `event` to every current subscriber. Returns the number of
    /// subscribers that received it — `0` is a normal, non-error outcome
    /// (e.g. no consultant currently has an open SSE connection), not a
    /// failure; [`broadcast::Sender::send`] only errors when there are zero
    /// receivers, which this method treats identically to "delivered to
    /// zero receivers" rather than surfacing as an ingestion failure.
    pub fn publish(&self, event: IngestedEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_BUS_CAPACITY)
    }
}

/// Ingests a batch of [`CapabilityEventReceived`] envelopes: classifies
/// each, builds the corresponding aggregate, saves it via the matching
/// repository, and — only on a fresh insert ([`SaveOutcome::Inserted`]) —
/// publishes it to `event_bus`. See the module docs for the two-layer
/// dedup this relies on (this function is layer 1, the correctness
/// guarantee).
///
/// Never panics or short-circuits the batch on one bad event: a malformed
/// event (fails aggregate construction) or a repository failure is recorded
/// as [`IngestionOutcome::Rejected`] and processing continues with the next
/// event (input validation/failure isolation at the ingestion boundary).
pub async fn ingest_events(
    events: Vec<CapabilityEventReceived>,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
    event_bus: &EventBus,
) -> IngestionResult {
    let mut result = IngestionResult::default();

    for event in events {
        let classification = classify(&event.event_type);
        let outcome = match classification {
            EventClassification::Notification => {
                ingest_notification(&event, notification_repo, event_bus).await
            }
            EventClassification::Action => {
                ingest_action(&event, action_queue_repo, event_bus).await
            }
        };
        result.outcomes.push(outcome);
    }

    result
}

async fn ingest_notification(
    event: &CapabilityEventReceived,
    notification_repo: &dyn NotificationRepository,
    event_bus: &EventBus,
) -> IngestionOutcome {
    let item = match build_notification_item(event) {
        Ok(item) => item,
        Err(err) => return rejected(event, err.to_string()),
    };

    match notification_repo.save(&item).await {
        Ok(save_outcome) => {
            if save_outcome == SaveOutcome::Inserted {
                event_bus.publish(IngestedEvent::Notification(item));
            }
            saved(event, EventClassification::Notification, save_outcome)
        }
        Err(err) => rejected(event, repo_error_reason(err)),
    }
}

async fn ingest_action(
    event: &CapabilityEventReceived,
    action_queue_repo: &dyn ActionQueueRepository,
    event_bus: &EventBus,
) -> IngestionOutcome {
    let entry = match build_action_queue_entry(event) {
        Ok(entry) => entry,
        Err(err) => return rejected(event, err.to_string()),
    };

    match action_queue_repo.save(&entry).await {
        Ok(save_outcome) => {
            if save_outcome == SaveOutcome::Inserted {
                event_bus.publish(IngestedEvent::Action(entry));
            }
            saved(event, EventClassification::Action, save_outcome)
        }
        Err(err) => rejected(event, repo_error_reason(err)),
    }
}

fn saved(
    event: &CapabilityEventReceived,
    classification: EventClassification,
    save_outcome: SaveOutcome,
) -> IngestionOutcome {
    IngestionOutcome::Saved {
        origin_capability: event.origin_capability.clone(),
        origin_event_id: event.origin_event_id.clone(),
        classification,
        save_outcome,
    }
}

fn rejected(event: &CapabilityEventReceived, reason: String) -> IngestionOutcome {
    IngestionOutcome::Rejected {
        origin_capability: event.origin_capability.clone(),
        origin_event_id: event.origin_event_id.clone(),
        reason,
    }
}

fn repo_error_reason(err: RepoError) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use uuid::Uuid;

    use super::*;

    fn t0() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn event(origin_event_id: &str, event_type: &str) -> CapabilityEventReceived {
        CapabilityEventReceived {
            origin_capability: "sales".to_string(),
            origin_event_id: origin_event_id.to_string(),
            event_type: event_type.to_string(),
            summary: "Something happened that the consultant should know about.".to_string(),
            deep_link: Some("https://app.example.com/sales/1".to_string()),
            received_at: t0(),
            consultant_id: "consultant-1".to_string(),
        }
    }

    #[derive(Default)]
    struct MockNotificationRepo {
        rows: Mutex<HashMap<(String, String), NotificationItem>>,
    }

    #[async_trait::async_trait]
    impl NotificationRepository for MockNotificationRepo {
        async fn find_by_consultant_id(
            &self,
            consultant_id: &str,
        ) -> Result<Vec<NotificationItem>, RepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .values()
                .filter(|item| item.consultant_id() == consultant_id)
                .cloned()
                .collect())
        }

        async fn save(&self, item: &NotificationItem) -> Result<SaveOutcome, RepoError> {
            let mut rows = self.rows.lock().unwrap();
            let key = (item.origin_capability().to_string(), item.origin_event_id().to_string());
            match rows.entry(key) {
                std::collections::hash_map::Entry::Occupied(_) => Ok(SaveOutcome::AlreadyExists),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(item.clone());
                    Ok(SaveOutcome::Inserted)
                }
            }
        }

        async fn mark_read(&self, _id: Uuid) -> Result<(), RepoError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockActionQueueRepo {
        rows: Mutex<HashMap<(String, String), ActionQueueEntry>>,
    }

    #[async_trait::async_trait]
    impl ActionQueueRepository for MockActionQueueRepo {
        async fn find_by_consultant_id(
            &self,
            consultant_id: &str,
        ) -> Result<Vec<ActionQueueEntry>, RepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .values()
                .filter(|entry| entry.consultant_id() == consultant_id)
                .cloned()
                .collect())
        }

        async fn save(&self, entry: &ActionQueueEntry) -> Result<SaveOutcome, RepoError> {
            let mut rows = self.rows.lock().unwrap();
            let key = (entry.origin_capability().to_string(), entry.origin_event_id().to_string());
            match rows.entry(key) {
                std::collections::hash_map::Entry::Occupied(_) => Ok(SaveOutcome::AlreadyExists),
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(entry.clone());
                    Ok(SaveOutcome::Inserted)
                }
            }
        }

        async fn mark_started(&self, _id: Uuid) -> Result<(), RepoError> {
            Ok(())
        }

        async fn mark_completed(&self, _id: Uuid, _confirmation_event_id: &str) -> Result<(), RepoError> {
            Ok(())
        }

        async fn expire_older_than(&self, _cutoff: DateTime<Utc>) -> Result<u64, RepoError> {
            Ok(0)
        }
    }

    // --- classifier -----------------------------------------------------

    #[test]
    fn classify_routes_known_action_event_types_to_action() {
        assert_eq!(classify("task_assigned"), EventClassification::Action);
        assert_eq!(
            classify("collaboration_request_acknowledged"),
            EventClassification::Action
        );
    }

    /// Robustness to casing convention: `event_type` matching is
    /// case/separator-insensitive (see `normalize_event_type`'s doc
    /// comment), since Nexus's real convention is unconfirmed and this
    /// repo's own source docs use both `snake_case` and `PascalCase` event
    /// names.
    #[test]
    fn classify_matches_known_action_event_types_regardless_of_casing() {
        assert_eq!(classify("CollaborationRequestAcknowledged"), EventClassification::Action);
        assert_eq!(classify("TaskAssigned"), EventClassification::Action);
    }

    #[test]
    fn classify_routes_informational_event_types_to_notification() {
        assert_eq!(classify("account_claim_determined"), EventClassification::Notification);
        assert_eq!(classify("referral_submitted"), EventClassification::Notification);
    }

    /// The conservative default: an `event_type` this repo has never seen
    /// before is never dropped, and never guessed to be actionable — it
    /// surfaces as an informational notification.
    #[test]
    fn classify_defaults_unknown_event_types_to_notification() {
        assert_eq!(classify("some_future_capability_event"), EventClassification::Notification);
    }

    // --- idempotent ingestion --------------------------------------------

    /// The headline idempotency proof (PROMPT-30's explicit requirement):
    /// the *same* event delivered twice (two separate `ingest_events`
    /// calls, simulating two poll cycles) results in exactly one saved row
    /// and exactly one event-bus publish — not two of either.
    #[tokio::test]
    async fn ingest_events_delivers_the_same_event_twice_and_saves_and_publishes_once() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);
        let mut subscription = bus.subscribe();

        let evt = event("evt-1", "account_claim_determined");

        let first = ingest_events(vec![evt.clone()], &notification_repo, &action_repo, &bus).await;
        let second = ingest_events(vec![evt.clone()], &notification_repo, &action_repo, &bus).await;

        assert_eq!(first.inserted(), 1);
        assert_eq!(first.duplicates(), 0);
        assert_eq!(second.inserted(), 0);
        assert_eq!(second.duplicates(), 1);

        assert_eq!(notification_repo.rows.lock().unwrap().len(), 1, "only one row should exist");

        // Exactly one publish reached the subscriber, not two.
        subscription.try_recv().expect("expected exactly one publish");
        assert!(matches!(
            subscription.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    /// Same idempotency proof, but both deliveries arrive in the *same*
    /// batch (one `ingest_events` call, two copies of the same event) — the
    /// within-batch case, not just the across-poll-cycles case.
    #[tokio::test]
    async fn ingest_events_deduplicates_within_a_single_batch() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let evt = event("evt-1", "referral_submitted");

        let result =
            ingest_events(vec![evt.clone(), evt.clone()], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.inserted(), 1);
        assert_eq!(result.duplicates(), 1);
        assert_eq!(notification_repo.rows.lock().unwrap().len(), 1);
    }

    // --- Sales events as real concrete test cases (PROMPT-30) ------------

    /// `AccountClaimDetermined` (informational: a query result, no action
    /// implied), `CollaborationRequestAcknowledged` (action-implying, per
    /// PROMPT-30's own worked example), and `ReferralSubmitted`
    /// (informational: a receipt confirmation) — used as the real
    /// capability events PROMPT-30 asks to be tested against, proving the
    /// mapping logic against Sales without being Sales-specific in the
    /// classifier itself.
    #[tokio::test]
    async fn sales_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let account_claim_determined = event("acd-1", "account_claim_determined");
        let collaboration_request_acknowledged = event("cra-1", "collaboration_request_acknowledged");
        let referral_submitted = event("rs-1", "referral_submitted");

        let result = ingest_events(
            vec![
                account_claim_determined.clone(),
                collaboration_request_acknowledged.clone(),
                referral_submitted.clone(),
            ],
            &notification_repo,
            &action_repo,
            &bus,
        )
        .await;

        assert_eq!(result.inserted(), 3);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(notifications.len(), 2, "AccountClaimDetermined and ReferralSubmitted are informational");
        assert!(notifications.contains_key(&("sales".to_string(), "acd-1".to_string())));
        assert!(notifications.contains_key(&("sales".to_string(), "rs-1".to_string())));

        let actions = action_repo.rows.lock().unwrap();
        assert_eq!(actions.len(), 1, "CollaborationRequestAcknowledged implies a required action");
        assert!(actions.contains_key(&("sales".to_string(), "cra-1".to_string())));
    }

    // --- construction -----------------------------------------------------

    #[test]
    fn build_notification_item_derives_a_title_from_the_event_type() {
        let evt = event("evt-1", "account_claim_determined");
        let item = build_notification_item(&evt).unwrap();

        assert_eq!(item.title(), "Account Claim Determined");
        assert_eq!(item.body(), evt.summary);
        assert_eq!(item.consultant_id(), "consultant-1");
        assert_eq!(item.origin_key(), ("sales", "evt-1"));
    }

    #[test]
    fn build_action_queue_entry_sets_expires_at_relative_to_received_at() {
        let evt = event("evt-1", "collaboration_request_acknowledged");
        let entry = build_action_queue_entry(&evt).unwrap();

        assert_eq!(entry.title(), "Collaboration Request Acknowledged");
        assert_eq!(
            entry.expires_at(),
            evt.received_at + chrono::Duration::hours(DEFAULT_ACTION_QUEUE_ENTRY_TTL_HOURS)
        );
    }

    /// A malformed event (blank `consultant_id`) is rejected, not
    /// panicked on and not silently dropped without a trace.
    #[tokio::test]
    async fn ingest_events_rejects_a_malformed_event_without_aborting_the_batch() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut malformed = event("evt-bad", "account_claim_determined");
        malformed.consultant_id = String::new();
        let valid = event("evt-good", "referral_submitted");

        let result =
            ingest_events(vec![malformed, valid], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.rejected(), 1);
        assert_eq!(result.inserted(), 1);
        assert_eq!(notification_repo.rows.lock().unwrap().len(), 1);
    }

    // --- deserialization ---------------------------------------------------

    /// Proves `CapabilityEventReceived` actually decodes from the wire
    /// shape `bff-api`'s polling loop will hand it — including the
    /// provisional `consultant_id` addition.
    #[test]
    fn capability_event_received_deserializes_from_the_documented_envelope_shape() {
        let json = serde_json::json!({
            "origin_capability": "sales",
            "origin_event_id": "evt-1",
            "event_type": "collaboration_request_acknowledged",
            "summary": "Sales acknowledged your collaboration request.",
            "deep_link": "https://app.example.com/sales/collab/1",
            "received_at": "2026-01-01T00:00:00Z",
            "consultant_id": "consultant-1",
        });

        let parsed: CapabilityEventReceived = serde_json::from_value(json).unwrap();

        assert_eq!(parsed.origin_capability, "sales");
        assert_eq!(parsed.origin_event_id, "evt-1");
        assert_eq!(parsed.event_type, "collaboration_request_acknowledged");
        assert_eq!(parsed.consultant_id, "consultant-1");
        assert_eq!(parsed.received_at, t0());
    }
}
