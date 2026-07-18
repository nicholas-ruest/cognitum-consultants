//! `NotificationItem` aggregate (`consultant-experience-context.md` §2.2)
//! and its repository port (`NotificationRepository`, implemented against
//! Postgres in `persistence`, ADR-010, PROMPT-29).
//!
//! The Notification & Action Queue context's display-safe, event-driven
//! inbox item: a normalized, deduped projection of an upstream Nexus
//! `CapabilityEventReceived` envelope that is purely informational (no
//! consultant action required — see [`crate::ActionQueueEntry`] for the
//! action-required counterpart).
//!
//! Invariants enforced here:
//! 1. **Idempotent ingestion.** `(origin_capability, origin_event_id)`
//!    identifies the upstream event this item was created from
//!    ([`NotificationItem::origin_key`]); re-delivery of the same event must
//!    not create a duplicate item. `bff-core` cannot enforce DB-level
//!    uniqueness itself (see `persistence`'s `notification_items` migration
//!    for the real `UNIQUE` constraint), but [`NotificationItem::origin_key`]
//!    lets any caller dedupe two in-memory instances by that key before
//!    ever reaching a repository, and is exactly what
//!    [`NotificationRepository::save`]'s no-op-on-conflict semantics are
//!    built around.
//! 2. **Display-safe summary only.** [`NotificationItem::title`] and
//!    [`NotificationItem::body`] are short text and [`NotificationItem::deep_link`]
//!    is an opaque reference string — there is no field on this type that
//!    could hold a full business object, and no constructor accepts one.
//!    Structural, not runtime-checked, same as
//!    `CrossCapabilityWorkflowSession`'s opaque-reference invariant.
//! 3. **Read state is one-way.** [`NotificationItem::mark_read`] succeeds
//!    exactly once per item: calling it again on an already-[`ReadState::Read`]
//!    item is rejected with [`NotificationItemError::AlreadyRead`], not a
//!    silent no-op. **Design decision** (the DDD doc explicitly leaves this
//!    to implementation judgment): an explicit error, not a no-op, was
//!    chosen to match this crate's existing convention
//!    (`WorkflowSessionStatus::is_valid_transition`'s `InvalidTransition`
//!    error for any out-of-order transition attempt) and so the one-way
//!    rule is testable as a hard invariant rather than an easily-overlooked
//!    silent no-op. A caller that wants idempotent "mark read" UX (e.g. a
//!    consultant re-clicking an already-read item) should treat
//!    `Err(AlreadyRead)` as a benign outcome, not surface it as a failure —
//!    see [`NotificationRepository::mark_read`]'s doc comment for how the
//!    repository-level convenience method handles this more leniently.
//! 4. **Belongs to exactly one consultant.** [`NotificationItem::consultant_id`]
//!    is a single, non-empty, immutable field set at construction — a
//!    capability event relevant to multiple consultants fans out into
//!    multiple `NotificationItem`s upstream of this type, never a single
//!    shared row.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::SaveOutcome;

/// One-way read-tracking status for a [`NotificationItem`] (invariant 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadState {
    Unread,
    Read,
}

impl ReadState {
    /// The wire/storage string for this state (DB `read_state` column).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Read => "read",
        }
    }
}

impl fmt::Display for ReadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A read-state string that isn't a known [`ReadState`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown notification read state: {0:?}")]
pub struct ParseReadStateError(String);

impl FromStr for ReadState {
    type Err = ParseReadStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unread" => Ok(Self::Unread),
            "read" => Ok(Self::Read),
            other => Err(ParseReadStateError(other.to_string())),
        }
    }
}

/// Display-safe, deduped inbox item (`consultant-experience-context.md`
/// §2.2). Root of its own aggregate — no child entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationItem {
    id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_event_id: String,
    title: String,
    body: String,
    deep_link: Option<String>,
    read_state: ReadState,
    created_at: DateTime<Utc>,
}

/// Errors constructing/mutating a [`NotificationItem`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotificationItemError {
    /// `consultant_id` was empty/blank.
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
    /// `origin_capability` was empty/blank.
    #[error("origin_capability must not be empty")]
    EmptyOriginCapability,
    /// `origin_event_id` was empty/blank.
    #[error("origin_event_id must not be empty")]
    EmptyOriginEventId,
    /// `title` was empty/blank.
    #[error("title must not be empty")]
    EmptyTitle,
    /// `body` was empty/blank.
    #[error("body must not be empty")]
    EmptyBody,
    /// A `Some("")`/blank `deep_link` was supplied — `None` is the correct
    /// way to say "no deep link", not an empty string.
    #[error("deep_link must not be empty when present")]
    EmptyDeepLink,
    /// Invariant 3: [`NotificationItem::mark_read`] was called on an item
    /// that is already [`ReadState::Read`].
    #[error("notification is already read")]
    AlreadyRead,
}

