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
use uuid::Uuid;

use crate::{
    ActionQueueEntry, ActionQueueEntryError, ActionQueueRepository, ActionState, NotificationItem,
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
    /// **PROMPT-38 addition, confirmation events only.** The `origin_event_id`
    /// of the *original* event that created the `ActionQueueEntry` this event
    /// confirms (e.g. the `task_assigned` event's own `origin_event_id`) —
    /// see [`CONFIRMATION_EVENT_TYPES`]'s doc comment for the full rationale.
    /// `None`/absent for every ordinary notification- or action-classified
    /// event; `#[serde(default)]` so existing payloads that predate this
    /// field still deserialize.
    #[serde(default)]
    pub related_origin_event_id: Option<String>,
}

/// Whether a [`CapabilityEventReceived`] implies a required consultant
/// action ([`ActionQueueEntry`]) or is purely informational
/// ([`NotificationItem`]) — see [`classify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClassification {
    Notification,
    Action,
    /// **PROMPT-38 addition.** The event is the owning capability's
    /// confirmation that a previously-created [`ActionQueueEntry`] is
    /// actually done — see [`CONFIRMATION_EVENT_TYPES`]'s doc comment. Never
    /// creates a new aggregate; routes to [`ingest_confirmation`] instead of
    /// [`ingest_notification`]/[`ingest_action`].
    Confirmation,
}

/// Known `event_type`s that imply a required consultant action, per
/// PROMPT-30's own examples (`task_assigned`,
/// `collaboration_request_acknowledged`). Matched case/separator-insensitive
/// (see [`normalize_event_type`]) since Nexus's real casing convention for
/// `event_type` is not yet confirmed (some source docs use `snake_case`,
/// others use the `PascalCase` event names from `../ddd/domain-events.md`).
///
/// **This list is expected to grow.** It is intentionally small and
/// explicit today; each subsequent capability integrated in Phase 4 that has
/// action-implying events should add its normalized `event_type`(s) here.
/// Unknown/future `event_type`s are never silently dropped — [`classify`]
/// defaults them to [`EventClassification::Notification`], the conservative
/// choice: an unrecognized event still reaches the consultant as an
/// informational item, rather than being lost or (worse) incorrectly
/// treated as actionable when this repo doesn't yet know what action it
/// implies.
///
/// # PROMPT-34 (Commit ACL) additions: `proposal_accepted` only
/// Commit's three inbound events (`anti-corruption-layers.md` §2:
/// `ProposalCreated`, `ProposalStatusChanged`, `ProposalAccepted`) were
/// each individually judged against "does this imply the consultant must
/// now go *do* something, beyond just being told":
/// - `ProposalCreated` -> **notification**. It fires right after this
///   repo's own `POST /api/commit/proposals` call already succeeded
///   (`crate::commit`) — the consultant is already looking at the new
///   proposal by the time this event would arrive; it's a confirmation
///   echo, not a prompt to act.
/// - `ProposalStatusChanged` -> **notification**. A generic status
///   transition (e.g. `draft -> internal_review`) is informational by
///   nature — there is no single action this repo could name generically
///   for "a proposal's status changed" the way `task_assigned` names one
///   concrete action ("go look at your new task"). Narrower, more specific
///   status-change events can be split out and reclassified individually
///   later if a specific one turns out to need it.
/// - `ProposalAccepted` -> **action** (added to this list). Unlike a
///   generic status change, an accepted proposal is the one Commit status
///   transition that concretely implies the consultant has real follow-up
///   work waiting (e.g. kicking off the engagement, coordinating next
///   steps with the client) — the same "acknowledgment implies a required
///   response" reasoning `collaboration_request_acknowledged` was already
///   added under, not a generic informational update.
///
/// # PROMPT-35 (Edu ACL) additions: `training_requirement_due` only
/// Edu's three inbound events (`anti-corruption-layers.md` §3:
/// `CourseCompleted`, `CertificationIssued`, `TrainingRequirementDue`) were
/// each individually judged the same way Commit's were:
/// - `CourseCompleted` -> **notification**. A completion is a confirmation
///   of something the consultant already did, not a prompt to now go do
///   something else.
/// - `CertificationIssued` -> **notification**. Same reasoning: it reports
///   an outcome, it does not itself demand a next step.
/// - `TrainingRequirementDue` -> **action** (added to this list). Unlike a
///   completion/issuance receipt, a due (or approaching-due) training
///   requirement concretely names something the consultant must now go do
///   — the same "names one concrete action" reasoning `task_assigned`'s
///   own doc comment above already establishes, not a generic status
///   update.
/// # PROMPT-37 (Customer ACL) additions: none
/// Customer's two inbound events (`anti-corruption-layers.md` §5:
/// `CustomerHealthChanged`, `CustomerInteractionLogged`) were each judged
/// against the same "does this imply the consultant must now go *do*
/// something, beyond just being told" test the other capabilities' events
/// were:
/// - `CustomerHealthChanged` -> **notification**. A health-status change is
///   informational — `anti-corruption-layers.md` §5 itself describes both
///   inbound events as feeding "dashboard cards / notifications", not an
///   action queue. There is no single concrete action this repo could name
///   for "a customer's health changed" the way `task_assigned` names one.
/// - `CustomerInteractionLogged` -> **notification**. A logged interaction
///   is a receipt of something that already happened, not a prompt to act —
///   same reasoning `ProposalCreated`/`CourseCompleted` were classified
///   under.
///
/// Neither is added to [`ACTION_EVENT_TYPES`]; both classify as
/// [`EventClassification::Notification`] via the existing default.
///
/// # PROMPT-38 (Execution ACL) additions: `delivery_risk_raised` (plus the
/// already-listed `task_assigned`)
/// Execution's three inbound events (`anti-corruption-layers.md` §6:
/// `MilestoneCompleted`, `DeliveryRiskRaised`, `TaskAssigned`) were each
/// individually judged the same way, and the doc itself names two of the
/// three as "natural `ActionQueueEntry` sources":
/// - `MilestoneCompleted` -> **notification**. Like `ProposalCreated`/
///   `CourseCompleted`, a completed milestone is a receipt of something that
///   already happened, not a prompt for the consultant to now go do
///   something.
/// - `DeliveryRiskRaised` -> **action** (added to this list). A newly
///   flagged delivery risk concretely names something the consultant must
///   now go address — the same "names one concrete action" reasoning
///   `task_assigned`'s own doc comment above already establishes.
/// - `TaskAssigned` -> **action**. Already present in this list since
///   PROMPT-30's own worked example (Execution's event of the same name
///   needs no separate entry) — per PROMPT-38's own prompt text, this is
///   also explicitly called out as an `ActionQueueEntry` source requiring
///   **confirmed completion via the owning capability, not a local state
///   flip**: no *route handler* anywhere in this repo ever calls
///   [`crate::ActionQueueEntry::complete`]/
///   [`crate::ActionQueueRepository::mark_completed`] — see
///   `bff-api::execution`'s module docs for how a consultant-initiated
///   completion *request* is routed to Execution without touching this
///   aggregate's state directly. The only caller of `mark_completed` in this
///   entire repo is [`ingest_confirmation`] below, reached exclusively via a
///   `task_completed` confirmation event ingested through this same
///   classify-and-route pipeline — see [`CONFIRMATION_EVENT_TYPES`].
/// # PROMPT-39 (Products ACL) additions: none
/// Products' one inbound event (`anti-corruption-layers.md` §7:
/// `ProductCatalogUpdated`) was judged against the same "does this imply
/// the consultant must now go *do* something, beyond just being told" test
/// the other capabilities' events were:
/// - `ProductCatalogUpdated` -> **notification**. This unit's own prompt
///   text is explicit: Products' catalog-change events are "low priority,
///   unlikely to warrant an `ActionQueueEntry`, just refresh the cache/send
///   a low-severity notification" — a changed catalog entry is informational
///   by nature (a consultant may want to know a product's pricing guidance
///   changed), not a prompt to go take some specific action the way
///   `task_assigned` names one. Not added to this list; classifies as
///   [`EventClassification::Notification`] via the existing default. See
///   `crate::event_ingestion`'s general "refresh the cache" framing for
///   `ProductCatalogUpdated` — no separate cache-invalidation mechanism is
///   added here: the frontend's own TanStack Query cache is what gets
///   refreshed (via the same SSE-pushed-notification -> `invalidateQueries`
///   path ADR-015 already wires up for every other notification), not
///   anything this crate's ingestion pipeline itself needs to special-case.
const ACTION_EVENT_TYPES: &[&str] = &[
    "task_assigned",
    "collaboration_request_acknowledged",
    "proposal_accepted",
    "training_requirement_due",
    "delivery_risk_raised",
];

