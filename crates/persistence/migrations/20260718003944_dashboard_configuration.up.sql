-- DashboardConfiguration aggregate (PROMPT-21, ADR-010).
--
-- Unlike `consultant_preferences` (PROMPT-20, a single-JSONB-column
-- pragmatic choice), this aggregate uses two normalized tables. That is a
-- deliberate, different tradeoff, not an inconsistency: invariant 2
-- ("card positions within one configuration must be unique") is called
-- out explicitly in `consultant-experience-context.md` §1.2, and a
-- `UNIQUE (consultant_id, card_position)` constraint on a real child
-- table lets Postgres itself enforce that invariant as defense-in-depth
-- alongside `bff_core::DashboardConfiguration::add_card`'s own check —
-- something a single JSONB array column cannot give us (Postgres has no
-- native "unique array element field" constraint). `consultant_preferences`
-- has no analogous per-item uniqueness invariant to defend, so its JSONB
-- shape remains the simpler, equally correct choice there.
--
-- `dashboard_configurations.consultant_id TEXT PRIMARY KEY` is what makes
-- invariant 3 ("exactly one DashboardConfiguration per consultant") a
-- database-native constraint, the same way `consultant_preferences` does.
--
-- `card_placements.consultant_id` is both a foreign key into
-- `dashboard_configurations` (`ON DELETE CASCADE`, so
-- `delete_by_consultant_id` only needs to delete the parent row) and part
-- of the `UNIQUE (consultant_id, card_position)` constraint that backs
-- invariant 2. The column is named `card_position` rather than `position`
-- because `POSITION` is a reserved SQL keyword (the `POSITION(... IN ...)`
-- function).
CREATE TABLE dashboard_configurations (
    consultant_id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE card_placements (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    consultant_id TEXT NOT NULL REFERENCES dashboard_configurations (consultant_id) ON DELETE CASCADE,
    module_id TEXT NOT NULL,
    card_position INT NOT NULL,
    UNIQUE (consultant_id, card_position)
);

CREATE INDEX card_placements_consultant_id_idx ON card_placements (consultant_id);
