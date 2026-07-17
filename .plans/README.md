# Planning Index — consultants.cognitum.one

## What is This?

`consultants.cognitum.one` is a unified consultant-facing workspace application within Cognitum One. It serves as the experience and orchestration layer that composes capabilities from ten sub-business services (Sales, Proposals/Commitments, Education, Capacity, Customers, Execution/Delivery, Products, Market Intelligence, Legal, Security) via `nexus.cognitum.one`, giving each consultant one login, one dashboard, one navigation system, and one activity feed. The application owns **no business records itself** — every fact displayed is fetched live (or cached transiently) from an authoritative sub-business service via Nexus. 

This planning package captures the architectural research, domain model, implementation strategy, and design decisions for this repo. See `research.md` for the original ownership hierarchy and business context; `implementation-plan.md` for the phased execution strategy.

**Status: Planning phase only.** Nothing in this document or its adjacent files represents implemented code. This is design and architectural decision-making ahead of implementation.

---

## Reading Order

Start here and follow this sequence to understand the full planning package:

1. **`research.md`** — the original architectural research establishing ownership hierarchy, role boundaries, and the Nexus-only integration rule. Read this first to understand *why* this repo exists and what it does not own.

2. **`implementation-plan.md`** — a phased implementation plan (6 phases from scaffolding to hardening) that translates the research into a concrete roadmap with explicit preconditions, deliverables, and open risks.

3. **`ddd/domain-map.md`** — bounded context map showing how consultants.cognitum.one relates to every upstream service and peer application; identifies the two contexts this repo owns and the ten it consumes via Nexus.

4. **`ddd/consultant-experience-context.md`** — detailed DDD model of the two contexts this repo owns: Consultant Workspace (shell, navigation, dashboard, preferences, workflow sessions) and Notification & Action Queue (aggregated cross-capability event feed).

5. **`ddd/anti-corruption-layers.md`** — the ten ACL adapter modules in `nexus-client`, one per external capability, showing request/response shapes and the "translate, never re-adjudicate" rule each must follow.

6. **`ddd/domain-events.md`** — consolidated event catalog: what this repo raises, what it consumes, and how external events flow through the two bounded contexts.

7. **`adr/*.md`** — Architecture Decision Records, one per architecturally significant choice. Read these to understand the specific decisions (language, framework, persistence, observability, resilience, etc.) and their trade-offs. Start with ADR-001 (the ADR process itself), then follow the dependency chains noted in each "Relationships" section.

---

## DDD Models

| Document | Scope | Key Concepts |
|---|---|---|
| **domain-map.md** | Bounded context map | Two owned contexts; ten consumed contexts via Nexus; Capital and Verdict explicitly excluded (Manage-only); one-time code-asset borrow from Manage |
| **consultant-experience-context.md** | Consultant Workspace & Notification/Action Queue | Five aggregates: `DashboardConfiguration`, `ConsultantPreferences`, `CrossCapabilityWorkflowSession`, `NotificationItem`, `ActionQueueEntry`; five domain events; repository interfaces for persistence |
| **anti-corruption-layers.md** | Gateway adapters (nexus-client) | Ten capability modules (Sales, Commit, Edu, Capacity, Customer, Execution, Products, Landscape, Legal, Armor); request/response shapes; "translate, never re-adjudicate" constraint |
| **domain-events.md** | Cross-context event catalog | 6 internal events (Workspace), 6 Notification/Action Queue events, 34 external events across the ten capabilities; inbound/outbound direction for each |

---

## Architecture Decision Records

All ADRs are in `Proposed` status — acceptance occurs at or just before the phase that depends on each decision.