/// Known `event_type`s that confirm a previously-created [`ActionQueueEntry`]
/// is actually done, per invariant 3 (`../ddd/consultant-experience-context.md`
/// §2.2): "`completed` may only be set in response to a confirmation event
/// routed back through Nexus from the owning capability." Matched the same
/// case/separator-insensitive way as [`ACTION_EVENT_TYPES`] (see
/// [`normalize_event_type`]).
///
/// # PROMPT-38 addition: `task_completed`
/// Neither `../ddd/domain-events.md` §3 (Execution) nor
/// `../ddd/anti-corruption-layers.md` §6 names an explicit "task completed"
/// inbound event — the closest named event, `MilestoneCompleted`, is scoped
/// to milestones, not individual tasks, and carries no `task_id` to
/// correlate back to a `TaskAssigned`-created entry. Invariant 3 is explicit
/// that completion *requires* a confirming event to exist, though
/// (`consultant-experience-context.md` §2.2), and PROMPT-38's own prompt
/// text calls this out directly: "Action queue completion requires a
/// confirmation event from Execution." Rather than leave that invariant
/// unimplementable, `task_completed` is added here as a flagged, provisional
/// assumption — the natural, symmetric counterpart to `task_assigned`
/// (same naming convention, same capability) — pending Nexus's actual
/// Execution event contract confirming or correcting the exact name. See
/// [`CapabilityEventReceived::related_origin_event_id`]'s doc comment for
/// how this event correlates back to the entry it confirms, and
/// [`ingest_confirmation`] for the completion logic itself.
///
/// **This list is expected to grow** exactly as [`ACTION_EVENT_TYPES`]'s own
/// doc comment describes — each capability with its own completion-implying
/// event(s) should add its normalized `event_type`(s) here as it integrates.
const CONFIRMATION_EVENT_TYPES: &[&str] = &["task_completed"];

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
    if CONFIRMATION_EVENT_TYPES.iter().any(|known| normalize_event_type(known) == normalized) {
        EventClassification::Confirmation
    } else if ACTION_EVENT_TYPES.iter().any(|known| normalize_event_type(known) == normalized) {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestedEvent {
    Notification(NotificationItem),
    Action(ActionQueueEntry),
}

/// Wire shape of the payload passed to Postgres `NOTIFY <channel>,
/// '<payload>'` (ADR-014's cross-instance SSE fan-out bridge, PROMPT-32).
///
/// # Pointer, not the full event — and why
/// Postgres caps a `NOTIFY` payload at 8000 bytes, server-enforced, with no
/// way for a producer to detect the cutoff ahead of time other than staying
/// well clear of it. Neither [`NotificationItem`] nor [`ActionQueueEntry`]
/// bound `title`/`body`'s length (structural, not runtime-checked) — a full
/// [`IngestedEvent`] JSON payload is *usually* small (a short title/body/
/// deep_link), but "usually" is not a safe bet against a hard server-side
/// limit a producer can't recover from mid-`NOTIFY`. So this repo instead
/// NOTIFYs a minimal pointer — `kind` plus `id`, comfortably under 100 bytes
/// regardless of the aggregate's actual text length — and has every
/// listener re-fetch the full aggregate from Postgres by `id`
/// ([`NotificationRepository::find_by_id`] /
/// [`ActionQueueRepository::find_by_id`], added for exactly this purpose;
/// see [`hydrate_notify_pointer`]). This trades one extra indexed read per
/// notification for a payload size that can never blow the 8000-byte
/// limit — the safer default absent a proven, tight bound on title/body
/// length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventNotifyPointer {
    Notification { id: Uuid },
    ActionQueueEntry { id: Uuid },
}

