//! `ActionQueueEntry` aggregate (`consultant-experience-context.md` §2.2)
//! and its repository port (`ActionQueueRepository`, implemented against
//! Postgres in `persistence`, ADR-010, PROMPT-29).
//!
//! The Notification & Action Queue context's action-required counterpart to
//! [`crate::NotificationItem`]: a normalized, deduped projection of an
//! upstream Nexus `CapabilityEventReceived` envelope that implies a
//! consultant must do something (e.g. respond to a collaboration request).
//!
//! Invariants enforced here:
//! 1. **Idempotent ingestion.** Same `(origin_capability, origin_event_id)`
//!    uniqueness rule as `NotificationItem` — see
//!    [`ActionQueueEntry::origin_key`] and invariant 1's doc comment on
//!    `NotificationItem` for the full rationale; it applies identically
//!    here.
//! 2. **Linear state machine, no regression.** `pending -> in_progress ->
//!    {completed | expired}`. See [`ActionState::is_valid_transition`] for
//!    the exact matrix. No transition is ever valid *out of* a terminal
//!    state ([`ActionState::Completed`], [`ActionState::Expired`]).
//!    `Pending -> Expired` is a valid direct transition (mirroring
//!    `CrossCapabilityWorkflowSession`'s `Started -> Expired`): an entry
//!    that a consultant never even started can still time out.
//! 3. **No local-only completion — this is the critical invariant.** This
//!    context cannot unilaterally decide the underlying business action is
//!    done. [`ActionQueueEntry::start`] (`Pending -> InProgress`) models a
//!    bare consultant click — it takes no confirmation evidence at all,
//!    because a click alone only *initiates* a command through Nexus.
//!    [`ActionQueueEntry::complete`] is structurally the only path to
//!    [`ActionState::Completed`], and it **requires** a non-empty
//!    `confirmation_event_id: &str` argument, rejected with
//!    [`ActionQueueEntryError::EmptyConfirmationEventId`] if blank. There is
//!    no method, and no combination of calls, that reaches `Completed`
//!    without supplying that argument — in particular `complete` also
//!    refuses to fire from `Pending` (invariant 2's state-machine check),
//!    so a caller cannot skip the confirmation-requiring path by never
//!    calling `start` either. The `confirmation_event_id` itself is
//!    deliberately **not** stored as a field on this aggregate (it is proof
//!    a confirmation was supplied at the call site, not persisted state);
//!    callers that need an audit trail of the confirming event should log
//!    it at the ingestion boundary (PROMPT-30), not in this aggregate.
//! 4. **Bounded by `expires_at`.** [`ActionQueueEntry::expire`] moves any
//!    non-terminal entry to [`ActionState::Expired`], but only if `now >=
//!    expires_at` — mirrors `CrossCapabilityWorkflowSession::is_expired`'s
//!    inclusive boundary.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::SaveOutcome;

/// An [`ActionQueueEntry`]'s linear state machine status (invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionState {
    /// Ingested, not yet acted on by the consultant.
    Pending,
    /// The consultant clicked/initiated the action; a command has been
    /// sent through Nexus but not yet confirmed (invariant 3).
    InProgress,
    /// Terminal: the owning capability confirmed the action is done.
    Completed,
    /// Terminal: the entry's `expires_at` elapsed before it completed.
    Expired,
}

impl ActionState {
    /// The wire/storage string for this state (DB `action_state` column).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Expired => "expired",
        }
    }

    /// Terminal states admit no further transition — see
    /// [`Self::is_valid_transition`], which is consistent with this by
    /// construction (every arm returning `true` there has a non-terminal
    /// `from`).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Expired)
    }

    /// Whether `from -> to` is a valid transition in the linear state
    /// machine `pending -> in_progress -> {completed | expired}`, with the
    /// `pending -> expired` shortcut documented in invariant 2 above.
    ///
    /// This function alone does **not** enforce invariant 3 (the
    /// confirmation-id requirement) — that is [`ActionQueueEntry::complete`]'s
    /// job, layered on top of this state-machine check.
    pub fn is_valid_transition(from: Self, to: Self) -> bool {
        matches!(
            (from, to),
            (Self::Pending, Self::InProgress)
                | (Self::Pending, Self::Expired)
                | (Self::InProgress, Self::Completed)
                | (Self::InProgress, Self::Expired)
        )
    }
}

impl fmt::Display for ActionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An action-state string that isn't a known [`ActionState`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown action queue entry state: {0:?}")]
pub struct ParseActionStateError(String);

