//! `ConsultantActionItem` aggregate (ADR-020 part B) and its repository
//! port (`ConsultantActionItemRepository`, implemented against Postgres in
//! `persistence`).
//!
//! **Deliberately not a variant, extension, or relaxation of
//! [`crate::ActionQueueEntry`]** — that aggregate's own invariant 3 ("No
//! local-only completion — this is the critical invariant") requires a
//! non-empty `confirmation_event_id` sourced from a real upstream Nexus
//! confirmation event before anything can reach `Completed`; "there is no
//! method, and no combination of calls, that reaches `Completed`" without
//! one. A consultant typing a freeform reminder has no such event to
//! confirm against, and never will — this is not a gap in that aggregate,
//! it is that aggregate correctly refusing to be something it was never
//! designed to be (ADR-020). `ConsultantActionItem` is the simpler,
//! separate aggregate that use case actually needs: a plain checklist, no
//! confirmation semantics, no upstream Nexus involvement at all.
//!
//! Invariants enforced here:
//! 1. Belongs to exactly one consultant — `consultant_id` set at
//!    construction, immutable. Ownership enforced at the `bff-api` route
//!    layer, same convention as [`crate::Prospect`]'s invariant 1.
//! 2. `title` must be non-empty.
//! 3. `done` is a plain, freely-reversible boolean — unlike
//!    [`crate::NotificationItem::mark_read`]'s strict one-way rule, marking
//!    an item done then undone is ordinary, expected use of a checklist
//!    (correcting an accidental check), not a violated invariant. This is a
//!    deliberate divergence from `NotificationItem`'s stricter pattern, not
//!    an oversight — see [`ConsultantActionItem::set_done`].
//! 4. `linked_prospect_id`, when present, is a **soft** reference to a
//!    [`crate::Prospect`] — not a value this aggregate validates exists
//!    (that would require a repository dependency this aggregate
//!    deliberately doesn't have, per the same "bff-core stays
//!    infra-agnostic" discipline `dashboard_configuration.rs`'s module docs
//!    already establish for permission checks). A consultant may have
//!    action items with no linked prospect at all — the link is optional
//!    context, not a requirement.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::RepoError;

/// A single consultant-authored checklist entry (ADR-020). Root of its own
/// aggregate — no child entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsultantActionItem {
    id: Uuid,
    consultant_id: String,
    title: String,
    notes: Option<String>,
    done: bool,
    linked_prospect_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Errors constructing/mutating a [`ConsultantActionItem`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConsultantActionItemError {
    /// `consultant_id` was empty/blank.
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
    /// `title` was empty/blank.
    #[error("title must not be empty")]
    EmptyTitle,
    /// A `Some("")`/blank `notes` was supplied — `None` is the correct way
    /// to say "no notes", not an empty string.
    #[error("notes must not be empty when present")]
    EmptyNotes,
}

impl ConsultantActionItem {
    /// Creates a brand-new, not-done item with a fresh `id`.
    pub fn new(
        consultant_id: impl Into<String>,
        title: impl Into<String>,
        notes: Option<String>,
        linked_prospect_id: Option<Uuid>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, ConsultantActionItemError> {
        Self::from_parts(
            Uuid::new_v4(),
            consultant_id.into(),
            title.into(),
            notes,
            false,
            linked_prospect_id,
            created_at,
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
        title: String,
        notes: Option<String>,
        done: bool,
        linked_prospect_id: Option<Uuid>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, ConsultantActionItemError> {
        if consultant_id.trim().is_empty() {
            return Err(ConsultantActionItemError::EmptyConsultantId);
        }
        if title.trim().is_empty() {
            return Err(ConsultantActionItemError::EmptyTitle);
        }
        if let Some(n) = &notes
            && n.trim().is_empty()
        {
            return Err(ConsultantActionItemError::EmptyNotes);
        }

        Ok(Self { id, consultant_id, title, notes, done, linked_prospect_id, created_at, updated_at })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }

    pub fn done(&self) -> bool {
        self.done
    }

    pub fn linked_prospect_id(&self) -> Option<Uuid> {
        self.linked_prospect_id
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    /// Invariant 3: freely sets `done` either direction — see the module
    /// docs for why this stays reversible rather than a one-way transition.
    pub fn set_done(&mut self, done: bool, now: DateTime<Utc>) {
        self.done = done;
        self.updated_at = now;
    }
}

/// Repository port for [`ConsultantActionItem`]. Implemented against
/// Postgres in `persistence` (ADR-010); `bff-core` only defines the
/// interface, per ADR-004's trait-interface-only dependency direction.
#[async_trait::async_trait]
pub trait ConsultantActionItemRepository: Send + Sync {
    /// All of `consultant_id`'s action items, newest first.
    async fn find_by_consultant_id(&self, consultant_id: &str) -> Result<Vec<ConsultantActionItem>, RepoError>;

    /// Looks up a single item by id, regardless of owner — callers
    /// (`bff-api` route handlers) compare `consultant_id` themselves, same
    /// convention as [`crate::ProspectRepository::find_by_id`].
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ConsultantActionItem>, RepoError>;

    /// Persists the full aggregate — upsert semantics on `id`.
    async fn save(&self, item: &ConsultantActionItem) -> Result<(), RepoError>;

    /// Deletes an item entirely. Not an error if `id` is unknown.
    async fn delete(&self, id: Uuid) -> Result<(), RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err = ConsultantActionItem::new("", "Call Acme", None, None, t0()).unwrap_err();
        assert_eq!(err, ConsultantActionItemError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_empty_title() {
        let err = ConsultantActionItem::new("consultant-1", "", None, None, t0()).unwrap_err();
        assert_eq!(err, ConsultantActionItemError::EmptyTitle);
    }

    #[test]
    fn new_rejects_blank_notes() {
        let err =
            ConsultantActionItem::new("consultant-1", "Call Acme", Some("   ".to_string()), None, t0()).unwrap_err();
        assert_eq!(err, ConsultantActionItemError::EmptyNotes);
    }

    #[test]
    fn new_creates_a_not_done_item() {
        let item = ConsultantActionItem::new("consultant-1", "Call Acme", None, None, t0()).unwrap();
        assert!(!item.done());
        assert_eq!(item.created_at(), item.updated_at());
    }

    #[test]
    fn new_accepts_an_optional_linked_prospect() {
        let prospect_id = Uuid::new_v4();
        let item =
            ConsultantActionItem::new("consultant-1", "Follow up", None, Some(prospect_id), t0()).unwrap();
        assert_eq!(item.linked_prospect_id(), Some(prospect_id));
    }

    /// Invariant 3: unlike `NotificationItem::mark_read`, this must be
    /// freely reversible.
    #[test]
    fn set_done_toggles_in_either_direction() {
        let mut item = ConsultantActionItem::new("consultant-1", "Call Acme", None, None, t0()).unwrap();
        let t1 = t0() + chrono::Duration::hours(1);
        let t2 = t0() + chrono::Duration::hours(2);

        item.set_done(true, t1);
        assert!(item.done());
        assert_eq!(item.updated_at(), t1);

        item.set_done(false, t2);
        assert!(!item.done());
        assert_eq!(item.updated_at(), t2);
    }
}
