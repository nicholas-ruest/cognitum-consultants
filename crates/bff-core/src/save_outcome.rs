//! Shared save-outcome signal for idempotent-ingestion repository saves
//! ([`crate::NotificationRepository::save`],
//! [`crate::ActionQueueRepository::save`],
//! `consultant-experience-context.md` §2.2 invariant 1, PROMPT-29). Both
//! aggregates' `save` accepts a duplicate `(origin_capability,
//! origin_event_id)` delivery as a safe no-op rather than an error —
//! [`SaveOutcome`] lets a caller (e.g. PROMPT-30's ingestion service) learn
//! which happened without a separate query.

/// Whether a repository `save` call inserted a brand-new row or found an
/// existing row with the same idempotency key and left it untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// No row existed for this `(origin_capability, origin_event_id)`; a
    /// new row was inserted.
    Inserted,
    /// A row already existed for this `(origin_capability, origin_event_id)`
    /// — this is a redelivery of an already-ingested event. The existing
    /// row was left untouched (see `persistence`'s repository impls for why
    /// `ON CONFLICT ... DO NOTHING`, not `DO UPDATE`, is correct here).
    AlreadyExists,
}