impl From<&IngestedEvent> for EventNotifyPointer {
    fn from(event: &IngestedEvent) -> Self {
        match event {
            IngestedEvent::Notification(item) => Self::Notification { id: item.id() },
            IngestedEvent::Action(entry) => Self::ActionQueueEntry { id: entry.id() },
        }
    }
}

/// The Postgres `NOTIFY`/`LISTEN` channel name this repo's cross-instance
/// SSE fan-out bridge uses (ADR-014, PROMPT-32) — shared verbatim between
/// the publisher (`persistence::PgNotifyPublisher`) and every instance's
/// listener bridge (`bff-api::event_notify_bridge`).
pub const EVENT_NOTIFY_CHANNEL: &str = "bff_ingested_events";

/// Reconstructs the full [`IngestedEvent`] a NOTIFY payload pointed to (see
/// [`EventNotifyPointer`]'s doc comment for why this indirection exists),
/// by `id`, via whichever repository matches the pointer's `kind`. Returns
/// `Ok(None)` — not an error — if the id is unknown to the repository: a
/// listener bridge treats that as "skip this notification" rather than a
/// hard failure (see `bff-api::event_notify_bridge` for the call site).
pub async fn hydrate_notify_pointer(
    pointer: EventNotifyPointer,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
) -> Result<Option<IngestedEvent>, RepoError> {
    match pointer {
        EventNotifyPointer::Notification { id } => {
            Ok(notification_repo.find_by_id(id).await?.map(IngestedEvent::Notification))
        }
        EventNotifyPointer::ActionQueueEntry { id } => {
            Ok(action_queue_repo.find_by_id(id).await?.map(IngestedEvent::Action))
        }
    }
}

/// Anything ingestion can hand a freshly-inserted [`IngestedEvent`] to.
/// [`EventBus`] implements this directly — single-instance, in-process
/// delivery, and still what this module's own unit tests exercise below.
/// The production cross-instance path
/// (`persistence::PgNotifyPublisher`, ADR-014/PROMPT-32) is the other
/// implementation: rather than writing straight into a local [`EventBus`],
/// it issues a Postgres `NOTIFY` so *every* `bff-api` instance's own
/// listener bridge learns about the event — including the instance that
/// did the ingesting, which now receives its own event back through the
/// same NOTIFY/LISTEN round-trip every other instance does. That round-trip
/// is not a meaningful latency/ordering concern in practice: a `NOTIFY`
/// issued inside the same Postgres server a `LISTEN`ing connection is
/// already attached to is delivered near-instantly (sub-millisecond,
/// same-process signaling within Postgres — no polling involved on either
/// side), and delivery order per channel matches commit order, so the
/// ingesting instance sees its own event essentially as fast as it would
/// have via a direct local publish, just through one extra (cheap) hop.
#[async_trait::async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: IngestedEvent);
}

#[async_trait::async_trait]
impl EventPublisher for EventBus {
    async fn publish(&self, event: IngestedEvent) {
        // Calls the inherent `EventBus::publish` below (inherent methods
        // take priority over trait methods with the same name, so this is
        // not infinite recursion) — this impl exists purely so `EventBus`
        // satisfies the `EventPublisher` trait object bound `ingest_events`
        // et al. take, letting a caller pass either an `&EventBus` (tests,
        // and this module's own examples) or an
        // `&persistence::PgNotifyPublisher` (production) interchangeably.
        EventBus::publish(self, event);
    }
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
    /// **PROMPT-38 addition.** A [`EventClassification::Confirmation`] event
    /// was processed against [`ActionQueueEntry`] via [`ingest_confirmation`].
    /// `transitioned` distinguishes "found the entry and moved it to
    /// `Completed`" from "found nothing to do" (no matching entry, or the
    /// entry wasn't `InProgress`) — both are legitimate, non-error outcomes,
    /// not [`IngestionOutcome::Rejected`]: a redelivered confirmation, or one
    /// that races ahead of the consultant clicking "take action", is
    /// expected traffic, not a failure.
    Confirmed { origin_capability: String, origin_event_id: String, transitioned: bool },
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

    /// Number of confirmation events processed (PROMPT-38), regardless of
    /// whether they actually transitioned an entry — see
    /// [`Self::completed_confirmations`] for the narrower count.
    pub fn confirmed(&self) -> usize {
        self.outcomes.iter().filter(|o| matches!(o, IngestionOutcome::Confirmed { .. })).count()
    }