impl NotificationItem {
    /// Creates a brand-new, [`ReadState::Unread`] item with a fresh `id`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        consultant_id: impl Into<String>,
        origin_capability: impl Into<String>,
        origin_event_id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        deep_link: Option<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, NotificationItemError> {
        Self::from_parts(
            Uuid::new_v4(),
            consultant_id.into(),
            origin_capability.into(),
            origin_event_id.into(),
            title.into(),
            body.into(),
            deep_link,
            ReadState::Unread,
            created_at,
        )
    }

    /// Reconstructs an aggregate from already-known parts (e.g. a
    /// repository loading a persisted row). Re-validates every field the
    /// same as [`Self::new`] would.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: Uuid,
        consultant_id: String,
        origin_capability: String,
        origin_event_id: String,
        title: String,
        body: String,
        deep_link: Option<String>,
        read_state: ReadState,
        created_at: DateTime<Utc>,
    ) -> Result<Self, NotificationItemError> {
        if consultant_id.trim().is_empty() {
            return Err(NotificationItemError::EmptyConsultantId);
        }
        if origin_capability.trim().is_empty() {
            return Err(NotificationItemError::EmptyOriginCapability);
        }
        if origin_event_id.trim().is_empty() {
            return Err(NotificationItemError::EmptyOriginEventId);
        }
        if title.trim().is_empty() {
            return Err(NotificationItemError::EmptyTitle);
        }
        if body.trim().is_empty() {
            return Err(NotificationItemError::EmptyBody);
        }
        if let Some(link) = &deep_link
            && link.trim().is_empty()
        {
            return Err(NotificationItemError::EmptyDeepLink);
        }

        Ok(Self {
            id,
            consultant_id,
            origin_capability,
            origin_event_id,
            title,
            body,
            deep_link,
            read_state,
            created_at,
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    pub fn origin_capability(&self) -> &str {
        &self.origin_capability
    }

    pub fn origin_event_id(&self) -> &str {
        &self.origin_event_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn deep_link(&self) -> Option<&str> {
        self.deep_link.as_deref()
    }

    pub fn read_state(&self) -> ReadState {
        self.read_state
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Invariant 1's dedupe key: two items with the same
    /// `(origin_capability, origin_event_id)` represent the same upstream
    /// event, regardless of any other field (including `id`) — see the
    /// module docs.
    pub fn origin_key(&self) -> (&str, &str) {
        (&self.origin_capability, &self.origin_event_id)
    }

    /// Invariant 3: transitions `Unread -> Read`. Rejects with
    /// [`NotificationItemError::AlreadyRead`] if already `Read` — see the
    /// module docs for why this is an explicit error rather than a no-op.
    pub fn mark_read(&mut self) -> Result<(), NotificationItemError> {
        if self.read_state == ReadState::Read {
            return Err(NotificationItemError::AlreadyRead);
        }
        self.read_state = ReadState::Read;
        Ok(())
    }
}

/// Repository port for [`NotificationItem`]
/// (`consultant-experience-context.md` §2.4). Implemented against Postgres
/// in `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn NotificationRepository>` in Axum application state, matching
/// `WorkflowSessionRepository`'s convention.
#[async_trait::async_trait]
pub trait NotificationRepository: Send + Sync {
    /// All of `consultant_id`'s notifications, newest first.
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<NotificationItem>, crate::RepoError>;

    /// Looks up a single notification by id. **Added for the cross-instance
    /// NOTIFY/LISTEN bridge** (ADR-014, PROMPT-32): a Postgres `NOTIFY`
    /// payload carries only a lightweight pointer (`{kind, id}` — see
    /// [`crate::EventNotifyPointer`]'s doc comment for why the payload isn't
    /// the full aggregate), and every instance's listener bridge calls this
    /// method to reconstruct the full [`NotificationItem`] before publishing
    /// it to its own local [`crate::EventBus`]. `Ok(None)` (not an error)
    /// when `id` is unknown — a listener receiving a NOTIFY for a row it
    /// can't find is logged and skipped, not treated as a hard failure.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<NotificationItem>, crate::RepoError>;

    /// Idempotent-ingestion insert (invariant 1): if a row already exists
    /// for `item.origin_key()`, this is a safe no-op that leaves the
    /// existing row untouched — see `persistence`'s
    /// `PgNotificationRepository::save` doc comment for why `ON CONFLICT
    /// ... DO NOTHING` (not `DO UPDATE`) is the correct semantics, and
    /// [`SaveOutcome`] for how a caller learns which happened.
    async fn save(&self, item: &NotificationItem) -> Result<SaveOutcome, crate::RepoError>;

    /// Marks a notification read by id. **Lenient/idempotent at the
    /// repository layer** (unlike [`NotificationItem::mark_read`]'s strict
    /// `Err(AlreadyRead)` behavior): a no-op, not an error, if the id is
    /// unknown or already read — this is the convenience path a consultant
    /// re-clicking an already-read notification hits, and it shouldn't
    /// surface as a failure. Callers that need the strict one-way check
    /// enforced should instead load the aggregate, call
    /// [`NotificationItem::mark_read`] themselves, and `save` the result.
    async fn mark_read(&self, id: Uuid) -> Result<(), crate::RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-01-01T00:00:00Z";

    fn t0() -> DateTime<Utc> {
        T0.parse().unwrap()
    }

    fn item() -> NotificationItem {
        NotificationItem::new(
            "consultant-1",
            "sales",
            "event-1",
            "Referral submitted",
            "A new referral was submitted for review.",
            Some("https://app.example.com/sales/referrals/1".to_string()),
            t0(),
        )
        .unwrap()
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err = NotificationItem::new("", "sales", "event-1", "t", "b", None, t0()).unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_empty_origin_capability() {
        let err =
            NotificationItem::new("consultant-1", "", "event-1", "t", "b", None, t0()).unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyOriginCapability);
    }

    #[test]
    fn new_rejects_empty_origin_event_id() {
        let err =
            NotificationItem::new("consultant-1", "sales", "", "t", "b", None, t0()).unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyOriginEventId);
    }

    #[test]
    fn new_rejects_empty_title() {
        let err =
            NotificationItem::new("consultant-1", "sales", "event-1", "", "b", None, t0())
                .unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyTitle);
    }

    #[test]
    fn new_rejects_empty_body() {
        let err =
            NotificationItem::new("consultant-1", "sales", "event-1", "t", "", None, t0())
                .unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyBody);
    }

    #[test]
    fn from_parts_rejects_blank_deep_link() {
        let err = NotificationItem::from_parts(
            Uuid::new_v4(),
            "consultant-1".to_string(),
            "sales".to_string(),
            "event-1".to_string(),
            "t".to_string(),
            "b".to_string(),
            Some("   ".to_string()),
            ReadState::Unread,
            t0(),
        )
        .unwrap_err();
        assert_eq!(err, NotificationItemError::EmptyDeepLink);
    }

    #[test]
    fn new_creates_an_unread_item() {
        let item = item();
        assert_eq!(item.read_state(), ReadState::Unread);
        assert_eq!(item.consultant_id(), "consultant-1");
        assert_eq!(item.origin_capability(), "sales");
        assert_eq!(item.origin_event_id(), "event-1");
    }

    /// Idempotency-intent at the type level (invariant 1): two items built
    /// from the *same* `(origin_capability, origin_event_id)` are
    /// equal-by-that-key even when every other field (id, title,
    /// consultant, created_at) differs — this is what lets a caller dedupe
    /// in-memory instances before ever reaching a repository.
    #[test]
    fn origin_key_is_equal_for_same_origin_pair_regardless_of_other_fields() {
        let a = NotificationItem::new(
            "consultant-1",
            "sales",
            "event-1",
            "First delivery",
            "body a",
            None,
            t0(),
        )
        .unwrap();
        let b = NotificationItem::new(
            "consultant-2",
            "sales",
            "event-1",
            "Redelivered with different text",
            "body b",
            None,
            t0() + chrono::Duration::hours(1),
        )
        .unwrap();

        assert_eq!(a.origin_key(), b.origin_key());
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn origin_key_differs_when_either_half_of_the_pair_differs() {
        let a = item();
        let different_event =
            NotificationItem::new("consultant-1", "sales", "event-2", "t", "b", None, t0())
                .unwrap();
        let different_capability =
            NotificationItem::new("consultant-1", "commit", "event-1", "t", "b", None, t0())
                .unwrap();

        assert_ne!(a.origin_key(), different_event.origin_key());
        assert_ne!(a.origin_key(), different_capability.origin_key());
    }

    #[test]
    fn mark_read_transitions_unread_to_read() {
        let mut item = item();
        item.mark_read().unwrap();
        assert_eq!(item.read_state(), ReadState::Read);
    }

    /// Invariant 3: no "unread again" transition, and a repeated mark-read
    /// attempt is rejected rather than silently succeeding.
    #[test]
    fn mark_read_twice_is_rejected() {
        let mut item = item();
        item.mark_read().unwrap();

        let err = item.mark_read().unwrap_err();

        assert_eq!(err, NotificationItemError::AlreadyRead);
        assert_eq!(item.read_state(), ReadState::Read);
    }

    #[test]
    fn read_state_round_trips_through_as_str_and_from_str() {
        for state in [ReadState::Unread, ReadState::Read] {
            assert_eq!(state.as_str().parse::<ReadState>().unwrap(), state);
        }
    }

    #[test]
    fn read_state_from_str_rejects_unknown_value() {
        let err = "not_a_real_state".parse::<ReadState>().unwrap_err();
        assert_eq!(err.to_string(), "unknown notification read state: \"not_a_real_state\"");
    }
}
