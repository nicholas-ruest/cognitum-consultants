# ADR-004: Cargo Workspace and Repository Layout

## Status
Proposed

## Context
With Rust as the primary backend language (ADR-002) and Axum as the web framework (ADR-003), the repo needs a
concrete crate/directory structure. `implementation-plan.md` §4 already proposes a layout; this ADR ratifies
it as a decision (with minor clarification) rather than leaving it as a "proposed" note in the plan, since the
crate boundaries directly shape how cleanly Phase 4's capability-by-capability integrations
(`implementation-plan.md` §5) can be added without re-deriving structure each time, and how the ten ACL
gateways from `../ddd/anti-corruption-layers.md` map to code.

## Decision
A single Cargo workspace at the repo root, with a TypeScript frontend workspace alongside it (not inside the
Cargo workspace):

```text
cognitum-consultants/
├── Cargo.toml                  # workspace manifest (members below)
├── crates/
│   ├── bff-api/                 # Axum HTTP server: routes, handlers, SSE endpoints, static-file serving
│   ├── bff-core/                 # domain-agnostic aggregation/composition logic, shared DTOs, aggregates
│   │                               #   (DashboardConfiguration, ConsultantPreferences, NotificationItem,
│   │                               #   ActionQueueEntry, CrossCapabilityWorkflowSession — see ../ddd/)
│   ├── nexus-client/              # one submodule per ACL gateway: sales, commit, edu, capacity, customer,
│   │                               #   execution, products, landscape, legal, armor (../ddd/anti-corruption-layers.md)
│   ├── auth/                       # session/auth middleware, token verification & propagation (ADR-008)
│   ├── persistence/                  # repository trait implementations over the chosen datastore (ADR-010)
│   └── config/                        # config loading (env, secrets, per-environment settings)
├── frontend/                            # Vite + TS + Tailwind SPA (own package.json, own toolchain — ADR-005)
├── .plans/                                # planning docs: research.md, implementation-plan.md, adr/, ddd/
├── docs/                                    # durable docs, generated API docs, integration-pattern write-ups
├── scripts/                                   # dev/build helper scripts (ADR-002 clause 2 scope)
└── infra/                                       # deployment manifests (ADR-014)
```

Crate responsibilities and boundaries:
- **`bff-api`** depends on `bff-core`, `nexus-client`, `auth`, `persistence`, `config`. It owns HTTP transport
  only — routing, extraction, response serialization, SSE stream wiring. It must not contain aggregation
  business logic itself (that lives in `bff-core`), keeping handlers thin and unit-testable via `bff-core`
  directly (ADR-013).
- **`bff-core`** is the only crate that defines this repo's own aggregates and their invariants (per
  `../ddd/consultant-experience-context.md`). It depends on `nexus-client`'s and `persistence`'s trait
  interfaces, not their concrete implementations, so it stays testable in isolation.
- **`nexus-client`** is deliberately its own crate, structured as one module per capability gateway, so every
  Phase 4 capability integration extends exactly one place (`implementation-plan.md` §4's stated rationale)
  rather than each route hand-rolling HTTP calls. It implements the ACL pattern from
  `../ddd/anti-corruption-layers.md` §11 ("every gateway is a pure translation boundary") — no gateway module
  may contain business policy logic, only request/response translation.
- **`auth`** is separated from `bff-core` because authentication/session concerns (ADR-008) are
  infrastructure, not domain logic, and need to be testable/replaceable independently (e.g. swapping the dev
  auth stub for real Armor integration per `implementation-plan.md` §6 risk #2 without touching `bff-core`).
- **`persistence`** is separated so the repository interfaces defined in `../ddd/consultant-experience-context.md`
  §1.4/§2.4 can be implemented against the ADR-010 datastore without `bff-core` depending on a specific
  database crate — keeps the "this DDD model is written to be indifferent to [the storage] choice" property
  (`consultant-experience-context.md` §2.5) real in code, not just in the doc.
- **`frontend/`** is intentionally outside the Cargo workspace (it is not a Rust crate); it is a sibling
  TypeScript/npm workspace, keeping `cargo build`/`cargo test` unaffected by frontend tooling and vice versa.

## Consequences
**Positive**
- One workspace, one `Cargo.lock`, atomic cross-crate refactors as Nexus contracts evolve — no version-skew
  risk between `bff-api` and `nexus-client` as would exist across separate repos.
- Crate boundaries mirror the DDD/ACL boundaries already modeled in `../ddd/`, so the code structure and the
  documented architecture stay in sync by construction rather than by discipline alone.
- `nexus-client`'s per-capability module layout gives Phase 4 a literal template: adding capability #4 means
  adding one module, not restructuring anything.

**Negative / Trade-offs**
- More crates than a single-binary approach means more `Cargo.toml` boilerplate and slightly slower full
  workspace builds during early development (mitigated by `cargo check` in CI per Phase 0, and incremental
  compilation).
- Strict dependency direction (bff-core depends on trait interfaces, not concretions) requires discipline to
  keep the workspace's `Cargo.toml` dependency graph from growing circular or leaky over time — worth a
  periodic audit as capabilities are added in Phase 4.

## Alternatives Considered
- **Single monolithic `bff` binary crate, no internal crate split.** Rejected — would let HTTP transport,
  aggregation logic, and Nexus-call code entangle, making it harder to unit-test aggregation logic without
  spinning up Axum, and harder to keep the ACL boundary (no business policy in gateways) enforced by
  compiler-checked module boundaries rather than convention alone.
- **Separate repos per crate (e.g. `nexus-client` as its own published crate).** Rejected for this stage —
  premature; nothing outside this repo consumes `nexus-client` yet, and cross-repo versioning overhead isn't
  justified until (if ever) another Cognitum One Rust service needs to share it.
- **`frontend/` inside the Cargo workspace via a build-script bridge.** Rejected — Vite/npm and Cargo have
  fundamentally different build lifecycles; forcing them into one workspace graph adds tooling complexity
  (e.g. `cargo` trying to reason about a non-Rust member) for no real benefit over two sibling toolchains
  coordinated by root-level `scripts/` (per `implementation-plan.md` §5 Phase 0 deliverable).

## Relationships
- Depends on: ADR-002 (Rust primary language), ADR-003 (Axum for `bff-api`).
- Informs: ADR-007 (`nexus-client` gateway structure), ADR-010 (`persistence` crate implements ADR-010's
  datastore choice), ADR-013 (per-crate testing boundaries), ADR-014 (what gets containerized).
- Source docs: `../implementation-plan.md` §4, `../ddd/anti-corruption-layers.md`, `../ddd/consultant-experience-context.md`.
