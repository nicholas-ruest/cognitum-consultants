# CI

CI is defined at [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) and runs on every push and pull
request targeting `main`. Two jobs run in parallel; a PR cannot merge unless both jobs, and every step within
them, succeed. Each step is a separate, named CI step, so a failure is directly attributable to the command
that failed.

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

Playwright e2e (ADR-013 layer 5, `frontend/e2e/`) is **not yet run in CI**. It requires a served app (built
frontend, and eventually the BFF/mocked Nexus) rather than the bare `npm run dev` the local harness uses;
wiring that up is explicitly deferred to U27 (Sales lead-conflict e2e), which establishes the reusable
Playwright CI pattern. Until then, `npm run test:e2e` (`playwright test`) is a local-only smoke check.

## Governing ADRs

- [ADR-013: Testing Strategy](../.plans/adr/ADR-013-testing-strategy.md) — §6 defines this CI gating layer;
  Playwright e2e tests (layer 5) are intentionally out of scope for this workflow and run on a slower cadence
  once added (see U27).
- [ADR-002: Primary Language Rust, Secondary TypeScript](../.plans/adr/ADR-002-primary-language-rust-secondary-typescript.md)
  — explains why CI has exactly two toolchain surfaces (Rust + Node/TS) rather than more.

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
npm run test:e2e      # Playwright (layer 5); needs `npx playwright install --with-deps` once
```
