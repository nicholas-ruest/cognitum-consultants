-- ConsultantPreferences aggregate (PROMPT-20, ADR-010).
--
-- `preferences` is stored as a single JSONB object (`{"theme": "dark", ...}`)
-- rather than one row per preference. This is a deliberate, pragmatic choice:
-- `bff-core::PreferenceKey`'s v1 allow-list has only three known keys
-- (`theme`, `default_view`, `notification_channel_opt_in`), all of which
-- belong to the same aggregate and are always read/written together (see
-- `ConsultantPreferencesRepository::find_by_consultant_id`/`save`) — a
-- normalized `consultant_preference_values(consultant_id, key, value)`
-- table would add a join and per-key-row bookkeeping that nothing in the
-- current DDD model (`consultant-experience-context.md` §1.2) requires.
-- `bff_core::PreferenceKey`'s allow-list enforcement (Rust enum + FromStr)
-- guards what can ever be written into this column; unknown keys never
-- reach this table.
--
-- `consultant_id TEXT PRIMARY KEY` (rather than a separate surrogate key
-- plus a unique index) is what makes invariant 2 ("exactly one
-- ConsultantPreferences aggregate exists per consultant") a database-native
-- constraint: an upsert keyed on this primary key can never produce a
-- second row for the same consultant.
CREATE TABLE consultant_preferences (
    consultant_id TEXT PRIMARY KEY,
    preferences JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
