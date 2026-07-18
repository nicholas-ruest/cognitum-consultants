# Deployment (PROMPT-28, ADR-014)

This document covers the container image, how `bff-api` serves the SPA + API from that one image, the
liveness/readiness health-check design, graceful shutdown, and the CI pipeline step that builds, migrates,
and smoke-tests the image as a stand-in for a real deploy. See
[ADR-014: Deployment and Runtime Topology](../.plans/adr/ADR-014-deployment-runtime-topology.md) for the
governing decision this implements.

**No real deployment target (cloud provider, Kubernetes, Nomad, etc.) is chosen yet** — ADR-014 leaves that
deliberately open (risk #8 in `.plans/implementation-plan.md` §6). What exists today is the deployable
*artifact* (the container image) and a CI job (`deploy` in `.github/workflows/ci.yml`) that proves the image
is correct — builds it, migrates a throwaway Postgres, runs the image against it, and smoke-tests
`/healthz`, `/readyz`, the SPA, and `/api/*` — as the CI-verifiable proxy for an actual rollout. Once a target
is chosen, that job's final "run the built image" step is where an orchestrator-specific deploy step slots
in; everything before it (build, migrate, smoke-test the image) stays the same.

## The image: `Dockerfile` (repo root)

Multi-stage build, five stages:

1. **`chef`** — `lukemathwalker/cargo-chef:latest-rust-1-bookworm` (the official cargo-chef image, which
   ships `cargo-chef` preinstalled — simpler than installing it via `cargo install` on a plain `rust:*` base).
2. **`planner`** — `cargo chef prepare --recipe-path recipe.json`: computes a dependency-only "recipe" from
   the full workspace source tree.
3. **`builder`** — `cargo chef cook --release --recipe-path recipe.json` builds *only* the dependency graph
   (cached across builds whenever `Cargo.toml`/`Cargo.lock` haven't changed, even if application source has —
   this is cargo-chef's whole point), then `COPY . .` and `cargo build --release -p bff-api`.
   `SQLX_OFFLINE=true` is set for this stage: there's no live Postgres reachable from inside a Docker build,
   so `sqlx`'s `query!`/`query_as!` macros type-check against the committed `.sqlx/` offline metadata instead
   (see `crates/persistence/README.md`, "Offline compile-time query checking" — verified working as of
   PROMPT-20).
4. **`frontend-builder`** — separate `node:24-slim` stage: `npm ci && npm run build` in `frontend/`,
   producing `frontend/dist/`.
5. **`runtime`** — the final image: the compiled `bff-api` binary + `frontend/dist/`'s contents, on a minimal
   base, running as a non-root user.

### Runtime base image choice: `debian:bookworm-slim`, not distroless

ADR-014 leaves this open ("a `distroless` or `slim` image"). This Dockerfile uses `debian:bookworm-slim`.
Trade-off, documented per the ADR's own ask:

- **Why not distroless** (`gcr.io/distroless/cc` or similar): distroless images have no shell and no package
  manager, which means no `useradd` to create the non-root user this stage runs as, and no `apt` to install
  `ca-certificates` (needed for rustls-based outbound TLS — Nexus calls, ADR-007; Postgres, ADR-010 — to
  verify certificates against the system trust store). Both are doable with distroless, but need a more
  involved multi-stage dance (e.g. building `/etc/passwd`/`ca-certificates.crt` in an earlier stage and
  copying them in) for a marginal size/CVE-surface win.
- **Why `slim`**: `apt-get install ca-certificates` + `useradd` are one-liners; the result is still small
  (~150MB total, see below) and — usefully for local debugging — still has a real shell (`docker exec -it
  <container> sh`) if something needs inspecting live, which distroless doesn't allow.
- **Revisit if**: image size or the base OS package CVE surface becomes a real constraint (e.g. a security
  policy that mandates distroless/scratch images). The application layers (binary + SPA assets) are a small
  fraction of the image; swapping the base would mostly need re-adding the non-root-user/CA-cert provisioning
  described above.

Measured image size (`docker images`): **~150MB** total, of which the Debian base is ~85MB, the `apt-get`
layer (ca-certificates + user creation) is ~10MB, and the application layers (binary + SPA) are ~15MB.

### `.dockerignore`

Excludes `target/`, `frontend/node_modules/`, `frontend/dist/` (host builds of these would otherwise bloat
the build context and — worse — could get copied into a stage instead of being rebuilt there), VCS metadata,
and unrelated tooling directories (`.claude/`, `.agents/`, etc.).

## Serving the SPA from `bff-api`

`crates/config`'s `Config` gained a `static_dir: Option<PathBuf>` field, sourced from the `STATIC_DIR`
environment variable — **unset by default**, not defaulted to a fixed path. `crates/bff-api/src/main.rs`
checks `cfg.static_dir` at startup: if it's set *and* the directory actually exists on disk, it mounts
`tower_http::services::ServeDir` (falling back to `<static_dir>/index.html` via `.not_found_service(...)`,
so client-side routes get `index.html` rather than a `404` — ADR-006, even though there's no real
client-side router yet per PROMPT-18's decision) as the router's `fallback_service`. Otherwise it logs a note
and skips static-file serving entirely.

This is why the existing Rust test suite (which never sets `STATIC_DIR` and has no built frontend on disk)
is unaffected — `cargo test --workspace` stays green with the static-file layer simply never mounted in any
test process.

The container image sets `STATIC_DIR=/app/frontend-dist` (see the `Dockerfile`'s `runtime` stage), where
`frontend/dist/`'s contents are copied.

**Route precedence**: the fallback service is added to the router *before* `.layer(...)` (metrics/correlation
middleware) is applied, and Axum's routing always tries every explicit `.route(...)`/`.nest(...)` match
before falling back — so `/healthz`, `/readyz`, `/api/*`, and `/metrics` can never be shadowed by the SPA
fallback; the static-file service only ever answers a request nothing else matched.

## Health checks: liveness (`/healthz`) vs. readiness (`/readyz`)

Split into two endpoints (`crates/bff-api/src/health.rs`), matching the two different questions an
orchestrator's probes ask:

- **`GET /healthz`** (liveness — "is the process up at all?"): always `200 {"status":"ok"}` once the listener
  is bound. Deliberately does **not** touch Postgres or any other dependency — a liveness probe failing
  typically causes an orchestrator to *restart* the container, which is the wrong response to "a downstream
  dependency is temporarily degraded."
- **`GET /readyz`** (readiness — "should traffic be routed to this instance right now?"): runs
  `persistence::check_connectivity` (a cheap `SELECT 1`) against the shared pool, bounded by a 2-second
  timeout so a hung database can't hang the probe itself. Returns `200
  {"status":"ok","checks":{"database":"ok"}}` when reachable, or `503
  {"status":"error","checks":{"database":"..."}}` (with `"error: <detail>"` or `"timeout"`) otherwise — the
  shape a readiness probe (which typically just stops routing traffic, not restarts) expects to poll on a
  short interval.

Both are unit-tested against a real (testcontainers) Postgres pool and against a deliberately-unreachable one
— see `crates/bff-api/src/health.rs`'s `tests` module and `crates/persistence/src/lib.rs`'s
`check_connectivity_*` tests.

## Graceful shutdown

`main.rs` wires `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`, where
`shutdown_signal()` races `tokio::signal::unix::signal(SignalKind::terminate())` (SIGTERM — what `docker
stop`/an orchestrator sends) against `tokio::signal::ctrl_c()` (SIGINT, for local-dev `Ctrl+C` parity),
resolving on whichever comes first. Once triggered, Axum stops accepting new connections and waits for
in-flight requests (and, per ADR-011, would wait for open SSE connections once those exist) to finish before
`axum::serve(...).await` returns; `main` then logs `"bff-api shut down"` and exits with code `0`.

**Verified against a real running container** (not just code review): starting the built image, sending
`docker stop` (which sends `SIGTERM`), and observing the logs shows `"received SIGTERM, starting graceful
shutdown"` followed immediately by `"bff-api shut down"`, with `docker stop` completing in well under 200ms —
nowhere near Docker's 10-second default grace period before it would escalate to `SIGKILL`. See the
Verification section of the PROMPT-28 work log / this repo's CI `deploy` job for the reproducible version.

## Cross-instance SSE fan-out: Postgres `NOTIFY`/`LISTEN` (PROMPT-32, ADR-014)

ADR-014 flags a requirement this container topology creates once more than one `bff-api` instance runs behind
a load balancer: `crates/bff-core/src/event_ingestion.rs`'s `EventBus` (PROMPT-30/31's SSE delivery mechanism)
is purely in-process (`tokio::sync::broadcast`). A notification ingested by instance A never reaches a
browser whose `GET /api/notifications/stream` connection is held by instance B, because each instance has its
own independent `EventBus`. ADR-014 names two options — sticky/session-affinity routing, or cross-instance
fan-out via Postgres `LISTEN`/`NOTIFY` — and recommends the latter; this is what's implemented.

### The two-hop delivery path

```text
instance A ingests a fresh NotificationItem/ActionQueueEntry (SaveOutcome::Inserted)
  -> persistence::PgNotifyPublisher::publish -> `SELECT pg_notify($1, $2)`
  -> Postgres fans the NOTIFY out to every connection currently LISTENing on that channel
       -> instance A's own event_notify_bridge::run_listen_bridge -> instance A's local EventBus -> instance A's SSE subscribers
       -> instance B's event_notify_bridge::run_listen_bridge -> instance B's local EventBus -> instance B's SSE subscribers
       -> ...every other running instance, identically
```

Every instance runs **two** background tasks (`crates/bff-api/src/main.rs`), not one:

1. `event_ingestion::run_polling_loop` (PROMPT-30) — polls Nexus, ingests events, and on a fresh insert hands
   the event to a `persistence::PgNotifyPublisher` (an `bff_core::EventPublisher`) instead of writing directly
   into this instance's own `EventBus`.
2. `event_notify_bridge::run_listen_bridge` (PROMPT-32, new) — holds a dedicated `persistence::PgListener`
   connection subscribed to the channel below for the life of the process; for every NOTIFY it receives, it
   reconstructs the full aggregate and publishes it into *this instance's* local `EventBus`, which
   `notifications_sse` (PROMPT-31, unchanged) already subscribes to.

The net effect: the instance that did the ingesting no longer publishes to its own subscribers directly — it
receives its own event back through the same Postgres round-trip every other instance does. That round-trip
is not a meaningful latency/ordering concern in practice: a `NOTIFY` issued on one connection is delivered to
every `LISTEN`ing connection on the same Postgres server via Postgres's own in-server signaling (not polling),
essentially immediately — see `crates/bff-api/src/event_notify_bridge.rs`'s module docs and its
cross-instance test (`a_notify_from_one_connection_reaches_two_independent_listener_bridges`), which measures
this reliably completing well under the test's 5-second timeout.

### Channel name and payload shape

- **Channel**: `bff_core::EVENT_NOTIFY_CHANNEL` = `"bff_ingested_events"` — one shared channel for both
  `NotificationItem` and `ActionQueueEntry` events, discriminated by the payload's `kind` field.
- **Payload**: a lightweight **pointer**, not the full event:
  ```json
  {"kind": "notification", "id": "8f14e45f-ceea-467e-bd9b-90b9cd7b8100"}
  {"kind": "action_queue_entry", "id": "3c6f1a2e-2f3a-4b8f-9c3d-7a1e2f4b5c6d"}
  ```
  **Why a pointer instead of the full `IngestedEvent` JSON**: Postgres caps a `NOTIFY` payload at 8000 bytes,
  server-enforced, with no way for a producer to detect the cutoff ahead of time other than staying well
  clear of it. Neither `NotificationItem` nor `ActionQueueEntry` bound `title`/`body`'s length (a structural
  choice, not a runtime-checked one) — a full event JSON payload is *usually* small, but "usually" is not a
  safe bet against a hard server-side limit a producer can't recover from mid-`NOTIFY`. The `{kind, id}`
  pointer is comfortably under 100 bytes regardless of the aggregate's actual text length, so it can never
  blow the limit. The cost is one extra indexed read per notification: every listener bridge re-fetches the
  full aggregate from Postgres by `id` (`NotificationRepository::find_by_id` /
  `ActionQueueRepository::find_by_id`, added for this purpose) before publishing it to its local `EventBus`.
  See `crates/bff-core/src/event_ingestion.rs`'s `EventNotifyPointer` doc comment for the full writeup.

### Reconnection

A `PgListener`'s connection can drop (network blip, Postgres restart/failover). `run_listen_bridge` never
returns — on a `recv()` error or a failed initial `listen()` call, it logs, waits 2 seconds, and retries from
scratch. A gap during reconnection only costs a real-time push, not data: the underlying row is already
durably in Postgres by the time it was ever NOTIFYed (publish only happens after `SaveOutcome::Inserted`), so
it still shows up on the consultant's next full-list fetch regardless of whether the SSE push arrived.

### Verified: genuine cross-instance delivery, not same-process reuse

`crates/bff-api/src/event_notify_bridge.rs`'s tests spin up two entirely independent `(PgListener, EventBus)`
pairs against one shared (testcontainers) Postgres — simulating two separate `bff-api` instances with no
shared in-process state at all — and NOTIFY from a third connection standing in for "instance A's ingestion"
(the same `persistence::PgNotifyPublisher` production uses). Both instances' independent `EventBus`es receive
the hydrated event; neither is ever touched directly by the publisher, so the only path an event can reach
either `EventBus` is the real NOTIFY/LISTEN mechanism. See that test module for the full test, and
`crates/persistence/src/event_notify.rs` for the lower-level unit-style proof that the JSON payload survives
an actual `pg_notify`/`PgListener` round-trip byte-for-byte.

## Migrations: an explicit pipeline step, not implicit at startup

Per ADR-014 ("migrations run... via an explicit CI/CD deploy step in production... not as an implicit
side-effect of application startup"), `bff-api` does **not** run migrations on boot. The CI `deploy` job (see
below) applies every `crates/persistence/migrations/*.up.sql` file directly via `psql` against the
provisioned Postgres *before* starting the built image — the same "explicit step ahead of the app starting"
shape a real deploy pipeline would use ahead of an orchestrator rollout.

## CI: the `deploy` job

`.github/workflows/ci.yml`'s `deploy` job (`needs: [rust, frontend]`, same "start after the fast gates pass"
rationale as the `e2e` job):

1. `docker build -t cognitum-consultants:ci .` — the multi-stage image above.
2. Starts a throwaway `postgres:17-alpine` container (`docker run`, published to the runner's `5432`) — the
   same pattern `frontend/e2e/support/test-stack.ts` already uses for the `e2e` job, reused here rather than
   introducing a second way to stand up Postgres in CI.
3. Applies every migration via `docker exec -i <postgres container> psql ... < migration.sql` — the explicit
   pre-rollout step described above.
4. Runs the built image (`docker run --network host ...`, `DATABASE_URL` pointed at the migrated Postgres) —
   this is the "deploy to a local Docker environment for validation" the PROMPT-28 acceptance criteria call
   for, standing in for a real target that doesn't exist yet.
5. Smoke-tests the running container: `/healthz` and `/readyz` both `200`, `/` returns the SPA's
   `index.html` (proving ADR-006's "one image serves both" model actually works, not just compiles), and
   `/api/session` with no session cookie still correctly returns `401` (proving the static-file fallback
   never shadows a real `/api/*` route).
6. `docker stop`s the container (graceful-shutdown validation — a hung shutdown would show up as this step
   running unusually long, since Docker only escalates to `SIGKILL` after its default grace period) and tears
   everything down.

See [`docs/ci.md`](ci.md) for the full pipeline (the `rust`, `frontend`, and `e2e` jobs this one runs after).

## Running it locally

```bash
docker build -t cognitum-consultants:local .

docker network create cognitum-local-net
docker run -d --name cognitum-local-db --network cognitum-local-net \
  -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=cognitum_consultants \
  -p 55432:5432 postgres:17-alpine

DATABASE_URL=postgres://postgres:postgres@localhost:55432/cognitum_consultants \
  sqlx migrate run --source crates/persistence/migrations

docker run -d --name cognitum-local-app --network cognitum-local-net -p 3000:3000 \
  -e DATABASE_URL=postgres://postgres:postgres@cognitum-local-db:5432/cognitum_consultants \
  -e APP_ENV=dev \
  cognitum-consultants:local

curl http://localhost:3000/healthz
curl http://localhost:3000/readyz
curl http://localhost:3000/          # SPA index.html

docker stop cognitum-local-app       # graceful shutdown
docker rm -f cognitum-local-app cognitum-local-db
docker network rm cognitum-local-net
```
