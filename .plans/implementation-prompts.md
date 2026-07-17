# Implementation Prompts — consultants.cognitum.one

## Overview

This document contains 42 coherent, implementation-ready prompts — one per unit from `.plans/implementation-sequence.md` — in the exact dependency order that sequence establishes. **These prompts must be executed in order.** Dependencies are not suggestions; they represent hard preconditions (e.g., U02's Axum service must exist before U07's observability middleware has anything to attach to). Each prompt is self-contained at the prompt-text level, but references specific ADRs and DDD artifacts that an implementation agent should read to avoid ambiguity. Acceptance criteria in each prompt are pulled from real ADR/DDD content, not generic platitudes.

**How to use this document:** Hand each prompt (from "Depends on:" through the end of "Prompt:") to a coding agent one at a time, in order, after the prior unit has landed. Reference the named ADRs and DDD docs as ground truth for details — they are the source, not summaries in the prompt itself.

---

## Table of Contents

| Unit | Title | Dependency Summary |
|------|-------|-------------------|
| U01 | Cargo workspace + crate stubs | — (start) |
| U02 | Axum health endpoint | U01 |
| U03 | Frontend Vite scaffold | U01 |
| U04 | CI pipeline | U01, U02, U03 |
| U05 | Testing tooling scaffolding | U04 |
| U06 | Dev orchestration scripts | U01, U02, U03 |
| U07 | Observability foundation | U02 |
| U08 | Config crate | U01 |
| U09 | Persistence crate + migrations | U08, U04 |
| U10 | Auth crate + dev-stub | U09, U08 |
| U11 | Session middleware + /api/session | U10, U02 |
| U12 | nexus-client foundation | U01, U07, U05 |
| U13 | Resilience middleware | U12 |
| U14 | Armor ACL gateway | U12, U13, U11 |
| U15 | Permission-aware presentation enforcement | U14, U11 |
| U16 | Frontend data-fetching (TanStack Query) | U03 |
| U17 | Frontend shell components | U03 |
| U18 | Frontend login flow | U11, U17, U16 |
| U19 | Permission-aware nav rendering | U15, U18, U17 |
| U20 | ConsultantPreferences aggregate | U09, U11 |
| U21 | DashboardConfiguration aggregate | U15, U20, U09 |
| U22 | CrossCapabilityWorkflowSession aggregate | U09, U20/U21 |
| U23 | Dashboard composition endpoints + shell | U21, U19, U16 |
| U24 | Sales ACL gateway | U12, U13, U05 |
| U25 | Lead-conflict-check endpoint | U24, U15, U13 |
| U26 | Sales frontend feature module | U25, U16, U17 |
| U27 | Sales e2e + pattern doc | U25, U26, U05 |
| U28 | Containerize + deploy pipeline | U09, U26/U27, U07, U04 |
| U29 | Notification/ActionQueue aggregates | U09, U24/U25 |
| U30 | Nexus event ingestion (polling) | U29, U12, U24/U25 |
| U31 | SSE endpoint + event bus | U30, U07, U02 |
| U32 | SSE connection-affinity/scaling | U31, U28 |
| U33 | Frontend notification/action queue UI | U31, U16, U17 |
| U34 | Commit ACL + feature module | baseline*, U22, U26 |
| U35 | Edu ACL + feature module | baseline* |
| U36 | Capacity ACL + feature module | baseline* |
| U37 | Customer ACL + feature module | baseline* |
| U38 | Execution ACL + feature module | baseline* |
| U39 | Products ACL + feature module | baseline* |
| U40 | Landscape ACL + feature module | baseline* |
| U41 | Legal ACL + feature module | baseline*, U34 |
| U42 | Design-system extraction | U17, U34–U41 (+ missing ADR) |

*baseline = U12, U13, U15, U16, U17, U23, U29, U30, U31.

---

## Prompts

### Group A — Foundational Scaffolding

## PROMPT-01: Cargo workspace + crate stubs

**Depends on:** none — first unit.

**Governing ADRs:** ADR-002 (Rust primary language), ADR-004 (workspace/repo layout).

**Governing DDD model:** None yet (pure scaffolding), but crate names/boundaries must match ADR-004's structure exactly so later units have the right home from the start.

**Prompt:**

Set up the Cargo workspace at the repo root, following ADR-004's crate layout precisely. Create six empty-but-compiling Rust crates: `bff-api`, `bff-core`, `nexus-client`, `auth`, `persistence`, `config`. Each crate should have a minimal `lib.rs` (or `main.rs` for `bff-api`) and declare dependencies on the standard library only at this stage. The workspace `Cargo.toml` at the repo root must list all six members and be wired to compile them together (`cargo check` and `cargo build` must succeed across the entire workspace).

**Acceptance criteria:**
- `Cargo.toml` exists at repo root with `[workspace]` and `members = ["crates/bff-api", ...]`.
- All six crates exist under `crates/` with compiling `Cargo.toml` files.
- `cargo check` succeeds (no compilation errors or warnings).
- `cargo clippy -- -D warnings` produces no warnings.
- Each crate can be addressed individually (`cargo check -p bff-core`, etc.).

---

## PROMPT-02: Axum health-check endpoint

**Depends on:** U01.

**Governing ADRs:** ADR-003 (Axum), ADR-004 (`bff-api` owns HTTP transport only).

**Governing DDD model:** None (infrastructure only).

**Prompt:**

Implement the `bff-api` crate as an Axum HTTP service. Add a `GET /healthz` endpoint that returns HTTP 200 with a JSON `{"status": "ok"}` body (or equivalent minimal response). The service should start on a configurable port (default `localhost:3000` in dev). The handler must be thin — no business/aggregation logic, no database calls. This endpoint will later be extended (U28) but for now it is a smoke test of the Axum stack. Ensure `cargo check` and `cargo clippy` pass for `bff-api`.

**Acceptance criteria:**
- `bff-api` has Axum as a dependency.
- A `main.rs` (or equivalent entry point) boots an Axum server.
- `GET /healthz` returns 200 with a JSON response.
- Service can be run locally (`cargo run -p bff-api` starts the server).
- No domain logic in the handler; no dependencies on `bff-core`, `nexus-client`, or aggregates yet.

---

## PROMPT-03: Frontend Vite + TypeScript + Tailwind + React scaffold

**Depends on:** U01 (parallel-safe with U02; both only need repo root to exist).

**Governing ADRs:** ADR-002 (TypeScript secondary language), ADR-005 (Vite + Tailwind + React).

**Governing DDD model:** `frontend/src/features/<capability>/` directory convention stubbed, one empty directory per capability from `domain-map.md` §1 context list: `sales`, `commit`, `edu`, `capacity`, `customer`, `execution`, `products`, `landscape`, `legal`. Do NOT implement capability logic yet — just the directory structure.

**Prompt:**

Create a Vite + TypeScript + React frontend at `frontend/` as a sibling TypeScript workspace (outside the Cargo workspace, with its own `package.json`). Install Tailwind CSS and configure it per Vite's standard setup. Create a minimal root `App.tsx` component that renders a placeholder heading and uses a Tailwind class to prove styling is wired. Create the `frontend/src/features/` directory with one empty subdirectory per capability (`sales/`, `commit/`, etc.). The build (`npm run build`) and dev server (`npm run dev`) must both succeed without errors.

**Acceptance criteria:**
- `frontend/package.json` exists with Vite, React, TypeScript, Tailwind dependencies.
- `npm run dev` starts a local dev server (on port 5173 by default).
- `npm run build` produces a `dist/` directory with compiled assets.
- Root `App.tsx` renders with a Tailwind class applied (visual proof of styling).
- `frontend/src/features/` directory structure exists with nine empty subdirectories (capability names from `domain-map.md`).

---

## PROMPT-04: CI pipeline (build/lint/test gates)

**Depends on:** U01, U02, U03.

**Governing ADRs:** ADR-013 (CI gating layer 6), ADR-002 (two-toolchain CI surface).

**Governing DDD model:** None.

**Prompt:**

Wire up CI to run on every PR and block merge on failure. The pipeline must execute: `cargo check`, `cargo clippy -- -D warnings`, `cargo test` (all Rust layers), and for the frontend: `npm run build` and a lint step (via `npm run lint` — set up any lightweight linter, e.g. ESLint basic config). All must pass for a PR to merge. Document the CI configuration (GitHub Actions YAML, or equivalent) in the repo so developers know what gates their changes are subject to.

**Acceptance criteria:**
- CI workflow file exists and is wired to run on PR/push.
- `cargo check`, `cargo clippy`, `cargo test`, `npm run build`, `npm run lint` all execute and pass.
- CI blocks merge on any step failure (or is documented as doing so).
- CI output is human-readable (logs show which step failed, if any).

---

## PROMPT-05: Testing tooling scaffolding

**Depends on:** U04.

**Governing ADRs:** ADR-013 (layers 1–4 tooling).

**Governing DDD model:** None directly, but wiremock fixture convention must be shaped to express per-gateway inbound shapes (e.g., `AccountClaimResult` fixtures covering each `match_status` value).

**Prompt:**