    /// Number of confirmation events that actually moved an
    /// [`ActionQueueEntry`] to [`ActionState::Completed`] — the narrower,
    /// "something really happened" count within [`Self::confirmed`].
    pub fn completed_confirmations(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, IngestionOutcome::Confirmed { transitioned: true, .. }))
            .count()
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
/// hands it to `publisher` (an [`EventPublisher`] — [`EventBus`] directly
/// for single-instance/test use, `persistence::PgNotifyPublisher` in
/// production, see that trait's doc comment). See the module docs for the
/// two-layer dedup this relies on (this function is layer 1, the
/// correctness guarantee).
///
/// Never panics or short-circuits the batch on one bad event: a malformed
/// event (fails aggregate construction) or a repository failure is recorded
/// as [`IngestionOutcome::Rejected`] and processing continues with the next
/// event (input validation/failure isolation at the ingestion boundary).
pub async fn ingest_events(
    events: Vec<CapabilityEventReceived>,
    notification_repo: &dyn NotificationRepository,
    action_queue_repo: &dyn ActionQueueRepository,
    publisher: &dyn EventPublisher,
) -> IngestionResult {
    let mut result = IngestionResult::default();

    for event in events {
        let classification = classify(&event.event_type);
        let outcome = match classification {
            EventClassification::Notification => {
                ingest_notification(&event, notification_repo, publisher).await
            }
            EventClassification::Action => {
                ingest_action(&event, action_queue_repo, publisher).await
            }
            EventClassification::Confirmation => {
                ingest_confirmation(&event, action_queue_repo, publisher).await
            }
        };
        result.outcomes.push(outcome);
    }

    result
}

async fn ingest_notification(
    event: &CapabilityEventReceived,
    notification_repo: &dyn NotificationRepository,
    publisher: &dyn EventPublisher,
) -> IngestionOutcome {
    let item = match build_notification_item(event) {
        Ok(item) => item,
        Err(err) => return rejected(event, err.to_string()),
    };

    match notification_repo.save(&item).await {
        Ok(save_outcome) => {
            if save_outcome == SaveOutcome::Inserted {
                publisher.publish(IngestedEvent::Notification(item)).await;
            }
            saved(event, EventClassification::Notification, save_outcome)
        }
        Err(err) => rejected(event, repo_error_reason(err)),
    }
}

async fn ingest_action(
    event: &CapabilityEventReceived,
    action_queue_repo: &dyn ActionQueueRepository,
    publisher: &dyn EventPublisher,
) -> IngestionOutcome {
    let entry = match build_action_queue_entry(event) {
        Ok(entry) => entry,
        Err(err) => return rejected(event, err.to_string()),
    };

    match action_queue_repo.save(&entry).await {
        Ok(save_outcome) => {
            if save_outcome == SaveOutcome::Inserted {
                publisher.publish(IngestedEvent::Action(entry)).await;
            }
            saved(event, EventClassification::Action, save_outcome)
        }
        Err(err) => rejected(event, repo_error_reason(err)),
    }
}

/// Processes a [`EventClassification::Confirmation`] event: resolves the
/// [`ActionQueueEntry`] it confirms via [`ActionQueueRepository::find_by_origin_event`]
/// (keyed by `event.related_origin_event_id`, not `event.origin_event_id` —
/// see [`CapabilityEventReceived::related_origin_event_id`]'s doc comment)
/// and, if it is currently [`ActionState::InProgress`], calls
/// [`ActionQueueRepository::mark_completed`] with the confirmation event's
/// own `origin_event_id` as the audit-trail `confirmation_event_id`
/// (invariant 3's required, non-empty proof-of-confirmation argument).
///
/// This function — reached only via [`CONFIRMATION_EVENT_TYPES`] — is the
/// **only** call site of `mark_completed` anywhere in this repo; no route
/// handler ever calls it directly (see `ACTION_EVENT_TYPES`'s PROMPT-38 doc
/// comment).
///
/// Never creates a new aggregate and never touches [`NotificationRepository`]/
/// [`ActionQueueRepository::save`] — a confirmation event is not itself an
/// idempotent-ingestion candidate the way [`ingest_notification`]/
/// [`ingest_action`]'s events are (there is no new row to dedupe against);
/// idempotency here instead falls out naturally from
/// [`ActionQueueEntry::complete`]'s own state-machine guard — a redelivered
/// confirmation for an already-[`ActionState::Completed`] entry is a no-op
/// exactly like the first successful call already made it so.
async fn ingest_confirmation(
    event: &CapabilityEventReceived,
    action_queue_repo: &dyn ActionQueueRepository,
    publisher: &dyn EventPublisher,
) -> IngestionOutcome {
    let related_origin_event_id = match event.related_origin_event_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id,
        _ => {
            return rejected(
                event,
                "confirmation event is missing a non-empty related_origin_event_id".to_string(),
            )
        }
    };

    let existing = match action_queue_repo.find_by_origin_event(&event.origin_capability, related_origin_event_id).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return confirmed(event, false),
        Err(err) => return rejected(event, repo_error_reason(err)),
    };

    if existing.action_state() != ActionState::InProgress {
        // Legitimate no-op — see the doc comment above and
        // `IngestionOutcome::Confirmed`'s own doc comment for why this is
        // not treated as a failure.
        return confirmed(event, false);
    }

    match action_queue_repo.mark_completed(existing.id(), &event.origin_event_id).await {
        Ok(()) => {
            if let Ok(Some(updated)) = action_queue_repo.find_by_id(existing.id()).await {
                publisher.publish(IngestedEvent::Action(updated)).await;
            }
            confirmed(event, true)
        }
        Err(err) => rejected(event, repo_error_reason(err)),
    }
}

