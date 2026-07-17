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

## `frontend` job

Working directory: `frontend/`. Node via `actions/setup-node` (Node 24), with `npm` cache keyed on
`frontend/package-lock.json`.

1. `npm ci` — install dependencies from the lockfile.
2. `npm run build` — `tsc -b && vite build` must succeed with no type errors.
3. `npm run lint` — runs `oxlint` against the frontend source; must report zero errors.

## Governing ADRs

- [ADR-013: Testing Strategy](../.plans/adr/ADR-013-testing-strategy.md) — §6 defines this CI gating layer;
  Playwright e2e tests (layer 5) are intentionally out of scope for this workflow and run on a slower cadence
  once added.
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
```
