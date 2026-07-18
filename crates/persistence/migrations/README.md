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
