# persistence

Postgres access for this repo's own aggregates (ADR-010). This crate owns:

- The `sqlx::PgPool` connection pool (`create_pool`, used by `bff-api`'s
  `main.rs` on startup).
- The `sqlx-cli` migrations directory (`migrations/`).
- Repository trait implementations for this repo's own aggregates.
  `PgConsultantPreferencesRepository` (U20/PROMPT-20) is the first;
  `DashboardConfiguration`, notification/action-queue state, etc. land in
  later units.

## Prerequisites

- A running Postgres instance for anything beyond `cargo check`/`cargo
  build` (see [Offline compile-time query checking](#offline-compile-time-query-checking)
  below for why `check`/`build` don't need one today).
- `sqlx-cli`, installed once per machine:
  ```sh
  cargo install sqlx-cli --no-default-features --features postgres,rustls
  ```
  Verify with `sqlx migrate --help`.

## Running migrations locally

Two paths, depending on what you're doing:

### 1. Automated tests (no setup required)

Integration tests in this crate use `testcontainers-modules`'s `postgres`
module (ADR-013 layer 3) to launch a throwaway, real Postgres container per
test run — Docker must be running, but no manual Postgres setup or manual
migration step is needed. See `src/lib.rs`'s
`create_pool_connects_and_runs_a_query` test for the pattern:
`Postgres::default().start()` gives you a live container; connect
`create_pool` to its mapped host/port.

```sh
cargo test -p persistence
```

### 2. Manual local development against a real Postgres

For running the app by hand (`cargo run -p bff-api`) or experimenting with
migrations:

```sh
# Start a local Postgres however you prefer, e.g.:
docker run -d --name cognitum-dev-db -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 postgres:17-alpine

export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

# Apply all pending migrations:
sqlx migrate run --source crates/persistence/migrations

# Check migration status:
sqlx migrate info --source crates/persistence/migrations
```

Per ADR-010, migrations are run automatically at `bff-api` startup in
non-production environments (not yet wired) and via an explicit CI/CD
deploy step in production.

### Adding a new migration

```sh
cargo sqlx migrate add -r --source crates/persistence/migrations <description>
```

`-r` generates a reversible pair (`<timestamp>_<description>.up.sql` /
`.down.sql`). See `migrations/README.md` for the per-aggregate convention.
`20260718002751_consultant_preferences` (U20/PROMPT-20) is the first real
migration.

## Offline compile-time query checking

`sqlx`'s `query!`/`query_as!` macros normally need a reachable database at
*build* time to type-check the SQL against the real schema. `sqlx-cli`'s
`prepare` workflow caches that metadata to disk (a `.sqlx/` directory) so
CI can `cargo check`/`cargo build` without a live database:

```sh
# Run once, with a real DB reachable, whenever query!/query_as! usages change:
DATABASE_URL=postgres://... cargo sqlx prepare

# Commit the resulting .sqlx/ directory.

# In CI (or any environment without a DB), builds are constrained to the
# cached metadata:
SQLX_OFFLINE=true cargo check
```

**Status as of U20: exercised.** `consultant_preferences_repository.rs`'s
`query!`/`query_as!` calls are the first real use of this mechanism in the
workspace. `cargo sqlx prepare --workspace` (run once, with a throwaway
`testcontainers`/Docker Postgres reachable and the
`consultant_preferences` migration applied) wrote the query metadata to
`.sqlx/` **at the workspace root** (not inside this crate — `--workspace`
aggregates every member's queries into one cache) and that directory is
committed. Both `SQLX_OFFLINE=true cargo check -p persistence` and plain
`cargo check`/`cargo test` with no `DATABASE_URL` set at all succeed using
that committed cache — sqlx prefers the on-disk cache automatically when no
live database is configured. Re-run `cargo sqlx prepare --workspace` (with
`DATABASE_URL` pointed at a real, migrated Postgres) and re-commit `.sqlx/`
whenever a `query!`/`query_as!` invocation changes.

## Connection pool defaults

`create_pool` uses `sqlx::PgPoolOptions` with `max_connections(10)` and a
5-second `acquire_timeout` for the initial connection (shorter than sqlx's
own 30s default, so an unreachable Postgres fails the fail-fast startup
check promptly instead of hanging). ADR-010 does not pin a pool size; `10`
is a conservative default chosen ahead of any real load testing and should
be revisited once U11+ introduces real query traffic.

## Startup behavior: fail-fast

`bff-api`'s `main.rs` calls `create_pool` on startup and panics (refusing to
start) if the pool cannot connect. This is deliberate — see the comment
above the call site in `crates/bff-api/src/main.rs` and this crate's
`create_pool` doc comment for the rationale (12-factor fail-fast startup;
ADR-010's multi-instance-correctness requirement treats Postgres as a
required dependency, not an optional one).
