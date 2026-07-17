# ADR-010: Persistence Strategy for This Repo's Own Aggregates

## Status
Proposed

## Context
Per `../research.md`'s Final Ownership Principle and `../ddd/consultant-experience-context.md`'s repo-wide
invariant, this repo "owns zero business records" — but it does own real, durable view-state:
`DashboardConfiguration`, `ConsultantPreferences`, `CrossCapabilityWorkflowSession` (Consultant Workspace
context), and `NotificationItem`, `ActionQueueEntry` (Notification & Action Queue context), each with
repository interfaces already specified in `consultant-experience-context.md` §1.4/§2.4. Those interfaces
require: idempotent-ingestion uniqueness (`origin_capability, origin_event_id`), housekeeping sweeps
(`expire_older_than`, `purge_older_than`), and simple per-consultant lookups. `../implementation-plan.md` §6
risk #6 flags the storage choice as open ("embedded DB, Postgres, Redis, or delegated"). `../implementation-
plan.md` §3.4 lists this as a required ADR. This repo intentionally owns minimal state, which argues for
simplicity, but the state it does own must survive restarts, support the housekeeping/uniqueness invariants
above, and work correctly if the BFF is horizontally scaled (a real possibility per ADR-014).

## Decision
**PostgreSQL, accessed via `sqlx` (async, compile-time-checked queries), is the datastore for all of this
repo's own aggregates**, implemented behind the `persistence` crate's repository trait implementations
(ADR-004).

Rationale:
- **Multi-instance correctness.** A horizontally-scaled BFF (ADR-014) cannot rely on any single instance's
  local memory or local file as the source of truth for `DashboardConfiguration`/`ConsultantPreferences`/
  notification state — a consultant's second request may land on a different instance. An embedded
  single-file store (SQLite, `redb`) would require a shared-filesystem or leader-election scheme to remain
  correct under horizontal scaling; Postgres is a shared, network-accessible datastore by default, avoiding
  that problem entirely.
- **Idempotent ingestion as a database-native constraint.** The `(origin_capability, origin_event_id)`
  uniqueness invariant on `NotificationItem` and `ActionQueueEntry` (`consultant-experience-context.md` §2.2)
  maps directly onto a Postgres unique index/constraint, giving atomic, race-free deduplication under
  concurrent event delivery — a correctness property harder to guarantee with a cache-oriented store like
  Redis, whose consistency/durability guarantees are weaker by design.
- **Housekeeping sweeps.** `expire_older_than`/`purge_older_than` map to straightforward scheduled `DELETE`/
  `UPDATE ... WHERE expires_at < now()` queries; Postgres's transactional guarantees make these safe to run
  concurrently with normal read/write traffic.
- **Boring, mature default.** Postgres is an operationally well-understood, widely-supported choice for a
  team that may not yet have settled cross-repo Rust conventions (`../implementation-plan.md` §6 risk #9) —
  it does not foreclose later introducing a cache layer (see below) once real access patterns are known.
- **`sqlx` over `diesel`**: `sqlx` is async-native (fits Tokio/Axum, ADR-003) and uses macro-checked raw SQL
  rather than a query-builder DSL — appropriate here because these aggregates are simple (mostly key lookups
  and narrow upserts per the documented repository interfaces), so `diesel`'s heavier ORM/query-builder
  machinery buys little, while `sqlx`'s compile-time query checking still catches schema drift without hiding
  the actual SQL being run — useful when the schema needs to closely mirror the DDD-documented invariants for
  reviewability.
- **Migrations**: managed via `sqlx-cli` (or `refinery`), version-controlled under `persistence/migrations/`,
  run automatically at BFF startup in non-production environments and via an explicit CI/CD deploy step in
  production (tied into ADR-014's pipeline).

**Explicitly out of scope for v1**: a dedicated cache (e.g. Redis) in front of Postgres for hot paths like
unread-notification counts. Nothing in the current repository interfaces requires it; if profiling later shows
it's needed (e.g. very high-frequency unread-count polling before ADR-011's SSE fully replaces polling), that
is a additive, backward-compatible optimization layered on top of this ADR, not a replacement for it — would
warrant its own follow-up ADR rather than reopening this one, since it wouldn't change the underlying
source-of-truth decision.

## Consequences
**Positive**
- One well-understood datastore backs every one of this repo's five aggregates — no per-aggregate storage
  fragmentation to reason about operationally.
- Correctness under horizontal scaling and concurrent event ingestion is handled by Postgres's own guarantees
  rather than bespoke in-app coordination logic.
- `sqlx`'s compile-time query checking catches a class of bugs (schema/query mismatch) at build time, fitting
  this repo's overall preference for compiler-checked correctness (Rust-first, ADR-002).

**Negative / Trade-offs**
- Introduces a real operational dependency (a running Postgres instance) for even the "thin" state this repo
  owns — heavier than an embedded database, though justified above by the multi-instance correctness
  requirement.
- `sqlx`'s compile-time checks require a reachable database (or its offline query-cache feature) at build
  time in CI — a small addition to Phase 0's CI setup (`../implementation-plan.md` §5 Phase 0) that must be
  accounted for when this ADR is implemented.

## Alternatives Considered
- **Embedded database (SQLite or `redb`), one file per BFF instance.** Rejected as the primary store — breaks
  under horizontal scaling (ADR-014) without an additional replication/leader-election layer that would add
  more complexity than just using Postgres directly. Would only make sense if this repo committed to a
  single-instance-only deployment permanently, which is not assumed here.
- **Redis as the primary store.** Rejected — Redis's default durability/consistency trade-offs (optimized for
  cache/ephemeral use) are a weaker fit for the idempotent-ingestion uniqueness invariant than a relational
  unique constraint, and Redis's data-modeling primitives are a worse match for the relational shape of
  `DashboardConfiguration`'s card placements. Remains a good candidate for a *future, additive* cache layer,
  not the source of truth.
- **Delegate persistence to Nexus or another Cognitum One service instead of storing anything locally.**
  Rejected — contradicts the plan's own framing that this state (dashboard layout, preferences, notification
  read-state) is legitimately BFF-local view-state, not a business record any sub-business service should own
  (`../implementation-plan.md` §6 risk #5/#6 explicitly distinguish this from business data); delegating it
  would misattribute ownership in the wrong direction.
- **`diesel` instead of `sqlx`.** Considered and rejected primarily on async-runtime fit (`diesel`'s async
  story is comparatively newer/less mature than its sync-first core) and on preferring visible SQL over a
  query-builder DSL for these simple, DDD-invariant-driven aggregates.

## Relationships
- Depends on: ADR-004 (`persistence` crate), ADR-002 (Rust-native driver).
- Informs: ADR-008 (session storage reuses this datastore), ADR-011 (notification ingestion writes here),
  ADR-014 (Postgres as a deployed dependency).
- Source docs: `../ddd/consultant-experience-context.md` §1.4, §2.4, §2.5; `../implementation-plan.md` §6
  risk #6.
