# ADR-014: Deployment and Runtime Topology

## Status
Proposed

## Context
`../implementation-plan.md` §3.4 requires a deployment/infra ADR, and §6 risk #8 flags that "no target
environment (cloud, container orchestration, CDN) is specified yet." This repo produces two build artifacts —
a Rust BFF binary (ADR-003/ADR-004) and a static SPA bundle (ADR-005/ADR-006) — plus a real dependency on
Postgres (ADR-010). Given the target orchestration platform is genuinely unknown today, this ADR fixes what
*can* be decided now (how the artifacts are built and packaged, and the default topology) in a way that stays
portable across whatever orchestrator is eventually chosen, rather than guessing a specific cloud platform.

## Decision
**Containerize the BFF as a minimal OCI image that also serves the compiled SPA (per ADR-006's default),
built via a multi-stage Dockerfile with `cargo-chef` for dependency-layer caching; target any OCI-compatible
orchestrator rather than a named platform; Postgres (ADR-010) is a provisioned dependency, not bundled in the
app image.**

- **Build**: a multi-stage `Dockerfile` — stage 1 uses `cargo-chef` to cache dependency compilation separately
  from application code (keeping CI rebuild times low as the workspace, ADR-004, grows more crates); stage 2
  builds the `frontend/` Vite bundle; the final stage copies the compiled `bff-api` binary and the SPA's
  static assets into a minimal runtime base (a `distroless` or `slim` image — no shell/package manager beyond
  what's needed for TLS certs), keeping the deployed image small and reducing attack surface.
- **Static asset serving default**: per ADR-006, the BFF serves the SPA's static assets itself via
  `tower-http` in the default/initial deployment — meaning the container image above is the single deployable
  unit for Phase 0–2. Splitting static-asset serving out to a CDN/edge origin (with the SPA calling the BFF's
  API origin cross-origin) remains an explicit, documented future optimization — not implemented now, not
  precluded by this decision, since the SPA build output is identical either way.
- **Orchestration target**: left deliberately generic — the image is built to run under any OCI-compatible
  orchestrator (Kubernetes, Nomad, a managed container service, etc.) rather than committing to a specific
  platform absent a stated requirement (risk #8). Concretely, this means: configuration via environment
  variables (12-factor style, per the `config` crate, ADR-004), health-check endpoint(s) suitable for
  liveness/readiness probes (extending Phase 0's existing health-check deliverable,
  `../implementation-plan.md` §5), and graceful-shutdown handling (draining in-flight requests and open SSE
  connections, ADR-011, on `SIGTERM`) — all orchestrator-agnostic requirements that any real target will need
  regardless of which one is eventually chosen.
- **Horizontal scaling implications**: because ADR-010 already requires Postgres (not an embedded store) and
  ADR-011's SSE connections are long-lived, running multiple BFF instances behind a load balancer is supported
  by this topology, with one caveat to resolve when a concrete orchestrator is chosen: SSE connections should
  either use sticky/session-affinity routing, or the internal event bus feeding SSE streams needs a
  cross-instance fan-out mechanism (e.g. Postgres `LISTEN`/`NOTIFY`) so a notification ingested by one instance
  reaches a consultant whose SSE connection is held by another. This ADR flags the requirement; the specific
  mechanism is an implementation detail to finalize once real scaling needs are known, not blocking Phase 0–2.
- **CI/CD**: the existing CI pipeline (`../implementation-plan.md` §5 Phase 0: `cargo check`/`clippy`/`test` +
  `npm run build`/lint, per ADR-013) gates merges; a separate deploy pipeline builds and pushes the container
  image on merge to main (or on tag, if a release-branch model is later adopted), then applies whatever
  orchestrator-specific deploy step is appropriate for the eventually-chosen target. Database migrations
  (ADR-010) run as an explicit pipeline step before the new image is rolled out, not as an implicit
  side-effect of application startup in production.

## Consequences
**Positive**
- Nothing here blocks on choosing a specific cloud/orchestrator — the image and its runtime contract
  (env-var config, health checks, graceful shutdown) work under essentially any modern container platform.
- `cargo-chef` layer caching keeps CI image-build times reasonable as the workspace grows across Phase 4's
  capability integrations.
- Single-image-serves-both-API-and-SPA default keeps Phase 0–2 deployment simple, while the CDN-split
  optimization remains available without an architecture change later.

**Negative / Trade-offs**
- Deferring the specific orchestrator/cloud decision means some real operational questions (autoscaling
  policy, secrets management specifics, network topology/VPC design) are not answered by this ADR and will
  need a follow-up decision once a target is chosen — explicitly acknowledged rather than guessed at.
- The SSE cross-instance fan-out requirement (Postgres `LISTEN`/`NOTIFY` or equivalent) is flagged but not
  fully specified here — a gap to close before this repo actually runs at more than one instance in
  production.

## Alternatives Considered
- **Commit to a specific cloud/orchestrator now (e.g. a named managed Kubernetes service).** Rejected —
  nothing in `../research.md` or `../implementation-plan.md` specifies one, and guessing wrong would mean
  redoing this ADR later for no benefit; the orchestrator-agnostic contract above is the more defensible
  default given real uncertainty (risk #8).
- **Split static-asset serving to a CDN from day one.** Rejected as the *default* — adds deployment complexity
  (a second artifact pipeline, CORS configuration between SPA and API origins) before there's a demonstrated
  need (real traffic/latency data); the single-image default is simpler for Phase 0–2 and the split remains a
  drop-in future option per ADR-006.
- **Serverless/FaaS deployment of the BFF (e.g. request-per-invocation functions).** Rejected — poor fit for
  ADR-011's long-lived SSE connections, which need a persistent process, not a short-lived function
  invocation; would force SSE to be redesigned around a different mechanism (e.g. a separate managed pub/sub
  service) for no clear benefit given container deployment is already viable.

## Relationships
- Depends on: ADR-003/ADR-004 (what gets built/packaged), ADR-006 (BFF serves SPA by default), ADR-010
  (Postgres as a provisioned dependency), ADR-011 (SSE connection-affinity implication), ADR-012 (metrics/
  tracing backend must be reachable from wherever this deploys), ADR-013 (CI gates precede deploy).
- Source docs: `../implementation-plan.md` §3.4, §5 Phase 0, §6 risk #8.
