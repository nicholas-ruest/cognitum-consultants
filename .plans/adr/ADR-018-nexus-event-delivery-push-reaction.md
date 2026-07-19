# ADR-018: Nexus → BFF Event Delivery — Push (Reaction Dispatch), Not Polling

## Status
Proposed

## Context
ADR-011 decided SSE for BFF→browser delivery and, as a secondary/staged decision, started Nexus→BFF ingestion
on polling — explicitly "upgradeable to a Nexus-pushed webhook once Nexus's own event-delivery contract is
confirmed," framed as "an additive change" that wouldn't touch the browser-facing SSE contract. PROMPT-30
implemented that polling loop against a guessed endpoint (`events/v1/poll`); a later fix (`d87c88c`, citing an
external "ADR-030 §3") re-guessed it as `GET api/v1/events/poll?consumer=<repo_id>`. Both were deployed and
both 404'd against the real nexus-server — confirmed live in production logs (`consultants-00011-m52`),
recurring every `event_poll_interval_seconds`.

Direct investigation against the real, deployed `nexus-server` (Cloud Run, `cognitum-20260110`), rather than a
third guess, established two facts:

1. **Nexus's real API has no consumer-facing poll route at all.** Its own public `GET /openapi.json` lists
   every route it exposes; the only `events`-related ones are `POST /api/v1/events` (event *ingestion into*
   nexus — the opposite direction) and `GET /api/v1/graph/events` (a graph/topology introspection endpoint,
   not a delivery mechanism). There is nothing shaped like "pull my events since a cursor" to find a correct
   path for.
2. **Nexus's real architecture is push-based reaction dispatch.** Pulling the live `nexus-server` image and
   reading its baked-in `config/registries/*.json` plus its binary's own strings confirms the model: nexus
   ingests a domain event via `POST /api/v1/events`, looks up `consumers.json` for every
   `{event_type, consumer_repo, reaction_handler}` registration matching that event type, and calls **out**
   to each matching `consumer_repo`'s registered `base_url` (from `repos.json`) at a route under `/reactions/`
   — carrying `x-cognitum-event-type` / `x-cognitum-reaction-handler` / `x-cognitum-timeout` headers. Nexus
   validates this wiring at its own boot time (`"ADR-007 boot validation: declared reactions without a
   registered handler"`, `"consumer/reaction mismatch: consumer_repo=..., reaction.target_repo=..."`), and
   every one of its 14 currently-registered repos (`cognitum-sales`, `cognitum-armor`, `cognitum-capacity`,
   etc.) is wired this same way in `reactions.json`/`consumers.json`. This is not a partially-built feature —
   it is nexus's one, already-proven, event-delivery mechanism.

Also established: `cognitum-consultants` is not currently registered anywhere in nexus's config
(`repos.json` has no entry — no `base_url`, no `service_account`; `consumers.json`/`reactions.json` have no
entries naming it). Nexus has no address to deliver to and no subscription telling it to try. This is a
nexus-side onboarding gap, not a bug in this repo's request path — no path fix on this side can produce
events nexus was never told to send here.

The failing poll loop (`event_ingestion::run_polling_loop`) has been disabled in production (`6aa742a`) so it
stops erroring against a route that doesn't exist, pending this decision.

## Decision
**Replace Nexus→BFF polling with an inbound push receiver**, matching nexus's real, already-proven reaction
dispatch model, exactly the "additive... upgrade to a Nexus-pushed webhook" ADR-011 already anticipated.

