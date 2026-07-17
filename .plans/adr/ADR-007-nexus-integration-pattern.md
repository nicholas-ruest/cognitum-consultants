# ADR-007: Integration Pattern with Nexus — REST/JSON + Per-Capability ACL Gateways

## Status
Proposed

## Context
`../research.md` and `../ddd/domain-map.md` establish a hard rule: `nexus.cognitum.one` is the *only*
integration point for all ten sub-business capabilities this repo consumes (Sales, Commit, Edu, Capacity,
Customer, Execution, Products, Landscape, Legal, Armor) — this repo must never call those services directly
(`../implementation-plan.md` §2.3). `../ddd/anti-corruption-layers.md` already specifies the ACL shape: one
gateway module per capability inside the `nexus-client` crate (ADR-004), each translating Nexus's normalized
responses into this repo's own vocabulary, never re-adjudicating business policy (the Sales lead-conflict
worked example is the canonical illustration — the BFF relays `AccountClaimDetermined` verbatim, it does not
decide `creation_allowed` itself).

What is *not* yet decided: the wire protocol Nexus speaks, and the concrete client abstraction pattern.
`../implementation-plan.md` §3.4 and §6 risk #3 flag Nexus's contract maturity and protocol as unknown/open.
This ADR makes the best available decision now, structured so it can absorb that uncertainty rather than be
blocked by it.

## Decision
**Protocol: REST/JSON**, accessed through a small internal `NexusTransport` trait, with auth propagation
carried on every call.

- **Why REST/JSON as the default assumption**: absent a confirmed Nexus contract, REST/JSON is the most
  universally compatible default — every capability team behind Nexus can implement/consume it without a
  shared IDL toolchain, and it requires no additional negotiation before Phase 2 can start against a mocked
  contract (`../implementation-plan.md` §5 Phase 2 precondition: "Nexus API contract available... or a stable
  mock of it"). gRPC would give stronger typing and streaming support but adds a heavier
  codegen/proto-management burden while the actual contract is still unknown; GraphQL is a poor fit because
  Nexus already performs its own normalization ("API normalization" per `../research.md`) and this repo wants
  explicit, narrow, per-capability request/response shapes (per the ACL doc), not a generic queryable graph
  that would encourage callers to bypass the ACL boundary by shaping ad hoc queries.
- **Transport abstraction**: `nexus-client` defines a `NexusTransport` trait (built on `reqwest` per
  `../implementation-plan.md` §3.1) that every capability gateway module depends on, not a concrete HTTP
  client directly. This contains protocol risk: if a real Nexus contract turns out to require gRPC for some or
  all capabilities, only the transport implementation changes — the ten gateway modules' request/response
  types and the ACL boundary they enforce (per `../ddd/anti-corruption-layers.md`) do not.
- **Gateway structure**: one Rust module per capability inside `nexus-client` (`sales`, `commit`, `edu`,
  `capacity`, `customer`, `execution`, `products`, `landscape`, `legal`, `armor` — matching
  `../ddd/anti-corruption-layers.md` §1–10 exactly), each exposing a typed trait (`SalesGateway`,
  `CommitGateway`, ...) with methods matching that section's documented Outbound commands/queries, returning
  the documented Inbound result/DTO shapes. No gateway method may contain conditional business logic beyond
  request/response shape validation (`anti-corruption-layers.md` §11's cross-cutting rule) — that constraint
  is enforced by code review, not the compiler, and should be checked explicitly by ADR-review tooling
  (`../../.claude/CLAUDE.md`'s ADR-compliance workflow, if wired up later).
- **Auth propagation**: every `NexusTransport` call attaches the consultant's current session credential
  (ADR-008) as an outbound header/token; Nexus is responsible for propagating it onward to Armor and the
  target capability. This repo's transport layer does not interpret or validate that credential itself beyond
  attaching it — validation is Armor's job (ADR-009).
- **Versioning/error semantics**: deferred to be negotiated per-capability as real contracts land (Phase 4
  staging, `../implementation-plan.md` §5); this ADR fixes the *shape* of the abstraction, not every
  capability's final wire contract.

## Consequences
**Positive**
- Unblocks Phase 2 immediately — REST/JSON against a mock is enough to build and test the Sales
  lead-conflict-warning reference flow without waiting on Nexus's real contract.
- The `NexusTransport` trait isolates protocol risk from the ACL/business boundary; a later protocol change
  (e.g. gRPC for high-volume capabilities) is a contained, mechanical change.
- Gateway-per-capability structure gives Phase 4 a literal template, consistent with ADR-004's crate layout.

**Negative / Trade-offs**
- REST/JSON forgoes gRPC's compile-time contract checking and streaming support; if Nexus later standardizes
  on gRPC, this repo pays a migration cost (mitigated, not eliminated, by the transport abstraction).
- The "no business logic in gateways" rule is a code-review-enforced convention, not a compiler-enforced one —
  carries ongoing discipline cost as Phase 4 adds more gateways.

## Alternatives Considered
- **gRPC as the default protocol.** Rejected as the *default* given unknown contract maturity — would require
  committing to a proto toolchain/codegen pipeline before any real Nexus contract is confirmed to use it.
  Remains a strong candidate to adopt later via the `NexusTransport` abstraction if Nexus standardizes on it.
- **GraphQL against Nexus.** Rejected — mismatched with the ACL pattern's intent (narrow, explicit,
  non-negotiable per-capability shapes) and duplicative of normalization Nexus already claims to own.
- **One shared generic `NexusClient` with a single `call(capability, command, payload)` method instead of
  typed per-capability gateways.** Rejected — would erase the compile-time safety and the "restricted ACL"
  pattern `../ddd/anti-corruption-layers.md` §4 depends on for Capacity (deliberately no query shape for
  cross-consultant data); a generic client can't structurally forbid a call shape the way a narrow trait can.

## Relationships
- Depends on: ADR-002 (Rust), ADR-003 (Axum/tower ecosystem, reused for outbound middleware), ADR-004
  (`nexus-client` crate structure).
- Informs: ADR-008 (auth propagation carried on every Nexus call), ADR-009 (Armor permission assertions
  arrive via the Armor gateway), ADR-011 (event delivery consumes Nexus-routed events), ADR-013 (contract
  testing against Nexus), ADR-016 (resilience/timeout layering on `NexusTransport`).
- Source docs: `../ddd/anti-corruption-layers.md`, `../ddd/domain-map.md`, `../ddd/domain-events.md`,
  `../implementation-plan.md` §2.3, §3.4, §6 risk #3.
