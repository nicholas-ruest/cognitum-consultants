# ADR-002: Primary Implementation Language Is Rust; TypeScript Is the Defined Secondary Language

## Status
Proposed

## Context
The project owner has set a hard, non-negotiable constraint: backend/service implementation must be **Rust,
as much as possible**, with **TypeScript** as the explicit secondary/fallback language for anything Rust
isn't practical for, and any other language only to fill a genuine remaining gap — with that choice justified
in its own ADR. Separately, the UI is required to use Vite + Tailwind CSS, which are JS/TS-ecosystem tools;
building a frontend with them necessarily means TypeScript/JavaScript on the client, and the constraint is
explicit that this does not violate the Rust-first rule, which applies to backend/service code.
`implementation-plan.md` §3 already leans on this split (Rust BFF, TS/Vite frontend) but does not itself state
the acceptance criteria for when TypeScript (or another language) is the right choice — this ADR is that
policy.

## Decision
1. **Default for all backend/service code is Rust.** This includes the BFF HTTP server, domain/aggregation
   logic, the Nexus client/ACL adapters, auth middleware, persistence access, and any CLI tooling that runs as
   part of the deployed system.
2. **TypeScript is the defined secondary language**, acceptable — without a separate ADR — in these specific
   cases:
   - **All frontend/client code.** Vite + Tailwind (ADR-005) make TypeScript/JavaScript unavoidable on the
     client; this is expected and explicitly carved out by the project's own constraint, not an exception to
     justify.
   - **Frontend-adjacent tooling that operates only within the `frontend/` workspace** — Vite/Tailwind config,
     frontend build/lint/test scripts, frontend codegen (e.g. generating a typed API client from the BFF's
     OpenAPI schema, see ADR-007).
   - **Repo-level developer-experience scripts where the ecosystem tool itself is JS/TS-native** (e.g. a
     Prettier/ESLint config script, a `package.json`-driven monorepo task runner) — narrowly, and only where
     no reasonable Rust equivalent exists for that specific tool's ecosystem.
3. **Any language other than Rust or TypeScript requires its own ADR** that names the specific gap Rust and
   TypeScript cannot fill (e.g. a vendor SDK that only ships a Go or Python client with no usable Rust/TS
   bindings), scopes the usage as narrowly as possible, and states the maintenance/consistency cost accepted.
   No such gap is known at the time of this ADR; none is pre-approved here.
4. **Ambiguous cases default to Rust.** If a piece of backend-adjacent tooling (e.g. a database migration
   runner, a CI helper script) could reasonably be written in either language, Rust is the default choice
   unless a concrete practicality reason is documented in the PR (e.g. "no maintained Rust crate exists for
   X"), in which case TypeScript is used under clause 2's "genuine remaining gap" spirit rather than requiring
   a full new ADR, since it stays within the two approved languages.

## Consequences
**Positive**
- Removes ambiguity for every future PR: "is this Rust or TypeScript" has a written default and a narrow,
  enumerated exception list rather than being decided ad hoc per contributor.
- Keeps the backend's memory-safety, performance, and single-binary-deployment benefits (relevant to ADR-014)
  consistent across the whole service layer.
- Bounds scope creep: a third language cannot enter the codebase silently — it requires its own ADR, which
  keeps the two-language policy auditable via `grep`/ADR review (per ADR-001).

**Negative / Trade-offs**
- Narrows the pool of readily reusable open-source glue code — some integrations that would be a quick
  Python/Go script now require either a Rust crate (which may not exist) or writing more from scratch.
- Two-language operational surface (Rust toolchain + Node/TS toolchain) still exists for CI, dependency
  updates, and security scanning, even though it is intentionally minimized versus a fully polyglot repo.

## Alternatives Considered
- **Rust-only, no TypeScript exception.** Rejected — infeasible given the hard Vite + Tailwind UI requirement;
  Vite is a JS/TS build tool by definition, and mature Rust-native SPA tooling does not yet match the
  React/Vue/Svelte + Vite ecosystem for this kind of dashboard UI (also see ADR-005, ADR-006).
- **Rust for backend, any frontend language/framework left open.** Rejected — the user's constraint already
  fixes Vite + Tailwind; leaving the *language* open when the *toolchain* is fixed would just reintroduce the
  same ambiguity this ADR exists to remove.
- **Allow scripting languages (Python/Bash) freely for tooling.** Rejected as a blanket allowance — Bash is
  already implicitly available for trivial shell glue (not treated as an "implementation language" under this
  policy), but Python/Go/etc. are treated as third languages requiring their own ADR per clause 3, to prevent
  gradual, undocumented erosion of the two-language policy.

## Relationships
- Depends on: ADR-001 (this ADR's existence follows the ADR process it defines).
- Informs: ADR-003 (Rust web framework), ADR-004 (Cargo workspace layout), ADR-005 (frontend stack), ADR-013
  (testing strategy spans both languages).
- Source docs: user's hard constraints (see task brief), `../implementation-plan.md` §3.