- **New inbound route**: `bff-api` adds `POST /reactions/:reaction_handler` (path convention matching the
  `/reactions/` segment found in nexus's own binary), one route handling every registered reaction for this
  repo rather than one route per handler — `reaction_handler` is a path parameter, not a fixed string, since
  `consumers.json` may eventually register more than one `event_type`/`reaction_handler` pair for this repo.
- **Caller verification**: the handler must verify the call genuinely came from nexus before trusting the
  payload — mirroring, in reverse, the Cloud-Run-to-Cloud-Run Google-signed identity-token pattern this repo
  already relies on for its *outbound* calls to nexus (`ADR-029`, `crates/nexus-client/src/reqwest_transport.rs`).
  The exact expected caller identity (nexus's own service account) is confirmed with whoever owns
  nexus-server's config as part of the `repos.json` registration below, not assumed here.
- **Payload**: nexus's `EventEnvelope` wire shape — the same shape `event_ingestion.rs` already models as
  `CapabilityEventReceived` for the (now-disabled) poll path. That mapping code is reused, not rewritten.
- **Ingestion path unchanged**: a verified push is fed into the same `notification_repository` /
  `action_queue_repository` / `workflow_session_repository` / `EventPublisher` pipeline the poll loop would
  have used (ADR-010, ADR-014's cross-instance `NOTIFY`/`LISTEN` fan-out) — this ADR only changes *how an
  event reaches the BFF process*, not what happens once it has. The `NotificationItem`/`ActionQueueEntry`
  aggregates, the SSE layer, and ADR-011's BFF→browser decision are all unaffected.
- **Idempotency**: a push can be retried by nexus (timeout, transient 5xx), so the handler must dedupe by
  event id the same way the poll path's "idempotent-save safety net" already does (`event_ingestion.rs`'s
  module docs) — no new dedup design needed, the existing per-event-id idempotent save is retry-safe
  regardless of which direction delivered the event.
- **Nexus-side prerequisite (external, not this repo's to execute)**: whoever owns nexus-server's config must
  add `cognitum-consultants` to `repos.json` (a real `base_url` reachable from nexus — e.g. this service's
  Cloud Run URL or `consultants.cognitum.one` — and a dedicated `service_account`, matching every other
  registered repo's pattern rather than sharing the generic default compute service account), plus one
  `reactions.json` + `consumers.json` entry per `event_type` this repo needs to receive. This repo has no
  write access to that config; it is tracked here as a hard dependency, not an action item this ADR can
  itself close out.

## Consequences
**Positive**
- Matches nexus's real, live, already-proven architecture instead of a third guessed contract — the same
  registration shape already working for 14 other repos.
- Removes the wasted/erroring poll loop entirely; no polling interval, no wasted calls against a nonexistent
  route.
- Lower latency than polling ever would have been — nexus calls in the moment an event is routed, not up to
  `event_poll_interval_seconds` later.
- ADR-011's BFF→browser SSE decision, and every aggregate/persistence decision downstream of ingestion
  (ADR-010, ADR-014), are untouched — confirms ADR-011's framing that this hop could change independently.

**Negative / Trade-offs**
- Requires a nexus-side config change this repo doesn't control — a coordination/external dependency, not a
  self-service fix. Until nexus registers `cognitum-consultants`, this repo receives zero nexus-originated
  events (already true today, so no regression — but also no improvement until that lands).
- A new authenticated inbound HTTP surface is new attack surface that didn't exist before; caller
  verification must be genuinely enforced (reject anything not signed by nexus's real service identity), not
  a rubber-stamp check.
- `event_ingestion.rs`'s cursor/watermark bookkeeping (built for polling's "since last successful poll"
  model) is no longer needed for delivery ordering — retained only if still useful for the idempotent-save
  dedup; otherwise it becomes dead code to remove alongside this change.

## Alternatives Considered
- **Keep guessing a poll path.** Rejected outright — nexus's own public `/openapi.json` and its live binary
  both confirm no such route exists. A third guess would repeat the exact failure mode `6aa742a` just stopped.
- **Ask nexus's owner to add a one-off poll endpoint just for this repo.** Rejected — inconsistent with the
  architecture nexus has already built and validated for every other consumer; would be more nexus-side work
  than registering into the existing, proven `repos.json`/`consumers.json`/`reactions.json` model, and would
  leave `cognitum-consultants` as a permanent special case.
- **Do nothing until nexus proactively reaches out.** Rejected — the receiver endpoint is this repo's own
  code and can be built and deployed now; there is no reason to block it on the nexus-side registration
  landing first, since it's inert (nothing calls it) until nexus actually does.

## Relationships
- Fulfills the upgrade path ADR-011 explicitly reserved ("upgradeable to a Nexus-pushed webhook... an
  additive change").
- Depends on: ADR-007 (Nexus integration pattern / `NexusTransport` abstraction, mirrored in reverse for the
  inbound side), ADR-010 (notification/action-queue persistence the receiver feeds), ADR-014 (cross-instance
  `NOTIFY`/`LISTEN` fan-out, unchanged), ADR-016 (resilience conventions — idempotency, retry-safety).
- Blocked on (external, nexus-owned, not tracked by an ADR in this repo): nexus's `repos.json` /
  `consumers.json` / `reactions.json` registration for `cognitum-consultants`.
- Source docs: `ADR-011-event-notification-delivery-sse.md`; live investigation findings (this session) —
  `nexus-server`'s `/openapi.json`, its `config/registries/*.json`, and its binary's own route/validation
  strings.
