# ADR-011: Event and Notification Delivery Mechanism — Server-Sent Events

## Status
Proposed

## Context
`../research.md` requires "one notification centre" and "one task list" as part of the unified consultant
experience. `../ddd/consultant-experience-context.md` §2 and `../ddd/domain-events.md` §2 define the
`NotificationItem`/`ActionQueueEntry` aggregates and the `CapabilityEventReceived` envelope that feeds them
from any of the ten external contexts via Nexus. `../implementation-plan.md` §3.4 lists the delivery mechanism
(polling vs SSE vs WebSocket vs Nexus webhook-to-BFF) as an open ADR, and Phase 3 (§5) is explicitly gated on
resolving it. The traffic shape is mostly one-directional: capability events flow inward (BFF ingests from
Nexus, however Nexus delivers them to the BFF — polling or webhook, itself a separate concern from
BFF-to-browser delivery) and need to reach the consultant's open browser tab promptly; the consultant's own
actions (marking read, starting an action-queue item) are ordinary request/response calls, not part of this
push channel.

## Decision
**Server-Sent Events (SSE)** is the mechanism for pushing notification/action-queue updates from the BFF to
the browser. Separately, Nexus→BFF ingestion uses polling initially, upgradeable to a Nexus-pushed webhook
once Nexus's own event-delivery contract is confirmed (an independent decision from the browser-facing
channel, addressed below).

**BFF → browser (this ADR's primary decision): SSE.**
- Axum has first-class SSE support (`axum::response::sse`, per ADR-003), avoiding a second framework or
  library.
- The traffic is fundamentally unidirectional (server pushes notification/action-queue changes; the
  consultant's responses — mark read, start an action — are separate normal `POST`/`PATCH` calls against
  `/api/*`, not sent back over the push channel). SSE fits a unidirectional push far more simply than
  WebSockets, which would provision full-duplex capability this repo doesn't need.
- SSE runs over plain HTTP/1.1 or HTTP/2, works through standard corporate proxies/load balancers more
  reliably than WebSockets (no protocol upgrade handshake to be blocked), and the browser's native
  `EventSource` API handles automatic reconnection, removing a class of custom reconnect logic this repo would
  otherwise have to write for WebSockets.
- **Documented fallback**: if a consultant's network environment blocks long-lived SSE connections (rare but
  possible in some corporate proxy setups), the frontend falls back to short-interval polling of a
  `/api/notifications/poll` endpoint (`../implementation-plan.md` §3.4's polling option, here demoted to a
  fallback rather than the primary mechanism) — this fallback is a Phase 3 nice-to-have, not a Phase 3
  blocker.

**Nexus → BFF ingestion (secondary, staged decision):** the BFF ingests `CapabilityEventReceived` envelopes
from Nexus via polling in the initial implementation (simplest, requires no inbound-webhook infrastructure or
Nexus-side push contract to be confirmed first), feeding the same internal event bus that drives the SSE
stream to browsers. If/when Nexus's own event-routing capability (`../research.md`: Nexus owns "Event
routing") supports pushing events to a Nexus-registered webhook, the BFF can add a webhook receiver endpoint
as an additive change — the SSE-to-browser layer and the `NotificationItem`/`ActionQueueEntry` ingestion logic
(ADR-010) are unaffected either way, since both sit behind the same internal event bus abstraction.

## Consequences
**Positive**
- No new framework/library needed beyond what ADR-003 already provides.
- Native browser reconnection (`EventSource`) reduces custom client-side resilience code.
- Decoupling "how the BFF learns about events" (polling now, webhook later) from "how the browser learns about
  them" (always SSE) lets Nexus's own event-delivery maturity evolve independently without touching the
  browser-facing contract.

**Negative / Trade-offs**
- SSE is one-directional; any future genuinely bidirectional real-time need (none identified today) would
  require WebSockets as an addition, not a replacement.
- Long-lived SSE connections mean the BFF must handle a large number of held-open connections per instance —
  relevant to ADR-014's scaling/connection-affinity design (a consultant's SSE stream should ideally stay
  pinned to one BFF instance, or the internal event bus needs to be shared across instances via Postgres
  LISTEN/NOTIFY or similar — a concrete mechanism to finalize when ADR-014's scaling approach is implemented).
- Initial Nexus-ingestion-by-polling adds latency between an upstream event occurring and it reaching a
  consultant's browser, bounded by the polling interval — acceptable for a v1 notification centre, revisited
  once Nexus's webhook capability is confirmed.

## Alternatives Considered
- **WebSockets.** Rejected as the primary mechanism — unnecessary complexity (full-duplex) for a
  predominantly server-to-client feed; consultant actions already have a natural request/response path.
  Revisit only if a genuinely bidirectional low-latency need emerges (none identified in
  `../ddd/domain-events.md`).
- **Polling only (no push channel at all).** Rejected as the primary mechanism — a "unified notification
  centre" implies near-real-time updates; pure polling forces a latency/load tradeoff (frequent polling wastes
  resources, infrequent polling feels stale) that SSE avoids for the common case, while still being kept as
  the documented fallback for constrained network environments.
- **Nexus webhook directly to the browser (no BFF intermediary).** Not viable — contradicts the Nexus-only,
  BFF-mediated integration model (`../implementation-plan.md` §2.3); the browser must never talk to Nexus
  directly.

## Relationships
- Depends on: ADR-003 (Axum SSE support), ADR-006 (`/api/*` namespace), ADR-007 (Nexus event ingestion),
  ADR-010 (notification/action-queue persistence feeding the stream).
- Informs: ADR-014 (connection-affinity/scaling consideration for long-lived SSE connections), ADR-015
  (frontend consumes SSE to drive TanStack Query cache invalidation).
- Source docs: `../ddd/domain-events.md` §2, §3; `../ddd/consultant-experience-context.md` §2;
  `../implementation-plan.md` §3.4, §5 Phase 3.
