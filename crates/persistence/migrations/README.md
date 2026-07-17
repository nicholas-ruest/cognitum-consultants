# Migrations

This directory is the `sqlx-cli` migrations directory for the `persistence`
crate (ADR-010). It is intentionally empty as of U09 — no aggregate tables
exist yet.

This file exists only so git tracks the (otherwise empty) directory; it is
not itself a migration and `sqlx migrate run` ignores it (only `*.sql` files
matter to the migrator).

## Convention

- One migration per aggregate/feature unit, added when that unit actually
  needs a table (the first real migration is expected in U20, per
  `../../../.plans/implementation-prompts.md`).
- Generate new migrations with `sqlx-cli`, never by hand-naming files:
  ```sh
  cargo sqlx migrate add -r --source crates/persistence/migrations <description>
  ```
  The `-r` flag generates a reversible pair (`<timestamp>_<description>.up.sql`
  / `.down.sql`); see `../README.md` for the full local dev workflow
  (running Postgres, running migrations, and the offline query-check
  convention).
