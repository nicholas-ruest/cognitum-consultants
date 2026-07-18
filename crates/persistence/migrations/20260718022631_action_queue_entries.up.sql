-- ActionQueueEntry aggregate (PROMPT-29, ADR-010).
--
-- Same `UNIQUE (origin_capability, origin_event_id)` idempotent-ingestion
-- rationale as `notification_items` (see that migration's comments) —
-- `consultant-experience-context.md` §2.2 invariant 1 applies identically
-- to this aggregate.
--
-- `action_state` stores `bff_core::ActionState`'s wire string (`pending` /
-- `in_progress` / `completed` / `expired`), matching
-- `cross_capability_workflow_sessions.status`'s convention.
--
-- `(consultant_id, action_state)` is indexed for the same reason
-- `cross_capability_workflow_sessions`'s equivalent index exists: it
-- backs both `find_by_consultant_id` (filtering/ordering by state is a
-- likely follow-up query shape) and, more importantly,
-- `expire_older_than`'s housekeeping sweep, which filters on
-- `action_state NOT IN (...) AND expires_at < $1` — see
-- `action_queue_entry_repository.rs`.
--
-- Note: `confirmation_event_id` (the proof-of-confirmation argument to
-- `bff_core::ActionQueueEntry::complete`) is deliberately **not** a column
-- here — per that method's own doc comment, it is not persisted aggregate
-- state, only a call-site guard against local-only completion.
CREATE TABLE action_queue_entries (
    id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    origin_capability TEXT NOT NULL,
    origin_event_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    deep_link TEXT,
    action_state TEXT NOT NULL DEFAULT 'pending',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (origin_capability, origin_event_id)
);

CREATE INDEX action_queue_entries_consultant_id_state_idx
    ON action_queue_entries (consultant_id, action_state);
