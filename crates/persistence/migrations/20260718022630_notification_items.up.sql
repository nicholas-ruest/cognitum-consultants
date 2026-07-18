-- NotificationItem aggregate (PROMPT-29, ADR-010).
--
-- `UNIQUE (origin_capability, origin_event_id)` is the real, DB-native
-- backing for `consultant-experience-context.md` §2.2 invariant 1
-- (idempotent ingestion) — matching PROMPT-21's pattern of backing an
-- explicitly-called-out DDD invariant with a real constraint, not just an
-- application-level check. `bff_core::NotificationRepository::save` relies
-- on this constraint via `INSERT ... ON CONFLICT (origin_capability,
-- origin_event_id) DO NOTHING` (see `notification_repository.rs` for why
-- `DO NOTHING`, not `DO UPDATE`, is the correct semantics).
--
-- `id UUID PRIMARY KEY` (not `(origin_capability, origin_event_id)`)
-- because callers address a specific notification by its own id (e.g.
-- `mark_read`), and a surrogate key keeps that addressing stable
-- regardless of the idempotency key's shape.
--
-- `read_state` stores `bff_core::ReadState`'s wire string (`unread` /
-- `read`) rather than a boolean, matching
-- `cross_capability_workflow_sessions.status`'s text-enum convention
-- elsewhere in this crate.
CREATE TABLE notification_items (
    id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    origin_capability TEXT NOT NULL,
    origin_event_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    deep_link TEXT,
    read_state TEXT NOT NULL DEFAULT 'unread',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (origin_capability, origin_event_id)
);

CREATE INDEX notification_items_consultant_id_idx ON notification_items (consultant_id);
