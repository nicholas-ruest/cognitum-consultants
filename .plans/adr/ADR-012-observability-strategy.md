# ADR-012: Observability Strategy

## Status
Proposed

## Context
`../implementation-plan.md` §3.4 lists an observability ADR as required, with an explicit emphasis on
"correlation IDs across Nexus hops." This matters more than it would for a typical service because this
repo's BFF fans out to up to ten external capabilities per request (`../ddd/anti-corruption-layers.md`), and a
single consultant-facing failure (e.g. a slow dashboard) could originate in any one of: this repo's own
handler logic, the Nexus routing layer, or a specific sub-business capability behind it. Without correlation
across that chain, diagnosing "why was this request slow/wrong" degrades into guesswork. `../implementation-
plan.md` §6 risk #9 also flags that aligning observability conventions with sibling Cognitum One Rust services
(if any) would reduce integration friction, though no such convention is confirmed to exist yet.

## Decision
**`tracing` for structured logging and spans, OpenTelemetry (via `tracing-opentelemetry`) for distributed
tracing across Nexus hops, and the `metrics` crate with a Prometheus exporter for metrics — all wired as Axum
`tower` middleware (ADR-003), with a mandatory correlation/request ID on every request.**

- **Logging**: the `tracing` crate, with `tracing-subscriber` configured for structured JSON output in
  non-local environments (human-readable formatting for local dev). Every log line carries the request's
  correlation ID as a span field, not as ad hoc string interpolation, so logs remain machine-parseable and
  filterable per request.
- **Correlation IDs across Nexus hops**: an Axum middleware layer (`tower::Layer`, consistent with ADR-003's
  middleware model) generates a correlation ID for every inbound request (or accepts one from an inbound
  header if the SPA/a caller already has one), stores it in the request's tracing span, and the `nexus-client`
  crate (ADR-007) propagates it as an outbound header on every Nexus call. This is the mechanism that lets a
  BFF log line, a Nexus log line, and a target capability's log line be joined by one ID during an incident —
  the single most direct answer to the plan's explicit ask.
- **Distributed tracing**: `tracing-opentelemetry` exports spans in the W3C Trace Context format
  (`traceparent` header), propagated the same way as the correlation ID above, so that if/when Nexus and the
  sub-business services adopt OpenTelemetry themselves, this repo's spans join the same distributed trace
  without any protocol renegotiation — W3C Trace Context is the interoperable standard, not a
  Cognitum-One-specific format, minimizing the risk that this repo's choice conflicts with whatever sibling
  services eventually standardize on (risk #9).
- **Metrics**: the `metrics` crate (facade) with a Prometheus exporter (`metrics-exporter-prometheus`),
  exposing a `/metrics` endpoint scraped by whatever infrastructure ADR-014 lands on. Minimum metric set at
  launch: per-route request count/latency histograms, per-Nexus-gateway call count/latency/error-rate
  (critical for ADR-016's resilience decisions to be evaluated against real data), and SSE connection count
  (ADR-011).
- **Frontend**: kept deliberately minimal for v1 — a lightweight client-side error boundary/reporting hook
  that forwards uncaught frontend errors to a BFF logging endpoint (tagged with the same correlation ID
  pattern where available, e.g. from the last API response). A full RUM (real-user-monitoring) tool is
  explicitly out of scope for this ADR; adopt one later via a follow-up ADR if warranted by real need, rather
  than speculatively now.

## Consequences
**Positive**
- Directly satisfies the plan's explicit ask for correlation IDs across Nexus hops — the single hardest
  observability problem this architecture creates (fan-out across an opaque routing layer to ten capabilities)
  has a concrete, load-bearing answer.
- W3C Trace Context standardization keeps this repo's tracing choice compatible with whatever the rest of
  Cognitum One adopts, rather than betting on a proprietary format.
- Per-gateway metrics give ADR-016's resilience/timeout decisions (and future tuning) a real data source
  instead of guesswork.

**Negative / Trade-offs**
- Running an OpenTelemetry pipeline (collector, backend) is additional infrastructure that must be provisioned
  (ADR-014's concern) — this ADR fixes the in-process instrumentation choice, not the full collection/storage
  backend, which may itself need a small follow-up decision once a deployment target is confirmed.
- Structured JSON logging is less convenient to read by eye during local development than plain text —
  mitigated by using human-readable formatting locally and JSON only in deployed environments.

## Alternatives Considered
- **Plain `log` crate + `env_logger`, no spans/tracing.** Rejected — `log`'s flat, non-hierarchical model can't
  represent the nested-span shape needed to correlate one inbound request across ten possible outbound Nexus
  calls; `tracing`'s span model is a direct fit for this repo's specific fan-out shape.
- **Vendor-specific tracing SDK (e.g. a specific APM vendor's proprietary agent) instead of OpenTelemetry.**
  Rejected — locks the correlation mechanism to one vendor's format, working against risk #9's goal of
  cross-repo consistency; OpenTelemetry's vendor-neutral W3C-standard propagation is strictly more flexible
  for a system with several sibling Rust services that haven't yet agreed on tooling. A specific backend can
  still be chosen later (ADR-014-adjacent) without re-instrumenting code.
- **StatsD-style metrics instead of Prometheus.** Rejected — Prometheus's pull-based `/metrics` model needs no
  additional metrics-relay infrastructure beyond a scraper, simpler to stand up for Phase 0/1 than a
  StatsD-compatible push pipeline, and is the more common default in the Rust ecosystem's `metrics` crate
  tooling.

## Relationships
- Depends on: ADR-003 (Axum/tower middleware for correlation ID + tracing), ADR-007 (correlation ID
  propagation to Nexus).
- Informs: ADR-014 (deployment must provision a tracing/metrics backend), ADR-016 (per-gateway metrics inform
  resilience tuning).
- Source docs: `../implementation-plan.md` §3.4, §6 risk #9.
