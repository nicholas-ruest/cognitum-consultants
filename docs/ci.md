# CI

CI is defined at [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) and runs on every push and pull
request targeting `main`. Four jobs run: `rust` and `frontend` run in parallel; `e2e` (PROMPT-27) and `deploy`
(PROMPT-28) both declare `needs: [rust, frontend]` and only start once both finish, giving them the "slower
cadence" ADR-013 §6 calls for. A PR cannot merge unless all four jobs, and every step within them, succeed.
Each step is a separate, named CI step, so a failure is directly attributable to the command that failed.

## `rust` job

Rust toolchain: `dtolnay/rust-toolchain@stable`. Cargo registry and `target/` are cached via
`Swatinem/rust-cache`.

1. `cargo check --workspace` — the workspace must compile.
2. `cargo clippy --workspace --all-targets -- -D warnings` — no clippy lints, including on test/bench targets;
   warnings are treated as errors.
3. `cargo test --workspace` — all Rust unit/integration tests must pass (see ADR-013 for the layered testing
   strategy; it is expected and acceptable for a crate to report zero tests before its first test is added).

`bff-api` carries a `testcontainers-modules` (Postgres feature, ADR-010) dev-dependency for the layer-3
integration tests ADR-013 §1 describes, and `nexus-client` carries a `wiremock` dev-dependency for the
layer-2 gateway contract tests — no test currently exercises either (both land with their respective ACL/
persistence units). `testcontainers` needs a reachable Docker daemon; GitHub's `ubuntu-latest` runners ship
one preinstalled and running, so no extra CI setup step is required today. If that stops being true (e.g. a
future self-hosted runner), a `docker info` sanity step should be added here before those tests start relying
on it.

## `frontend` job

Working directory: `frontend/`. Node via `actions/setup-node` (Node 24), with `npm` cache keyed on
`frontend/package-lock.json`.

1. `npm ci` — install dependencies from the lockfile.
2. `npm run build` — `tsc -b && vite build` must succeed with no type errors.
3. `npm run lint` — runs `oxlint` against the frontend source; must report zero errors.
4. `npm run test` — runs `vitest run` (ADR-013 layer 4: component/unit tests via Vitest + React Testing
   Library); must report zero failures.

Playwright e2e (ADR-013 layer 5, `frontend/e2e/`) now runs in CI as the separate `e2e` job below — it is no
longer local-only.

## `e2e` job (PROMPT-27)

Working directory: `frontend/` (checkout is the whole repo — this job also builds the Rust workspace).
`needs: [rust, frontend]`.

Brings up the **full real stack** and drives it with Playwright:

1. Install the Rust stable toolchain (`dtolnay/rust-toolchain@stable`) and cache the cargo registry/`target/`
   (`Swatinem/rust-cache`) — `bff-api` is built for real, not mocked, by this job.
2. Install Node.js (Node 24) and run `npm ci` in `frontend/`.
3. `npx playwright install --with-deps chromium` — installs the browser binary this job's one configured
   project (`chromium`) needs.
4. `npx playwright test` — runs every spec under `frontend/e2e/`. `playwright.config.ts`'s `globalSetup`
   (`frontend/e2e/support/global-setup.ts`) orchestrates everything the specs need *before* any test runs:
   - A throwaway Postgres container (`docker run postgres:17-alpine`, per `frontend/e2e/support/test-stack.ts`)
     — the same "GitHub-hosted `ubuntu-latest` ships Docker preinstalled" assumption the `rust` job's
     `testcontainers`-based tests already rely on, just invoked from Node instead of Rust.
   - That Postgres instance migrated by applying every `crates/persistence/migrations/*.up.sql` file directly
     via `psql` (not `sqlx migrate run` — see `test-stack.ts`'s doc comment for why raw SQL was chosen over
     depending on `sqlx-cli` being installed).
   - A mock Nexus HTTP server (`frontend/e2e/support/mock-nexus-server.ts`, ADR-007) standing in for
     `nexus.cognitum.one` at the HTTP boundary — plain `node:http`, no added dependency.
   - `cargo build -p bff-api`, then the resulting binary spawned with `DATABASE_URL`/`NEXUS_ENDPOINT_URL`
     pointed at the two services above.
   - Playwright's own pre-existing `webServer` (the real Vite dev server, `npm run dev`) around all of that,
     proxying `/api/*` to the now-live `bff-api` (`frontend/vite.config.ts`'s existing proxy config —
     unchanged, since it already targets `127.0.0.1:3000`, the port `bff-api` binds to either way).
5. On failure, the Playwright HTML report (`frontend/playwright-report/`) is uploaded as a build artifact
   (`actions/upload-artifact@v4`) for post-mortem inspection.

See [`docs/SALES_FLOW_PATTERN.md`](SALES_FLOW_PATTERN.md) §5 for how this fits into the full testing pyramid,
and for the reference this job's orchestration modules (`e2e/support/*`) are meant to be reused by Phase 4's
own e2e specs (PROMPT-34+) with no changes beyond a new spec file.

## `deploy` job (PROMPT-28)

`needs: [rust, frontend]`, same slower-cadence rationale as `e2e`. Builds the repo-root `Dockerfile` image,
migrates a throwaway Postgres, runs the built image against it, and smoke-tests `/healthz`, `/readyz`, the
SPA at `/`, and `/api/session`'s `401` before tearing everything down — see
[`docs/deployment.md`](deployment.md) for the full walkthrough, the runtime base image trade-off, and why
this is presently the CI-verifiable proxy for a real deploy (no target environment is chosen yet, ADR-014).

## Governing ADRs

- [ADR-013: Testing Strategy](../.plans/adr/ADR-013-testing-strategy.md) — §6 defines this CI gating layer;
  Playwright e2e tests (layer 5) run on the slower cadence the `e2e` job's `needs: [rust, frontend]` gives it.
- [ADR-002: Primary Language Rust, Secondary TypeScript](../.plans/adr/ADR-002-primary-language-rust-secondary-typescript.md)
  — explains why CI has exactly two toolchain surfaces (Rust + Node/TS) rather than more; `e2e` uses both in
  one job because the flow it drives spans both.
- [ADR-014: Deployment and Runtime Topology](../.plans/adr/ADR-014-deployment-runtime-topology.md) — governs
  the `deploy` job: the multi-stage `Dockerfile` it builds, migrations as an explicit pre-rollout step, and
  the health-check/graceful-shutdown contract it smoke-tests. See [`docs/deployment.md`](deployment.md).

## Running the gates locally

```bash
# Rust
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Frontend
cd frontend
npm ci
npm run build
npm run lint
npm run test          # Vitest (layer 4)

# E2E (layer 5) — needs a reachable Docker daemon and `cargo`/`sqlx` etc. on
# PATH (`~/.cargo/bin`, per `crates/persistence/README.md`); Playwright
# browsers install once with `npx playwright install --with-deps`.
npm run test:e2e
```