Install and wire testing infrastructure per ADR-013's five layers. For Rust: add `wiremock-rs` as a dev-dependency on `nexus-client`, and `testcontainers` as a dev-dependency on `bff-api` (for Postgres test containers). For the frontend: install Vitest, React Testing Library, and Playwright (used later but installed now). Create a trivial smoke test in each harness (a passing Rust unit test, a passing Vitest component test, a Playwright test that opens the app homepage) to prove each layer runs in CI. Ensure Docker is available in CI for `testcontainers` and document any CI environment setup required.

**Acceptance criteria:**
- `wiremock-rs` and `testcontainers` installed in `bff-api` `Cargo.toml` as dev-dependencies.
- A trivial Rust unit test exists and passes (`cargo test`).
- Vitest, React Testing Library, Playwright installed; `npm run test` (Vitest) passes at least one smoke test.
- A Playwright smoke test exists and passes (opens the app, checks for a basic element).
- CI has Docker available (or is documented as requiring it).

---

## PROMPT-06: Dev orchestration scripts

**Depends on:** U01, U02, U03.

**Governing ADRs:** None directly (implementation-plan.md Phase 0 deliverable; ADR-004's workspace/frontend split makes this necessary).

**Governing DDD model:** None.

**Prompt:**

Create a `scripts/` directory at the repo root with a shell script (or npm script) that boots both the `bff-api` server and the Vite frontend dev server together for local development. Running a single command (e.g., `npm run dev` at the root, or `./scripts/dev.sh`) should start both servers and log output from each, making it easy for developers to start the full stack locally without managing two terminals. The script should be documented (short README or inline comment) so its purpose is clear.

**Acceptance criteria:**
- A script (shell, npm, or Rust CLI) exists in `scripts/` that boots both the BFF and frontend.
- Running the script starts both `cargo run -p bff-api` and `npm run dev` (from `frontend/`).
- Output from both processes is visible (interleaved or in a clear format).
- Script can be interrupted cleanly (Ctrl+C stops both).

---

## PROMPT-07: Observability foundation

**Depends on:** U02 (needs an Axum app to attach middleware to).

**Governing ADRs:** ADR-012 (tracing, correlation ID middleware, OpenTelemetry, `metrics` crate + Prometheus exporter).

**Governing DDD model:** None (infrastructure), but this mechanism will later let a BFF log line, a Nexus log line, and a target-capability log line be joined during incident diagnosis.

**Prompt:**

Implement observability per ADR-012. Add `tracing`, `tracing-subscriber`, and `tracing-opentelemetry` to `bff-api`. Create an Axum middleware layer that generates a correlation ID for every inbound request (or accepts one from an inbound header) and stores it as a tracing span field. Every log line should carry that correlation ID so request-scoped logs are traceable. Add structured JSON logging in non-local environments (human-readable locally). Implement a `/metrics` endpoint (via the `metrics` crate and Prometheus exporter) that exposes per-route request count/latency. The correlation ID mechanism must exist and be propagatable (mechanism exists even though no outbound caller exists yet — that comes in U12).

**Acceptance criteria:**
- `tracing`, `tracing-subscriber`, `tracing-opentelemetry`, `metrics`, `metrics-exporter-prometheus` added to `bff-api`.
- Axum middleware generates/accepts correlation IDs and stores them as span fields.
- `/metrics` endpoint exists and returns Prometheus-format metrics (even if sparse at this stage).
- JSON logging works in a test environment (configure via env var or feature flag).
- Logs contain correlation ID for request tracing.

---

### Group B — Core Infra Plumbing

## PROMPT-08: Config crate

**Depends on:** U01.

**Governing ADRs:** ADR-004 (`config` crate responsibility), ADR-014 (12-factor env-var config).

**Governing DDD model:** None.

**Prompt:**

Implement the `config` crate to load typed configuration from environment variables with per-environment (dev/staging/prod) override support. Create a `Config` struct that holds application settings (database URL, port, log level, Nexus endpoint URL, etc.). Load it from environment variables on startup, with sensible dev defaults. Consume this in `bff-api`'s startup (even if just to prove it's wired — e.g., log the chosen log level). The config crate should be simple and decoupled; no domain logic, just env-var parsing.

**Acceptance criteria:**
- `config` crate defines a `Config` struct with typed fields (String, u16, etc.).
- Config loads from environment variables (e.g., `DATABASE_URL`, `LOG_LEVEL`, `PORT`).
- `bff-api` calls `config::load()` on startup and uses at least one setting (e.g., port for the server).
- Dev defaults are sensible (e.g., `localhost:5432` for a local Postgres).
- `cargo check -p config` passes.

---

## PROMPT-09: Postgres persistence crate + migrations tooling

**Depends on:** U08 (needs DB connection config), U04 (CI must be able to run `sqlx` compile-time checks).

**Governing ADRs:** ADR-010 (Postgres via `sqlx`, migrations via `sqlx-cli`, multi-instance correctness rationale).

**Governing DDD model:** `consultant-experience-context.md` §1.4/§2.4 repository interfaces are the target shape this crate's trait implementations must eventually satisfy. This unit only stands up the datastore connection, migration runner, and empty `persistence/migrations/` directory; no concrete aggregate tables yet.

**Prompt:**

Set up the `persistence` crate to connect to a Postgres database using `sqlx` (with compile-time query checking via offline mode). Create a connection pool that is initialized on startup and passed to the rest of the application (injected into handlers, etc.). Set up `sqlx-cli` for managing migrations. Create an empty `persistence/migrations/` directory (migrations will be added per aggregate in later units). In CI, ensure `testcontainers` spins up a Postgres instance for integration tests; in deployed environments, connection details come from config (U08). Ensure `sqlx`'s compile-time query checking runs in CI (offline mode) and passes.

**Acceptance criteria:**
- `persistence` crate has `sqlx` (with `postgres` feature) and connection pool logic.
- `bff-api` can instantiate a connection pool on startup (via `config` from U08).
- `persistence/migrations/` directory exists (initially empty).
- `sqlx-cli` can run (`sqlx migrate --help` works).
- CI has Docker for `testcontainers` and can spin up a Postgres instance for tests.
- `sqlx` compile-time checks run in CI (offline mode) and pass.

---

## PROMPT-10: Auth crate: session model + dev-stub login provider

**Depends on:** U09 (session storage reuses this datastore), U08 (config for feature-flagging the dev stub).

**Governing ADRs:** ADR-008 (BFF-managed server-side session; interim dev-stub gated to never boot outside `dev` env).

**Governing DDD model:** None directly (auth is infrastructure, deliberately separated from `bff-core` per ADR-004).

**Prompt:**

Implement the `auth` crate with a session interface and a dev-stub provider. Define a `Session` struct with minimal data (session id, consultant id, grant expiry). Create a `SessionProvider` trait that can look up a session by id. Implement a dev-stub provider (feature-gated behind a `dev-auth` flag) that returns fixed consultant identities (e.g., `consultant_id: "dev-consultant-001"`). The stub must refuse to start outside a `dev` environment (check via config/env var and panic if activated in prod). Session rows are persisted via the `persistence` crate (to be wired next in U11). Do NOT implement real Armor integration yet — the stub is interim (per `implementation-plan.md` §6 risk #2).

**Acceptance criteria:**
- `auth` crate defines `Session` and `SessionProvider` traits.
- A dev-stub provider exists and implements `SessionProvider`.
- Dev-stub is feature-gated (`dev-auth` or similar).
- Stub refuses to load outside `dev` environment (panics with clear error message).
- Session storage/retrieval skeleton exists (will be filled in U11).
- No real Armor integration code present.

---

## PROMPT-11: BFF session middleware + `/api/session` endpoint

**Depends on:** U10, U02.

**Governing ADRs:** ADR-008 (`HttpOnly`/`Secure`/`SameSite=Strict` cookie, session lookup middleware), ADR-006 (`/api/*` namespace).

**Governing DDD model:** `consultant-experience-context.md`'s "Consultant" ubiquitous-language term (`consultant_id` reference, no locally-held identity data).

**Prompt:**

Implement session middleware in `bff-api` that extracts session cookies from requests and uses the `auth` crate's `SessionProvider` to look up the authenticated consultant. Middleware should attach the session/consultant identity to the request context (via Axum's `Extension<>` or similar). Implement a `GET /api/session` endpoint that returns the authenticated consultant's identity (just the `consultant_id` for now; permission assertions come later in U14/U15). Successful dev-stub login should set a session cookie with flags: `HttpOnly`, `Secure` (in non-local), `SameSite=Strict`. Unauthenticated requests to any protected route should be rejected with 401 or redirected to login.

**Acceptance criteria:**
- Session middleware exists and can extract cookies.
- `/api/session` returns a JSON object with `consultant_id`.
- Successful session creation sets an `HttpOnly` cookie.
- Unauthenticated requests to protected routes return 401 or 403.
- Session lookup works (dev-stub returns a fixed consultant identity).
- `Secure` and `SameSite=Strict` flags are set on cookies (or documented as set for deployed envs).

---

## PROMPT-12: `nexus-client` foundation: `NexusTransport` trait + correlation-ID propagation

