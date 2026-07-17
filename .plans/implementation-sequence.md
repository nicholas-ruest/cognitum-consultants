# Implementation Sequence — consultants.cognitum.one

Status: intermediate working artifact. This is a dependency-ordered unit graph, not polished hand-off prompts —
a follow-on docs-writer agent turns this into ready-to-execute implementation prompts. Prioritize correctness
and completeness of the dependency graph over prose.

Method: GOAP-style planning.

- **Start state**: empty repo (only `.plans/` planning artifacts exist — research.md, implementation-plan.md,
  16 ADRs, 4 DDD docs).
- **Goal state**: all 16 ADRs' decisions implemented, consistent with the DDD model
  (`domain-map.md`, `consultant-experience-context.md`, `anti-corruption-layers.md`, `domain-events.md`), in
  the order `implementation-plan.md`'s phases imply (Phase 0 scaffolding → Phase 5 design-system extraction,
  itself explicitly deferred/speculative).
- **Units**: each is a discrete, roughly PR-sized implementation chunk. Every unit lists the prior unit(s) that
  must land first (its hard preconditions), the ADR(s) governing it, and the exact DDD doc/aggregate/ACL
  name(s) it must stay consistent with.
- **Ordering rule applied throughout**: a unit only appears after every unit whose output it consumes. Where
  the implementation plan's phase boundaries and a strict dependency reading would disagree (e.g. an aggregate
  whose invariant depends on infrastructure introduced in a "later phase" heading), the dependency wins and the
  unit is placed by dependency, with a note explaining the deviation from the phase's nominal position.

## Explicit cross-cutting decisions (read before the sequence)

**Observability (ADR-012) — one early foundational unit, then threaded.** ADR-012's correlation-ID
propagation is a hard precondition for ADR-007's `nexus-client` (every outbound Nexus call must already carry
a correlation ID and W3C Trace Context header per ADR-012's decision). So core tracing/correlation-ID
middleware and the `/metrics` skeleton are built once, early (U07), *before* `nexus-client` exists (U12).
After that, observability is **threaded, not re-built per unit**: every unit that adds a new BFF route or a
new `nexus-client` gateway is expected to extend the existing middleware's coverage (new route span, new
per-gateway metric labels) as part of its own "done" criteria, not as a separate sequenced unit. The one
exception is U32, a small dedicated unit, because SSE connection-affinity is a distinct architectural concern
(ADR-011 informing ADR-014) rather than routine metrics/tracing extension.

**Testing (ADR-013) — threaded through every unit, with two dedicated infrastructure units.** ADR-013 defines
five test layers (bff-core unit tests, nexus-client contract tests, bff-api integration tests, frontend
component tests, e2e) mapped onto this repo's existing crate/module boundaries. Rather than sequencing "write
tests for X" as a separate unit after every X, each functional unit's own "done" criteria includes the test
layer(s) ADR-013 assigns to the code it introduces (e.g. a new aggregate ships with its `bff-core` unit tests;
a new gateway ships with its wiremock contract-test fixtures). Two units are dedicated purely to test
*infrastructure* because later units depend on the tooling existing first: **U05** (installs
Vitest/RTL/wiremock-rs/testcontainers/Playwright and wires layers 1–4 into CI) and **U27** (stands up the
Playwright e2e harness itself, which can only be built once a first real flow — Sales — exists to drive it).

**Capital/Verdict exclusion carried forward.** Per `domain-map.md` §1 and `anti-corruption-layers.md`'s
explicit exclusion note, no unit in this sequence produces a `CapitalGateway` or `VerdictGateway` module, or
any code path calling `capital.cognitum.one` / `verdict.cognitum.one`. This is deliberate at every relevant
unit (U12, U14, U24, U34–U41), not an oversight.

**Design-system extraction (Phase 5) is last and separately flagged.** Per `implementation-plan.md` §5's own
framing, this is explicitly deferred/speculative. Unlike every other unit, **U42 has no governing ADR** —
`implementation-plan.md` §3.4 lists "Design-system extraction packaging strategy" as a required ADR topic, but
none of the 16 ADRs in `.plans/adr/` actually resolves it. U42 is included for completeness (the task's goal
state technically includes it) but is explicitly marked as blocked on an ADR that does not yet exist.

---

## Sequence

### Group A — Phase 0: Foundational Scaffolding

#### U01 — Cargo workspace + crate stubs
- **Depends on**: none (start state).
- **ADRs**: ADR-002 (Rust primary language), ADR-004 (workspace/repo layout).
- **DDD consistency**: none yet (pure scaffolding) — but crate names/boundaries must match ADR-004's layout
  exactly (`bff-api`, `bff-core`, `nexus-client`, `auth`, `persistence`, `config`) so later units mapping DDD
  aggregates/ACLs onto crates (e.g. `bff-core` → Consultant Workspace + Notification & Action Queue
  aggregates; `nexus-client` → the ten ACL gateway modules) have the right home from the start.
- **Done when**: `Cargo.toml` workspace manifest exists at repo root; `crates/{bff-api,bff-core,nexus-client,auth,persistence,config}` all exist as empty-but-compiling crates (`cargo check` passes across the workspace).