impl FromStr for ActionState {
    type Err = ParseActionStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "expired" => Ok(Self::Expired),
            other => Err(ParseActionStateError(other.to_string())),
        }
    }
}

/// Action-required inbox item (`consultant-experience-context.md` §2.2).
/// Root of its own aggregate — no child entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionQueueEntry {
    id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_event_id: String,
    title: String,
    body: String,
    deep_link: Option<String>,
    action_state: ActionState,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

/// Errors constructing/mutating an [`ActionQueueEntry`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionQueueEntryError {
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
    /// Invariant 2: `from -> to` is not a valid state-machine transition
    /// (including any attempted transition out of a terminal state, or
    /// [`ActionQueueEntry::complete`] attempted from [`ActionState::Pending`]).
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: ActionState, to: ActionState },
    /// Invariant 3: [`ActionQueueEntry::complete`] was called with a blank
    /// `confirmation_event_id` — the structural guard against local-only
    /// completion.
    #[error("completion requires a non-empty confirmation_event_id")]
    EmptyConfirmationEventId,
    /// Invariant 4: [`ActionQueueEntry::expire`] was called with `now`
    /// still strictly before `expires_at`.
    #[error("entry is not yet due to expire at {expires_at}")]
    NotYetExpired { expires_at: DateTime<Utc> },
}

