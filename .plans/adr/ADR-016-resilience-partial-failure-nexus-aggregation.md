# ADR-016: Resilience and Partial-Failure Handling for Nexus Aggregation

## Status
Proposed

## Context
This is an additional ADR beyond the task's minimum checklist, added because `../implementation-plan.md` §3.4
explicitly lists "Resilience patterns for concurrent multi-capability aggregation (timeouts, partial failure/
graceful degradation when one Nexus-routed capability is slow/down)" as a required decision, and nothing in
ADR-007 (which fixes the Nexus integration *shape*) addresses failure behavior. This matters concretely: a
single dashboard render (`../ddd/consultant-experience-context.md` §1.2's `DashboardConfiguration`) may need
data from several capabilities concurrently (e.g. a Sales card, a Commit card, an Execution card all on one
screen). Per `../research.md`, this repo's dashboard composition is exactly this kind of fan-out. Without a
defined resilience strategy, one slow or down capability (fully outside this repo's control, since it's a
peer sub-business service reachable only via Nexus) could degrade or fail the entire dashboard render, which
is a worse failure mode than the architecture needs to accept.

## Decision
**Per-gateway timeout budgets, bounded retries with backoff, and per-card graceful degradation — no single
slow/failed capability call may fail an entire aggregation request.**

- **Timeouts**: every `NexusTransport` call (ADR-007) has an explicit timeout, applied via a `tower::timeout`
  layer on the `nexus-client` HTTP stack (reusing ADR-003's `tower` middleware model). Default timeout budgets
  are set per capability based on the ACL doc's read-vs-write shape (`../ddd/anti-corruption-layers.md`) —
  e.g. a read-mostly `ProductsGateway`/`LegalGateway` query gets a longer allowance than a synchronous,
  user-blocking `SalesGateway` conflict-check the consultant is actively waiting on — with concrete values
  tuned against ADR-012's per-gateway latency metrics once real traffic data exists, not fixed permanently by
  this ADR.
- **Retries**: idempotent read-only queries (e.g. `RequestProductCatalogQuery`, `RequestLearningCatalogQuery`)
  get a small number of retries with exponential backoff on transient failures (timeouts, 5xx). Non-idempotent
  commands (e.g. `RequestCollaborationCommand`, `CreateProposalCommand`) are **not** automatically retried by
  this repo — a retried write against an unknown-outcome prior attempt risks a duplicate side effect in the
  owning capability; instead, a failed command surfaces as an explicit error to the frontend, which the
  consultant can consciously retry (matching the general principle that this repo never silently re-triggers
  business actions it doesn't own).
- **Concurrent fan-out with per-call isolation**: when a handler needs data from multiple gateways for one
  response (e.g. a dashboard composition spanning several cards), each gateway call is issued concurrently
  (via `tokio::join!`/`futures::future::join_all`) and each call's success/failure is captured independently —
  a failure or timeout in one call does not cancel or fail the others. The aggregation handler returns a
  **partial result**: successful cards render normally; a card whose data call failed/timed out renders a
  documented "temporarily unavailable" state (with a retry affordance) instead of the handler returning a
  500 for the whole request. This directly implements the plan's "graceful degradation" requirement at the
  card granularity, matching `../ddd/consultant-experience-context.md`'s framing of cards as independent
  presentation slots, not a single monolithic payload.
- **Circuit breaking (per-gateway)**: if a specific capability's calls are failing at a high rate (tracked via
  ADR-012's per-gateway metrics), the corresponding gateway trips a circuit breaker (e.g. via a `tower`
  middleware layer implementing a standard breaker pattern) to fail fast for a cooldown period rather than
  continuing to spend timeout budget on calls likely to fail — protecting both this repo's own latency and
  avoiding piling load onto an already-struggling upstream capability.
- **Frontend contract**: the BFF's aggregation responses use a shape that distinguishes "card succeeded with
  data," "card failed, retryable," and "card not applicable" (e.g. no permission) so the frontend (using
  TanStack Query's per-query error state, ADR-015) can render each card's state independently without the
  whole dashboard query failing.

## Consequences
**Positive**
- One slow/down external capability degrades only its own card(s), not the whole consultant experience —
  directly matching this repo's role as a composition layer over independently-owned, independently-reliable
  services.
- Explicit non-retry-of-commands policy avoids a class of duplicate-side-effect bugs that would be easy to
  introduce accidentally (e.g. a naive "retry on timeout" wrapper applied uniformly).
- Circuit breaking protects both this repo's own responsiveness and, cooperatively, the health of upstream
  capabilities under load — a good citizen behavior toward services this repo doesn't own.

**Negative / Trade-offs**
- Partial-result responses add real complexity to both the BFF's response shape and the frontend's rendering
  logic (every card-consuming component must handle three states, not just success/failure).
- Per-capability timeout/retry/circuit-breaker tuning is an ongoing operational responsibility, not a
  set-once decision — requires the observability investment from ADR-012 to do well.

## Alternatives Considered
- **Fail the whole aggregation request if any one gateway call fails.** Rejected — directly contradicts the
  plan's explicit "graceful degradation" requirement and would make this repo's reliability strictly worse
  than the sum of its dependencies', which is the opposite of what a composition layer should offer.
- **Uniform, single global timeout/retry policy for all gateways.** Rejected — the ten capabilities have
  meaningfully different read/write and latency-sensitivity shapes per `../ddd/anti-corruption-layers.md`
  (e.g. a synchronous conflict-check the consultant is staring at vs. a background catalog refresh); a single
  policy would either be too aggressive for slow-but-fine capabilities or too lenient for latency-sensitive
  ones.
- **Retry all failed calls, including commands, for simplicity.** Rejected — the duplicate-side-effect risk
  on non-idempotent commands (e.g. accidentally submitting `RequestCollaborationCommand` twice) is a real
  correctness hazard this repo has no way to detect or undo on its own, since the owning capability is
  authoritative for the outcome.

## Relationships
- Depends on: ADR-003 (tower middleware for timeout/circuit-breaker layers), ADR-007 (gateway structure this
  applies to), ADR-012 (metrics driving tuning).
- Informs: ADR-015 (frontend per-card error state consumption).
- Source docs: `../implementation-plan.md` §3.4, `../ddd/anti-corruption-layers.md`, `../ddd/consultant-experience-context.md` §1.2.
