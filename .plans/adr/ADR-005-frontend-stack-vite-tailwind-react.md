# ADR-005: Frontend Stack — Vite + Tailwind CSS + React

## Status
Proposed

## Context
The project owner has fixed Vite as the build tool and Tailwind CSS for styling — both hard requirements, not
open decisions. Neither is a complete frontend stack on its own: Vite is a build tool, Tailwind is a styling
layer, and neither provides component composition, client-side routing, or state management. A component
framework must still be chosen. `implementation-plan.md` §3.2 flags this explicitly as undecided and notes a
"strong pull toward whatever framework manage.cognitum.one's dashboard is built in," since Phase 1
(`implementation-plan.md` §5) depends on literally copying manage's dashboard shell/layout/components
(`../research.md` §"Dashboard Relationship"). §6 risk #1 and risk #4 both flag this as blocking-or-entangled
with Phase 1 if manage's actual stack turns out to differ from whatever is chosen here.

## Decision
**React** (with TypeScript, per ADR-002) is the component framework, on top of Vite + Tailwind CSS.

Rationale:
- **Highest-probability match for manage.cognitum.one's dashboard.** Enterprise admin/dashboard shells (the
  described "layout, sidebar, header, cards, tables, forms, search, filters, alerts, dialogs" in
  `../research.md`) are overwhelmingly built on React in current industry practice, and React has the deepest
  catalogue of dashboard-shell starting points and component primitives (e.g. Radix UI, shadcn/ui) to
  reproduce that exact list quickly if a literal copy from manage is not viable and a close reimplementation
  is needed instead.
- **Ecosystem depth for this repo's specific needs.** ADR-015 (frontend server-state) leans on TanStack
  Query, which is React-first (also available for other frameworks, but most mature and most commonly paired
  with React); ADR-011's SSE consumption pattern is well-trodden in React via hooks; React Router (or TanStack
  Router) covers the "frontend routing" responsibility this repo owns per `../research.md`.
  `frontend/src/features/<capability>/` (per `implementation-plan.md` §4) maps naturally onto React's
  component-per-feature convention.
- **Talent/familiarity assumption.** Absent a stated team preference, React remains the most broadly familiar
  choice, minimizing onboarding cost for a repo that Phase 4 will keep extending with new capability modules.

**Explicit contingency clause**: if, during Phase 1, manage.cognitum.one's actual dashboard framework is
confirmed to be something other than React (Vue, Svelte, Angular, etc.), this ADR should be revisited before
component-copying work begins, per `implementation-plan.md` §6 risk #1/#4 — porting into a mismatched
framework defeats the "copy, don't rebuild" premise of Phase 1 and may change this decision's cost/benefit.
This ADR's choice is the best default *absent* that information, not a claim the information has been
verified.

## Consequences
**Positive**
- Full frontend stack (Vite + Tailwind + React + TypeScript) is now unambiguous for Phase 0 scaffolding.
- `frontend/src/features/<capability>/` (implementation-plan.md §4) has an obvious, idiomatic React
  implementation (one feature module = one directory of components/hooks/routes).
- Large ecosystem reduces the risk of needing custom-built primitives for standard dashboard UI patterns.

**Negative / Trade-offs**
- If manage's actual stack differs, this decision must be revisited (see contingency clause) — an explicit,
  acknowledged risk rather than a hidden one.
- React's ecosystem breadth is also a footgun: without conventions (enforced via lint rules and code review),
  feature modules could diverge in patterns (state management, data-fetching) faster than in more opinionated
  frameworks — mitigated by ADR-015 fixing the data-fetching approach explicitly.

## Alternatives Considered
- **Vue.** A legitimate alternative with strong Tailwind/Vite integration (Vite originated in the Vue
  ecosystem) and a gentler learning curve. Rejected as the default here mainly on the manage-framework-match
  probability judgment above; would be reconsidered immediately if manage's stack is confirmed to be Vue.
- **Svelte / SolidJS.** Rejected — smaller ecosystems for enterprise dashboard component libraries, higher
  risk of needing to hand-build primitives this repo doesn't have time to invest in early, and lower
  probability of matching manage's stack, compounding both risks #1 and #4 rather than mitigating either.
- **No framework (vanilla TS + Web Components).** Rejected — the permission-aware conditional rendering this
  repo needs across ten capability integrations (`../ddd/domain-map.md`) benefits significantly from a
  component framework's declarative composition; hand-rolled DOM management would slow every future Phase 4
  integration for no offsetting benefit, and Vite's dev-experience advantages are largely framework-oriented.

## Relationships
- Depends on: ADR-002 (TypeScript is the approved frontend language).
- Informs: ADR-006 (interop model — SPA served by the Axum BFF), ADR-015 (TanStack Query pairs with React).
- Source docs: `../research.md` §"Dashboard Relationship", `../implementation-plan.md` §3.2, §6 risks #1/#4.
