# ADR-006: BFF-to-Frontend Interop Model — Served JSON API + SPA (No SSR)

## Status
Proposed

## Context
With Rust/Axum on the backend (ADR-003) and React/TypeScript/Vite on the frontend (ADR-005), the repo needs a
decided interop model: how does the Rust service and the TypeScript client actually talk, and does the server
render any HTML/view state? `implementation-plan.md` §3.3 already leans toward a served BFF-API + SPA model
over server-side rendering (SSR); this ADR formalizes that as an accepted decision.

Key facts driving this: this is an internal, authenticated, highly interactive consultant tool
(`../research.md`'s "One dashboard... one navigation system... one unified workflow experience"), not a
public content site — there is no SEO requirement and first-paint latency for an internal tool is a much
smaller concern than for a marketing site. Rust-native SSR frameworks (e.g. Leptos) are comparatively immature
next to the React + Vite ecosystem for building rich, permission-aware dashboard UIs. Phase 1's plan to copy
manage.cognitum.one's dashboard components (`../research.md` §"Dashboard Relationship") also presumes those
components are plain client-side components, not SSR-coupled.

## Decision
The Axum BFF exposes a JSON API under `/api/*`. The Vite build produces a static SPA bundle. The BFF has no
view-rendering responsibility — no server-rendered HTML beyond the SPA's index shell.

- **API shape**: JSON over HTTP(S), versioned implicitly via the BFF's own release cadence (no `/v1/` prefix
  needed while this repo has exactly one consumer — its own SPA; revisit if a second consumer appears).
- **Static asset serving**: for Phase 0–2 simplicity, the Axum service serves the compiled SPA's static assets
  directly via `tower-http`'s `ServeDir`/`ServeFile` (consistent with ADR-003's `tower-http` ecosystem choice),
  so one deployable container serves both the API and the UI (see ADR-014 for the full deployment decision).
  Splitting static-asset serving to a CDN/edge origin is an explicit, non-blocking future optimization, not
  ruled out by this ADR — the SPA build output is CDN-portable by construction either way.
- **Client-side routing**: React Router (or TanStack Router, decided alongside ADR-015) owns all navigation;
  the BFF is agnostic to frontend routes and serves the SPA shell for any non-`/api/*` path (standard SPA
  fallback routing).
- **Real-time push**: layered on top via SSE (ADR-011) as a separate `/api/*` stream endpoint, not a
  view-rendering concern.

## Consequences
**Positive**
- Clean separation of concerns: Rust owns aggregation/business-adjacent logic and API correctness; TypeScript
  owns presentation — matches the two-language policy's spirit (ADR-002) of using each language where it's
  strongest.
- BFF-API and SPA can be deployed, scaled, and cached independently if/when needed, without an architecture
  change (the split already exists at the build-output level).
- Keeps the "copy manage's dashboard components" plan (Phase 1) straightforward — those components are
  presumably plain client components, not entangled with a server-rendering model this repo would have to
  replicate.
- Avoids taking on Rust SSR framework immaturity risk for a use case (internal, authenticated dashboard) that
  doesn't need SSR's benefits in the first place.

**Negative / Trade-offs**
- No server-rendered first paint — initial load shows a loading state until the SPA bundle executes and calls
  the API. Acceptable for an internal, authenticated tool behind a login wall (no SEO crawler ever sees it,
  and repeat users benefit from browser caching of the SPA bundle).
- Two runtime processes conceptually (API logic, SPA bundle) even when served from one container — requires
  discipline to keep `/api/*` and SPA-route namespaces from colliding as Phase 4 adds capability routes.

## Alternatives Considered
- **Server-side rendering (Rust, e.g. Leptos) or hybrid islands architecture.** Rejected — SSR/islands would
  require rebuilding the "copy manage's React/Vue/Svelte dashboard components" plan around a comparatively
  immature Rust rendering framework, for a use case (internal tool, no SEO need) where SSR's main benefits
  don't apply. Revisit only if a future requirement (e.g. public-facing pages) actually needs SSR.
  This decision also depends on ADR-005's React choice remaining valid — reopen alongside ADR-005 if that
  changes.
- **Next.js-style Node.js SSR/BFF, dropping the Rust BFF entirely.** Rejected outright — violates ADR-002's
  hard Rust-first constraint; not a real option under this project's language policy.
- **GraphQL API instead of REST/JSON between BFF and SPA.** Rejected for this internal boundary — the SPA has
  exactly one backend (this BFF), so GraphQL's main benefit (flexible querying across many potential clients)
  doesn't apply; REST/JSON keeps the Axum handler-per-aggregation-endpoint model (ADR-003) simple and avoids a
  second query language/tooling surface on top of the one already needed for Nexus (ADR-007).

## Relationships
- Depends on: ADR-003 (Axum serves both API and static assets), ADR-005 (React/Vite SPA).
- Informs: ADR-011 (SSE endpoint lives under `/api/*`), ADR-014 (deployment packages API + SPA together by
  default), ADR-015 (client-side data fetching against this API shape).
- Source docs: `../implementation-plan.md` §3.3, `../research.md` §"Dashboard Relationship".
