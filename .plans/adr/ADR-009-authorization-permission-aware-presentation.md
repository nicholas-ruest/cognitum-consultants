# ADR-009: Authorization and Permission-Aware Presentation Strategy

## Status
Proposed

## Context
`../research.md` lists "permission-aware presentation" as one of this repo's core responsibilities, but is
explicit elsewhere that authorization *policy* belongs to Armor: "Armor owns... Authorization policy, Access
enforcement... Consultants do not need a general Armor interface... Armor operates beneath the consultant
experience." `../ddd/anti-corruption-layers.md` §10 models this precisely: the `ArmorGateway` ACL exposes only
`PermissionAssertion { consultant_id, capability, scope, expires_at }` — a set of *grants*, never the
underlying policy/rules. `../ddd/consultant-experience-context.md` §1.2 further ties this into a concrete
invariant: `DashboardConfiguration` invariant #1 forbids persisting a card the consultant doesn't currently
hold a Permission Assertion for. This ADR defines how the BFF actually uses those assertions to drive
presentation, without this thin repo ever becoming a second authorization decision-maker.

## Decision
**This repo enforces presentation-only filtering using Permission Assertions sourced from Armor via Nexus; it
never issues, computes, or overrides authorization decisions, and every mutating action is still
independently authorized by the owning capability itself.**

Three layers, all downstream of the same Armor-sourced assertions:

1. **Server-side (BFF) filtering — defense in depth, source of truth for this repo's own state.** On session
   establishment (ADR-008) and on `PermissionAssertionChanged` (`../ddd/domain-events.md` §1), the BFF fetches
   the consultant's current Permission Assertions via `ArmorGateway` (ADR-007) and uses them to: (a) validate
   `DashboardConfiguration` invariant #1 before persisting any card placement (ADR-010's aggregate boundary,
   not just a UI nicety); (b) filter the navigation-entry list returned by `/api/session` (implementation-plan
   §5 Phase 1 deliverable) to only permission-eligible entries; (c) filter which `nexus-client` gateway calls a
   given handler is even willing to attempt, short-circuiting with a 403 before a Nexus round-trip if the
   consultant has no assertion for that capability at all. This is filtering, not policy synthesis — the BFF
   never derives a permission from anything other than an assertion Armor already granted.
2. **Client-side rendering — UX only, never trusted as enforcement.** The frontend receives the same
   permission assertions (via `/api/session` and reactive updates, ADR-011) and uses them to conditionally
   render nav items, dashboard cards, and action buttons for responsiveness (avoiding a round-trip just to
   discover something is hidden). The frontend must never treat "not rendered" as "not accessible" — every
   real mutation still goes through the BFF, which re-checks per point 1.
3. **Downstream authorization — always re-checked by the owning capability, never assumed satisfied by this
   repo's filtering.** Every `nexus-client` command (`../ddd/anti-corruption-layers.md`'s outbound
   commands/queries) still gets authorized by the target capability (e.g. Sales still authorizes
   `RequestCollaborationCommand` itself) via Armor's enforcement propagated through Nexus. This repo's
   filtering is a UX/efficiency optimization and a defense-in-depth check on its *own* aggregates
   (DashboardConfiguration), never a substitute for the real authorization check, matching the same principle
   already established for business policy in the Sales lead-conflict worked example
   (`../ddd/anti-corruption-layers.md` §1: "the Consultants frontend must not independently decide").

**Caching/TTL**: Permission Assertions carry their own `expires_at` (per the ACL shape); the BFF caches them
server-side (in-memory per session, refreshed on expiry or on receiving `PermissionAssertionChanged`) rather
than re-fetching from Armor/Nexus on every request, to avoid adding Armor-round-trip latency to every
dashboard render — bounded by the assertion's own expiry so staleness is never unbounded.

## Consequences
**Positive**
- Keeps this repo strictly within its "own zero business/policy records" mandate (`../research.md`'s Final
  Ownership Principle) while still delivering the "permission-aware presentation" responsibility it does own.
- Three-layer structure (server filter, client render, downstream re-check) means a bug in any one layer
  degrades UX or performance, not security — the owning capability's own authorization is always the real
  backstop.
- Reactive invalidation via `PermissionAssertionChanged` avoids stale nav/dashboard state after an access
  change, directly satisfying `../ddd/consultant-experience-context.md` §1.3's documented event consumption.

**Negative / Trade-offs**
- Server-side caching of assertions introduces a small staleness window between an Armor-side change and this
  repo noticing it via the event — bounded by assertion `expires_at` and by however promptly Nexus delivers
  `PermissionAssertionChanged`, neither of which this repo controls.
- Three layers of the same check (BFF pre-filter, client render, downstream authorization) is intentional
  defense-in-depth but does mean permission logic conceptually "appears" in three places; discipline is needed
  to keep all three reading from the same assertion shape rather than drifting into independent interpretations.

## Alternatives Considered
- **Client-only permission filtering (BFF passes assertions through unfiltered, frontend hides what it
  shouldn't show).** Rejected — leaves `DashboardConfiguration` invariant #1 unenforced at the point where it
  actually matters (persistence), and gives a compromised/modified client no server-side check at all before
  a card placement is saved.
- **This repo computing its own coarse role/permission model instead of consuming Armor's assertions
  directly.** Rejected outright — would directly violate `../research.md`'s explicit statement that Armor owns
  authorization policy; this repo is not permitted to have an opinion about what a consultant is allowed to do
  beyond relaying Armor's own assertions.
- **Skip server-side pre-filtering of nexus-client calls; let every call attempt and rely solely on the
  downstream capability's 403.** Rejected as the sole mechanism — wastes a Nexus round-trip on calls this repo
  already knows will be rejected, and provides a worse UX (a full request/response cycle instead of an
  immediate local check) for no correctness benefit, though the downstream check remains mandatory regardless.

## Relationships
- Depends on: ADR-007 (Armor assertions arrive via `nexus-client`'s `ArmorGateway`), ADR-008 (session identity
  underlying every assertion fetch).
- Informs: ADR-010 (`DashboardConfiguration` invariant #1 enforcement touches persistence), ADR-011
  (permission-change events delivered the same way as other notifications).
- Source docs: `../research.md` §"Role of consultants.cognitum.one", §"Security and Access";
  `../ddd/anti-corruption-layers.md` §10; `../ddd/consultant-experience-context.md` §1.2, §1.3.