impl ActionQueueEntry {
    /// Creates a brand-new, [`ActionState::Pending`] entry with a fresh
    /// `id`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        consultant_id: impl Into<String>,
        origin_capability: impl Into<String>,
        origin_event_id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        deep_link: Option<String>,
        expires_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, ActionQueueEntryError> {
        Self::from_parts(
            Uuid::new_v4(),
            consultant_id.into(),
            origin_capability.into(),
            origin_event_id.into(),
            title.into(),
            body.into(),
            deep_link,
            ActionState::Pending,
            expires_at,
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
        action_state: ActionState,
        expires_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, ActionQueueEntryError> {
        if consultant_id.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyConsultantId);
        }
        if origin_capability.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyOriginCapability);
        }
        if origin_event_id.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyOriginEventId);
        }
        if title.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyTitle);
        }
        if body.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyBody);
        }
        if let Some(link) = &deep_link
            && link.trim().is_empty()
        {
            return Err(ActionQueueEntryError::EmptyDeepLink);
        }

        Ok(Self {
            id,
            consultant_id,
            origin_capability,
            origin_event_id,
            title,
            body,
            deep_link,
            action_state,
            expires_at,
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

    pub fn action_state(&self) -> ActionState {
        self.action_state
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Invariant 1's dedupe key — see `NotificationItem::origin_key`'s doc
    /// comment; identical rationale applies here.
    pub fn origin_key(&self) -> (&str, &str) {
        (&self.origin_capability, &self.origin_event_id)
    }

    /// Models a bare consultant click: `Pending -> InProgress`. Takes no
    /// confirmation evidence, deliberately — a click only *initiates* a
    /// command through Nexus (invariant 3); it never sets
    /// [`ActionState::Completed`].
    pub fn start(&mut self) -> Result<(), ActionQueueEntryError> {
        if !ActionState::is_valid_transition(self.action_state, ActionState::InProgress) {
            return Err(ActionQueueEntryError::InvalidTransition {
                from: self.action_state,
                to: ActionState::InProgress,
            });
        }
        self.action_state = ActionState::InProgress;
        Ok(())
    }

    /// Invariant 3's structural guard: the **only** path to
    /// [`ActionState::Completed`]. Requires `confirmation_event_id` to be
    /// non-empty (rejecting a blank string, not just absence — there is no
    /// `Option` overload that could be called with `None`), proving a
    /// confirmation event was actually routed back through Nexus, and
    /// requires the entry to currently be [`ActionState::InProgress`] (so
    /// `Pending -> Completed` — completing an entry the consultant never
    /// even clicked — is also rejected, even with a valid confirmation id).
    pub fn complete(
        &mut self,
        confirmation_event_id: &str,
    ) -> Result<(), ActionQueueEntryError> {
        if confirmation_event_id.trim().is_empty() {
            return Err(ActionQueueEntryError::EmptyConfirmationEventId);
        }
        if !ActionState::is_valid_transition(self.action_state, ActionState::Completed) {
            return Err(ActionQueueEntryError::InvalidTransition {
                from: self.action_state,
                to: ActionState::Completed,
            });
        }
        self.action_state = ActionState::Completed;
        Ok(())
    }

    /// Invariant 4: moves a non-terminal entry to [`ActionState::Expired`],
    /// but only once `now >= expires_at` (inclusive boundary, matching
    /// `CrossCapabilityWorkflowSession::is_expired`). Rejects with
    /// [`ActionQueueEntryError::InvalidTransition`] if already terminal, or
    /// [`ActionQueueEntryError::NotYetExpired`] if called too early.
    pub fn expire(&mut self, now: DateTime<Utc>) -> Result<(), ActionQueueEntryError> {
        if !ActionState::is_valid_transition(self.action_state, ActionState::Expired) {
            return Err(ActionQueueEntryError::InvalidTransition {
                from: self.action_state,
                to: ActionState::Expired,
            });
        }
        if now < self.expires_at {
            return Err(ActionQueueEntryError::NotYetExpired { expires_at: self.expires_at });
        }
        self.action_state = ActionState::Expired;
        Ok(())
    }
}

/// Repository port for [`ActionQueueEntry`]
/// (`consultant-experience-context.md` §2.4). Implemented against Postgres
/// in `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn ActionQueueRepository>` in Axum application state, matching
/// `WorkflowSessionRepository`'s convention.
#[async_trait::async_trait]
pub trait ActionQueueRepository: Send + Sync {
    /// All of `consultant_id`'s entries, newest first.
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<ActionQueueEntry>, crate::RepoError>;

    /// Looks up a single entry by id — same rationale and `Ok(None)`-on-
    /// unknown-id contract as [`crate::NotificationRepository::find_by_id`];
    /// see that method's doc comment for the full cross-instance NOTIFY/
    /// LISTEN bridge context (ADR-014, PROMPT-32).
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ActionQueueEntry>, crate::RepoError>;

    /// Looks up the single entry an ingested `(origin_capability,
    /// origin_event_id)` pair created — the repository interface
    /// `../ddd/consultant-experience-context.md` §2.4 names but PROMPT-29
    /// left unimplemented until PROMPT-38 needed it: a later *confirmation*
    /// event from the owning capability (e.g. Execution's `task_completed`,
    /// see `event_ingestion::CONFIRMATION_EVENT_TYPES`) does not carry the
    /// entry's own `id`, only a reference back to the `origin_event_id` that
    /// originally created it, so completion has to be resolved by this
    /// lookup rather than [`Self::find_by_id`]. `Ok(None)` — not an error —
    /// when no entry matches, same leniency contract as [`Self::find_by_id`].
    async fn find_by_origin_event(
        &self,
        origin_capability: &str,
        origin_event_id: &str,
    ) -> Result<Option<ActionQueueEntry>, crate::RepoError>;

    /// Idempotent-ingestion insert (invariant 1) — same no-op-on-conflict
    /// semantics as `NotificationRepository::save`; see that method's doc
    /// comment for the full rationale.
    async fn save(&self, entry: &ActionQueueEntry) -> Result<SaveOutcome, crate::RepoError>;

    /// Repository-layer convenience for [`ActionQueueEntry::start`]
    /// (`Pending -> InProgress`, a bare consultant click). Lenient at this
    /// layer: a no-op, not an error, if `id` is unknown or not currently
    /// `Pending` — mirrors `NotificationRepository::mark_read`'s
    /// leniency rationale. Callers that need the strict state-machine
    /// error should load the aggregate, call
    /// [`ActionQueueEntry::start`] themselves, and `save` the result.
    async fn mark_started(&self, id: Uuid) -> Result<(), crate::RepoError>;

    /// Repository-layer convenience for [`ActionQueueEntry::complete`]
    /// (`InProgress -> Completed`). **Takes and validates
    /// `confirmation_event_id` exactly as the aggregate method does** — a
    /// blank value is rejected here too, so invariant 3's guard cannot be
    /// bypassed by going through the repository directly instead of the
    /// aggregate. Lenient about state otherwise: a no-op if `id` is unknown
    /// or not currently `InProgress`.
    async fn mark_completed(
        &self,
        id: Uuid,
        confirmation_event_id: &str,
    ) -> Result<(), crate::RepoError>;

    /// Housekeeping sweep: bulk-transitions every non-terminal entry whose
    /// `expires_at` is before `cutoff` to [`ActionState::Expired`]. Returns
    /// the number of rows affected. Already-terminal entries and entries
    /// not yet past `cutoff` are left untouched. Mirrors
    /// `WorkflowSessionRepository::expire_older_than`.
    async fn expire_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, crate::RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-01-01T00:00:00Z";

    fn t0() -> DateTime<Utc> {
        T0.parse().unwrap()
    }

    fn expires_at() -> DateTime<Utc> {
        t0() + chrono::Duration::hours(24)
    }

    fn entry() -> ActionQueueEntry {
        ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "event-1",
            "Collaboration request",
            "A collaboration request needs your response.",
            Some("https://app.example.com/sales/collab/1".to_string()),
            expires_at(),
            t0(),
        )
        .unwrap()
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err =
            ActionQueueEntry::new("", "sales", "event-1", "t", "b", None, expires_at(), t0())
                .unwrap_err();
        assert_eq!(err, ActionQueueEntryError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_empty_origin_capability() {
        let err = ActionQueueEntry::new(
            "consultant-1",
            "",
            "event-1",
            "t",
            "b",
            None,
            expires_at(),
            t0(),
        )
        .unwrap_err();
        assert_eq!(err, ActionQueueEntryError::EmptyOriginCapability);
    }

    #[test]
    fn new_rejects_empty_origin_event_id() {
        let err = ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "",
            "t",
            "b",
            None,
            expires_at(),
            t0(),
        )
        .unwrap_err();
        assert_eq!(err, ActionQueueEntryError::EmptyOriginEventId);
    }

    #[test]
    fn from_parts_rejects_blank_deep_link() {
        let err = ActionQueueEntry::from_parts(
            Uuid::new_v4(),
            "consultant-1".to_string(),
            "sales".to_string(),
            "event-1".to_string(),
            "t".to_string(),
            "b".to_string(),
            Some("   ".to_string()),
            ActionState::Pending,
            expires_at(),
            t0(),
        )
        .unwrap_err();
        assert_eq!(err, ActionQueueEntryError::EmptyDeepLink);
    }

    #[test]
    fn new_creates_a_pending_entry() {
        let entry = entry();
        assert_eq!(entry.action_state(), ActionState::Pending);
        assert_eq!(entry.expires_at(), expires_at());
    }

    /// Idempotency-intent at the type level (invariant 1) — same shape as
    /// `NotificationItem::origin_key`'s equivalent test.
    #[test]
    fn origin_key_is_equal_for_same_origin_pair_regardless_of_other_fields() {
        let a = ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "event-1",
            "First delivery",
            "body a",
            None,
            expires_at(),
            t0(),
        )
        .unwrap();
        let b = ActionQueueEntry::new(
            "consultant-2",
            "sales",
            "event-1",
            "Redelivered",
            "body b",
            None,
            expires_at() + chrono::Duration::hours(1),
            t0() + chrono::Duration::hours(1),
        )
        .unwrap();

        assert_eq!(a.origin_key(), b.origin_key());
        assert_ne!(a.id(), b.id());
    }

    // --- state machine: start -------------------------------------------

    #[test]
    fn start_transitions_pending_to_in_progress() {
        let mut entry = entry();
        entry.start().unwrap();
        assert_eq!(entry.action_state(), ActionState::InProgress);
    }

    #[test]
    fn start_rejects_when_already_in_progress() {
        let mut entry = entry();
        entry.start().unwrap();

        let err = entry.start().unwrap_err();

        assert_eq!(
            err,
            ActionQueueEntryError::InvalidTransition {
                from: ActionState::InProgress,
                to: ActionState::InProgress
            }
        );
    }

    // --- state machine + invariant 3: complete ---------------------------

    /// The headline invariant-3 test: a non-empty `confirmation_event_id`
    /// on an `InProgress` entry is the only way to reach `Completed`.
    #[test]
    fn complete_transitions_in_progress_to_completed_with_confirmation() {
        let mut entry = entry();
        entry.start().unwrap();

        entry.complete("nexus-confirmation-42").unwrap();

        assert_eq!(entry.action_state(), ActionState::Completed);
    }

    /// Structural guard, half 1: an empty `confirmation_event_id` is
    /// rejected outright, even from the otherwise-valid `InProgress` state.
    #[test]
    fn complete_rejects_empty_confirmation_event_id() {
        let mut entry = entry();
        entry.start().unwrap();

        let err = entry.complete("").unwrap_err();

        assert_eq!(err, ActionQueueEntryError::EmptyConfirmationEventId);
        assert_eq!(entry.action_state(), ActionState::InProgress);
    }

    /// Structural guard, half 1b: a whitespace-only `confirmation_event_id`
    /// is rejected too (not just literally empty).
    #[test]
    fn complete_rejects_whitespace_only_confirmation_event_id() {
        let mut entry = entry();
        entry.start().unwrap();

        let err = entry.complete("   ").unwrap_err();

        assert_eq!(err, ActionQueueEntryError::EmptyConfirmationEventId);
    }

    /// Structural guard, half 2: there is no direct `Pending -> Completed`
    /// path, even with a perfectly valid, non-empty confirmation id — the
    /// entry must have been started (bare consultant click through Nexus)
    /// first.
    #[test]
    fn complete_rejects_direct_pending_to_completed_even_with_valid_confirmation() {
        let mut entry = entry();
        assert_eq!(entry.action_state(), ActionState::Pending);

        let err = entry.complete("nexus-confirmation-42").unwrap_err();

        assert_eq!(
            err,
            ActionQueueEntryError::InvalidTransition {
                from: ActionState::Pending,
                to: ActionState::Completed
            }
        );
        assert_eq!(entry.action_state(), ActionState::Pending);
    }

    #[test]
    fn complete_rejects_when_already_completed() {
        let mut entry = entry();
        entry.start().unwrap();
        entry.complete("nexus-confirmation-42").unwrap();

        let err = entry.complete("nexus-confirmation-99").unwrap_err();

        assert_eq!(
            err,
            ActionQueueEntryError::InvalidTransition {
                from: ActionState::Completed,
                to: ActionState::Completed
            }
        );
    }

    #[test]
    fn complete_rejects_when_already_expired() {
        let mut entry = entry();
        entry.expire(expires_at()).unwrap();

        let err = entry.complete("nexus-confirmation-42").unwrap_err();

        assert_eq!(
            err,
            ActionQueueEntryError::InvalidTransition {
                from: ActionState::Expired,
                to: ActionState::Completed
            }
        );
    }

    // --- expiry ------------------------------------------------------------

    #[test]
    fn expire_transitions_pending_to_expired_when_due() {
        let mut entry = entry();
        entry.expire(expires_at()).unwrap();
        assert_eq!(entry.action_state(), ActionState::Expired);
    }

    #[test]
    fn expire_transitions_in_progress_to_expired_when_due() {
        let mut entry = entry();
        entry.start().unwrap();
        entry.expire(expires_at() + chrono::Duration::seconds(1)).unwrap();
        assert_eq!(entry.action_state(), ActionState::Expired);
    }

    /// Invariant 4: `expire` is a no-op-that-errors before `now >=
    /// expires_at` — it must not fire early.
    #[test]
    fn expire_rejects_before_expires_at() {
        let mut entry = entry();
        let just_before = expires_at() - chrono::Duration::seconds(1);

        let err = entry.expire(just_before).unwrap_err();

        assert_eq!(err, ActionQueueEntryError::NotYetExpired { expires_at: expires_at() });
        assert_eq!(entry.action_state(), ActionState::Pending);
    }

    #[test]
    fn expire_rejects_from_a_terminal_state() {
        let mut entry = entry();
        entry.start().unwrap();
        entry.complete("nexus-confirmation-42").unwrap();

        let err = entry.expire(expires_at()).unwrap_err();

        assert_eq!(
            err,
            ActionQueueEntryError::InvalidTransition {
                from: ActionState::Completed,
                to: ActionState::Expired
            }
        );
    }

    #[test]
    fn is_valid_transition_exhaustively_matches_the_documented_matrix() {
        use ActionState::*;
        let valid_pairs =
            [(Pending, InProgress), (Pending, Expired), (InProgress, Completed), (InProgress, Expired)];
        let all = [Pending, InProgress, Completed, Expired];

        for from in all {
            for to in all {
                let expected = valid_pairs.contains(&(from, to));
                assert_eq!(
                    ActionState::is_valid_transition(from, to),
                    expected,
                    "from={from} to={to}"
                );
            }
        }
    }

    #[test]
    fn terminal_states_report_is_terminal_true() {
        assert!(ActionState::Completed.is_terminal());
        assert!(ActionState::Expired.is_terminal());
        assert!(!ActionState::Pending.is_terminal());
        assert!(!ActionState::InProgress.is_terminal());
    }

    #[test]
    fn action_state_round_trips_through_as_str_and_from_str() {
        for state in [ActionState::Pending, ActionState::InProgress, ActionState::Completed, ActionState::Expired] {
            assert_eq!(state.as_str().parse::<ActionState>().unwrap(), state);
        }
    }

    #[test]
    fn action_state_from_str_rejects_unknown_value() {
        let err = "not_a_real_state".parse::<ActionState>().unwrap_err();
        assert_eq!(err.to_string(), "unknown action queue entry state: \"not_a_real_state\"");
    }
}
