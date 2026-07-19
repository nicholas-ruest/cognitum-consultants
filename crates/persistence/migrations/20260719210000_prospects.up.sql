-- Prospect aggregate (ADR-020 part A).
--
-- Unlike every other aggregate table in this repo, `prospects` has no
-- upstream Nexus event driving it -- it is entirely consultant-authored,
-- so `id` (not `consultant_id`) is the primary key: a consultant
-- accumulates many prospects over time, same shape as
-- `notification_items`/`cross_capability_workflow_sessions`, not
-- `dashboard_configurations`' one-row-per-consultant shape.
--
-- `prospect_notes` is a real child table (append-only, ADR-020 invariant
-- 4 -- nothing in this repo's own code ever UPDATEs or DELETEs a row here,
-- only INSERTs), `ON DELETE CASCADE` so deleting a prospect removes its
-- note history with it.
CREATE TABLE prospects (
    id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    company_name TEXT NOT NULL,
    contact_name TEXT,
    stage TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX prospects_consultant_id_idx ON prospects (consultant_id);

CREATE TABLE prospect_notes (
    id UUID PRIMARY KEY,
    prospect_id UUID NOT NULL REFERENCES prospects (id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author_consultant_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX prospect_notes_prospect_id_idx ON prospect_notes (prospect_id);