**Depends on:** U01, U07 (correlation ID must exist to be propagated), U05 (wiremock available for this unit's own tests).

**Governing ADRs:** ADR-007 (REST/JSON, `NexusTransport` trait built on `reqwest`, no gateway modules yet), ADR-003 (tower middleware reused on outbound calls), ADR-012 (correlation ID + W3C Trace Context attached to every outbound call).

**Governing DDD model:** `anti-corruption-layers.md`'s framing that Nexus is the *only* integration point — this unit is the literal enforcement point: no code outside this crate may construct an HTTP client aimed at any `*.cognitum.one` sub-business service.

**Prompt:**

Create the `nexus-client` crate with a `NexusTransport` trait and a `reqwest`-backed implementation. The trait should have a method (e.g., `call()` or generic `send()`) that takes a request and returns a response, abstracting away the HTTP details. The implementation must attach correlation ID (from U07) and W3C Trace Context headers (`traceparent`) on every outbound call, pulled from the current tracing span. No capability-specific gateway modules exist yet — this is transport-layer only. Add `wiremock-rs` tests (smoke test fixtures) to prove the transport can make and mock HTTP calls. `bff-api` does not yet import `nexus-client`, but it exists and compiles.

**Acceptance criteria:**
- `nexus-client` crate exists with `NexusTransport` trait.
- `reqwest`-backed implementation exists.
- Correlation ID (from tracing span) is attached to outbound requests.
- W3C `traceparent` header is propagated.
- Wiremock smoke tests exist and pass.
- No gateway modules (sales, commit, etc.) yet — just transport.

---

## PROMPT-13: Resilience middleware on `NexusTransport`

**Depends on:** U12.

**Governing ADRs:** ADR-016 (per-gateway timeout budgets via `tower::timeout`, bounded retries with backoff for idempotent reads only, concurrent fan-out with per-call isolation, circuit breaker skeleton).

**Governing DDD model:** None directly — this is transport-layer infrastructure applied uniformly ahead of any specific ACL, consistent with `anti-corruption-layers.md` §11.

**Prompt:**

Add resilience layers to `NexusTransport` (or the `reqwest` client it wraps) via `tower` middleware. Implement: (1) a `tower::timeout` layer with configurable per-gateway timeout budgets (default values to be tuned later against ADR-012 metrics; for now, use sensible placeholders like 5s for reads, 3s for user-blocking writes); (2) a retry wrapper for read-only/idempotent calls only, with exponential backoff and a bounded retry count (e.g., 3 retries); (3) a circuit breaker skeleton (can be stubbed for now but the API/trait must exist) that tracks per-gateway failure rates. Do NOT apply retry automatically to all calls — provide two distinct wrapper functions (one for idempotent queries, one for commands) so non-idempotent calls are never auto-retried. No per-capability tuning values yet — all deferred to future ADR-012 metrics analysis.

**Acceptance criteria:**
- `tower::timeout` layer wraps `NexusTransport` calls; timeout is configurable per gateway.
- Retry wrapper exists for idempotent read calls (exponential backoff, bounded count).
- Retry wrapper does NOT apply to command/write calls (two distinct APIs).
- Circuit breaker trait/interface exists (implementation can be minimal; real tuning deferred).
- Wiremock tests cover timeout and retry scenarios (timeout triggers, retry succeeds on 2nd attempt).

---

## PROMPT-14: Armor ACL gateway (`ArmorGateway`)

**Depends on:** U12, U13, U11 (session identity needed to request assertions on the consultant's behalf).

**Governing ADRs:** ADR-007 (gateway module structure/typed trait pattern), ADR-008 (identity propagation this gateway rides on), ADR-009 (this is the sole source of Permission Assertions).

**Governing DDD model:** `anti-corruption-layers.md` §10 (Armor ACL — `PermissionAssertion { consultant_id, capability, scope, expires_at }`, no outbound business command, inbound `PermissionAssertionChanged`); `domain-events.md` §1 (`PermissionAssertionChanged` consumed by Consultant Workspace).

**Prompt:**

Implement an `ArmorGateway` module inside `nexus-client/src/armor.rs` (or similar). Define a `PermissionAssertion` struct with fields: `consultant_id`, `capability`, `scope`, `expires_at` (matching `anti-corruption-layers.md` §10). Create an `ArmorGateway` trait with a method to fetch current Permission Assertions for a consultant (e.g., `fetch_assertions(consultant_id: &str) -> Result<Vec<PermissionAssertion>>`). Implement it using `NexusTransport` and the session identity from U11 (pass the consultant's token/credential on every call). Add wiremock fixtures covering various assertion scenarios (consultant with 5 capabilities, with 1, with none). No other gateway module exists yet — Armor is the sole permission source for now.

**Acceptance criteria:**
- `nexus-client/src/armor/` module (or `armor.rs`) exists with `PermissionAssertion` struct.
- `ArmorGateway` trait + implementation exists.
- `fetch_assertions()` calls Nexus via `NexusTransport`, passing consultant identity.
- Wiremock fixtures exist for multiple assertion scenarios.
- Wiremock tests pass (mock returns assertions, gateway parses them).

---

## PROMPT-15: Permission-aware presentation enforcement (BFF)

**Depends on:** U14, U11.

**Governing ADRs:** ADR-009 (three-layer enforcement: server-side filtering/caching with TTL bounded by `expires_at`, short-circuit 403 before a wasted Nexus round-trip).

**Governing DDD model:** `consultant-experience-context.md` §1.2 `DashboardConfiguration` invariant #1 (a card the consultant has no Permission Assertion for cannot be persisted) — this unit builds the *general* filtering/caching primitive; the aggregate to protect exists in U21.

**Prompt:**

Implement permission-aware presentation enforcement in the BFF. Create an in-memory per-session cache of a consultant's Permission Assertions (fetched via `ArmorGateway` from U14), with a TTL bounded by the shortest `expires_at` in the assertion set. Expose an internal "is capability X permitted" check (e.g., `is_permitted(&consultant_id, capability: &str) -> bool`) that BFF handlers can call. Before attempting any Nexus call for an unpermitted capability, short-circuit with HTTP 403 (Forbidden). The cache should refresh automatically when expired or when a `PermissionAssertionChanged` event arrives (event consumption comes in U30, but the cache structure must be ready). Document the caching behavior and TTL semantics.

**Acceptance criteria:**
- Per-session permission cache exists and is populated from `ArmorGateway`.
- Cache TTL is bounded by the shortest `expires_at` across assertions.
- `is_permitted()` check works (returns true/false).
- Handlers can call `is_permitted()` before attempting Nexus calls.
- Short-circuit with 403 before wasting a Nexus round-trip.
- Cache is testable in integration tests (mock `ArmorGateway` responses and verify cache behavior).

---

### Group C — Phase 1: Shell & Navigation

## PROMPT-16: Frontend data-fetching foundation (TanStack Query)

**Depends on:** U03.

**Governing ADRs:** ADR-015 (TanStack Query as the sole server-state library; query-key convention namespaced by capability + consultant).

**Governing DDD model:** Query-key namespace must mirror `frontend/src/features/<capability>/` (U03), which itself mirrors `domain-map.md`'s context list.

**Prompt:**

Set up TanStack Query (React Query) in the frontend. Wrap the app root with `QueryClientProvider`. Document and enforce a query-key naming convention: capability-scoped keys (e.g., `['sales', 'conflicts', consultant_id]`, `['commit', 'proposals', consultant_id]`) so query keys mirror the capability structure and facilitate cache invalidation per capability. Create a typed key-builder helper (e.g., `queryKeys.sales.conflicts(id)`) to make it easy for every feature module to follow the convention. Test that `useQuery` and `useMutation` work in a component.

**Acceptance criteria:**
- `@tanstack/react-query` installed.
- `QueryClientProvider` wraps the root `<App>`.
- Query-key convention is documented (e.g., in a `frontend/src/lib/queryKeys.ts`).
- A typed key-builder helper exists (TypeScript).
- At least one example component uses `useQuery` or `useMutation` with the convention.
- Vitest tests for query-key helpers pass.

---

## PROMPT-17: Frontend shell components (ported from manage)

**Depends on:** U03.

**Governing ADRs:** ADR-005 (React), ADR-006 (SPA has no server-rendering responsibility — plain client components).

**Governing DDD model:** `research.md` §"Dashboard Relationship" (layout, sidebar, header, cards, tables, forms, search, filters, alerts, dialogs — copied/adapted from manage.cognitum.one, Manage-specific business logic stripped).

**Prompt:**

Port dashboard shell/layout components from manage.cognitum.one's React codebase (sidebar, header, card grid, basic form components, dialog/modal primitives, alert components). Adapt them to fit this repo's Tailwind config and remove any Manage-specific business logic (e.g., strip out Manage's particular navigation items or role-specific UI). These are pure UI primitives — no API calls, no state management beyond React props/useState, no capability-specific logic. Place them in `frontend/src/components/` (shared UI library). The components should render with placeholder/empty data so they can be tested in isolation (no dependency on live Nexus data yet). Document where each component came from (URL reference to manage repo or similar) so the one-time borrow is clear.

**Acceptance criteria:**
- Shell components (layout, sidebar, header, card grid, forms, dialogs, alerts) exist in `frontend/src/components/`.
- Components render correctly with Tailwind styling applied.
- No Manage-specific business logic remains (payment flows, role-specific items, etc.).
- Components accept props for content (children, title, etc.) but no business state.
- Vitest smoke tests render each major component without errors.
- Source comments/docs note the one-time borrow from manage.

---

## PROMPT-18: Frontend login flow + session wiring

**Depends on:** U11, U17, U16.

**Governing ADRs:** ADR-008 (browser never holds a long-lived credential; relies on the `HttpOnly` session cookie), ADR-006 (calls `/api/session`).

**Governing DDD model:** None beyond the "Consultant" ubiquitous-language term already established in U11.

**Prompt:**

Implement a login flow in the frontend. Create a simple login form (email/dev-consultant-id input, submit button). On submit, POST to a `/api/login` endpoint (to be implemented in the BFF, using the dev-stub from U10). On success, the session cookie is set (automatically, since it's `HttpOnly`). Fetch the authenticated consultant's identity via `GET /api/session` using TanStack Query (from U16). On successful login, store the consultant identity in a React context or app state, and redirect to the dashboard. The login form should be displayed when unauthenticated; once `/api/session` returns a consultant, render the app shell (from U17) as authenticated.

**Acceptance criteria:**
- A login form component exists (`frontend/src/pages/LoginPage.tsx` or similar).
- Form POSTs to `/api/login` (endpoint implementation in BFF).
- Session query (GET `/api/session`) is wired via TanStack Query.
- Successful login redirects to dashboard/home.
- Session identity is available to the rest of the app (context/state).
- Unauthenticated requests (no session) show login page.
- Vitest tests confirm login flow logic.

---

## PROMPT-19: Permission-aware nav rendering

**Depends on:** U15, U18, U17.

**Governing ADRs:** ADR-009 (client-side rendering layer — UX only, never trusted as enforcement; nav items conditionally rendered from Permission Assertions returned via `/api/session`).

**Governing DDD model:** `anti-corruption-layers.md` §10 (`PermissionAssertion` shape consumed here).

**Prompt:**

Extend the `/api/session` endpoint (from U11) to include the consultant's current Permission Assertions (fetched from the permission cache in U15). Modify the login/session query (from U18) to fetch and expose assertions. In the sidebar component (from U17), conditionally render nav items based on Permission Assertions: only show nav items for capabilities the consultant is permitted to use. This is a UX/rendering layer — not enforcement (that happens server-side in U15) — so the frontend must not treat "not rendered" as authoritative. Include a test that verifies nav items appear/disappear based on assertion mocking.

**Acceptance criteria:**
- `/api/session` includes `permission_assertions: PermissionAssertion[]` in its response.
- Session query fetches assertions via TanStack Query.
- Sidebar nav is conditionally rendered based on `permission_assertions`.
- Nav items render only for permitted capabilities.
- Vitest tests mock assertions and verify nav renders correctly.
- Frontend code includes a comment reminding that this is UX only, not enforcement.

---

### Group D — Workspace Aggregates

## PROMPT-20: `ConsultantPreferences` aggregate + Postgres repository

**Depends on:** U09, U11 (needs `consultant_id`).

**Governing ADRs:** ADR-010 (Postgres/`sqlx` repository implementation), ADR-004 (`bff-core` owns the aggregate; `persistence` implements its repository trait).

**Governing DDD model:** `consultant-experience-context.md` §1.2 `ConsultantPreferences` aggregate — invariants: known/versioned key allow-list only, exactly one per consultant, values never encode business data.

**Prompt:**

Implement the `ConsultantPreferences` aggregate in `bff-core`. Define it with: (1) a `consultant_id` field, (2) a map/dict of preferences keyed by a known, versioned allow-list of preference type names (e.g., `"theme"`, `"default_view"`, `"notification_channel_opt_in"`), (3) invariant enforcement that rejects unknown keys. Define the `ConsultantPreferencesRepository` trait in `bff-core` with methods: `find_by_consultant_id()`, `save()`, `upsert_preference()`. Implement the repository against Postgres in the `persistence` crate using `sqlx`. Create the `consultant_preferences` table via migration (`.sql` file in `persistence/migrations/`). Unit-test the aggregate in `bff-core` (invariants enforced) without spinning up Axum or a database.

**Acceptance criteria:**
- `ConsultantPreferences` aggregate defined in `bff-core/src/` with invariant enforcement.
- Preference key allow-list is versioned/centralized (not scattered).
- `ConsultantPreferencesRepository` trait defined in `bff-core`.
- Postgres table created via migration (`.sql` file).
- Repository implemented in `persistence` with `sqlx` queries.
- Unit tests in `bff-core` verify invariants (unknown keys rejected, etc.).
- Integration tests in `persistence` verify DB round-trip.

---

## PROMPT-21: `DashboardConfiguration` aggregate + Postgres repository

**Depends on:** U15 (invariant #1 needs Permission Assertions to validate against), U20 (shared `bff-core`/`persistence` patterns), U09.

**Governing ADRs:** ADR-009 (invariant #1 enforcement — a card without a Permission Assertion cannot be persisted), ADR-010 (Postgres repository).

**Governing DDD model:** `consultant-experience-context.md` §1.2 `DashboardConfiguration` aggregate — all four invariants (permission-fit cards, unique card positions, exactly one config per consultant, default card set on creation).

**Prompt:**

Implement the `DashboardConfiguration` aggregate in `bff-core` with a `CardPlacement` child entity. Enforce all four invariants from `consultant-experience-context.md` §1.2: (1) every card's `module_id` must match a capability the consultant has a Permission Assertion for (call the `is_permitted()` check from U15); (2) card positions are unique within one configuration; (3) exactly one config per consultant; (4) default card set is applied at creation. Define the `DashboardConfigurationRepository` trait with `find_by_consultant_id()`, `save()`, `delete_by_consultant_id()`. Implement the repository against Postgres in `persistence` (table + migration). Unit-test aggregate invariants in `bff-core` (verify that saving a card without permission is rejected).

**Acceptance criteria:**
- `DashboardConfiguration` and `CardPlacement` defined in `bff-core`.
- All four invariants enforced in aggregate code.
- Invariant #1 explicitly calls `is_permitted()` (permission check at aggregate boundary).
- `DashboardConfigurationRepository` trait defined.
- Postgres tables created via migrations.
- Repository implemented with `sqlx`.
- Unit test verifies that saving a card without permission is rejected.
- Integration test verifies DB persistence.

---

## PROMPT-22: `CrossCapabilityWorkflowSession` aggregate + repository

**Depends on:** U09, U20/U21 (established `bff-core`/`persistence` aggregate pattern).

**Governing ADRs:** ADR-010 (Postgres repository, including the `expire_older_than` housekeeping sweep).

**Governing DDD model:** `consultant-experience-context.md` §1.2 `CrossCapabilityWorkflowSession` aggregate — invariants: opaque-reference-only origin/target, bounded TTL with no resume past `expires_at`, linear state machine, completion never itself mutates the target capability's data.

**Prompt:**

Implement the `CrossCapabilityWorkflowSession` aggregate in `bff-core`. It tracks an in-progress hop between capabilities (e.g., from Sales to Commit). Fields: `session_id`, `consultant_id`, `origin_capability`, `origin_reference` (opaque ID), `target_capability`, `target_reference` (opaque ID, optional initially), `status` (linear state machine: `started` → `in_progress` → `{completed|abandoned|expired}`), `expires_at` (TTL). Invariants: references are opaque (never stored/duplicated), status follows the state machine (no regression), expiry is enforced (cannot resume past `expires_at`), completion does not mutate external data. Define the repository trait with `find_by_id()`, `find_active_by_consultant_id()`, `save()`, `expire_older_than()` (housekeeping sweep). Implement against Postgres in `persistence`. Unit-test state machine and TTL logic; not yet wired to any real capability handoff (that is Phase 4 starting at U34).

**Acceptance criteria:**
- `CrossCapabilityWorkflowSession` aggregate defined with state machine.
- Invariants enforced: opaque references, linear state machine, TTL expiry.
- Repository trait defined with `expire_older_than()` for housekeeping.
- Postgres table + migration created.
- Repository implemented with `sqlx`.
- Unit tests verify state machine (valid transitions pass, invalid rejected).
- Unit tests verify TTL (cannot resume past `expires_at`).
- Integration test verifies `expire_older_than()` housekeeping.

---

## PROMPT-23: BFF dashboard composition endpoints + frontend dashboard shell rendering

**Depends on:** U21, U19, U16.

**Governing ADRs:** ADR-006 (`/api/*` shape), ADR-009 (dashboard endpoint enforces permission filtering), ADR-010 (persisted via U21's repository).

**Governing DDD model:** `consultant-experience-context.md` §1.2 `DashboardConfiguration` — this unit exposes `GET`/`PUT` over that aggregate.

**Prompt:**

Implement `GET /api/dashboard` and `PUT /api/dashboard` endpoints in the BFF. The GET endpoint returns the consultant's current `DashboardConfiguration` (via the repository from U21), applying the permission-filtering invariant (if a card references a capability the consultant lost access to, don't return it, or flag it as unavailable). If no configuration exists, apply the default card set (from U21's invariant #4). The PUT endpoint accepts a new card layout and persists it via the repository (invariants enforced at aggregate boundary). Wire the frontend's session query (from U18) to call `GET /api/dashboard` on login and store the result in TanStack Query. Render a dashboard shell (using components from U17) with placeholder slots for each card, bound to `module_id` references (no live data from Nexus yet — that comes in Phase 2/4).

**Acceptance criteria:**
- `GET /api/dashboard` returns `DashboardConfiguration` JSON (or default if none exists).
- `PUT /api/dashboard` accepts new layout and persists via repository.
- GET response respects permission filtering (U15's `is_permitted()` enforced).
- Dashboard endpoint is permission-gated (403 if consultant isn't authenticated).
- Frontend fetches dashboard via `useQuery` and renders shell with card slots.
- Cards render as placeholder boxes with `module_id` labels (no real data yet).
- Vitest tests verify dashboard query/mutation wiring.

---

### Group E — Phase 2: Sales ACL Reference Flow

## PROMPT-24: Sales ACL gateway (`SalesGateway`)

**Depends on:** U12, U13, U05 (wiremock fixtures).

**Governing ADRs:** ADR-007 (gateway trait/module pattern), ADR-016 (tighter timeout budget for user-blocking calls).

**Governing DDD model:** `anti-corruption-layers.md` §1 Sales ACL — `AccountClaimResult { match_status, creation_allowed, display_message, permitted_actions }`; outbound `CheckAccountClaimCommand`, `RequestCollaborationCommand`, `SubmitReferralCommand`; inbound `AccountClaimDetermined`, `CollaborationRequestAcknowledged`, `ReferralSubmitted`.

**Prompt:**

Implement a `SalesGateway` module in `nexus-client/src/sales.rs`. Define DTOs for the checked capabilities (e.g., `AccountClaimResult`, `PermissionAction` enum). Create a `SalesGateway` trait with methods: `check_account_claim(company_name: &str, consultant_id: &str) -> Result<AccountClaimResult>`, `request_collaboration(...)`, `submit_referral(...)`. Implement using `NexusTransport`, applying the shorter timeout budget from U13 (this is synchronous, user-blocking). Add wiremock fixtures covering multiple `match_status` values (e.g., "active_owned_account", "available_claim", "no_match") to mock Sales' decision verdicts per the worked example in `anti-corruption-layers.md` §1. Wiremock tests must verify that the gateway parses each fixture correctly.

**Acceptance criteria:**
- `nexus-client/src/sales/` module exists with `SalesGateway` trait and implementation.
- DTOs match `anti-corruption-layers.md` §1 (e.g., `AccountClaimResult`, `match_status`).
- Three outbound methods (`check_account_claim`, `request_collaboration`, `submit_referral`) defined.
- Uses `NexusTransport` with timeout applied (shorter budget for user-blocking).
- Wiremock fixtures cover 3+ `match_status` values from the worked example.
- Wiremock tests pass (gateway correctly parses mock responses).

---

## PROMPT-25: BFF lead-conflict-check endpoint

**Depends on:** U24, U15 (permission short-circuit), U13 (timeout applied).

**Governing ADRs:** ADR-016 (per-card/per-response shape), ADR-009 (permission gate).

**Governing DDD model:** `anti-corruption-layers.md` §1 worked example — BFF relays `AccountClaimDetermined` verbatim; no re-adjudication of `creation_allowed`.

**Prompt:**

Implement `POST /api/sales/lead-conflict-check` endpoint in the BFF. Accept a company name (JSON payload). Call `SalesGateway.check_account_claim()` (from U24) after checking permission (U15 `is_permitted("sales")` short-circuits with 403 if unpermitted). Relay the `AccountClaimResult` verbatim to the frontend (do NOT override `creation_allowed` — that is Sales' decision). Also implement handlers for `RequestCollaborationCommand` and `SubmitReferralCommand` (accept consultant input, call the corresponding gateway methods, relay responses). Return a JSON response with the Sales verdict shape exactly as received.

**Acceptance criteria:**
- `POST /api/sales/lead-conflict-check` accepts `{"company_name": "..."}`.
- Permission check (U15) rejects unpermitted calls with 403.
- Calls `SalesGateway.check_account_claim()` and relays result verbatim.
- `AccountClaimResult` (match_status, creation_allowed, display_message, permitted_actions) returned as-is.
- `POST /api/sales/request-collaboration` and `POST /api/sales/submit-referral` handlers exist.
- Integration tests mock `SalesGateway` and verify endpoint behavior.
- No re-adjudication of `creation_allowed` logic in BFF code.

---

## PROMPT-26: Frontend Sales lead-conflict feature module

**Depends on:** U25, U16, U17.

**Governing ADRs:** ADR-015 (TanStack Query mutation for the check call).

**Governing DDD model:** `anti-corruption-layers.md` §1 — company-entry form + conditional rendering of `display_message` and buttons listed in `permitted_actions`.

**Prompt:**

Create a Sales feature module at `frontend/src/features/sales/`. Implement a component that: (1) renders a form with a company name input field, (2) on submit, calls `POST /api/sales/lead-conflict-check` via TanStack Query mutation (from U16), (3) on response, renders the `display_message` from the result, (4) renders only the buttons listed in `permitted_actions` (e.g., "Request Collaboration", "Submit Referral"), (5) each button triggers the corresponding command endpoint. The result is never cached/reused (per ADR-015's rule for this flow); each company entry yields a fresh check. This feature module becomes one live card on the dashboard (U23).

**Acceptance criteria:**
- `frontend/src/features/sales/` directory exists.
- Company name form input renders.
- Form submits to `POST /api/sales/lead-conflict-check` via `useMutation`.
- Response `display_message` is rendered.
- Only `permitted_actions` buttons are shown (not hardcoded).
- "Request Collaboration" / "Submit Referral" buttons are wired to their endpoints.
- Vitest tests mock the mutation and verify conditional rendering.
- Component integrates as a card/module on dashboard (card placement via U23).

---

## PROMPT-27: Sales flow e2e (Playwright) + pattern write-up doc

**Depends on:** U25, U26, U05 (Playwright installed).

**Governing ADRs:** ADR-013 (layer 5: Playwright e2e, canonical smoke test).

**Governing DDD model:** This flow becomes the template Phase 4 (U34–U41) replicates.

**Prompt:**

Create a Playwright e2e test that drives the full stack (frontend + BFF + mocked Nexus) through the lead-conflict-warning flow end-to-end. Steps: (1) load the app, (2) log in via dev-stub, (3) navigate to Sales (or verify it's visible in nav if on dashboard), (4) enter a company name, (5) submit the lead-conflict check, (6) verify the response message and permitted actions are rendered, (7) optionally click one of the permitted actions. Mock Nexus entirely (use `wiremock` or Playwright's request mocking). The test must pass in CI (on merge-to-main, slower cadence). Additionally, write a short `docs/` document (e.g., `docs/SALES_FLOW_PATTERN.md`) that captures the pattern: DTO shape → gateway → BFF handler → frontend feature module → tests. This becomes the reference for replicating in Phase 4.

**Acceptance criteria:**
- Playwright test file exists (e.g., `tests/e2e/sales-lead-conflict.spec.ts`).
- Test covers full flow: login → navigate → enter company → submit → verify response → action.
- Nexus is mocked (wiremock fixtures or Playwright request mocking).
- Test passes locally and in CI.
- A pattern write-up doc exists (`docs/SALES_FLOW_PATTERN.md` or similar).
- Doc explains: DTOs, gateway, BFF handler, frontend feature, tests (the template for Phase 4).

---

### Group F — Deployment Validation

## PROMPT-28: Containerize BFF + SPA, deploy pipeline, health/graceful shutdown

**Depends on:** U09 (Postgres as provisioned dependency), U26/U27 (real Sales flow worth deploying), U07 (metrics/tracing must be reachable), U04 (CI gates precede deploy).

**Governing ADRs:** ADR-014 (multi-stage Dockerfile with `cargo-chef`, single image serves API + SPA, orchestrator-agnostic env-var config, health-check endpoints, graceful shutdown on `SIGTERM`, migrations as explicit pipeline step).

**Governing DDD model:** None directly — validates ADR-006 "served API + SPA" model actually runs as one deployable unit.

**Prompt:**

Create a multi-stage Dockerfile (at repo root) that: (1) builds the Rust BFF using `cargo-chef` for layer caching, (2) builds the frontend SPA (npm build), (3) uses a runtime stage with both artifacts (API binary + frontend dist), serving the SPA as static files from the BFF. The single image serves both `/api/*` routes and `/*` (SPA fallback routing). Extend the `/healthz` endpoint (from U02) to include readiness checks (database connection, external service pings if relevant). Implement graceful shutdown: on `SIGTERM`, drain in-flight requests (e.g., cancel new accepts, wait for current requests to finish) before exiting. Create a deploy pipeline step (CI/CD workflow, e.g., GitHub Actions) that: runs migrations on the provisioned Postgres before rollout, builds the image, and deploys it (to a local Docker environment for testing, or a real target if available). Document the build/deploy process.

**Acceptance criteria:**
- Dockerfile exists (multi-stage, `cargo-chef`, single image).
- Image builds locally (`docker build .` succeeds).
- Running the image serves both `/api/*` and the SPA (static files).
- Extended `/healthz` endpoint (liveness + readiness checks).
- `SIGTERM` triggers graceful shutdown (drains requests).
- Deploy pipeline runs migrations, builds, and deploys.
- CI runs the pipeline (or it's documented as a deployment prerequisite).
- Single image serves both API + SPA (validates ADR-006 model).

---

### Group G — Phase 3: Notifications & Action Queue

## PROMPT-29: `NotificationItem` + `ActionQueueEntry` aggregates + Postgres repositories

**Depends on:** U09, U24/U25 (first real capability whose events these aggregates ingest).

**Governing ADRs:** ADR-010 (Postgres unique constraint on `(origin_capability, origin_event_id)` for idempotent ingestion).

**Governing DDD model:** `consultant-experience-context.md` §2.2 — both aggregates with idempotency, display-safe-summary-only, linear state machines, confirmed-completion semantics.

**Prompt:**

Implement `NotificationItem` and `ActionQueueEntry` aggregates in `bff-core`. `NotificationItem` has: `id`, `consultant_id`, `origin_capability`, `origin_event_id` (unique key together), `title`, `body` (short summary only), `deep_link`, `read_state` (unread → read), `created_at`. Invariants: `(origin_capability, origin_event_id)` is unique (idempotent ingestion), payload is display-safe summary only, read state is one-way. `ActionQueueEntry` has: similar fields plus `action_state` (pending → in_progress → {completed|expired}), `expires_at`. Invariants: same idempotency, state machine (linear, no regression), completion only via confirmation event (invariant #3 — this context cannot unilaterally mark complete). Define repository traits in `bff-core` (find, save, update state, mark read/dismissed, purge/expire older than). Implement against Postgres in `persistence` with migrations. Unit-test all invariants; integration tests verify idempotency under redelivery.

**Acceptance criteria:**
- `NotificationItem` aggregate defined in `bff-core` with all invariants.
- `ActionQueueEntry` aggregate defined with state machine and invariant #3 (no local completion).
- Repository traits defined with required methods (purge/expire housekeeping included).
- Postgres tables with `(origin_capability, origin_event_id)` unique constraint.
- Migrations created.
- Repositories implemented with `sqlx`.
- Unit tests verify invariants (idempotency, state machine, read-state one-way).
- Integration tests verify idempotency under duplicate event delivery.

---

## PROMPT-30: Nexus event ingestion (polling) → Notification/ActionQueue mapping

**Depends on:** U29, U12 (transport for polling), U24/U25 (Sales events as first source).

**Governing ADRs:** ADR-011 (Nexus→BFF ingestion via polling, feeding internal event bus).

**Governing DDD model:** `domain-events.md` §2 `CapabilityEventReceived` envelope, mapped into `NotificationItem` or `ActionQueueEntry`.

**Prompt:**

Implement an event ingestion service in `bff-core` that polls Nexus for `CapabilityEventReceived` envelopes at a configured interval (e.g., every 5 seconds). For each envelope received, classify it: if the event implies a required consultant action (e.g., `TaskAssigned`, `CollaborationRequestAcknowledged`), create an `ActionQueueEntry`; if it's informational, create a `NotificationItem`. Use the unique `(origin_capability, origin_event_id)` constraint from U29 to prevent duplicates (idempotent ingestion). Route successful ingestion into an internal event bus (simple in-process pubsub). Test idempotency by delivering the same event twice and verifying only one row is created. Start with Sales events (`AccountClaimDetermined`, `CollaborationRequestAcknowledged`, `ReferralSubmitted`) as real test cases; the mapping logic is capability-agnostic.

**Acceptance criteria:**
- Event ingestion service exists (polling loop, configurable interval).
- Polls a Nexus endpoint for `CapabilityEventReceived` envelopes.
- Classifies events into `NotificationItem` or `ActionQueueEntry` based on event type.
- Uses `(origin_capability, origin_event_id)` unique constraint for idempotency.
- Routes ingested events into an internal event bus.
- Integration tests verify idempotent ingestion (same event delivered twice → one row).
- Sales events are tested as real examples.

---

## PROMPT-31: SSE endpoint + internal event bus

**Depends on:** U30, U07 (Axum SSE support, tower middleware), U02.

**Governing ADRs:** ADR-011 (SSE for BFF→browser push; `axum::response::sse`; unidirectional; consultant actions remain ordinary POST/PATCH calls).

**Governing DDD model:** `consultant-experience-context.md` §2 — this is the delivery mechanism for `NotificationItem`/`ActionQueueEntry` changes.

**Prompt:**

Implement a `GET /api/notifications/stream` SSE endpoint in the BFF that connects a consultant's browser to a push stream. When the event ingestion service (U30) creates or updates a `NotificationItem` or `ActionQueueEntry`, publish it to the internal event bus. The SSE endpoint listens to that bus and pushes events to connected clients (send JSON with the notification/action item data). The connection should be consultant-scoped (a consultant only receives their own notifications). Consultant actions (marking read, taking an action, etc.) remain ordinary `POST`/`PATCH` endpoints (not sent over SSE — unidirectional push only per ADR-011). Document the event format pushed to browsers.

**Acceptance criteria:**
- `GET /api/notifications/stream` endpoint exists.
- Connects to internal event bus and sends SSE events.
- Events are consultant-scoped (no cross-consultant bleed).
- Event format includes `notification_id`, `title`, `body`, `deep_link`, etc.
- Consultant actions use separate POST/PATCH endpoints (not SSE).
- Wiremock/mock integration tests verify event flow from ingestion → SSE push.

---

## PROMPT-32: SSE connection-affinity / horizontal-scaling fan-out

**Depends on:** U31, U28 (deployment topology this must integrate with).

**Governing ADRs:** ADR-014 (sticky/session-affinity routing or cross-instance fan-out via Postgres LISTEN/NOTIFY).

**Governing DDD model:** None directly — pure infrastructure closing ADR-011/ADR-014 loop.

**Prompt:**

Implement horizontal-scaling fan-out for SSE notifications. When the BFF scales to multiple instances, an event ingested on instance A must reach a browser whose SSE connection is held by instance B. Choose one approach: (1) sticky/session-affinity routing (load balancer routes the same consultant to the same instance), or (2) cross-instance fan-out via Postgres `LISTEN`/`NOTIFY` (event ingestion publishes to Postgres, all instances subscribe). Option 2 is recommended per ADR-014. Implement the chosen approach and test it: deploy two BFF instances, ingest an event on one, verify it reaches a client connected to the other. Document the scaling approach.

**Acceptance criteria:**
- Sticky routing or Postgres `LISTEN`/`NOTIFY` implemented (one chosen).
- Event ingested on instance A reaches client connected to instance B.
- Integration test verifies multi-instance fan-out.
- Approach documented (ADR link + deployment notes).

---

## PROMPT-33: Frontend notification centre + action queue UI

**Depends on:** U31, U16, U17.

**Governing ADRs:** ADR-011 (SSE consumption), ADR-015 (SSE handler calls `queryClient.invalidateQueries`).

**Governing DDD model:** `consultant-experience-context.md` §2.1 glossary — UI reflects same one-way read-state transition and confirmed-only completion semantics.

**Prompt:**

Implement a notification centre and action queue UI in the frontend. Create components that: (1) subscribe to the `/api/notifications/stream` SSE endpoint (e.g., using a custom React hook), (2) render a live list of notifications (title, body, dismiss button, deep-link anchor), (3) render a separate action queue list (items with pending/in-progress state, a "take action" button, expiry indicator), (4) on SSE event arrival, update the respective list and/or invalidate the relevant TanStack Query keys (U16) to re-fetch data if needed, (5) mark notifications as read (PATCH endpoint), (6) confirm/complete action items (POST endpoint that triggers the underlying capability command and marks complete only when the owning capability confirms), (7) implement one-way read-state (no "unread" toggle). Render as a persistent sidebar, modal, or dashboard card depending on design. Verify SSE-triggered cache invalidation works in Vitest.

**Acceptance criteria:**
- Notification centre component exists and renders notifications.
- Action queue component renders pending/in-progress items.
- Subscribe to `/api/notifications/stream` SSE endpoint.
- On SSE event, update UI (new notification appears, action state changes).
- TanStack Query keys invalidated on SSE events (triggering re-fetches).
- Mark-read endpoint called (one-way transition).
- Action completion routed to owning capability (not local decision).
- Vitest tests verify SSE subscription and cache invalidation.

---

### Group H — Phase 4: Remaining ACL Integrations

## PROMPT-34: Commit ACL + routes + frontend feature module (+ Sales→Commit deep link)

**Depends on:** baseline (U12, U13, U15, U16, U17, U23, U29, U30, U31), U22 (`CrossCapabilityWorkflowSession` — first real cross-capability handoff), U26 (Sales feature module as deep-link origin).

**Governing ADRs:** ADR-007 (gateway pattern), ADR-016 (idempotent-read retries only; non-idempotent commands never auto-retried).

**Governing DDD model:** `anti-corruption-layers.md` §2 Commit ACL — `ProposalSummary` shape, outbound `CreateProposalCommand`/`RequestProposalActionCommand`, inbound `ProposalCreated`/`ProposalStatusChanged`/`ProposalAccepted`; `domain-events.md` §3 Commit table.

**Prompt:**

Implement the Commit ACL following the Sales flow pattern (U27). (1) Create `CommitGateway` in `nexus-client/src/commit.rs` with `ProposalSummary` DTO, methods for `create_proposal()`, `request_proposal_action()`, and handlers for inbound events. (2) Implement BFF endpoints: `POST /api/commit/proposals` (creates a proposal, optionally from a `CrossCapabilityWorkflowSession` originating in Sales), `GET /api/commit/proposals` (lists consultant's proposals), `POST /api/commit/proposals/{id}/actions` (requests an action on a proposal). (3) Wire inbound events (ProposalCreated, ProposalStatusChanged, ProposalAccepted) into `NotificationItem`/`ActionQueueEntry` via U30's ingestion. (4) Create `frontend/src/features/commit/` with a proposal workspace component: list of proposals, detail view, action buttons. (5) Implement deep-link from Sales: when a consultant resolves a lead conflict in the Sales feature module, optionally trigger `POST /api/commit/proposals` with origin reference (via `CrossCapabilityWorkflowSession`), redirecting to the newly created proposal. Follow U27's documented pattern exactly.

**Acceptance criteria:**
- `CommitGateway` implemented (matching `anti-corruption-layers.md` §2).
- Wiremock fixtures for Commit events (ProposalCreated, etc.).
- BFF endpoints for proposals (list, create, action).
- Proposal creation works with `CrossCapabilityWorkflowSession` origin.
- Inbound events mapped to `NotificationItem`/`ActionQueueEntry`.
- Frontend Commit feature module (list, detail, actions).
- Deep-link from Sales: lead conflict resolution → proposal creation → redirect.
- Playwright e2e test drives Sales→Commit flow.
- No duplicate ADR-016 retries on `CreateProposalCommand`.

---

## PROMPT-35: Edu ACL + routes + frontend feature module

**Depends on:** baseline (U12, U13, U15, U16, U17, U23, U29, U30, U31).

**Governing ADRs:** ADR-007, ADR-016 (read-mostly, longer timeout allowance).

**Governing DDD model:** `anti-corruption-layers.md` §3 Edu ACL — `LearningSnapshot` shape, outbound `RequestLearningCatalogQuery`, inbound `CourseCompleted`/`CertificationIssued`/`TrainingRequirementDue`.

**Prompt:**

Implement the Edu ACL following the Sales pattern. (1) Create `EduGateway` in `nexus-client/src/edu.rs` with `LearningSnapshot` DTO and `request_learning_catalog()` method. Apply a longer timeout (read-mostly, per ADR-016). (2) Implement `GET /api/edu/catalog` endpoint that fetches and returns the consultant's learning snapshot. (3) Wire inbound events (CourseCompleted, CertificationIssued, TrainingRequirementDue) into notifications/action queue. (4) Create `frontend/src/features/edu/` with a learning dashboard card: display courses, certifications, training requirements. (5) Add as a card on the main dashboard (U23). Test via Playwright following the Sales pattern.

**Acceptance criteria:**
- `EduGateway` implemented (read-mostly).
- Longer timeout budget applied (ADR-016).
- Wiremock fixtures for Edu events.
- `GET /api/edu/catalog` endpoint returns LearningSnapshot.
- Inbound events mapped to notifications.
- Frontend Edu feature module (courses, certs, training due).
- Rendered as a dashboard card.
- Playwright test covers Edu flow.

---

## PROMPT-36: Capacity ACL + routes + frontend feature module

**Depends on:** baseline.

**Governing ADRs:** ADR-007 (deliberately narrow — no query for cross-consultant data).

**Governing DDD model:** `anti-corruption-layers.md` §4 Capacity ACL — write-heavy, read-narrow: own profile only. `domain-map.md` explicitly: "Consultants must not receive internal Capacity access."

**Prompt:**

Implement the Capacity ACL with a deliberately restricted API. (1) Create `CapacityGateway` in `nexus-client/src/capacity.rs` with `ConsultantProfileIntake` DTO and `update_own_profile()` method. **Crucially: no query shape for cross-consultant data in the gateway itself** — the structure forbids it, not just runtime filtering. (2) Implement BFF endpoint `PATCH /api/capacity/profile` that accepts profile updates (skills, certifications, languages, availability, geographic coverage) and submits them via the gateway. Return the response from Capacity (accepted/rejected + reason). (3) Implement `GET /api/capacity/profile` to fetch the consultant's own profile (for display in a profile-edit form). **Verify by code review that no code path can query another consultant's data.** (4) Create `frontend/src/features/capacity/` with a profile-edit form (readonly or controlled, per Capacity's response). (5) Add as a card on the dashboard or as a settings page. Test via Playwright.

**Acceptance criteria:**
- `CapacityGateway` implemented (no cross-consultant query shape in trait definition).
- No code can query another consultant's data (structural, not filtering).
- `PATCH /api/capacity/profile` accepts updates and calls gateway.
- `GET /api/capacity/profile` returns own profile only.
- Wiremock fixtures for ProfileUpdateAccepted/Rejected events.
- Frontend Capacity feature (profile-edit form).
- Rendered as card/page.
- Code review confirms no cross-consultant query paths.

---

## PROMPT-37: Customer ACL + routes + frontend feature module

**Depends on:** baseline.

**Governing ADRs:** ADR-007, ADR-009 (query itself scoped to "assigned or permitted").

**Governing DDD model:** `anti-corruption-layers.md` §5 Customer ACL — `CustomerContextCard` shape, outbound `RequestAssignedCustomerContextQuery`, inbound `CustomerHealthChanged`/`CustomerInteractionLogged`.

**Prompt:**

Implement the Customer ACL. (1) Create `CustomerGateway` in `nexus-client/src/customer.rs` with `CustomerContextCard` DTO and `request_assigned_customer_context()` method. The query is scoped to assigned/permitted customers — permission filtering happens at the query boundary, not client-side. (2) Implement `GET /api/customer/assigned` endpoint that returns the consultant's assigned/permitted customer list (context cards with health status, relationship summary, deep links). (3) Wire inbound events (CustomerHealthChanged, CustomerInteractionLogged) into notifications. (4) Create `frontend/src/features/customer/` component: list of assigned customers, detail card with health/interaction summary. (5) Add as dashboard card. Test via Playwright.

**Acceptance criteria:**
- `CustomerGateway` implemented (scope to assigned/permitted).
- `GET /api/customer/assigned` returns customer list.
- Permission filtering at query boundary (not post-fetch filtering).
- Inbound events mapped to notifications.
- Frontend Customer feature (list, details, health/interaction summary).
- Dashboard card integration.
- Playwright test covers Customer flow.

---

## PROMPT-38: Execution ACL + routes + frontend feature module

**Depends on:** baseline.

**Governing ADRs:** ADR-007, ADR-016 (TaskAssigned/DeliveryRiskRaised route through ActionQueueEntry with confirmed-completion).

**Governing DDD model:** `anti-corruption-layers.md` §6 Execution ACL — `EngagementSnapshot` shape, outbound `RequestAssignedEngagementsQuery`, inbound `MilestoneCompleted`/`DeliveryRiskRaised`/`TaskAssigned`.

**Prompt:**

Implement the Execution ACL. (1) Create `ExecutionGateway` in `nexus-client/src/execution.rs` with `EngagementSnapshot` DTO (engagements, workstreams, milestones, tasks, delivery status) and `request_assigned_engagements()` method. (2) Implement `GET /api/execution/engagements` that returns the consultant's assigned delivery workspace. (3) **Important**: inbound events `TaskAssigned` and `DeliveryRiskRaised` are not just notifications — they are `ActionQueueEntry` sources (per ADR-016) requiring confirmed completion via the owning capability, not a local state flip. Wire them into U29's action queue (not just notifications). (4) Create `frontend/src/features/execution/` component: delivery workspace with engagements, milestones, tasks, risks. Action queue items for assigned tasks should route completion through the BFF back to Execution (confirmed by inbound event). (5) Add as dashboard card. Test via Playwright.

**Acceptance criteria:**
- `ExecutionGateway` implemented.
- Wiremock fixtures for Execution events.
- `GET /api/execution/engagements` returns EngagementSnapshot.
- TaskAssigned and DeliveryRiskRaised routed to `ActionQueueEntry` (not just notifications).
- Action queue completion requires confirmation event from Execution.
- Frontend Execution feature (workspace, tasks, risks).
- Task assignment appears in action queue with confirmed-completion semantics.
- Dashboard card integration.
- Playwright test covers Execution flow.

---

## PROMPT-39: Products ACL + routes + frontend feature module

**Depends on:** baseline.

**Governing ADRs:** ADR-007, ADR-016 (read-only, most cacheable, longest timeout, most aggressive retry).

**Governing DDD model:** `anti-corruption-layers.md` §7 Products ACL — `ProductReferenceCard` shape, outbound `RequestProductCatalogQuery`, inbound `ProductCatalogUpdated`.

**Prompt:**

Implement the Products ACL. (1) Create `ProductsGateway` in `nexus-client/src/products.rs` with `ProductReferenceCard` DTO and `request_product_catalog()` method. Apply the longest timeout and most aggressive retry budget (read-only, least latency-sensitive per ADR-016). (2) Implement `GET /api/products/catalog` endpoint that returns approved product reference data (name, packaging, pricing guidance, demo assets). Consider caching this response aggressively (products change rarely). (3) Wire inbound events (ProductCatalogUpdated) — these are low priority, unlikely to warrant `ActionQueueEntry`, just refresh the cache/send a low-severity notification. (4) Create `frontend/src/features/products/` component: product catalog card showing approved products/services usable during proposal/selling conversations. (5) Add as dashboard card. Test via Playwright.

**Acceptance criteria:**
- `ProductsGateway` implemented (longest timeout, most retry).
- Wiremock fixtures for Products events.
- `GET /api/products/catalog` returns ProductReferenceCard list.
- Aggressive client-side caching (products rarely change).
- ProductCatalogUpdated events handled (cache refresh, low-severity notification).
- Frontend Products feature (catalog card).
- Dashboard integration.
- Playwright test covers Products flow.

---

## PROMPT-40: Landscape ACL + routes + frontend feature module (read + write)

**Depends on:** baseline.

**Governing ADRs:** ADR-007 (this gateway has a real outbound write path — `SubmitFieldObservationCommand` — still non-idempotent, never auto-retried).

**Governing DDD model:** `anti-corruption-layers.md` §8 Landscape ACL — `IntelligenceDigestItem` (inbound) / `FieldObservationSubmission` (outbound); this repo is a minor upstream contributor but Landscape governs "approved."

**Prompt:**

Implement the Landscape ACL (read + write). (1) Create `LandscapeGateway` in `nexus-client/src/landscape.rs` with `IntelligenceDigestItem` and `FieldObservationSubmission` DTOs. Methods: `request_intelligence_digest()` (read) and `submit_field_observation()` (write). (2) Implement `GET /api/landscape/intelligence` (fetch approved intelligence items) and `POST /api/landscape/observations` (submit a field observation). Note: `SubmitFieldObservationCommand` is a command (non-idempotent), so per ADR-016 it is never auto-retried; frontend must consciously retry if it fails. (3) Wire inbound events (IntelligenceItemPublished) into notifications (feeds a low-priority refresh). (4) Create `frontend/src/features/landscape/` component: intelligence digest card (read approved items), observation-submission form (write). (5) Add as dashboard card. Test via Playwright.

**Acceptance criteria:**
- `LandscapeGateway` implemented (read + write).
- Wiremock fixtures for Landscape events.
- `GET /api/landscape/intelligence` returns IntelligenceDigestItem list.
- `POST /api/landscape/observations` submits observation (non-idempotent, no auto-retry).
- IntelligenceItemPublished events handled (notification).
- Frontend Landscape feature (digest card, observation form).
- Dashboard integration.
- Playwright test covers read + write paths.

---

## PROMPT-41: Legal ACL + routes + frontend feature module

**Depends on:** baseline, U34 (Commit — Legal's `LegalClauseUpdated` is mostly relevant to Commit proposal flow).

**Governing ADRs:** ADR-007 (pure read-only, conformist relationship).

**Governing DDD model:** `anti-corruption-layers.md` §9 Legal ACL — `ApprovedLegalSnippet` shape, outbound `RequestApprovedClausesQuery { context: proposal_id | topic }`, inbound `LegalClauseUpdated` (implement conservatively, surface only if tied to in-progress Commit proposal).

**Prompt:**

Implement the Legal ACL (read-only). (1) Create `LegalGateway` in `nexus-client/src/legal.rs` with `ApprovedLegalSnippet` DTO and `request_approved_clauses()` method (query takes either a proposal ID or a topic string). (2) Implement `GET /api/legal/clauses?proposal_id={id}` or `?topic={topic}` endpoint that returns approved clause text. (3) **Important**: inbound events `LegalClauseUpdated` — these are rare and mostly relevant to Commit's proposal flow. Only surface a notification if a proposal-in-progress references a now-stale clause (conservative implementation per the ADR's own caveat). (4) Create `frontend/src/features/legal/` component (if needed) or integrate legal clauses into the Commit proposal flow (display approved clauses when editing/reviewing proposals). (5) Test via Playwright (focus on the Commit integration).

**Acceptance criteria:**
- `LegalGateway` implemented (read-only).
- Wiremock fixtures for Legal events.
- `GET /api/legal/clauses` returns ApprovedLegalSnippet list (by proposal_id or topic).
- LegalClauseUpdated events handled conservatively (surface only if proposal-in-progress references stale clause).
- Integration with Commit proposal flow (clauses displayed/available when editing).
- Playwright test covers Legal flow (ideally as part of Commit tests).

---

### Group I — Phase 5: Design-System Extraction

## PROMPT-42: Design-system extraction (`@cognitum/design-system`, `@cognitum/dashboard-components`)

**Depends on:** U17 (original ported shell components) and U34–U41 (enough real component reuse/drift identified).

**Governing ADRs:** **None** — `implementation-plan.md` §3.4 lists a "Design-system extraction packaging strategy" ADR as required, but no such ADR exists. This unit is **blocked on that ADR being written first.**

**Governing DDD model:** `research.md` §"Dashboard Relationship" (long-term note: `@cognitum/design-system`, `@cognitum/dashboard-components`); `domain-map.md` §3 diagram's note on when the one-time Manage borrow stops being relevant.

**Prompt:**

**This unit cannot be started until an ADR resolving the design-system extraction packaging strategy exists.** At this stage, after all ten capability feature modules (U34–U41) have been built, real component reuse and drift patterns should be evident. The task is to: (1) identify shared component patterns across U17 (ported shell) and U34–U41 (capability features) — e.g., card layouts, forms, filter/search patterns, dialog structures that appear 3+ times, (2) extract them into one or more npm packages (`@cognitum/design-system` for foundational primitives like buttons/inputs, `@cognitum/dashboard-components` for domain-specific patterns like card grids, stat tiles), (3) consume the extracted package(s) from `frontend/` instead of local copies, (4) do the same for manage.cognitum.one if it adopts the same package (making the one-time source-code borrow "stop being relevant" as the plan notes). This unit is complete when both this repo and manage consume shared packages instead of duplication. **Acceptance criteria are deferred pending the packaging ADR.** This unit is included for completeness (goal state in the sequence) but is explicitly marked blocked and speculative.

---

## Out-of-Scope Reminders

The following must NEVER be implemented, regardless of unit or phase:

1. **Capital/Verdict services** — no `CapitalGateway` or `VerdictGateway` modules in `nexus-client`. These are consumed exclusively by `manage.cognitum.one` and never appear in this repo's Nexus routing table. Any future unit suggesting integration with capital.cognitum.one or verdict.cognitum.one signals a scope violation.

2. **Direct service calls** — this repo MUST NEVER call `sales.cognitum.one`, `commit.cognitum.one`, `edu.cognitum.one`, etc. directly. Nexus is the only integration point per `domain-map.md` §1 and `implementation-plan.md` §2.3. Every call crosses Nexus, every response crosses Nexus. No exceptions, no "direct optimizations."

3. **Business record duplication** — per `consultant-experience-context.md`'s opening invariant, this repo owns zero business records (leads, proposals, courses, engagements, etc.). Aggregates in `bff-core` store only view-state, configuration, or transient coordination state. If a unit finds itself storing a full copy of an external entity, it's out of scope.

4. **Permission re-adjudication** — per `anti-corruption-layers.md` §11 and the Sales worked example in §1, this repo never re-decides business policy. If Sales says a lead cannot be created, the BFF relays that verdict verbatim, it does not override it with its own logic. Similarly, `DashboardConfiguration` enforces that only permitted cards are stored (U21's invariant #1) but it does not re-evaluate Armor's permission assertions — those are opaque external decisions.

5. **Command auto-retry** — per ADR-016, non-idempotent commands (`CreateProposalCommand`, `RequestCollaborationCommand`, `SubmitReferralCommand`, `UpdateOwnProfileCommand`, `SubmitFieldObservationCommand`) are NEVER auto-retried by this repo. Retries are the responsibility of the consultant (conscious re-submission via the frontend). Auto-retry is reserved for read-only/idempotent queries only.

6. **Business logic in gateways** — every ACL gateway (sales, commit, edu, ...) is a pure translation boundary. No conditional business logic ("if this, then that"), no policy decisions, no domain invariants enforced at the gateway level. Gateways translate shapes and propagate verdicts; business rules live in `bff-core` aggregates or remain authoritatively in the owning capability (Nexus-routed).

---

**End of Implementation Prompts**