fn confirmed(event: &CapabilityEventReceived, transitioned: bool) -> IngestionOutcome {
    IngestionOutcome::Confirmed {
        origin_capability: event.origin_capability.clone(),
        origin_event_id: event.origin_event_id.clone(),
        transitioned,
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
            related_origin_event_id: None,
        }
    }

    /// Same shape as [`event`], plus a non-empty `related_origin_event_id` —
    /// the confirmation-event test helper.
    fn confirmation_event(origin_event_id: &str, event_type: &str, related_origin_event_id: &str) -> CapabilityEventReceived {
        CapabilityEventReceived {
            related_origin_event_id: Some(related_origin_event_id.to_string()),
            ..event(origin_event_id, event_type)
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

        async fn find_by_id(&self, id: Uuid) -> Result<Option<NotificationItem>, RepoError> {
            Ok(self.rows.lock().unwrap().values().find(|item| item.id() == id).cloned())
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

        async fn find_by_id(&self, id: Uuid) -> Result<Option<ActionQueueEntry>, RepoError> {
            Ok(self.rows.lock().unwrap().values().find(|entry| entry.id() == id).cloned())
        }

        async fn find_by_origin_event(
            &self,
            origin_capability: &str,
            origin_event_id: &str,
        ) -> Result<Option<ActionQueueEntry>, RepoError> {
            Ok(self.rows.lock().unwrap().get(&(origin_capability.to_string(), origin_event_id.to_string())).cloned())
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

        async fn mark_started(&self, id: Uuid) -> Result<(), RepoError> {
            let mut rows = self.rows.lock().unwrap();
            if let Some(entry) = rows.values_mut().find(|entry| entry.id() == id) {
                // Lenient no-op on an invalid transition, mirroring
                // `PgActionQueueRepository::mark_started`'s WHERE-guarded
                // semantics — not an error at this layer.
                let _ = entry.start();
            }
            Ok(())
        }

        async fn mark_completed(&self, id: Uuid, confirmation_event_id: &str) -> Result<(), RepoError> {
            if confirmation_event_id.trim().is_empty() {
                return Err(RepoError::OperationFailed(
                    ActionQueueEntryError::EmptyConfirmationEventId.to_string(),
                ));
            }
            let mut rows = self.rows.lock().unwrap();
            if let Some(entry) = rows.values_mut().find(|entry| entry.id() == id) {
                // Lenient no-op on an invalid transition (e.g. still
                // `Pending`), mirroring `PgActionQueueRepository::mark_completed`'s
                // WHERE-guarded semantics — not an error at this layer.
                let _ = entry.complete(confirmation_event_id);
            }
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

    // --- Commit events as real concrete test cases (PROMPT-34) -----------

    /// `ProposalCreated`/`ProposalStatusChanged` (informational) and
    /// `ProposalAccepted` (action-implying, per `ACTION_EVENT_TYPES`'s
    /// PROMPT-34 doc comment) — used as the real Commit events PROMPT-34
    /// asks to be classified against, matching `sales_events_are_classified_
    /// and_ingested_as_documented`'s shape for Sales above.
    #[tokio::test]
    async fn commit_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut proposal_created = event("pc-1", "proposal_created");
        proposal_created.origin_capability = "commit".to_string();
        let mut proposal_status_changed = event("psc-1", "proposal_status_changed");
        proposal_status_changed.origin_capability = "commit".to_string();
        let mut proposal_accepted = event("pa-1", "proposal_accepted");
        proposal_accepted.origin_capability = "commit".to_string();

        let result = ingest_events(
            vec![proposal_created.clone(), proposal_status_changed.clone(), proposal_accepted.clone()],
            &notification_repo,
            &action_repo,
            &bus,
        )
        .await;

        assert_eq!(result.inserted(), 3);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(notifications.len(), 2, "ProposalCreated and ProposalStatusChanged are informational");
        assert!(notifications.contains_key(&("commit".to_string(), "pc-1".to_string())));
        assert!(notifications.contains_key(&("commit".to_string(), "psc-1".to_string())));

        let actions = action_repo.rows.lock().unwrap();
        assert_eq!(actions.len(), 1, "ProposalAccepted implies required follow-up work");
        assert!(actions.contains_key(&("commit".to_string(), "pa-1".to_string())));
    }

    #[test]
    fn classify_matches_proposal_accepted_regardless_of_casing() {
        assert_eq!(classify("proposal_accepted"), EventClassification::Action);
        assert_eq!(classify("ProposalAccepted"), EventClassification::Action);
    }

    #[test]
    fn classify_routes_proposal_created_and_status_changed_to_notification() {
        assert_eq!(classify("proposal_created"), EventClassification::Notification);
        assert_eq!(classify("ProposalCreated"), EventClassification::Notification);
        assert_eq!(classify("proposal_status_changed"), EventClassification::Notification);
        assert_eq!(classify("ProposalStatusChanged"), EventClassification::Notification);
    }

    // --- Edu events as real concrete test cases (PROMPT-35) ---------------

    /// `CourseCompleted`/`CertificationIssued` (informational) and
    /// `TrainingRequirementDue` (action-implying, per `ACTION_EVENT_TYPES`'s
    /// PROMPT-35 doc comment) — used as the real Edu events PROMPT-35 asks
    /// to be classified against, matching
    /// `commit_events_are_classified_and_ingested_as_documented`'s shape for
    /// Commit above.
    #[tokio::test]
    async fn edu_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut course_completed = event("cc-1", "course_completed");
        course_completed.origin_capability = "edu".to_string();
        let mut certification_issued = event("ci-1", "certification_issued");
        certification_issued.origin_capability = "edu".to_string();
        let mut training_requirement_due = event("trd-1", "training_requirement_due");
        training_requirement_due.origin_capability = "edu".to_string();

        let result = ingest_events(
            vec![course_completed.clone(), certification_issued.clone(), training_requirement_due.clone()],
            &notification_repo,
            &action_repo,
            &bus,
        )
        .await;

        assert_eq!(result.inserted(), 3);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(notifications.len(), 2, "CourseCompleted and CertificationIssued are informational");
        assert!(notifications.contains_key(&("edu".to_string(), "cc-1".to_string())));
        assert!(notifications.contains_key(&("edu".to_string(), "ci-1".to_string())));

        let actions = action_repo.rows.lock().unwrap();
        assert_eq!(actions.len(), 1, "TrainingRequirementDue implies required follow-up work");
        assert!(actions.contains_key(&("edu".to_string(), "trd-1".to_string())));
    }

    #[test]
    fn classify_matches_training_requirement_due_regardless_of_casing() {
        assert_eq!(classify("training_requirement_due"), EventClassification::Action);
        assert_eq!(classify("TrainingRequirementDue"), EventClassification::Action);
    }

    #[test]
    fn classify_routes_course_completed_and_certification_issued_to_notification() {
        assert_eq!(classify("course_completed"), EventClassification::Notification);
        assert_eq!(classify("CourseCompleted"), EventClassification::Notification);
        assert_eq!(classify("certification_issued"), EventClassification::Notification);
        assert_eq!(classify("CertificationIssued"), EventClassification::Notification);
    }

    // --- Customer events as real concrete test cases (PROMPT-37) ---------

    /// `CustomerHealthChanged`/`CustomerInteractionLogged` — both
    /// informational per `ACTION_EVENT_TYPES`'s PROMPT-37 doc comment —
    /// used as the real Customer events PROMPT-37 asks to be classified
    /// against, matching `edu_events_are_classified_and_ingested_as_documented`'s
    /// shape above.
    #[tokio::test]
    async fn customer_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut customer_health_changed = event("chc-1", "customer_health_changed");
        customer_health_changed.origin_capability = "customer".to_string();
        let mut customer_interaction_logged = event("cil-1", "customer_interaction_logged");
        customer_interaction_logged.origin_capability = "customer".to_string();

        let result = ingest_events(
            vec![customer_health_changed.clone(), customer_interaction_logged.clone()],
            &notification_repo,
            &action_repo,
            &bus,
        )
        .await;

        assert_eq!(result.inserted(), 2);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(
            notifications.len(),
            2,
            "CustomerHealthChanged and CustomerInteractionLogged are both informational"
        );
        assert!(notifications.contains_key(&("customer".to_string(), "chc-1".to_string())));
        assert!(notifications.contains_key(&("customer".to_string(), "cil-1".to_string())));

        assert_eq!(action_repo.rows.lock().unwrap().len(), 0, "neither Customer event implies a required action");
    }

    #[test]
    fn classify_routes_customer_events_to_notification() {
        assert_eq!(classify("customer_health_changed"), EventClassification::Notification);
        assert_eq!(classify("CustomerHealthChanged"), EventClassification::Notification);
        assert_eq!(classify("customer_interaction_logged"), EventClassification::Notification);
        assert_eq!(classify("CustomerInteractionLogged"), EventClassification::Notification);
    }

    // --- Execution events as real concrete test cases (PROMPT-38) --------

    /// `MilestoneCompleted` (informational) and `DeliveryRiskRaised`/
    /// `TaskAssigned` (both action-implying, per `ACTION_EVENT_TYPES`'s
    /// PROMPT-38 doc comment) — used as the real Execution events PROMPT-38
    /// asks to be classified against, matching
    /// `customer_events_are_classified_and_ingested_as_documented`'s shape
    /// above.
    #[tokio::test]
    async fn execution_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut milestone_completed = event("mc-1", "milestone_completed");
        milestone_completed.origin_capability = "execution".to_string();
        let mut delivery_risk_raised = event("drr-1", "delivery_risk_raised");
        delivery_risk_raised.origin_capability = "execution".to_string();
        let mut task_assigned = event("ta-1", "task_assigned");
        task_assigned.origin_capability = "execution".to_string();

        let result = ingest_events(
            vec![milestone_completed.clone(), delivery_risk_raised.clone(), task_assigned.clone()],
            &notification_repo,
            &action_repo,
            &bus,
        )
        .await;

        assert_eq!(result.inserted(), 3);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(notifications.len(), 1, "MilestoneCompleted is informational");
        assert!(notifications.contains_key(&("execution".to_string(), "mc-1".to_string())));

        let actions = action_repo.rows.lock().unwrap();
        assert_eq!(actions.len(), 2, "DeliveryRiskRaised and TaskAssigned are both ActionQueueEntry sources");
        assert!(actions.contains_key(&("execution".to_string(), "drr-1".to_string())));
        assert!(actions.contains_key(&("execution".to_string(), "ta-1".to_string())));
    }

    #[test]
    fn classify_matches_delivery_risk_raised_regardless_of_casing() {
        assert_eq!(classify("delivery_risk_raised"), EventClassification::Action);
        assert_eq!(classify("DeliveryRiskRaised"), EventClassification::Action);
    }

    #[test]
    fn classify_routes_milestone_completed_to_notification() {
        assert_eq!(classify("milestone_completed"), EventClassification::Notification);
        assert_eq!(classify("MilestoneCompleted"), EventClassification::Notification);
    }

    // --- Products events as real concrete test cases (PROMPT-39) ---------

    /// `ProductCatalogUpdated` — informational per `ACTION_EVENT_TYPES`'s
    /// PROMPT-39 doc comment — used as the real Products event PROMPT-39
    /// asks to be classified against, matching
    /// `customer_events_are_classified_and_ingested_as_documented`'s shape
    /// above.
    #[tokio::test]
    async fn products_events_are_classified_and_ingested_as_documented() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut product_catalog_updated = event("pcu-1", "product_catalog_updated");
        product_catalog_updated.origin_capability = "products".to_string();

        let result = ingest_events(vec![product_catalog_updated.clone()], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.inserted(), 1);
        assert_eq!(result.rejected(), 0);

        let notifications = notification_repo.rows.lock().unwrap();
        assert_eq!(notifications.len(), 1, "ProductCatalogUpdated is informational, low-priority");
        assert!(notifications.contains_key(&("products".to_string(), "pcu-1".to_string())));

        assert_eq!(action_repo.rows.lock().unwrap().len(), 0, "ProductCatalogUpdated never implies a required action");
    }

    #[test]
    fn classify_routes_product_catalog_updated_to_notification() {
        assert_eq!(classify("product_catalog_updated"), EventClassification::Notification);
        assert_eq!(classify("ProductCatalogUpdated"), EventClassification::Notification);
    }

    // --- confirmation events / ingest_confirmation (PROMPT-38) ------------

    #[test]
    fn classify_matches_task_completed_regardless_of_casing_as_confirmation() {
        assert_eq!(classify("task_completed"), EventClassification::Confirmation);
        assert_eq!(classify("TaskCompleted"), EventClassification::Confirmation);
    }

    /// The headline end-to-end proof for invariant 3
    /// (`consultant-experience-context.md` §2.2): a `TaskAssigned` event
    /// creates a `Pending` `ActionQueueEntry`; a bare consultant "start"
    /// click (`ActionQueueRepository::mark_started`, simulating `POST
    /// /api/action-queue/{id}/start`) moves it to `InProgress`; only then
    /// does a `task_completed` **confirmation** event — never the consultant
    /// directly — move it to `Completed`, and only when it carries the
    /// original `TaskAssigned` event's `origin_event_id` as
    /// `related_origin_event_id`.
    #[tokio::test]
    async fn ingest_confirmation_completes_an_in_progress_entry_created_by_task_assigned() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);
        let mut subscription = bus.subscribe();

        let mut task_assigned = event("ta-1", "task_assigned");
        task_assigned.origin_capability = "execution".to_string();
        let created = ingest_events(vec![task_assigned], &notification_repo, &action_repo, &bus).await;
        assert_eq!(created.inserted(), 1);
        subscription.try_recv().expect("expected a publish for the TaskAssigned action item");

        let entry_id = {
            let entries = action_repo.find_by_consultant_id("consultant-1").await.unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].action_state(), ActionState::Pending);
            entries[0].id()
        };

        // The bare consultant click — `Pending -> InProgress`. Not part of
        // `ingest_events`/`ingest_confirmation`, just simulating the
        // `POST /api/action-queue/{id}/start` route's own effect for this
        // test's purposes.
        action_repo.mark_started(entry_id).await.unwrap();
        assert_eq!(
            action_repo.find_by_id(entry_id).await.unwrap().unwrap().action_state(),
            ActionState::InProgress
        );

        let mut confirmation = confirmation_event("tc-1", "task_completed", "ta-1");
        confirmation.origin_capability = "execution".to_string();

        let result = ingest_events(vec![confirmation], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.confirmed(), 1);
        assert_eq!(result.completed_confirmations(), 1);
        assert_eq!(result.rejected(), 0);

        let completed_entry = action_repo.find_by_id(entry_id).await.unwrap().unwrap();
        assert_eq!(completed_entry.action_state(), ActionState::Completed);

        // The completion was published too — not just the original creation.
        let published = subscription.try_recv().expect("expected a publish for the completion");
        match published {
            IngestedEvent::Action(entry) => assert_eq!(entry.action_state(), ActionState::Completed),
            other => panic!("expected an Action publish, got {other:?}"),
        }
    }

    /// A confirmation event whose `related_origin_event_id` matches no known
    /// entry is a lenient no-op — not a rejection (e.g. ordering races, or a
    /// confirmation for an entry this consultant's view doesn't have).
    #[tokio::test]
    async fn ingest_confirmation_is_a_lenient_noop_when_no_matching_entry_exists() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let confirmation = confirmation_event("tc-1", "task_completed", "does-not-exist");
        let result = ingest_events(vec![confirmation], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.confirmed(), 1);
        assert_eq!(result.completed_confirmations(), 0);
        assert_eq!(result.rejected(), 0);
    }

    /// A confirmation arriving for an entry the consultant never started
    /// (still `Pending`) must not complete it — invariant 3's "`Pending ->
    /// Completed` is never valid, even with a confirmation" guard, exercised
    /// here through the ingestion pipeline rather than the aggregate
    /// directly.
    #[tokio::test]
    async fn ingest_confirmation_does_not_complete_a_still_pending_entry() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut task_assigned = event("ta-1", "task_assigned");
        task_assigned.origin_capability = "execution".to_string();
        ingest_events(vec![task_assigned], &notification_repo, &action_repo, &bus).await;

        let mut confirmation = confirmation_event("tc-1", "task_completed", "ta-1");
        confirmation.origin_capability = "execution".to_string();
        let result = ingest_events(vec![confirmation], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.confirmed(), 1);
        assert_eq!(result.completed_confirmations(), 0);

        let entries = action_repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(entries[0].action_state(), ActionState::Pending, "must not have been completed");
    }

    /// A confirmation event missing `related_origin_event_id` entirely is
    /// malformed (there is nothing to confirm) — rejected, not silently
    /// dropped.
    #[tokio::test]
    async fn ingest_confirmation_rejects_a_missing_related_origin_event_id() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let confirmation = event("tc-1", "task_completed");
        assert_eq!(confirmation.related_origin_event_id, None);

        let result = ingest_events(vec![confirmation], &notification_repo, &action_repo, &bus).await;

        assert_eq!(result.rejected(), 1);
        assert_eq!(result.confirmed(), 0);
    }

    /// The same confirmation redelivered twice only completes the entry
    /// once — the second call is a no-op (already `Completed`), matching
    /// `ActionQueueEntry::complete`'s own terminal-state guard.
    #[tokio::test]
    async fn ingest_confirmation_redelivered_is_idempotent() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let bus = EventBus::new(16);

        let mut task_assigned = event("ta-1", "task_assigned");
        task_assigned.origin_capability = "execution".to_string();
        ingest_events(vec![task_assigned], &notification_repo, &action_repo, &bus).await;
        let entry_id = action_repo.find_by_consultant_id("consultant-1").await.unwrap()[0].id();
        action_repo.mark_started(entry_id).await.unwrap();

        let mut confirmation = confirmation_event("tc-1", "task_completed", "ta-1");
        confirmation.origin_capability = "execution".to_string();

        let first = ingest_events(vec![confirmation.clone()], &notification_repo, &action_repo, &bus).await;
        let second = ingest_events(vec![confirmation], &notification_repo, &action_repo, &bus).await;

        assert_eq!(first.completed_confirmations(), 1);
        assert_eq!(second.completed_confirmations(), 0, "already Completed — the second delivery is a no-op");
        assert_eq!(second.rejected(), 0);

        let entry = action_repo.find_by_id(entry_id).await.unwrap().unwrap();
        assert_eq!(entry.action_state(), ActionState::Completed);
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

    // --- EventNotifyPointer / hydrate_notify_pointer (PROMPT-32) ----------

    /// The NOTIFY payload's wire shape: `{"kind":"notification","id":"..."}`
    /// / `{"kind":"action_queue_entry","id":"..."}` — small and stable
    /// regardless of the underlying aggregate's `title`/`body` length (the
    /// whole point of pointing rather than embedding, see the type's doc
    /// comment).
    #[test]
    fn event_notify_pointer_serializes_to_the_documented_wire_shape() {
        let id = Uuid::new_v4();

        let notification_json =
            serde_json::to_value(EventNotifyPointer::Notification { id }).unwrap();
        assert_eq!(
            notification_json,
            serde_json::json!({"kind": "notification", "id": id.to_string()})
        );

        let action_json =
            serde_json::to_value(EventNotifyPointer::ActionQueueEntry { id }).unwrap();
        assert_eq!(
            action_json,
            serde_json::json!({"kind": "action_queue_entry", "id": id.to_string()})
        );
    }

    /// `EventNotifyPointer` round-trips through JSON — the shape every
    /// `PgListener` payload is decoded back into.
    #[test]
    fn event_notify_pointer_round_trips_through_json() {
        for pointer in [
            EventNotifyPointer::Notification { id: Uuid::new_v4() },
            EventNotifyPointer::ActionQueueEntry { id: Uuid::new_v4() },
        ] {
            let json = serde_json::to_string(&pointer).unwrap();
            let decoded: EventNotifyPointer = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, pointer);
        }
    }

    /// `From<&IngestedEvent>` picks the right variant and carries the
    /// aggregate's own `id`, not a fresh one.
    #[test]
    fn event_notify_pointer_from_ingested_event_carries_the_aggregate_id() {
        let notification = notification_for_hydrate_tests("consultant-1", "evt-1");
        let notification_id = notification.id();
        let ingested = IngestedEvent::Notification(notification);
        assert_eq!(
            EventNotifyPointer::from(&ingested),
            EventNotifyPointer::Notification { id: notification_id }
        );

        let entry = action_entry_for_hydrate_tests("consultant-1", "evt-2");
        let entry_id = entry.id();
        let ingested = IngestedEvent::Action(entry);
        assert_eq!(
            EventNotifyPointer::from(&ingested),
            EventNotifyPointer::ActionQueueEntry { id: entry_id }
        );
    }

    fn notification_for_hydrate_tests(consultant_id: &str, origin_event_id: &str) -> NotificationItem {
        NotificationItem::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Referral submitted",
            "A new referral was submitted for review.",
            None,
            t0(),
        )
        .unwrap()
    }

    fn action_entry_for_hydrate_tests(consultant_id: &str, origin_event_id: &str) -> ActionQueueEntry {
        ActionQueueEntry::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Collaboration request",
            "A collaboration request needs your response.",
            None,
            t0() + chrono::Duration::hours(72),
            t0(),
        )
        .unwrap()
    }

    /// `hydrate_notify_pointer` re-fetches the full aggregate the pointer
    /// named, via the matching repository — the reconstruction half of the
    /// NOTIFY/LISTEN bridge, exercised here against the same in-memory
    /// mocks the rest of this module already uses (no Postgres needed to
    /// prove this plumbing; the real `find_by_id` SQL is covered by
    /// `persistence`'s own repository tests).
    #[tokio::test]
    async fn hydrate_notify_pointer_reconstructs_a_notification() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let item = notification_for_hydrate_tests("consultant-1", "evt-1");
        notification_repo.save(&item).await.unwrap();

        let hydrated =
            hydrate_notify_pointer(EventNotifyPointer::Notification { id: item.id() }, &notification_repo, &action_repo)
                .await
                .unwrap();

        assert_eq!(hydrated, Some(IngestedEvent::Notification(item)));
    }

    #[tokio::test]
    async fn hydrate_notify_pointer_reconstructs_an_action_queue_entry() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();
        let entry = action_entry_for_hydrate_tests("consultant-1", "evt-1");
        action_repo.save(&entry).await.unwrap();

        let hydrated = hydrate_notify_pointer(
            EventNotifyPointer::ActionQueueEntry { id: entry.id() },
            &notification_repo,
            &action_repo,
        )
        .await
        .unwrap();

        assert_eq!(hydrated, Some(IngestedEvent::Action(entry)));
    }

    /// A pointer naming an id the repository doesn't have is `Ok(None)`,
    /// not an error — the listener bridge's "skip, don't crash" contract.
    #[tokio::test]
    async fn hydrate_notify_pointer_returns_none_for_an_unknown_id() {
        let notification_repo = MockNotificationRepo::default();
        let action_repo = MockActionQueueRepo::default();

        let hydrated = hydrate_notify_pointer(
            EventNotifyPointer::Notification { id: Uuid::new_v4() },
            &notification_repo,
            &action_repo,
        )
        .await
        .unwrap();

        assert_eq!(hydrated, None);
    }
}
