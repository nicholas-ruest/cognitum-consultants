# Migrations

This directory is the `sqlx-cli` migrations directory for the `persistence`
crate (ADR-010).

`20260718002751_consultant_preferences.{up,down}.sql` (U20/PROMPT-20) is
the first real migration: it creates the `consultant_preferences` table
backing `bff_core::ConsultantPreferences`
(`../../../.plans/ddd/consultant-experience-context.md` §1.2). Preferences
are stored as a single JSONB object per consultant, keyed by
`bff_core::PreferenceKey`'s wire strings, rather than a normalized
per-preference-row table — see the migration's own comments for why that's
the pragmatic choice for a three-key allow-list.

`20260718003944_dashboard_configuration.{up,down}.sql` (U21/PROMPT-21)
creates `dashboard_configurations` + `card_placements` for
`bff_core::DashboardConfiguration`.

`20260718004942_cross_capability_workflow_sessions.{up,down}.sql`
(U22/PROMPT-22) creates `cross_capability_workflow_sessions` for
`bff_core::CrossCapabilityWorkflowSession`. Unlike the two aggregates
above, this is not one row per consultant — `session_id` is the primary
key, and `(consultant_id, status)` is indexed to support
`WorkflowSessionRepository::find_active_by_consultant_id` and the
`expire_older_than` housekeeping sweep. See the migration's own comments
for the full rationale.

This README file also exists so git tracks this directory even when a
migration set is otherwise removed; it is not itself a migration and `sqlx
migrate run` ignores it (only `*.sql` files matter to the migrator).

## Convention

- One migration per aggregate/feature unit, added when that unit actually
  needs a table.
- Generate new migrations with `sqlx-cli`, never by hand-naming files:
  ```sh
  cargo sqlx migrate add -r --source crates/persistence/migrations <description>
  ```
  The `-r` flag generates a reversible pair (`<timestamp>_<description>.up.sql`
  / `.down.sql`); see `../README.md` for the full local dev workflow
  (running Postgres, running migrations, and the offline query-check
  convention).
