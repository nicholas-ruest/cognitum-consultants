# ADR-001: Record Architecture Decisions

## Status
Proposed

## Context
`consultants.cognitum.one` is a brand-new repository being planned from `.plans/research.md` (ownership
hierarchy and the Nexus-only integration rule) and `.plans/implementation-plan.md` (a phased plan that
explicitly flags roughly a dozen cross-cutting decisions as needed but undecided — see its §3.4 checklist).
Those decisions carry real weight: this repo is bound by a hard external rule (all sub-business capability
integration routes through `nexus.cognitum.one`, never direct — implementation-plan.md §2.3) and a hard
technology rule (Rust as much as possible, TypeScript as the defined secondary language). Made informally, or
only implicitly in code, decisions like these erode quickly — the next engineer or agent has no record of why
Axum was chosen over Actix-web, why SSE and not WebSockets, or why this repo never calls `sales.cognitum.one`
directly. The plan is explicitly executed in phases by multiple agents over time (§5), and GOAP-style
replanning is expected as facts change (Nexus contract maturity, auth provider, manage's dashboard stack —
§6). Decisions need a durable, versioned, individually-referenceable record that later work can supersede or
amend without losing the original reasoning.

## Decision
This repository uses Architecture Decision Records (ADRs) to capture every architecturally significant
decision — anything expensive to reverse, that constrains future work, or that resolves an open question
flagged in `implementation-plan.md` §3.4 / §6.

- **Location**: ADRs live in `.plans/adr/` for the duration of this planning phase. If/when the repo moves
  from planning into active implementation, relocating them to `docs/adr/` is itself a future decision, not
  assumed here.
- **Numbering**: sequential, zero-padded three digits, `ADR-NNN`, never reused even after supersession.
- **Filename**: `ADR-NNN-kebab-case-title.md`.
- **Required sections**: `Status`, `Context`, `Decision`, `Consequences` (positive and negative),
  `Alternatives Considered`, `Relationships` (supersedes / amends / depends-on, plus references to
  `.plans/research.md`, `.plans/implementation-plan.md`, and `.plans/ddd/*` where relevant).
- **Lifecycle**: `proposed` → `accepted` → (optionally) `deprecated` or `superseded`. Nothing in this initial
  batch is `accepted` — this is a planning exercise against an empty repository; acceptance happens when the
  decision is ratified, typically at or just before the phase that depends on it begins (per
  implementation-plan.md §5's phase preconditions).
- **Change discipline**: an accepted ADR is never edited to reverse its own decision. A reversal is a new ADR
  that supersedes the old one; the old one's `Status` becomes `superseded by ADR-NNN` and its content is left
  intact as history.

## Consequences
**Positive**
- Every item in `implementation-plan.md`'s ADR checklist gets a durable, individually-linkable answer instead
  of remaining an open question indefinitely.
- Future agents/engineers can read `.plans/adr/` to learn *why*, not just *what*, before touching code that
  implements a decision.
- Supersession makes architecture drift visible instead of silently rewriting history.

**Negative / Trade-offs**
- Adds process overhead: a real architectural change now requires a written ADR, not just a code change.
- Numbering and cross-references must be kept consistent by hand as ADRs are added; a stale `depends-on` link
  degrades the record's value.

## Alternatives Considered
- **No formal ADR process; decisions live only in code/PR descriptions.** Rejected — this repo's decisions
  are unusually externally constrained (Nexus-only rule, Rust-first rule, strict ownership boundaries per
  `../ddd/domain-map.md`) and are made across multiple phases/agents; undocumented rationale would force
  re-litigating settled questions.
- **Decisions recorded directly inside `implementation-plan.md`.** Rejected — the plan explicitly defers this
  work to "a follow-on ADR agent" (§3.4) and treats the plan and the ADR set as separately-evolving documents;
  conflating them would make individual decisions harder to supersede independently.
- **A wiki or external issue tracker.** Rejected for this phase — ADRs need to live next to the plan/DDD docs
  they govern and be reviewable in the same PRs, which a repo-local Markdown file supports and an external
  tool does not.

## Relationships
- Supersedes: none (first ADR).
- Depends on: none.
- Governs: every subsequent ADR in this directory.
- Source docs: `../research.md`, `../implementation-plan.md` (§3.4 checklist, §6 risks).
