# Implementation Plan — consultants.cognitum.one

Status: planning only — no application code in this document or produced alongside it.
Source of truth for architecture/ownership: `.plans/research.md` (read in full before this plan; every scope claim below traces back to it).

This plan uses a GOAP-style approach: define the goal state, assess the current state (empty repo + research doc), enumerate the actions available, and sequence them into phased milestones with explicit preconditions, so that progress can be replanned as real-world facts (Nexus contract maturity, auth provider, manage's actual dashboard stack) become known.

---

## 1. Goal State Definition

"Done" for the scope of this planning exercise means a **deployable BFF + consultant-facing UI shell** that:

- Runs a Rust backend-for-frontend (BFF) service that consultants' browsers talk to for all data needs.
- Serves a Vite + TypeScript + Tailwind single-page application providing the consultant application shell: login, navigation, dashboard composition, notification/action queue.
- Composes at least one real sub-business capability end-to-end through `nexus.cognitum.one` (the lead-conflict-warning flow from `research.md` is the reference flow) — proving the integration pattern, not just scaffolding.
- Owns zero business records itself; every business fact displayed is fetched live (or cached transiently) from a sub-business service via Nexus.
- Has a repo structure, CI, and documented conventions that let further capability integrations (proposals, education, capacity, customers, execution, products, ...) be added by following an established pattern rather than re-deriving one each time.
- Has all cross-cutting architectural decisions captured as ADRs (`.plans/adr/`) and the domain/bounded-context model captured separately (`.plans/ddd/`), rather than baked ad hoc into code.

This plan does **not** claim "done" means all twelve sub-business integrations are complete — that is explicitly staged into later phases (Phase 4+) and may extend beyond this plan's horizon.

---

## 2. Scope Boundaries

### 2.1 What this repo OWNS

Per `research.md` §"Role of consultants.cognitum.one", this repo owns only the **experience and orchestration layer**:

- Consultant-facing application shell (Rust BFF + Vite/TS/Tailwind SPA)
- Consultant-specific navigation
- Dashboard composition (aggregating views from multiple capabilities on one screen)
- Cross-capability workflow coordination (e.g., deep link from a Sales lead into a Commit proposal)
- Permission-aware presentation (rendering/hiding UI based on permissions asserted elsewhere)
- Aggregated consultant views (read-side composition, not storage of the underlying records)
- Notifications and action queues (aggregation/presentation; see open question in §6 about where read/unread state should live)
- Frontend routing
- Backend-for-frontend request aggregation and normalization
- Consultant-specific UI preferences (thin, presentation-only state — not business records)
- Deep links and transitions between capabilities

### 2.2 What this repo explicitly does NOT own

Per `research.md`'s ownership hierarchy, this repo must never become the system of record for:

| Domain | Owning service (external) |
|---|---|
| Companies, leads, contacts, opportunities, pipeline, lead protection | `sales.cognitum.one` |
| Proposal templates, generation, pricing, approvals, SOWs | `commit.cognitum.one` |
| Courses, learning paths, certifications, completion records | `edu.cognitum.one` |
| Consultant skills, expertise, availability, internal capacity/staffing intelligence | `capacity.cognitum.one` |
| Customer accounts, stakeholders, relationship history, health | `customer.cognitum.one` |
| Engagements, workstreams, tasks, milestones, delivery status | `execution.cognitum.one` |
| Product/service catalogue, packaging, pricing guidance | `products.cognitum.one` |
| Market/competitive/regulatory intelligence | `landscape.cognitum.one` |
| Legal clauses, templates, contract policy, approvals | `legal.cognitum.one` |
| Authorization policy, access enforcement, data classification, audit | `armor.cognitum.one` |
| Financial/earnings oversight | `capital.cognitum.one` |
| Enterprise decision oversight | `verdict.cognitum.one` |

This repo also does **not** own enterprise-wide oversight (that is `manage.cognitum.one`, a separate, peer application consuming the same underlying services — not a system this repo serves or is served by at runtime). The one exception is a **one-time, one-directional asset import**: copying manage's dashboard shell/component code as a starting point (§5, Phase 1) — this is a source-code borrow, not a runtime dependency on manage.

### 2.3 External systems this repo integrates with (not implemented here)

- **`nexus.cognitum.one`** — the *only* integration point for sub-business capabilities. This repo's BFF must never call sales/commit/edu/capacity/customer/execution/products/landscape/legal/armor/capital services directly; all calls route through Nexus, which owns service routing, cross-domain invocation, auth propagation, and API normalization.
- All twelve sub-business services listed in §2.2, reachable only via Nexus.
- `manage.cognitum.one` — peer application, not a dependency; only relevant as the (external) source of dashboard shell code to copy once.

---

## 3. Technology Stack Decisions (high level)

Detailed rationale for each of these is deferred to individual ADRs (§3.3) produced by a follow-on ADR agent. This section records the leaning/default this plan assumes so phased work isn't blocked, but each is a decision to be formally ratified, not treated as final.

### 3.1 Backend / BFF

- **Language**: Rust (hard requirement from project constraints).
- **Web framework**: **Axum**, leaning over Actix-web. Rationale (to be formalized in ADR): Axum is tower/tokio-native, which suits a BFF whose core job is fanning out concurrent calls to Nexus and aggregating responses — tower's `Service`/middleware model composes cleanly for cross-cutting concerns (auth propagation, tracing, timeouts, retries) that every capability integration in Phase 4 will need. It also has a lower-ceremony extractor model, which keeps the many small aggregation handlers (one per capability) consistent and easy to review.
- **HTTP client to Nexus**: `reqwest` (or `hyper` directly if lower-level control is needed) wrapped in a dedicated `nexus-client` crate — see §4.

### 3.2 Frontend

- **Build tool**: Vite (hard requirement).
- **Styling**: Tailwind CSS (hard requirement).
- **Language**: TypeScript (secondary/fallback language, expected here since Vite is a JS/TS toolchain — this does not violate the "Rust as much as possible" constraint, which applies to the backend/BFF/service layer).
- **Component framework** (React/Vue/Svelte/Solid/etc.): **not decided by this plan** — see ADR checklist (§3.3). Strong pull toward whatever framework manage.cognitum.one's dashboard is built in, since Phase 1 depends on copying that code; if manage's stack is unknown or mismatched, this becomes a blocking decision before Phase 1 can start.

### 3.3 Rust ↔ TypeScript interop model

**Chosen approach: served BFF API + SPA**, not server-side rendering (SSR):

- The Axum service exposes a JSON API under (e.g.) `/api/*`.
- The Vite build produces static SPA assets, served either by the Axum service itself (via `tower-http`'s static-file serving) in simple deployments, or independently via CDN/edge hosting with the SPA calling the BFF's API origin.
- Client-side routing (framework TBD, §3.2) handles navigation; the BFF has no view-rendering responsibility.

Rationale (to be formalized in ADR): this is an internal, highly interactive, permission-sensitive consultant tool, not a public content site needing SEO/first-paint optimization — the case for SSR is weak. Rust SSR frameworks (e.g. Leptos) are comparatively immature versus the React/Vue/Svelte + Vite ecosystem for building rich dashboard UIs, and splitting BFF-API from static-SPA lets each be deployed/scaled/cached independently and keeps the "copy manage's dashboard components" plan (§5 Phase 1) straightforward, since those components are presumably plain frontend components, not SSR-coupled.

### 3.4 ADR checklist (topics only — arguments live in `.plans/adr/`)

A follow-on ADR agent should produce one ADR per topic below. This plan intentionally does not pre-argue these:

- [ ] ADR: Rust web framework selection (Axum vs Actix-web vs other)
- [ ] ADR: Rust↔TS interop / rendering model (BFF+SPA vs SSR vs hybrid)
- [ ] ADR: Frontend component framework atop Vite (React/Vue/Svelte/Solid/none)
- [ ] ADR: Authentication & session strategy (provider, token format e.g. JWT/opaque, propagation to Nexus/Armor, session storage)
- [ ] ADR: Nexus API contract & client abstraction pattern (protocol — REST/GraphQL/gRPC —, versioning, error/retry semantics)
- [ ] ADR: Frontend server-state management approach (e.g. TanStack Query-equivalent, caching/invalidation)
- [ ] ADR: Notification/event delivery mechanism (polling vs SSE vs WebSocket vs Nexus webhook-to-BFF)
- [ ] ADR: Consultant preferences & UI view-state persistence (embedded store vs Postgres vs Redis vs delegate to another service)
- [ ] ADR: Observability stack for the Rust BFF (tracing/logging/metrics libraries, correlation IDs across Nexus hops)
- [ ] ADR: Deployment/infra target and CI/CD pipeline shape
- [ ] ADR: Design-system extraction packaging strategy (npm workspace package vs separate repo vs private registry) — Phase 5
- [ ] ADR: Testing strategy against Nexus (contract tests, mocking/stub strategy, e2e approach)
- [ ] ADR: Resilience patterns for concurrent multi-capability aggregation (timeouts, partial failure/graceful degradation when one Nexus-routed capability is slow/down)

---

## 4. Repository / Workspace Layout (proposed)

Rust-primary, TypeScript-secondary monorepo:

```text
cognitum-consultants/
├── Cargo.toml                  # workspace manifest
├── crates/
│   ├── bff-api/                 # Axum HTTP server: routes, handlers, wiring
│   ├── bff-core/                 # domain-agnostic aggregation/composition logic, shared DTOs
│   ├── nexus-client/              # typed client(s) for nexus.cognitum.one; one module per capability
│   ├── auth/                       # session/auth middleware, token verification & propagation
│   └── config/                      # config loading (env, secrets, per-environment settings)
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tailwind.config.ts
│   ├── src/
│   │   ├── app/                    # shell: layout, routing, providers
│   │   ├── features/                # one directory per capability (sales, proposals, education, capacity, customers, execution, products, notifications)
│   │   ├── components/               # dashboard shell components copied/adapted from manage.cognitum.one
│   │   ├── lib/                       # BFF API client, hooks, utilities
│   │   └── styles/
│   └── public/
├── .plans/
│   ├── research.md
│   ├── implementation-plan.md       # this document
│   ├── adr/                          # produced separately — see §3.4 checklist
│   └── ddd/                           # produced separately — see §7
├── docs/
├── scripts/
└── infra/                              # deployment manifests — shape is an open question, §6
```

Notes:
- `nexus-client` is deliberately its own crate so every capability integration in Phase 4 extends one place rather than each BFF route hand-rolling HTTP calls.
- `bff-core` separates aggregation/composition logic from HTTP transport (`bff-api`), keeping handlers thin and testable.
- `frontend/src/features/*` mirrors the capability boundaries in §2.2 1:1, so the repo's structure visibly reflects "thin orchestration over many owned domains" rather than growing its own domain model.

---

## 5. Phased Milestones

### Phase 0 — Scaffolding & Tooling

**Goal**: an empty-but-running BFF and an empty-but-running frontend shell, with CI enforcing they stay buildable.

**Preconditions**: none — can start immediately from current state (empty repo + research doc).

**Key deliverables**:
- Cargo workspace with `bff-api` (Axum) exposing a health-check endpoint, plus empty `bff-core`, `nexus-client`, `auth`, `config` crate stubs.
- Vite + TypeScript + Tailwind scaffold in `frontend/` (component framework choice pending ADR — may start framework-agnostic or with a placeholder pending §3.4 decision).
- CI pipeline: Rust build/lint/test (`cargo check`, `cargo clippy`, `cargo test`), frontend build/lint (`npm run build`, lint), both gating merges.
- `.plans/adr/` and `.plans/ddd/` seeded (by other agents, per §7) — this phase just ensures the plan's references to them resolve.

**Rough actions**:
1. `cargo new` workspace + crates; wire `bff-api` health endpoint.
2. `npm create vite@latest` frontend scaffold; add Tailwind.
3. Add CI workflow (build+lint+test both stacks).
4. Add root-level dev scripts (`scripts/`) for running BFF + frontend together in dev mode.

---

### Phase 1 — Auth + Shell + Navigation

**Goal**: a consultant can authenticate and see the application shell with consultant-specific, permission-aware navigation — no real business data yet.

**Preconditions**: Phase 0 complete; ADR on auth strategy resolved (or a dev-stub auth accepted as an interim); clarity on manage.cognitum.one's dashboard stack (to know whether "copy" is literal or requires porting) — **blocking open question, see §6**.

**Key deliverables**:
- BFF session/auth middleware (`auth` crate) — real integration with Armor-issued credentials, or an interim stub if Armor isn't ready.
- Frontend shell: layout, sidebar, header, routing skeleton — copied/adapted from manage.cognitum.one's dashboard code per `research.md`'s explicit plan, with manage-specific business logic stripped.
- Permission-aware nav rendering (nav items shown/hidden based on permission data — source of permission data itself is Armor via Nexus, per §2.3).
- `/api/session` BFF endpoint.

**Rough actions**:
1. Resolve/confirm auth ADR; implement BFF-side session verification.
2. Import and adapt manage's shell/layout/sidebar/header/card/table/form/search/filter/alert/dialog components into `frontend/src/components/`.
3. Build nav config driven by a permission model (shape TBD pending Armor/Nexus contract).
4. Wire login flow end-to-end (frontend → BFF → auth provider).

---

### Phase 2 — Dashboard Composition + First Nexus-Backed Capability

**Goal**: prove the full BFF→Nexus→sub-business-service integration pattern end-to-end using the reference flow from `research.md`: lead-conflict-warning against `sales.cognitum.one`.

**Preconditions**: Phase 1 complete; Nexus API contract available (at minimum for the Sales conflict-check capability) or a stable mock of it; `nexus-client` abstraction pattern decided (ADR).

**Key deliverables**:
- `nexus-client` crate implements a typed call for the Sales conflict-check capability.
- BFF endpoint (e.g. `POST /api/sales/lead-conflict-check`) that normalizes consultant input and forwards to Nexus — **the BFF must not decide conflict policy itself**, only relay Sales' decision (per `research.md`'s explicit warning that the frontend must never independently decide business policy).
- Frontend: company-entry form + result rendering (warning message + permitted actions: request collaboration, submit referral, cancel — per the example payload in `research.md`).
- First dashboard composition: at least one dashboard card/module driven by this live Nexus-backed data.
- This flow documented as the **template pattern** other capability integrations in Phase 4 will follow.

**Rough actions**:
1. Define request/response DTOs matching (or wrapping) the Nexus/Sales contract.
2. Implement `nexus-client` trait + Sales-specific implementation.
3. Implement BFF handler: validate/normalize input, call Nexus, pass through response.
4. Implement frontend form + conditional UI for `match_status`/`permitted_actions`.
5. Integration tests against a mocked Nexus (contract tests once real contract is available).
6. Write up the pattern (short doc in `docs/`) so Phase 4 integrations replicate it consistently.

---

### Phase 3 — Notifications & Action Queues

**Goal**: a unified notification centre and action/task queue aggregating events across capabilities, per `research.md`'s "One notification centre / one task list" requirement.

**Preconditions**: Phase 2's integration pattern established; resolution of the open question in §6 about where notification read/unread state should live; ADR on event delivery mechanism.

**Key deliverables**:
- Notification aggregation in the BFF (polling Nexus and/or consuming pushed events — mechanism per ADR).
- Frontend notification centre + action queue components.
- Read/unread and action-taken state handling — scoped as thin BFF-owned view-state, *not* a new business record store, per §2.1 (to be confirmed, §6).

**Rough actions**:
1. Design notification DTO schema (capability-agnostic envelope + capability-specific payload).
2. Implement BFF aggregation endpoint(s) or event listener per chosen delivery mechanism.
3. Implement frontend notification centre + action queue UI.
4. Define minimal persistence for view-state only (if needed) — pending ADR on storage choice.

---

### Phase 4 — Additional Capability Integrations

**Goal**: extend the Phase 2 pattern to the remaining sub-business capabilities, staged by Nexus contract readiness.

**Preconditions**: Phase 2 pattern proven; per-capability Nexus contracts available (integrations may be staggered/reordered as contracts mature — this is where GOAP-style replanning applies most directly, since availability will shift the optimal order).

**Suggested staging** (adjust based on actual contract readiness at execution time):
1. Proposals/commitments (`commit.cognitum.one`)
2. Education (`edu.cognitum.one`)
3. Capacity/consultant profile (`capacity.cognitum.one`) — restricted intake-only surface, per `research.md`'s explicit note that consultants must not receive internal Capacity access
4. Customers (`customer.cognitum.one`) — permitted/assigned records only
5. Execution/delivery (`execution.cognitum.one`)
6. Products (`products.cognitum.one`)
7. Landscape (read-only intelligence consumption + field-observation submission)
8. Legal (approved-content consumption only, via Commit/Consultants, per `research.md`)

**Key deliverables per capability**: DTOs, `nexus-client` extension, BFF route, frontend feature module under `frontend/src/features/<capability>/`, deep-link wiring to/from related capabilities (e.g., lead → proposal, engagement → customer).

**Rough actions** (repeated per capability): confirm Nexus contract → extend `nexus-client` → BFF route → frontend module → deep-link wiring → tests.

---

### Phase 5 — Design-System Extraction

**Goal**: extract reusable shell/dashboard components into standalone packages, per `research.md`'s long-term note about `@cognitum/design-system` / `@cognitum/dashboard-components`.

**Preconditions**: Phases 1–4 stable; enough real shared-component surface identified (with manage.cognitum.one and potentially other Cognitum One frontends) to justify extraction; ADR on packaging strategy (§3.4).

**Key deliverables**: extracted package(s), consultants app refactored to consume them instead of local copies.

**Rough actions**: audit actual component reuse/drift since Phase 1's copy, extract shared primitives, publish internally, update imports, document versioning/upgrade policy.

---

### Phase 6 — Hardening / Production Readiness (brief)

Not detailed here beyond flagging it exists: observability wiring, performance/load testing, accessibility pass, security review (Armor-aligned), before general availability. Should get its own plan once Phases 0–3 are further along.

---

## 6. Risks and Open Questions

These need product/architecture decisions before or during the relevant phase — listed roughly in the order they become blocking:

1. **Manage's dashboard stack is unverified.** `research.md` assumes a literal "copy" of manage's dashboard shell/components is feasible (Phase 1). If manage.cognitum.one is not built on Vite/TS/Tailwind (or a compatible component framework), this becomes a port, not a copy, with materially different Phase 1 effort. **Blocking for Phase 1.**
2. **Auth provider/strategy undecided.** How Armor issues/validates credentials, what the BFF verifies, and how identity propagates through Nexus to sub-business services is unspecified in `research.md`. **Blocking for Phase 1.**
3. **Nexus API contract maturity.** Is Nexus already built and versioned, and in what protocol (REST/GraphQL/gRPC)? Phase 2 needs at least the Sales conflict-check contract; later phases need the rest, likely on a staggered timeline. **Blocking for Phase 2, staging driver for Phase 4.**
4. **Frontend component framework unchosen.** Neither the user's constraints nor `research.md` specify React/Vue/Svelte/etc. This is entangled with risk #1 (should match manage's framework if copying components).
5. **Where does notification/action-queue state live?** `research.md` says the portal presents notifications but doesn't say whether read/unread/action-taken state is itself business data owned by a sub-business service (or Nexus's event system) versus legitimate BFF-local view-state. Getting this wrong would violate the "own nothing" principle. **Needs resolution before Phase 3.**
6. **Consultant preferences storage.** Similarly, where BFF-owned "consultant preferences" are persisted (embedded DB, Postgres, Redis, or delegated) is undecided — affects Phase 1/3 persistence choices.
7. **Permission model shape.** "Permission-aware presentation" (Phase 1) requires a concrete permission data shape from Armor/Nexus; undefined today.
8. **Deployment/infrastructure target.** No target environment (cloud, container orchestration, CDN) is specified yet; affects Phase 0 CI/CD design and the BFF-serves-static-vs-CDN choice in §3.3.
9. **Cross-repo Rust conventions.** If other Cognitum One services are also Rust, aligning crate-layout/observability/error-handling conventions across repos (rather than inventing this repo's own) would reduce integration friction — worth checking before Phase 0 conventions solidify.
10. **Data caching/residency constraints.** Legal/Armor may impose constraints on how long the BFF may cache upstream data from sub-business services (relevant to dashboard composition performance vs. compliance) — unaddressed in `research.md`.

---

## 7. Relationship to ADRs and DDD Models

This plan intentionally does not fully argue technology choices or model the domain — that work belongs to dedicated follow-on agents and lives outside this document:

- **Architecture Decision Records** — one per item in the checklist at §3.4 — will be produced separately and stored in `.plans/adr/`. This plan references them by topic only.
- **DDD bounded-context / aggregate models** — since this repo is intentionally domain-thin, the DDD work is less about modeling rich aggregates here and more about precisely bounding the "consultant experience/orchestration" context against the twelve owning sub-business contexts listed in §2.2, and defining the anti-corruption/normalization boundary at the Nexus integration point (relevant to Phase 2's pattern). This will be produced separately and stored in `.plans/ddd/`. This plan references it by topic only and does not duplicate its content.

Future revisions of this plan should treat resolved ADRs and the DDD model as new preconditions that may unblock, reorder, or reshape the phases above (GOAP-style replanning), particularly Phase 1 (auth, component framework) and Phase 4 (integration staging order).
