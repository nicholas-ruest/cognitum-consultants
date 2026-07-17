# ADR-008: Authentication and Session Strategy

## Status
Proposed

## Context
`../implementation-plan.md` §6 risk #2 flags this as an explicitly open, blocking-for-Phase-1 question:
"How Armor issues/validates credentials, what the BFF verifies, and how identity propagates through Nexus to
sub-business services is unspecified in `research.md`." `../research.md` states Armor "operates beneath the
consultant experience" and this repo "do[es] not need a general Armor interface" — meaning this repo consumes
identity/authorization signals but does not own or directly implement Armor's authorization policy (that is
ADR-009's concern). This ADR is narrower: how does a consultant's browser session get authenticated at all,
and how does that identity reach Nexus/Armor on every BFF→Nexus call (ADR-007)?

Because the real Armor-issued-credential integration details are unknown today, this ADR defines a strategy
that is correct in shape now and pluggable against the real provider once confirmed — consistent with this
plan's GOAP-style replanning approach (`../implementation-plan.md` §1).

## Decision
**BFF-managed server-side session, backed by an upstream identity assertion from Armor (via Nexus), with a
defined interim dev-stub for pre-Armor-integration phases.**

- **Session model**: the browser never holds a long-lived credential capable of calling Nexus/Armor directly.
  On successful login, the BFF (`auth` crate, ADR-004) establishes a server-side session and gives the browser
  an opaque session identifier in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie. This follows the standard
  "BFF pattern" for session handling, consistent with ADR-006's decision that the BFF, not the SPA, is the
  trust boundary talking to Nexus.
- **Identity source**: authentication itself is delegated to an OIDC/OAuth2-compatible identity provider
  fronted by Armor (`armor.cognitum.one`) — the specific provider (Armor-native, or Armor wrapping a
  third-party IdP) is Armor's implementation detail, opaque to this repo per `../ddd/anti-corruption-layers.md`
  §10's `ArmorGateway` framing ("Permission Assertion... never the underlying authorization policy/rules
  themselves"). The BFF performs a standard OIDC authorization-code exchange against whatever endpoint Armor
  (via Nexus, or a documented direct auth endpoint if Nexus's routing model excludes the login handshake
  itself) exposes, and stores the resulting upstream token(s) server-side, associated with the session — never
  forwarded to or readable by the browser.
- **Propagation to Nexus**: every outbound `NexusTransport` call (ADR-007) attaches the session's associated
  upstream credential (or a BFF-minted, short-lived assertion derived from it, if Nexus's contract prefers
  that shape) so Nexus can propagate identity to Armor and onward to the target capability, per
  `../research.md`'s "Authentication propagation" responsibility for Nexus.
- **Session storage**: session state (opaque id → upstream token/claims mapping) is stored in this repo's own
  persistence layer (ADR-010), not in-memory-only, so sessions survive a BFF instance restart/redeploy and
  work correctly under the horizontal scaling implied by ADR-014 — this reuses ADR-010's datastore rather than
  introducing a separate session store, keeping the persistence surface area small per this repo's "owns
  minimal state" principle.
- **Interim dev-stub**: for Phase 0/1 work that precedes a confirmed, integration-ready Armor auth endpoint,
  the `auth` crate defines the same session interface behind a feature flag / config switch backed by a
  trivial local dev-login (e.g. a fixed set of dev consultant identities, no real credential check) — this
  keeps the rest of the system (permission-aware nav, ADR-009) developable without blocking on Armor's
  readiness, and is explicitly never enabled outside local/dev environments (enforced at startup by refusing
  to boot the stub provider if the environment is not flagged as `dev`).

## Consequences
**Positive**
- Keeps long-lived credentials out of browser JavaScript entirely, minimizing XSS blast radius — consistent
  with the BFF-owns-the-Nexus-boundary model already established by ADR-006/ADR-007.
- The interim dev-stub unblocks Phase 1 work (`../implementation-plan.md` §5) without waiting on Armor's
  actual readiness, while keeping the real integration point (the `auth` crate's session interface) stable
  across the swap.
- Session storage in the shared datastore (ADR-010) avoids introducing a second stateful dependency (e.g. a
  dedicated Redis) purely for sessions, unless ADR-010's later performance findings justify adding a cache
  layer in front of it.

**Negative / Trade-offs**
- This ADR's specifics (exact OIDC flow details, whether Nexus itself brokers the login handshake or Armor
  exposes a separate auth endpoint) remain genuinely unknown until Armor's real contract is available —
  flagged explicitly as **provisional on confirmation**, per `../implementation-plan.md` §6 risk #2. This ADR
  should be revisited (not silently reinterpreted) once that contract is known.
- Server-side session storage adds a dependency on ADR-10's datastore being available and correctly deployed
  before login works at all — a stronger coupling than a stateless-JWT-in-browser approach would have, traded
  deliberately for the XSS/security benefit above.

## Alternatives Considered
- **SPA holds a JWT directly (e.g. in `localStorage` or a non-HttpOnly cookie), calls Nexus indirectly only
  through the BFF but authenticates itself with the JWT on every BFF call.** Rejected — exposes a
  Nexus/Armor-trusted credential to browser JavaScript, meaningfully increasing XSS impact; also weaker fit
  with ADR-006's "BFF is the sole trust boundary" model.
- **Stateless BFF sessions (self-contained encrypted cookie, no server-side store).** Considered as a way to
  avoid the ADR-010 dependency above. Rejected as the primary approach because revocation (e.g. on Armor-side
  permission change, per `../ddd/domain-events.md`'s `PermissionAssertionChanged`) is materially harder with
  a stateless cookie that can't be invalidated server-side before its expiry — a real requirement given
  `../ddd/consultant-experience-context.md` §1.3 lists this event as something the Workspace context reacts
  to.
- **No interim dev-stub; block all Phase 1 work on real Armor integration.** Rejected — contradicts the plan's
  own phased/GOAP structure (`../implementation-plan.md` §1), which expects replanning as facts like Armor's
  readiness become known, not a hard stop.

## Relationships
- Depends on: ADR-004 (`auth` crate), ADR-007 (propagation on every Nexus call).
- Informs: ADR-009 (permission assertions ride the same identity), ADR-010 (session storage reuses the chosen
  datastore).
- Source docs: `../ddd/anti-corruption-layers.md` §10, `../implementation-plan.md` §6 risk #2.
