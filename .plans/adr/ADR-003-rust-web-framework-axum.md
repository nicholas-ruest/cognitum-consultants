# ADR-003: Rust Web Framework for the BFF — Axum

## Status
Proposed

## Context
Per ADR-002, the BFF is implemented in Rust. `implementation-plan.md` §3.1 already leans toward Axum over
Actix-web; this ADR formalizes that choice against this repo's actual shape of work.

The BFF's core job (per `../research.md` and `../ddd/anti-corruption-layers.md`) is not serving views — it is
fanning out concurrent calls to `nexus.cognitum.one` across up to ten capability-specific ACL gateways
(Sales, Commit, Edu, Capacity, Customer, Execution, Products, Landscape, Legal, Armor), aggregating and
normalizing their responses, and applying uniform cross-cutting behavior on every hop: auth/session
propagation (ADR-008), permission-aware filtering (ADR-009), tracing/correlation IDs (ADR-012), and
timeout/resilience handling (ADR-016). Every one of Phase 4's future capability integrations
(`implementation-plan.md` §5) will repeat this same handler shape. The framework choice needs to make that
repetition cheap and consistent, not just serve HTTP requests.

## Decision
**Axum** is the BFF's web framework.

Rationale:
- **Tower/Tokio-native.** Axum is built directly on `tower::Service` and `hyper`/`tokio`. The cross-cutting
  concerns this BFF needs on every Nexus-routed call — auth propagation, tracing spans, timeouts, retries,
  rate limiting — are exactly what `tower::Layer`/`tower::Service` middleware composition is designed for, and
  that same middleware stack is reusable both on the inbound Axum router *and* on the outbound `nexus-client`
  HTTP calls (both can be built on `tower`), giving one consistent middleware model across the whole request
  lifecycle instead of two different ones.
- **Low-ceremony extractor model.** Handlers are plain async functions with typed extractors
  (`Json<T>`, `State<T>`, `Extension<T>`); this keeps the many small, structurally-similar aggregation
  handlers (one per capability, per `implementation-plan.md` §4's `bff-api` crate) consistent and easy to
  review — a reviewer checking the Sales handler against the Commit handler sees the same shape every time.
- **Maturity and maintenance.** Axum is maintained by the Tokio project itself, tracks `hyper`/`tokio` closely,
  and has a large, active ecosystem of `tower` middleware crates (`tower-http` for static file serving,
  tracing, CORS, compression — directly useful for ADR-006's SPA-serving requirement and ADR-012's
  observability requirement).
- **SSE support.** ADR-011 selects Server-Sent Events for notification delivery; Axum has first-class,
  well-documented SSE support (`axum::response::sse`), avoiding a second framework or a bolted-on extension
  for that requirement.
- **Team-familiarity assumption.** Absent a stated existing team convention, Axum is currently the more
  commonly adopted choice for new Rust HTTP services in the broader ecosystem, which reduces onboarding cost
  for future contributors and keeps this repo aligned if sibling Cognitum One Rust services (risk #9 in
  `../implementation-plan.md` §6) converge on a shared convention later.

## Consequences
**Positive**
- One middleware model (`tower`) spans inbound HTTP handling and outbound Nexus calls, reducing duplicated
  cross-cutting logic across `bff-api` and `nexus-client`.
- Native SSE support removes a dependency risk for ADR-011.
- Large `tower-http` ecosystem covers static-file serving (ADR-006/ADR-014), tracing, and CORS out of the box.

**Negative / Trade-offs**
- Axum's extractor/type-driven style has a learning curve for contributors unfamiliar with `tower`'s
  `Service`/`Layer` abstractions specifically (versus Actix-web's more self-contained actor model).
- Tighter coupling to the `tokio` runtime specifically (not a practical concern here, since nothing in this
  repo's stack calls for a different async runtime).

## Alternatives Considered
- **Actix-web.** Rejected as primary choice. Actix-web's actor-model runtime is largely vestigial for typical
  HTTP services today (most Actix-web apps use it as a conventional async framework, not via actors), and its
  middleware system, while mature, is its own abstraction rather than the `tower` ecosystem this repo wants to
  share between inbound routing and outbound Nexus calls. Actix-web is a legitimate, high-performance,
  production-proven alternative and this decision is not a criticism of it — it is a fit judgment for this
  repo's specific fan-out/aggregate/middleware-heavy shape.
- **Warp.** Rejected — filter-combinator composition becomes hard to read once handlers accumulate several
  cross-cutting concerns (exactly this repo's shape); smaller ecosystem than Axum/tower-http.
- **Rocket.** Rejected — historically slower to reach a stable async story, more macro-heavy request routing
  that is harder to keep uniform across ten structurally similar capability handlers, and no first-party SSE
  primitive as clean as Axum's.
- **Hand-rolled on raw `hyper`.** Rejected — would reimplement routing/extraction/middleware composition that
  Axum already provides, for no benefit given this repo's needs are conventional (not an unusual protocol or
  extreme low-level control requirement).

## Relationships
- Depends on: ADR-002 (Rust is the backend language).
- Informs: ADR-004 (workspace layout — `bff-api` crate is Axum-based), ADR-006 (interop model — Axum serves
  the SPA via `tower-http`), ADR-007 (Nexus client shares the `tower` middleware model), ADR-011 (SSE),
  ADR-012 (tracing middleware), ADR-016 (resilience middleware).
