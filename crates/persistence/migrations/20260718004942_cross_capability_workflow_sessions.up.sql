-- CrossCapabilityWorkflowSession aggregate (PROMPT-22, ADR-010).
--
-- Unlike `dashboard_configurations`/`consultant_preferences`, this
-- aggregate is *not* one row per consultant: a consultant may have several
-- (recent, expired, or terminal) workflow sessions over time, so
-- `session_id UUID PRIMARY KEY` is the row identity, and `consultant_id`
-- is an ordinary indexed column, not the primary key.
--
-- `status TEXT NOT NULL` stores `bff_core::WorkflowSessionStatus::as_str()`
-- (`"started" | "in_progress" | "completed" | "abandoned" | "expired"`) —
-- same "plain TEXT + Rust-side allow-list enforced via `FromStr`" choice as
-- `PreferenceKey`, not a Postgres `CHECK`/enum type, so the allow-list only
-- has to be maintained in one place (`workflow_session.rs`).
--
-- `origin_reference`/`target_reference` are opaque reference strings only
-- (invariant 1: never the business entity itself) — this table has no
-- column that could hold a richer payload.
--
-- The `(consultant_id, status)` index is what makes
-- `WorkflowSessionRepository::find_active_by_consultant_id` (which filters
-- on both columns) and the `expire_older_than` housekeeping sweep (which
-- filters on `status` plus `expires_at`) efficient without a full table
-- scan as session volume grows.
CREATE TABLE cross_capability_workflow_sessions (
    session_id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    origin_capability TEXT NOT NULL,
    origin_reference TEXT NOT NULL,
    target_capability TEXT NOT NULL,
    target_reference TEXT,
    status TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX cross_capability_workflow_sessions_consultant_id_status_idx
    ON cross_capability_workflow_sessions (consultant_id, status);
