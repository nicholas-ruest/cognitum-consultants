# ADR-015: Frontend Server-State and Data-Fetching Strategy — TanStack Query

## Status
Proposed

## Context
This is an additional ADR beyond the task's minimum checklist, added because `../implementation-plan.md`
§3.4 explicitly lists "Frontend server-state management approach (e.g. TanStack Query-equivalent, caching/
invalidation)" as a required decision, and it is not fully resolved by ADR-005 (component framework) or
ADR-006 (interop model) alone. With React chosen (ADR-005) and a JSON API + SPA model chosen (ADR-006), the
frontend still needs a consistent answer for how it fetches, caches, and invalidates data from the BFF's
`/api/*` endpoints across ten capability feature modules (`frontend/src/features/<capability>/`,
`../implementation-plan.md` §4), and how that cache reacts to SSE-pushed updates (ADR-011). Without a fixed
answer, each feature module (Phase 4) would likely reinvent its own fetching/caching pattern, directly
undermining the plan's goal of an "established pattern" other integrations can follow
(`../implementation-plan.md` §1).

## Decision
**TanStack Query (React Query)** is the server-state/data-fetching library for all `/api/*` calls from the
SPA.

- **Query layer**: every capability feature module's data needs (e.g. the Sales conflict-check call from the
  Phase 2 reference flow, `../ddd/anti-corruption-layers.md` §1) go through TanStack Query hooks
  (`useQuery`/`useMutation`), giving consistent loading/error/retry states across all ten feature modules
  without hand-rolled `useEffect`/`useState` data-fetching in each one.
- **Cache invalidation from SSE**: when the BFF pushes a notification/action-queue update via SSE (ADR-011),
  the frontend's SSE event handler calls TanStack Query's `queryClient.invalidateQueries` (or directly
  updates the relevant query cache entry) for the affected query keys — this is the concrete mechanism that
  connects ADR-011's push channel to what the UI actually re-renders, rather than leaving that wiring
  undecided.
- **Query key convention**: query keys are namespaced by capability and consultant (e.g.
  `['sales', 'account-claim', consultantId, companyRef]`), mirroring the `frontend/src/features/<capability>/`
  directory convention (`../implementation-plan.md` §4), so cache scoping visibly matches the repo's own
  capability boundaries.
- **Mutations and BFF-relayed policy verdicts**: for flows like the Sales lead-conflict check
  (`../ddd/anti-corruption-layers.md` §1), a mutation call triggers the BFF request and the returned
  `AccountClaimResult`-shaped response is rendered directly from the mutation's result — the frontend does not
  cache or reuse a stale conflict verdict across a different company entry, consistent with the "opaque
  policy verdict, never re-adjudicated" rule already established for the BFF layer (ADR-007) — the frontend
  data layer must not undermine that rule by caching a decision past its relevance.

## Consequences
**Positive**
- One consistent data-fetching pattern across all Phase 4 feature modules, directly supporting the plan's
  "established pattern... rather than re-deriving one each time" goal.
- Built-in request deduplication, retry, and stale-while-revalidate behavior reduces redundant `/api/*` calls
  against the BFF (which itself may be fanning out to Nexus per call, ADR-007) — a meaningful load reduction
  given the BFF's own fan-out cost.
- Clear, single integration point (SSE handler → `invalidateQueries`) connects ADR-011's push channel to
  actual UI updates, rather than leaving that connection to be reinvented per feature.

**Negative / Trade-offs**
- Adds a specific library dependency and convention (query-key structure) that every feature module must
  follow consistently — requires the same kind of code-review discipline as ADR-007's "no business logic in
  gateways" rule.
- TanStack Query's caching behavior (staleness windows, background refetch) needs sensible per-query defaults
  tuned per data volatility (e.g. `ProductReferenceCard` data, `../ddd/anti-corruption-layers.md` §7, is far
  more cacheable than an in-progress `AccountClaimResult`) — a tuning responsibility left to each feature
  module's implementation, not fully specified by this ADR.

## Alternatives Considered
- **SWR.** A comparable library; rejected mainly on ecosystem-maturity-for-this-use-case grounds — TanStack
  Query's mutation API and query-key-based invalidation model map slightly more directly onto this repo's
  capability-namespaced, SSE-invalidated cache needs, and it has broader adoption for exactly this
  "many independent server resources across feature modules" shape.
- **Redux Toolkit Query (RTK Query).** Rejected — would pull in Redux's global-store model for what is
  fundamentally server-cache state, not client UI state; TanStack Query is purpose-built for server state and
  avoids that unnecessary coupling. (Client-only UI state, if needed, can use plain React state/context — not
  addressed by this ADR since none of the DDD-documented data is client-only UI state beyond ephemeral form
  input.)
- **Hand-rolled fetch + `useEffect`/`useState` per feature, no shared library.** Rejected — directly
  contradicts the plan's "established pattern" goal; ten independently-reinvented fetching implementations
  would be exactly the inconsistency this ADR exists to prevent.

## Relationships
- Depends on: ADR-005 (React), ADR-006 (JSON API shape to fetch against), ADR-011 (SSE drives invalidation).
- Source docs: `../implementation-plan.md` §3.4, §4.