| # | Title | Decision | Phase(s) |
|---|---|---|---|
| 001 | Record Architecture Decisions | Use ADRs in `.plans/adr/` for every significant decision; versioned, never edited after acceptance, superseded by new ADRs. | Ongoing |
| 002 | Primary Language: Rust; Secondary: TypeScript | Rust for backend/BFF/services; TypeScript for frontend and frontend-adjacent tooling only; other languages require their own ADR. | 0+ |
| 003 | Rust Web Framework: Axum | Axum (tower/tokio-native) for the BFF, chosen for middleware composability across ten ACL gateways and native SSE support. | 0+ |
| 004 | Cargo Workspace Layout | Single Cargo workspace with six Rust crates (`bff-api`, `bff-core`, `nexus-client`, `auth`, `persistence`, `config`) plus a separate TypeScript/npm frontend; structure mirrors DDD/ACL boundaries. | 0+ |
| 005 | Frontend Stack: React + Vite + Tailwind | React with TypeScript on Vite + Tailwind CSS; contingent on manage.cognitum.one's framework matching (if not, ADR must be revisited before Phase 1 component-copy work). | 0–1 |
| 006 | BFF-to-Frontend Interop: JSON API + SPA | Axum serves JSON API under `/api/*`; Vite build produces static SPA; no server-side rendering; client-side routing via React Router. | 0–1 |
| 007 | Nexus Integration Pattern: REST/JSON + ACL Gateways | REST/JSON via a `NexusTransport` trait abstraction; one typed gateway trait per capability in `nexus-client`; auth propagation on every call. | 0+ (Phase 2 blocks on Nexus contract availability) |
| 008 | Authentication & Session Strategy | BFF-managed server-side sessions backed by Armor identity (via Nexus OIDC/OAuth2 flow); interim dev-stub for pre-Armor phases; session storage in shared Postgres (ADR-010). | 0–1 |
| 009 | Authorization & Permission-Aware Presentation | Three-layer enforcement (BFF server filter, client-side render, downstream capability re-check); consume Permission Assertions from Armor via Nexus; never compute or override policy. | 0–1 |
| 010 | Persistence: PostgreSQL + sqlx | PostgreSQL for all five owned aggregates (DashboardConfiguration, ConsultantPreferences, notification/action state, workflow sessions); sqlx for compile-time query checking; migrations via sqlx-cli. | 0+ |
| 011 | Event Notification Delivery: SSE | Server-Sent Events for BFF→browser push (native Axum support, fallback to polling); Nexus→BFF uses polling initially, upgradeable to webhook once Nexus contract confirmed. | Phase 3 |
| 012 | Observability: tracing + OpenTelemetry + Prometheus | `tracing` for structured logging, OpenTelemetry for distributed tracing (W3C Trace Context for correlation IDs across Nexus hops), Prometheus metrics; per-gateway metrics for resilience tuning (ADR-016). | 0+ |
| 013 | Testing Strategy | Layered: unit tests (`bff-core`, gateways, aggregates), mock-fixture contract tests (upgrading to real contracts as Nexus matures), integration tests (test-container Postgres), frontend component tests (Vitest + RTL), e2e (Playwright). | 0+ |
| 014 | Deployment & Runtime Topology | Multi-stage Dockerfile with cargo-chef, minimal OCI image, serves both API and SPA, targets generic OCI orchestrator (env-var config, health checks, graceful shutdown), horizontal scaling ready via Postgres + Postgres `LISTEN`/`NOTIFY` for SSE fan-out. | 0+ |
| 015 | Frontend Server-State & Data-Fetching: TanStack Query | TanStack Query for all `/api/*` calls; query keys namespaced by capability; cache invalidation driven by SSE events; per-query caching tuned by data volatility. | Phase 1+ |
| 016 | Resilience & Partial-Failure Handling | Per-gateway timeouts (tuned by read/write latency profile), idempotent-only retries, concurrent fan-out with per-call isolation (no single failure fails whole dashboard), circuit breaking per capability, partial-result response shape so cards degrade independently. | Phase 2+ |

---

## Implementation Phases