#### U02 — Axum `bff-api` health-check endpoint
- **Depends on**: U01.
- **ADRs**: ADR-003 (Axum), ADR-004 (`bff-api` owns HTTP transport only).
- **DDD consistency**: none (infra only).
- **Done when**: `bff-api` runs as an Axum service exposing a `GET /healthz` (or equivalent) endpoint returning 200; no business/aggregation logic present in `bff-api` yet (kept in line with ADR-004's "handlers thin, no aggregation logic in bff-api" rule from the start).

#### U03 — Frontend Vite + TypeScript + Tailwind + React scaffold
- **Depends on**: U01 (parallel-safe with U02; both only need the repo root to exist).
- **ADRs**: ADR-002 (TypeScript as secondary language), ADR-005 (Vite + Tailwind + React).
- **DDD consistency**: `frontend/src/features/<capability>/` directory convention stubbed per
  `implementation-plan.md` §4, one empty directory per capability named to match `domain-map.md` §1's context
  list (sales, commit, edu, capacity, customer, execution, products, landscape, legal) — establishes the
  structural mapping later units fill in.
- **Done when**: `npm run build` and `npm run dev` both succeed against a placeholder React root component; Tailwind classes render correctly in a smoke page.

#### U04 — CI pipeline (build/lint/test gates)
- **Depends on**: U01, U02, U03.
- **ADRs**: ADR-013 (CI gating layer 6), ADR-002 (two-toolchain CI surface).
- **DDD consistency**: none.
- **Done when**: CI runs `cargo check`, `cargo clippy -- -D warnings`, `cargo test` and `npm run build`, frontend lint on every PR and blocks merge on failure.

#### U05 — Testing tooling scaffolding
- **Depends on**: U04.
- **ADRs**: ADR-013 (layers 1–4 tooling: Rust unit tests are just `cargo test`, already covered by U04; this unit adds `wiremock-rs` as a `nexus-client` dev-dependency, `testcontainers` as a `bff-api` dev-dependency requiring Docker in CI, Vitest + React Testing Library for the frontend, and installs — but does not yet use — Playwright).
- **DDD consistency**: none directly, but the wiremock fixture convention this unit establishes must be shaped
  so later fixtures can express `anti-corruption-layers.md`'s per-gateway inbound shapes (e.g. `AccountClaimResult` fixtures covering each `match_status` value).
- **Done when**: CI has Docker available for `testcontainers`; `wiremock-rs`, `testcontainers`, Vitest, React Testing Library, and Playwright are installed and a trivial smoke test exists in each to prove the harness runs in CI.

#### U06 — Dev orchestration scripts
- **Depends on**: U01, U02, U03.
- **ADRs**: none directly (implementation-plan.md §5 Phase 0 deliverable; ADR-004's workspace/frontend split is what makes this necessary).
- **DDD consistency**: none.
- **Done when**: a single `scripts/` command boots `bff-api` and the Vite dev server together for local development.

#### U07 — Observability foundation
- **Depends on**: U02 (needs an Axum app to attach middleware to).
- **ADRs**: ADR-012 (tracing, correlation ID middleware, OpenTelemetry via `tracing-opentelemetry`, `metrics` crate + Prometheus exporter), ADR-003 (tower middleware model).
- **DDD consistency**: none (infra), but this is the mechanism that will later let a BFF log line, a Nexus log line, and a target-capability log line be joined during incident diagnosis across every ACL gateway added from U14 onward.
- **Done when**: every inbound request gets/generates a correlation ID stored as a tracing span field and returned/propagatable on outbound calls (mechanism exists even though no outbound caller exists yet); `/metrics` endpoint exists with baseline per-route request count/latency; structured JSON logging in non-local envs, human-readable locally.

---

### Group B — Core Infra Plumbing

#### U08 — Config crate
- **Depends on**: U01.
- **ADRs**: ADR-004 (`config` crate responsibility), ADR-014 (12-factor env-var config, anticipated here).
- **DDD consistency**: none.
- **Done when**: `config` crate loads typed configuration from environment variables with per-environment (dev/staging/prod) override support; consumed by at least a placeholder in `bff-api` startup.

#### U09 — Postgres persistence crate + migrations tooling
- **Depends on**: U08 (needs DB connection config), U04 (CI must be able to run `sqlx` compile-time checks / offline query cache).
- **ADRs**: ADR-010 (Postgres via `sqlx`, migrations via `sqlx-cli`, multi-instance correctness rationale).
- **DDD consistency**: `consultant-experience-context.md` §1.4/§2.4 repository interfaces are the target shape this crate's trait implementations must eventually satisfy — this unit only stands up the datastore connection, migration runner, and empty `persistence/migrations/` directory; no concrete aggregate tables yet (those land with each aggregate's own unit, U20/U21/U22/U29).
- **Done when**: `persistence` crate connects to a Postgres instance (via `testcontainers` in CI per U05, a real instance in deployed envs), migrations run automatically in dev and via explicit CI/CD step elsewhere, and `sqlx`'s compile-time query checking is wired into CI (offline mode).

#### U10 — Auth crate: session model + dev-stub login provider
- **Depends on**: U09 (session storage reuses this datastore per ADR-008), U08 (config for feature-flagging the dev stub).
- **ADRs**: ADR-008 (BFF-managed server-side session; interim dev-stub gated to never boot outside `dev` env).
- **DDD consistency**: none directly (auth is infrastructure, deliberately separated from `bff-core` domain logic per ADR-004's rationale).
- **Done when**: `auth` crate defines a session interface (opaque session id → identity/claims), a dev-stub provider implementing it against fixed dev consultant identities, session rows persisted via U09, and a startup check that refuses to boot the stub provider outside a `dev`-flagged environment.

#### U11 — BFF session middleware + `/api/session` endpoint
- **Depends on**: U10, U02.
- **ADRs**: ADR-008 (`HttpOnly`/`Secure`/`SameSite=Strict` cookie, session lookup middleware), ADR-006 (`/api/*` namespace).
- **DDD consistency**: `consultant-experience-context.md`'s "Consultant" ubiquitous-language term (`consultant_id` reference, no locally-held identity data beyond presentation projections).
- **Done when**: successful dev-stub login sets a session cookie; `GET /api/session` returns the authenticated consultant's identity (no permission data yet — that's U14/U15); unauthenticated requests to protected routes are rejected.

#### U12 — `nexus-client` foundation: `NexusTransport` trait + correlation-ID propagation
- **Depends on**: U01, U07 (correlation ID must exist to be propagated), U05 (wiremock available for this unit's own tests).
- **ADRs**: ADR-007 (REST/JSON, `NexusTransport` trait built on `reqwest`, no gateway modules yet), ADR-003 (tower middleware reused on outbound calls), ADR-012 (correlation ID + W3C Trace Context attached to every outbound call).
- **DDD consistency**: `anti-corruption-layers.md`'s framing that Nexus is the *only* integration point — this unit is the literal enforcement point: no code outside this crate may construct an HTTP client aimed at any `*.cognitum.one` sub-business service.
- **Done when**: `nexus-client` crate exposes a `NexusTransport` trait + `reqwest`-backed implementation that attaches correlation ID/trace-context headers on every call; zero capability-specific gateway modules exist yet (those start at U14).

#### U13 — Resilience middleware on `NexusTransport`
- **Depends on**: U12.
- **ADRs**: ADR-016 (per-gateway timeout budgets via `tower::timeout`, bounded retries with backoff for idempotent reads only, concurrent fan-out with per-call isolation, circuit breaker skeleton).
- **DDD consistency**: none directly — this is transport-layer infrastructure applied uniformly ahead of any specific ACL, consistent with `anti-corruption-layers.md` §11's "no gateway contains business logic" rule (retry/timeout policy lives here, not in any gateway).
- **Done when**: every `NexusTransport` call can be wrapped with a configurable timeout and a circuit-breaker layer; a retry-with-backoff wrapper exists for read-only/idempotent call sites and is *not* applied by default to write/command call sites (enforced by API shape, e.g. two distinct wrapper functions/traits for queries vs. commands). No real per-capability tuning values yet (deferred to ADR-012 metrics data, noted as future work).

#### U14 — Armor ACL gateway (`ArmorGateway`)
- **Depends on**: U12, U13, U11 (session identity needed to request assertions on the consultant's behalf).
- **ADRs**: ADR-007 (gateway module structure/typed trait pattern), ADR-008 (identity propagation this gateway rides on), ADR-009 (this is the sole source of Permission Assertions driving all downstream permission-aware presentation).
- **DDD consistency**: `anti-corruption-layers.md` §10 (Armor ACL — `PermissionAssertion { consultant_id, capability, scope, expires_at }`, no outbound business command, inbound `PermissionAssertionChanged`); `domain-events.md` §1 (`PermissionAssertionChanged` consumed by Consultant Workspace).
- **Done when**: `ArmorGateway` trait + implementation can fetch current Permission Assertions for a consultant via Nexus and expose a hook for consuming `PermissionAssertionChanged`; no other gateway module exists yet.

#### U15 — Permission-aware presentation enforcement (BFF)
- **Depends on**: U14, U11.
- **ADRs**: ADR-009 (three-layer enforcement: server-side filtering/caching with TTL bounded by `expires_at`, short-circuit 403 before a wasted Nexus round-trip; client-side and downstream-recheck layers land later in U18/U19 and per-capability units respectively).
- **DDD consistency**: `consultant-experience-context.md` §1.2 `DashboardConfiguration` invariant #1 (a card the consultant has no Permission Assertion for cannot be persisted) — this unit builds the *general* filtering/caching primitive that invariant will be enforced against once `DashboardConfiguration` exists (U21); it does not yet have an aggregate to protect.
- **Done when**: BFF caches a consultant's Permission Assertions server-side (in-memory per session, refreshed on expiry or `PermissionAssertionChanged`), exposes an internal "is capability X permitted" check usable by any future route/handler, and short-circuits with 403 rather than attempting a Nexus call for an unpermitted capability.

---

### Group C — Phase 1: Shell & Navigation

#### U16 — Frontend data-fetching foundation (TanStack Query)
- **Depends on**: U03.
- **ADRs**: ADR-015 (TanStack Query as the sole server-state library; query-key convention namespaced by capability + consultant).
- **DDD consistency**: query-key namespace must mirror `frontend/src/features/<capability>/` (U03), which itself mirrors `domain-map.md`'s context list.
- **Done when**: `QueryClientProvider` wraps the app root; the query-key convention is documented/enforced (e.g. a typed key-builder helper) so every later feature module follows it rather than inventing its own.

#### U17 — Frontend shell components (ported from manage)
- **Depends on**: U03.
- **ADRs**: ADR-005 (React), ADR-006 (SPA has no server-rendering responsibility — these are plain client components).
- **DDD consistency**: `research.md` §"Dashboard Relationship" (layout, sidebar, header, cards, tables, forms, search, filters, alerts, dialogs — copied/adapted from manage.cognitum.one, Manage-specific business logic stripped); `domain-map.md` §3 diagram's explicit note that this is a "ONE-TIME source-code borrow... not a runtime dependency."
- **Done when**: `frontend/src/components/` contains the ported shell primitives, rendering with placeholder/empty data, with zero Manage-specific business logic or API calls remaining.
- **Note**: per `implementation-plan.md` §6 risk #1/#4, this unit is contingent on manage.cognitum.one's actual dashboard framework being confirmed as React (ADR-005's stated contingency clause) — if not, this unit's approach (port vs. rebuild) must be revisited before starting.

#### U18 — Frontend login flow + session wiring
- **Depends on**: U11, U17, U16.
- **ADRs**: ADR-008 (browser never holds a long-lived credential; relies on the `HttpOnly` session cookie), ADR-006 (calls `/api/session`).
- **DDD consistency**: none beyond the "Consultant" ubiquitous-language term already established in U11.
- **Done when**: a consultant can complete the dev-stub login flow end-to-end (frontend → BFF → session cookie set) and the shell renders as authenticated, calling `/api/session` via TanStack Query.

#### U19 — Permission-aware nav rendering
- **Depends on**: U15, U18, U17.
- **ADRs**: ADR-009 (client-side rendering layer — UX only, never trusted as enforcement; nav items conditionally rendered from Permission Assertions returned via `/api/session`).
- **DDD consistency**: `anti-corruption-layers.md` §10 (`PermissionAssertion` shape consumed here); this is the first place ADR-009's three-layer model becomes fully observable (server filter from U15 + this client render).
- **Done when**: nav items are shown/hidden per the consultant's Permission Assertions; the frontend contains no logic treating "not rendered" as authoritative (a direct URL to a hidden route still round-trips through the BFF's own check).

---

### Group D — Workspace Aggregates (`bff-core`)

#### U20 — `ConsultantPreferences` aggregate + Postgres repository
- **Depends on**: U09, U11 (needs `consultant_id`).
- **ADRs**: ADR-010 (Postgres/`sqlx` repository implementation), ADR-004 (`bff-core` owns the aggregate; `persistence` implements its repository trait).
- **DDD consistency**: `consultant-experience-context.md` §1.2 `ConsultantPreferences` aggregate — invariants: known/versioned key allow-list only, exactly one per consultant, values never encode business data (references only).
- **Done when**: `bff-core` defines `ConsultantPreferences` with its invariants enforced in code (not just documented), `persistence` implements `ConsultantPreferencesRepository` (`find_by_consultant_id`, `save`, `upsert_preference`) against Postgres, unit-tested per ADR-013 layer 1 without spinning up Axum or a real database.

#### U21 — `DashboardConfiguration` aggregate + Postgres repository
- **Depends on**: U15 (invariant #1 needs Permission Assertions to validate against), U20 (shares `bff-core`/`persistence` patterns just established), U09.
- **ADRs**: ADR-009 (invariant #1 enforcement — a card the consultant has no Permission Assertion for cannot be persisted), ADR-010 (Postgres repository).
- **DDD consistency**: `consultant-experience-context.md` §1.2 `DashboardConfiguration` aggregate — all four invariants (permission-fit cards, unique card positions, exactly one config per consultant, default card set on first creation).
- **Done when**: `DashboardConfiguration` and its `CardPlacement` child entities exist in `bff-core` with all four invariants enforced and unit-tested; `DashboardConfigurationRepository` implemented against Postgres; attempting to persist a card without a matching Permission Assertion is rejected at the aggregate boundary, not just filtered in the UI.

#### U22 — `CrossCapabilityWorkflowSession` aggregate + repository
- **Depends on**: U09, U20/U21 (established `bff-core`/`persistence` aggregate pattern).
- **ADRs**: ADR-010 (Postgres repository, including the `expire_older_than` housekeeping sweep).
- **DDD consistency**: `consultant-experience-context.md` §1.2 `CrossCapabilityWorkflowSession` aggregate — invariants: opaque-reference-only origin/target, bounded TTL with no resume past `expires_at`, linear state machine (`started → in_progress → {completed|abandoned|expired}`), completion never itself mutates the target capability's data.
- **Done when**: the aggregate and its state machine are implemented and unit-tested in `bff-core`; `CrossCapabilityWorkflowSessionRepository` (including `expire_older_than`) is implemented against Postgres. Not yet wired to any real capability handoff — that begins at U34 (Sales→Commit), the first real cross-capability flow needing it. Placed here (with its sibling Workspace aggregates) rather than deferred to just before U34, since its only real preconditions (persistence, aggregate pattern) are already satisfied.

#### U23 — BFF dashboard composition endpoints + frontend dashboard shell rendering
- **Depends on**: U21, U19, U16.
- **ADRs**: ADR-006 (`/api/*` shape), ADR-009 (dashboard endpoint enforces permission filtering), ADR-010 (persisted via U21's repository).
- **DDD consistency**: `consultant-experience-context.md` §1.2 `DashboardConfiguration` — this unit exposes `GET`/`PUT` over that aggregate and applies its default-card-set-on-creation invariant.
- **Done when**: a consultant sees their (initially default/empty) dashboard shell render real persisted layout state from `DashboardConfiguration`; no capability card yet carries live Nexus-backed data (that is Phase 2, U24–U26) — cards render as placeholders bound to a `module_id` with no data source resolved yet.

---

### Group E — Phase 2: Sales ACL Reference Flow

This is the pattern-proving flow per `implementation-plan.md` §5 Phase 2 and `research.md` §"Lead Conflict
Warning" — built first among the ten ACLs, before any other capability integration.

#### U24 — Sales ACL gateway (`SalesGateway`)
- **Depends on**: U12, U13, U05 (wiremock fixtures).
- **ADRs**: ADR-007 (gateway trait/module pattern), ADR-016 (this is the synchronous, user-blocking call type ADR-016 calls out for a tighter timeout budget than read-mostly gateways).
- **DDD consistency**: `anti-corruption-layers.md` §1 Sales ACL — `AccountClaimResult { match_status, creation_allowed, display_message, permitted_actions }`; outbound `CheckAccountClaimCommand`, `RequestCollaborationCommand`, `SubmitReferralCommand`; inbound `AccountClaimDetermined`, `CollaborationRequestAcknowledged`, `ReferralSubmitted`. `domain-events.md` §3 Sales table (exact event names/payloads).
- **Done when**: `SalesGateway` trait + implementation exists with wiremock fixtures covering every `match_status` value from the worked example; zero business-policy logic in the gateway (verdict is relayed, not computed).

#### U25 — BFF lead-conflict-check endpoint
- **Depends on**: U24, U15 (permission short-circuit before attempting the call), U13 (timeout budget applied).
- **ADRs**: ADR-016 (per-card/per-response partial-success shape, though this is a single-verdict endpoint not a multi-card fan-out — still uses the same "card failed, retryable" response contract for consistency), ADR-009 (permission gate).
- **DDD consistency**: `anti-corruption-layers.md` §1 worked example step 5 — the BFF **relays `AccountClaimDetermined` verbatim**; no invariant in this repo re-derives or overrides `creation_allowed`.
- **Done when**: `POST /api/sales/lead-conflict-check` normalizes consultant input, calls `SalesGateway`, and returns the Sales verdict unmodified; a follow-up `RequestCollaborationCommand`/`SubmitReferralCommand` handler exists for the permitted-actions the frontend can trigger.

#### U26 — Frontend Sales lead-conflict feature module
- **Depends on**: U25, U16, U17.
- **ADRs**: ADR-015 (TanStack Query mutation for the check call; result is rendered directly from the mutation response, never cached/reused across a different company entry — an explicit ADR-015 rule), ADR-005 (React feature module under `frontend/src/features/sales/`).
- **DDD consistency**: `anti-corruption-layers.md` §1 — company-entry form + conditional rendering of `display_message` and only the buttons listed in `permitted_actions`; frontend never independently decides `creation_allowed`.
- **Done when**: a consultant can enter a company name, see the Sales-determined conflict verdict rendered, and trigger only the permitted actions; this becomes at least one live-data card in the U23 dashboard shell.

#### U27 — Sales flow e2e (Playwright) + pattern write-up doc
- **Depends on**: U25, U26, U05 (Playwright installed).
- **ADRs**: ADR-013 (layer 5: Playwright e2e, canonical smoke test).
- **DDD consistency**: this flow becomes the literal template Phase 4 (U34–U41) is instructed to replicate, per `implementation-plan.md` §5 Phase 2's "write up the pattern" deliverable and `anti-corruption-layers.md` §1's closing line: "This flow demonstrates the general shape every other ACL below follows."
- **Done when**: a Playwright test drives the full stack (frontend + BFF + mocked Nexus) through the lead-conflict-warning flow end-to-end and passes in CI (slower cadence, e.g. merge-to-main); a short `docs/` write-up captures the pattern (DTO shape → gateway → BFF handler → frontend feature module → tests) for reuse.

---

### Group F — Deployment Validation

#### U28 — Containerize BFF + SPA, deploy pipeline, health/graceful shutdown
- **Depends on**: U09 (Postgres as a provisioned dependency), U26/U27 (something real — the Sales flow — worth deploying and validating end-to-end), U07 (metrics/tracing must be reachable from wherever this deploys), U04 (CI gates precede deploy).
- **ADRs**: ADR-014 (multi-stage Dockerfile with `cargo-chef`, single image serves API + SPA per ADR-006 default, orchestrator-agnostic env-var config, health-check endpoints extending U02's, graceful shutdown draining in-flight requests on `SIGTERM`, migrations as an explicit pipeline step).
- **DDD consistency**: none directly — validates the ADR-006 "served API + SPA" model actually runs as one deployable unit, with real data (Sales flow) flowing through it.
- **Done when**: a single container image builds (via CI), serves both `/api/*` and the SPA, passes liveness/readiness checks, drains gracefully on `SIGTERM`, and a deploy pipeline step runs migrations before rollout. Placed here — after the first real capability (Sales) rather than immediately after Phase 0/1 — deliberately: early enough to validate the architecture end-to-end, late enough to deploy something with real data flowing through it, per the task's framing.

---

### Group G — Phase 3: Notifications & Action Queue

Placed after the Sales ACL (Group E) rather than before it: this is the first real event-producing flow to
carry, and `implementation-plan.md` §6 risk #5's "where does notification state live" question is easier to
resolve concretely with real capability events (Sales' `CollaborationRequestAcknowledged`, `ReferralSubmitted`)
to model against, rather than in the abstract.

#### U29 — `NotificationItem` + `ActionQueueEntry` aggregates + Postgres repositories
- **Depends on**: U09, U24/U25 (first real capability whose events these aggregates will actually ingest — Sales).
- **ADRs**: ADR-010 (Postgres unique constraint on `(origin_capability, origin_event_id)` for idempotent ingestion).
- **DDD consistency**: `consultant-experience-context.md` §2.2 — `NotificationItem` invariants (idempotent ingestion, display-safe-summary-only payload, one-way `unread → read`, one consultant per row) and `ActionQueueEntry` invariants (idempotent ingestion, linear state machine `pending → in_progress → {completed|expired}`, `completed` settable **only** by a confirmation event routed back through Nexus, never by a bare consultant click).
- **Done when**: both aggregates exist in `bff-core` with invariants enforced and unit-tested; `NotificationRepository`/`ActionQueueRepository` (§2.4's exact method lists, including `purge_older_than`/`expire_older_than` housekeeping sweeps) implemented against Postgres.

#### U30 — Nexus event ingestion (polling) → Notification/ActionQueue mapping
- **Depends on**: U29, U12 (transport for polling Nexus), U24/U25 (Sales as the first real event source).
- **ADRs**: ADR-011 (Nexus→BFF ingestion via polling initially, feeding an internal event bus; upgradeable to webhook later without touching downstream logic).
- **DDD consistency**: `domain-events.md` §2 `CapabilityEventReceived` envelope, mapped by this unit into either a `NotificationItem` or `ActionQueueEntry` depending on whether the event implies a required consultant action; initial real payloads come from Sales's `CollaborationRequestAcknowledged`/`ReferralSubmitted` (§3 Sales table).
- **Done when**: the BFF polls Nexus for `CapabilityEventReceived` envelopes, classifies each into the correct aggregate type, and idempotent ingestion (via U29's unique constraint) is verified under redelivery.

#### U31 — SSE endpoint + internal event bus
- **Depends on**: U30, U07 (Axum SSE support, tower middleware), U02.
- **ADRs**: ADR-011 (SSE for BFF→browser push; `axum::response::sse`; unidirectional; consultant actions remain ordinary `POST`/`PATCH` calls, not sent over the push channel), ADR-006 (`/api/*` namespace for the stream endpoint).
- **DDD consistency**: `consultant-experience-context.md` §2 — this is the delivery mechanism for `NotificationItem`/`ActionQueueEntry` changes reaching the browser.
- **Done when**: `/api/notifications/stream` pushes notification/action-queue changes to a connected browser via SSE, sourced from U30's ingestion; a documented polling fallback endpoint exists for constrained network environments (nice-to-have, not blocking).

#### U32 — SSE connection-affinity / horizontal-scaling fan-out
- **Depends on**: U31, U28 (deployment topology this must integrate with).
- **ADRs**: ADR-014 (sticky/session-affinity routing or a cross-instance fan-out mechanism, e.g. Postgres `LISTEN`/`NOTIFY`, so an event ingested by one BFF instance reaches a consultant whose SSE connection is held by another), ADR-011 (this is the concrete mechanism ADR-011 flagged as "to finalize when ADR-014's scaling approach is implemented").
- **DDD consistency**: none directly — pure infrastructure closing the loop between ADR-011 and ADR-014.
- **Done when**: a notification ingested on BFF instance A reliably reaches a browser whose SSE connection is held by instance B, verified under a multi-instance test/deployment.

#### U33 — Frontend notification centre + action queue UI
- **Depends on**: U31, U16, U17.
- **ADRs**: ADR-011 (SSE consumption), ADR-015 (SSE event handler calls `queryClient.invalidateQueries` for affected query keys — the concrete wiring connecting the push channel to actual UI re-renders).
- **DDD consistency**: `consultant-experience-context.md` §2.1 glossary (Notification Item, Action Queue Entry, Read State, Action State, Deep Link Reference) — UI must reflect the same one-way read-state transition and confirmed-only completion semantics as the backend aggregates.
- **Done when**: a consultant sees a live-updating notification centre and action queue, driven by SSE-triggered cache invalidation, with correct deep-link navigation back to the originating capability (Sales, for now).

---

### Group H — Phase 4: Remaining ACL Integrations (staged)

Staging order follows `implementation-plan.md` §5 Phase 4's suggested sequence exactly. Every unit in this
group shares the same infrastructure preconditions, listed once here rather than repeated ten times:
**U12/U13 (nexus-client + resilience), U15 (permission-aware presentation), U16/U17 (frontend data-fetching +
shell), U23 (dashboard composition, to host a new card), U29/U30/U31 (notification/action-queue system, since
every capability's inbound events feed it)**. Each unit below lists only its *additional*, capability-specific
dependencies beyond this common baseline.

#### U34 — Commit ACL + routes + frontend feature module (+ Sales→Commit deep link)
- **Depends on**: common baseline, U22 (`CrossCapabilityWorkflowSession` — this is the first real cross-capability handoff, lead→proposal), U26 (Sales feature module, as the deep-link origin).
- **ADRs**: ADR-007 (gateway pattern), ADR-016 (idempotent-read retries only; `CreateProposalCommand` is a non-idempotent command, never auto-retried).
- **DDD consistency**: `anti-corruption-layers.md` §2 Commit ACL — `ProposalSummary` shape, outbound `CreateProposalCommand`/`RequestProposalActionCommand`, inbound `ProposalCreated`/`ProposalStatusChanged`/`ProposalAccepted` feeding `NotificationItem`/`ActionQueueEntry`; `domain-events.md` §3 Commit table.
- **Done when**: a consultant can deep-link from a Sales lead-conflict resolution into starting a Commit proposal (via a `CrossCapabilityWorkflowSession`), see proposal status surfaced as a dashboard card and notifications, following the exact template U27 documented.

#### U35 — Edu ACL + routes + frontend feature module
- **Depends on**: common baseline.
- **ADRs**: ADR-007, ADR-016 (read-mostly gateway, longer timeout allowance per ADR-016's read-vs-write budget distinction).
- **DDD consistency**: `anti-corruption-layers.md` §3 Edu ACL — `LearningSnapshot` shape, outbound `RequestLearningCatalogQuery`, inbound `CourseCompleted`/`CertificationIssued`/`TrainingRequirementDue`.
- **Done when**: education/learning snapshot renders as a dashboard card/feature module; completion/certification/due-training events feed notifications.

#### U36 — Capacity ACL (restricted) + routes + frontend feature module
- **Depends on**: common baseline.
- **ADRs**: ADR-007 (this gateway is deliberately narrow — no query shape for cross-consultant data at all, a structural omission, not a filtering afterthought).
- **DDD consistency**: `anti-corruption-layers.md` §4 Capacity ACL — `ConsultantProfileIntake` shape (write-heavy, read-narrow: own profile only), `domain-map.md`'s explicit "Consultants must not receive internal Capacity access" relationship framing.
- **Done when**: a consultant can view/update only their own restricted profile fields (skills, certifications, languages, availability, geographic coverage); no code path in this unit can query another consultant's data — verify by code review against the ACL's structural restriction, not just a runtime permission check.

#### U37 — Customer ACL + routes + frontend feature module
- **Depends on**: common baseline.
- **ADRs**: ADR-007, ADR-009 (query itself scoped to "assigned or permitted" — permission-filtered at the query boundary, not filtered client-side after a broader fetch).
- **DDD consistency**: `anti-corruption-layers.md` §5 Customer ACL — `CustomerContextCard` shape, outbound `RequestAssignedCustomerContextQuery`, inbound `CustomerHealthChanged`/`CustomerInteractionLogged`.
- **Done when**: assigned/permitted customer context renders as a dashboard card; health/interaction events feed notifications.

#### U38 — Execution ACL + routes + frontend feature module
- **Depends on**: common baseline.
- **ADRs**: ADR-007, ADR-016 (`TaskAssigned`/`DeliveryRiskRaised` are natural `ActionQueueEntry` sources, not just notifications — must route through the `ActionQueueEntry` aggregate's confirmed-completion invariant from U29, not a bare local state flip).
- **DDD consistency**: `anti-corruption-layers.md` §6 Execution ACL — `EngagementSnapshot` shape, outbound `RequestAssignedEngagementsQuery`, inbound `MilestoneCompleted`/`DeliveryRiskRaised`/`TaskAssigned`.
- **Done when**: the consultant's assigned delivery workspace (engagements/workstreams/milestones/tasks) renders as a dashboard card; task assignments and delivery risks surface in the action queue with correctly-gated completion.

#### U39 — Products ACL + routes + frontend feature module
- **Depends on**: common baseline.
- **ADRs**: ADR-007, ADR-016 (read-only, most cacheable/least latency-sensitive gateway per ADR-016's tuning note — longest default timeout, most aggressive retry-on-transient-failure).
- **DDD consistency**: `anti-corruption-layers.md` §7 Products ACL — `ProductReferenceCard` shape, outbound `RequestProductCatalogQuery`, inbound `ProductCatalogUpdated` (low priority, unlikely to warrant an `ActionQueueEntry` — should surface as a low-severity notification/refresh at most).
- **Done when**: approved product/service reference data renders as a dashboard card usable during proposal/selling conversations.

#### U40 — Landscape ACL + routes + frontend feature module (read + write)
- **Depends on**: common baseline.
- **ADRs**: ADR-007 (this is the one gateway with a real outbound write path from this repo, beyond commands that mirror UI actions — `SubmitFieldObservationCommand` — still non-idempotent, still never auto-retried per ADR-016).
- **DDD consistency**: `anti-corruption-layers.md` §8 Landscape ACL — `IntelligenceDigestItem` (inbound) / `FieldObservationSubmission` (outbound); this repo is a minor upstream contributor here but Landscape still governs what counts as "approved" — no local "publish" concept.
- **Done when**: consultants can read approved intelligence digest items and submit field observations; Landscape retains sole authority over what becomes "approved" content.

#### U41 — Legal ACL + routes + frontend feature module
- **Depends on**: common baseline, U34 (Commit — Legal's inbound `LegalClauseUpdated` path is documented as "mostly relevant to Commit's proposal flow," surfaced only if a proposal-in-progress references a now-stale clause).
- **ADRs**: ADR-007 (pure read-only conformist relationship).
- **DDD consistency**: `anti-corruption-layers.md` §9 Legal ACL — `ApprovedLegalSnippet` shape, outbound `RequestApprovedClausesQuery { context: proposal_id | topic }`, inbound `LegalClauseUpdated` (flagged as an assumption in the ACL doc itself — implement conservatively, surfacing only as a notification tied to an in-progress Commit proposal, per the doc's own caveat).
- **Done when**: approved legal clause text renders read-only wherever a proposal flow (Commit, U34) needs it; a stale-clause update on an in-progress proposal surfaces as a notification.

---

### Group I — Phase 5: Design-System Extraction (deferred/speculative — last)

#### U42 — Design-system extraction (`@cognitum/design-system`, `@cognitum/dashboard-components`)
- **Depends on**: U17 (the original ported shell components) and U34–U41 (enough real component reuse/drift across all ten capability feature modules to justify extraction, per `implementation-plan.md` §5's own precondition: "enough real shared-component surface identified... to justify extraction").
- **ADRs**: **none** — `implementation-plan.md` §3.4 lists a "Design-system extraction packaging strategy" ADR as required, but no such ADR exists among the 16 in `.plans/adr/`. This unit is blocked on that ADR being written first; it is sequenced last both because the plan explicitly defers/speculates on it and because a real packaging-strategy ADR cannot itself be usefully written until this sequence's other 41 units reveal what's actually worth extracting.
- **DDD consistency**: `research.md` §"Dashboard Relationship" long-term note (`@cognitum/design-system`, `@cognitum/dashboard-components`); `domain-map.md` §3 diagram's note that this is when the one-time Manage source-code borrow "stops being relevant... since both apps [would] have a shared package instead."
- **Done when**: N/A as a hard target — this unit's real "done" is "an ADR resolving packaging strategy exists, and shared primitives are extracted and consumed by this app instead of local copies." Not to be started before that ADR lands.

---

## Summary table

| # | Unit | Depends on |
|---|---|---|
| U01 | Cargo workspace + crate stubs | — |
| U02 | Axum health endpoint | U01 |
| U03 | Frontend scaffold | U01 |
| U04 | CI pipeline | U01,U02,U03 |
| U05 | Testing tooling scaffolding | U04 |
| U06 | Dev orchestration scripts | U01,U02,U03 |
| U07 | Observability foundation | U02 |
| U08 | Config crate | U01 |
| U09 | Persistence crate + migrations | U08,U04 |
| U10 | Auth crate + dev-stub | U09,U08 |
| U11 | Session middleware + /api/session | U10,U02 |
| U12 | nexus-client foundation | U01,U07,U05 |
| U13 | Resilience middleware | U12 |
| U14 | Armor ACL gateway | U12,U13,U11 |
| U15 | Permission-aware presentation enforcement | U14,U11 |
| U16 | Frontend data-fetching (TanStack Query) | U03 |
| U17 | Frontend shell components | U03 |
| U18 | Frontend login flow | U11,U17,U16 |
| U19 | Permission-aware nav rendering | U15,U18,U17 |
| U20 | ConsultantPreferences aggregate | U09,U11 |
| U21 | DashboardConfiguration aggregate | U15,U20,U09 |
| U22 | CrossCapabilityWorkflowSession aggregate | U09,U20/U21 |
| U23 | Dashboard composition endpoints + shell | U21,U19,U16 |
| U24 | Sales ACL gateway | U12,U13,U05 |
| U25 | Lead-conflict-check endpoint | U24,U15,U13 |
| U26 | Sales frontend feature module | U25,U16,U17 |
| U27 | Sales e2e + pattern doc | U25,U26,U05 |
| U28 | Containerize + deploy pipeline | U09,U26/U27,U07,U04 |
| U29 | Notification/ActionQueue aggregates | U09,U24/U25 |
| U30 | Nexus event ingestion | U29,U12,U24/U25 |
| U31 | SSE endpoint + event bus | U30,U07,U02 |
| U32 | SSE connection-affinity/scaling | U31,U28 |
| U33 | Frontend notification/action queue UI | U31,U16,U17 |
| U34 | Commit ACL + feature module | common baseline,U22,U26 |
| U35 | Edu ACL + feature module | common baseline |
| U36 | Capacity ACL + feature module | common baseline |
| U37 | Customer ACL + feature module | common baseline |
| U38 | Execution ACL + feature module | common baseline |
| U39 | Products ACL + feature module | common baseline |
| U40 | Landscape ACL + feature module | common baseline |
| U41 | Legal ACL + feature module | common baseline,U34 |
| U42 | Design-system extraction | U17,U34-U41 (+ missing ADR) |

"common baseline" (U34–U41) = U12, U13, U15, U16, U17, U23, U29, U30, U31.
