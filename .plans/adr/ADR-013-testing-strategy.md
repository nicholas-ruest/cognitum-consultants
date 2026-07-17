# ADR-013: Testing Strategy

## Status
Proposed

## Context
`../implementation-plan.md` §3.4 requires a testing-strategy ADR covering "unit/integration/contract testing
across the Rust BFF and TS frontend" and explicitly "how ACL adapters get tested against Nexus contracts
before Nexus is fully mature" (risk #3: Nexus's contract maturity/protocol is unconfirmed). This repo's crate
boundaries (ADR-004) and ACL structure (ADR-007) already separate concerns in a way that maps cleanly onto
distinct test layers; this ADR fixes which layer tests what, and how testing survives Nexus contract
immaturity rather than being blocked by it.

## Decision
**Layered testing, one layer per crate/concern boundary, with an explicit staged approach to Nexus contract
testing that upgrades in place as real contracts mature.**

1. **Rust unit tests (`bff-core`, `nexus-client` gateway logic, `auth`, `persistence` trait implementations).**
   Standard `cargo test`, colocated with source per Rust convention. `bff-core`'s aggregate invariants
   (`../ddd/consultant-experience-context.md` §1.2/§2.2 — e.g. `DashboardConfiguration`'s unique-card-position
   invariant, `ActionQueueEntry`'s linear state machine) are unit-tested directly against `bff-core`, without
   spinning up Axum or a database, since `bff-core` depends only on trait interfaces (ADR-004) — this is the
   layer where invariant regressions are caught cheapest and fastest.
2. **`nexus-client` gateway contract tests, staged by Nexus maturity.** Before a real Nexus contract exists for
   a given capability, each gateway (`SalesGateway`, `CommitGateway`, ...) is tested against a hand-written
   mock server (`wiremock-rs`, or an equivalent Rust HTTP-mocking crate) that returns fixtures matching the
   documented shapes in `../ddd/anti-corruption-layers.md` (e.g. `AccountClaimResult` fixtures covering each
   `match_status` value from the worked example in §1). Once a capability's real Nexus contract is published
   (staged per `../implementation-plan.md` §5 Phase 4), the same gateway is additionally tested via
   consumer-driven contract tests against that published contract (e.g. schema validation against an
   OpenAPI/protobuf definition if Nexus publishes one, or a Pact-style consumer contract if Nexus adopts
   that) — the mock-fixture tests are not deleted, since they remain useful as fast, offline regression tests;
   the contract test is additive, catching drift between the mock fixtures and Nexus's real behavior.
3. **`bff-api` integration tests.** Axum's `tower::ServiceExt::oneshot`/`axum::body` test utilities exercise
   full HTTP routes (request in, response out) against a real (test-container-provisioned) Postgres instance
   (ADR-010) via `testcontainers`, with `nexus-client` swapped for the same mock-server fixtures used in layer
   2 — this validates the BFF's own routing/auth/permission-filtering (ADR-009) logic end-to-end without
   depending on a live Nexus.
4. **Frontend unit/component tests.** Vitest + React Testing Library (matching ADR-005's React choice) for
   component logic and rendering, particularly the permission-conditional rendering paths (ADR-009) and the
   `match_status`/`permitted_actions` conditional UI from the Sales lead-conflict worked example
   (`../ddd/anti-corruption-layers.md` §1).
5. **End-to-end tests.** Playwright, driving the full stack (frontend + BFF + mocked Nexus) through the
   Phase 2 reference flow (`../implementation-plan.md` §5: lead-conflict-warning) as the canonical smoke test,
   extended to each Phase 4 capability integration as it lands, following the "template pattern" the plan
   asks Phase 2 to document.
6. **CI gating.** Per `../implementation-plan.md` §5 Phase 0: `cargo check`, `cargo clippy -- -D warnings`,
   `cargo test` (layers 1–3) for Rust; `npm run build`, lint, `vitest run` (layer 4) for the frontend; all
   gating merges. Playwright e2e tests (layer 5) run in CI on a slower cadence (e.g. on merge to main, not
   every PR) given their relative cost, unless CI budget analysis later says otherwise.

## Consequences
**Positive**
- Nexus's unconfirmed maturity (risk #3) does not block any test layer — mock-fixture-based contract tests
  work today, and the strategy has a defined, additive upgrade path (not a rewrite) once real contracts land.
- Aggregate invariants are tested at the cheapest possible layer (`bff-core` unit tests), keeping the test
  suite fast despite the BFF's structurally many moving parts (ten gateways, five aggregates).
- The Sales lead-conflict flow serves as a concrete, reusable e2e template other capability integrations
  (Phase 4) can copy, directly following the plan's own "write up the pattern" instruction (§5 Phase 2).

**Negative / Trade-offs**
- Maintaining hand-written mock fixtures for ten gateways is real, ongoing effort, and those fixtures can
  silently drift from Nexus's actual (eventual) behavior until a real contract test is added per capability —
  mitigated, not eliminated, by adding contract tests as each capability's Nexus contract matures.
- `testcontainers`-based integration tests require Docker (or an equivalent) in CI, adding a dependency to the
  CI environment beyond a pure Rust/Node toolchain.

## Alternatives Considered
- **Wait for real Nexus contracts before writing any `nexus-client` tests.** Rejected — would block Phase 0–2
  work entirely on an external team's timeline, contradicting the plan's GOAP-style replanning philosophy
  (`../implementation-plan.md` §1); mock-fixture tests derived from the already-documented ACL shapes
  (`../ddd/anti-corruption-layers.md`) are a legitimate, if provisional, substitute.
- **Snapshot testing as the primary frontend testing approach.** Rejected as primary — snapshot tests are
  weak at catching permission-aware conditional-rendering regressions specifically (the highest-risk frontend
  behavior per ADR-009); explicit assertion-based component tests are preferred, with snapshots used sparingly
  for pure layout/markup stability where useful.
- **Full contract-testing framework (e.g. Pact) adopted repo-wide from day one, before Nexus's protocol is
  even confirmed (ADR-007).** Rejected as premature — committing to a specific contract-testing tool ahead of
  knowing Nexus's actual protocol (REST/JSON assumed per ADR-007, but not confirmed) risks choosing a tool
  that doesn't fit the eventual real contract shape; the mock-then-upgrade approach above defers that tooling
  choice to when it's actually needed per capability.

## Relationships
- Depends on: ADR-003 (Axum test utilities), ADR-004 (crate boundaries define test layers), ADR-005 (Vitest/
  RTL/Playwright fit React), ADR-007 (mock/contract test targets), ADR-010 (testcontainers Postgres).
- Informs: none directly, but its Phase 4 template obligation is referenced by every future capability-
  integration PR.
- Source docs: `../implementation-plan.md` §3.4, §5 Phase 2/4, §6 risk #3.
