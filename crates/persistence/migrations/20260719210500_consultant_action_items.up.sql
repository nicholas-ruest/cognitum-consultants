-- ConsultantActionItem aggregate (ADR-020 part B) -- deliberately its own
-- table, not a repurposing of `action_queue_entries` (that table's own
-- aggregate, `ActionQueueEntry`, enforces "no local-only completion": it
-- can only reach `completed` via a real upstream Nexus confirmation event,
-- which a consultant-authored checklist item structurally has none of).
--
-- `linked_prospect_id` is a soft, optional reference to `prospects` --
-- `ON DELETE SET NULL` (not CASCADE): deleting a prospect should not
-- silently delete a consultant's unrelated checklist entries, only detach
-- the link.
CREATE TABLE consultant_action_items (
    id UUID PRIMARY KEY,
    consultant_id TEXT NOT NULL,
    title TEXT NOT NULL,
    notes TEXT,
    done BOOLEAN NOT NULL DEFAULT FALSE,
    linked_prospect_id UUID REFERENCES prospects (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX consultant_action_items_consultant_id_idx ON consultant_action_items (consultant_id);
-- Indexes the `ON DELETE SET NULL` FK's own lookup (Postgres does not
-- automatically index the referencing side of a foreign key).
CREATE INDEX consultant_action_items_linked_prospect_id_idx ON consultant_action_items (linked_prospect_id);