All phases are sequenced with explicit preconditions and deliverables; later phases may replan based on external factors (Nexus contract maturity, auth provider readiness, manage's dashboard framework).

| Phase | Goal | Key Deliverables | Preconditions |
|---|---|---|---|
| **0** | Scaffolding & CI | Empty Cargo workspace + frontend scaffold, health-check endpoint, CI enforcing build/lint/test | None |
| **1** | Auth + Shell + Navigation | Session/auth middleware, shell copied/adapted from Manage, permission-aware nav, `/api/session` endpoint, CI still gated | Phase 0 complete; ADR-008 (auth) resolved or dev-stub accepted; clarity on Manage's dashboard framework |
| **2** | Dashboard + First Nexus Capability | Prove the BFF→Nexus→Sales lead-conflict-warning flow end-to-end; implement `SalesGateway`, BFF route, frontend form, first dashboard card; document pattern for Phase 4 | Phase 1 complete; Nexus API contract available (or stable mock); ADR-007 resolved |
| **3** | Notifications & Action Queues | Notification centre + action queue UI, read/unread state, `NotificationItem`/`ActionQueueEntry` aggregates, SSE stream | Phase 2 complete; ADR-011 resolved; storage question settled (ADR-010 applies) |
| **4** | Additional Capability Integrations | Iteratively add Sales, Commit, Edu, Capacity, Customer, Execution, Products, Landscape, Legal — each following Phase 2's template pattern; deep-link wiring between capabilities | Phase 2 pattern proven; per-capability Nexus contracts available (staggered, reordered by readiness) |
| **5** | Design-System Extraction | Extract reusable shell/dashboard components into shared packages (e.g. `@cognitum/design-system`); both Manage and Consultants consume from it; phase-1 code copies become versioned package references | Phases 1–4 stable; actual component-reuse surface identified; ADR-002 design-system packaging strategy finalized |
| **6** | Hardening & Production Readiness | Observability tuning, performance/load testing, accessibility audit, security review (Armor-aligned), incident runbooks, before general availability | Phases 0–5 complete; sufficient real traffic/testing data to justify hardening effort |

---

## Key Open Risks

These require resolution before or during the indicated phase. The implementation plan (§6) documents all ten open questions in detail; the highest-impact are surfaced here:

**Before Phase 1 can start:**
- **Manage's dashboard framework is unverified.** Phase 1 assumes a literal copy of Manage's shell/layout/components is feasible (if Manage uses Vue, Svelte, or a mismatched stack, this becomes a port, materially increasing Phase 1 effort).
- **Auth provider/strategy undecided.** How Armor issues credentials, what the BFF verifies, and how identity propagates through Nexus is unspecified; ADR-008 defines a shape that accommodates this uncertainty, but the real contract must be confirmed.

**Before Phase 2 can start:**
- **Nexus API contract maturity.** Phase 2 needs at least the Sales conflict-check capability's contract (or a stable mock); later phases need the rest, likely staggered. ADR-013 stages testing to work with mocks until real contracts land.

**Before Phase 3 can start:**
- **Notification/action-queue state ownership unclear.** This repo frames read/unread and action-state as legitimate BFF-local view-state (per ADR-010), but confirmation from architecture leadership is needed to ensure it doesn't violate the "own nothing" principle.

All risks and their dependencies are detailed in `implementation-plan.md` §6.

---

## Next Steps

**For architecture/planning validation**: 
- Review `.plans/adr/` for decision rationale and trade-offs on any topic (language, framework, persistence, observability, resilience).
- Cross-check the DDD model (`ddd/domain-map.md`, `ddd/consultant-experience-context.md`) against your domain expertise to ensure bounded-context boundaries are sound.
- Confirm the open risks (`implementation-plan.md` §6) are understood and prioritized for pre-implementation resolution.

**For implementation readiness**:
- Confirm or dispute each ADR's decision before starting the phase that depends on it (preconditions in implementation phases above).
- Resolve the three highest-severity open risks (Manage's framework, auth provider, Nexus contract) — these gate Phase 1 start.
- Ensure the `nexus-client` gateway structure and the DDD aggregate invariants (`ddd/consultant-experience-context.md`) are acceptable to whoever will own those boundaries in code.
- Align observability conventions with sibling Cognitum One Rust services if they exist (`implementation-plan.md` §6 risk #9).

**The repository itself** (code, tests, CI, live deployments) does not yet exist — this planning package is the input to implementation phase work.
