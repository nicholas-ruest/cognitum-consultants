# persistence

Postgres access for this repo's own aggregates (ADR-010). This crate owns:

- The `sqlx::PgPool` connection pool (`create_pool`, used by `bff-api`'s
  `main.rs` on startup).
- The `sqlx-cli` migrations directory (`migrations/`).
- Repository trait implementations for `DashboardConfiguration`,
  `ConsultantPreferences`, notification/action-queue state, etc. — **not
  yet implemented** as of U09; this unit only stands up the connection and
  tooling.

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
non-production environments (not yet wired — no migrations exist to run as
of U09) and via an explicit CI/CD deploy step in production.

### Adding a new migration

```sh
cargo sqlx migrate add -r --source crates/persistence/migrations <description>
```

`-r` generates a reversible pair (`<timestamp>_<description>.up.sql` /
`.down.sql`). See `migrations/README.md` for the per-aggregate convention —
the first real migration is expected in U20.

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

**Status as of U09: not yet meaningfully exercised.** There are zero
`query!`/`query_as!`/`query_scalar!` invocations anywhere in this crate (no
aggregate tables exist yet — see `migrations/README.md`), so there is no
query metadata to cache: running `cargo sqlx prepare` today produces an
empty `.sqlx/` directory (verified manually; not committed, since an
all-empty directory carries no information and git doesn't track empty
directories anyway). `cargo check`/`cargo build` already succeed with no
`DATABASE_URL` and no `SQLX_OFFLINE` set, because there is nothing for the
macros to check. This section documents the convention so it's ready to use
the moment U20 adds the first real query — at that point, run `cargo sqlx
prepare` as shown above, commit `.sqlx/`, and set `SQLX_OFFLINE=true` in CI
(or rely on the cached metadata being preferred automatically when
`DATABASE_URL` is unset).

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
