# ADR-017: Design-System Extraction Packaging Strategy

## Status
Proposed

## Context
`../implementation-plan.md` §3.4 lists "Design-system extraction packaging strategy (npm workspace package vs
separate repo vs private registry) — Phase 5" as a required ADR that, until now, did not exist.
`../implementation-prompts.md` PROMPT-42 (the design-system extraction unit, the last remaining unit in the
plan) is explicitly blocked on it: "This unit cannot be started until an ADR resolving the design-system
extraction packaging strategy exists." Phases 0–4 (all ten capability integrations, `../implementation-plan.md`
§5) are implemented and committed; this ADR is the only precondition left before PROMPT-42 can start.

**Repo topology, verified, not assumed.** This repo's only git remote is
`https://github.com/nicholas-ruest/cognitum-consultants` (`.git/config`) — there is no root `package.json` and
no npm `workspaces` field referencing any sibling application; the only existing workspace-style structure is
the single Cargo workspace (ADR-004) plus one independent `frontend/package.json`. `../research.md`
§"Dashboard Relationship" and `../ddd/domain-map.md` §3 both confirm `manage.cognitum.one` is a **separate
peer application, not a runtime dependency and not part of this repo** — the domain map's context diagram
labels the relationship "Separate Ways at runtime, with one explicit, one-time exception": the Phase 1
(PROMPT-17) source-code borrow of manage's dashboard shell into `frontend/src/components/`. Per the domain
map: that one-time borrow relationship "stops being relevant" once Phase 5 "gives both apps a shared package
instead." Nothing in `../research.md`, `../implementation-plan.md`, or this repo's git history proposes
merging the two applications' repositories.

**Real duplication, verified, not assumed.** Inspection of `frontend/src/features/*` (the ten capability
modules from PROMPT-34–41) against `frontend/src/components/` (the PROMPT-17 shell) found:
- **Card/detail layout — confirmed, 4+ instances.** An identical hand-rolled list-item + detail-panel idiom
  (`className="w-full rounded border border-gray-200 p-3 text-left hover:bg-gray-50"` for the row,
  `className="rounded border border-gray-300 p-3"` for the detail panel), duplicated instead of using the
  existing shared `Card`/`CardGrid` primitives, in `customer/CustomerContextList.tsx`,
  `products/ProductCatalog.tsx`, `execution/ExecutionWorkspace.tsx`, and `commit/ProposalWorkspace.tsx`, with a
  simpler row-only variant of the same idiom also in `edu/LearningDashboard.tsx`,
  `landscape/LandscapeWorkspace.tsx`, `legal/ApprovedClauses.tsx`, `notifications/NotificationCentre.tsx`, and
  `notifications/ActionQueue.tsx`.
- **Forms — confirmed, 4 instances.** The same `<form className="flex flex-col gap-3">` wrapping `TextInput`
  + submit `Button`, with hand-duplicated mutation-error `Alert` boilerplate, in
  `capacity/ProfileEditForm.tsx`, `commit/ProposalWorkspace.tsx`, `landscape/LandscapeWorkspace.tsx`, and
  `sales/LeadConflictCheck.tsx`. No shared `Form`/`FormField` component exists today.
- **Filter/search — not confirmed.** No search inputs, filter bars, or query-text UI state exist anywhere in
  the ten feature modules today.
- **Dialogs — not confirmed.** The shared `Dialog` primitive exists in `frontend/src/components/` but is not
  used by any of the ten feature modules; no local/duplicated modal implementation exists to extract from.

PROMPT-42's own text names "card layouts, forms, filter/search patterns, dialog structures" as the categories
to check for 3+ occurrences. Two of those four are real today; two are not — this ADR's scope decision reflects
that, per this repo's "no premature abstraction" convention (`CLAUDE.md`).

## Decision
**Two packages — `@cognitum/design-system` (foundational primitives) and `@cognitum/dashboard-components`
(domain-specific dashboard patterns) — published as versioned artifacts from a private, npm-compatible
registry (e.g. GitHub Packages under whichever GitHub org/user ends up hosting the Cognitum One family of
repos, or an equivalent private registry such as Artifactory/Verdaccio if that's already standardized
elsewhere in the organization). Package source lives inside *this* repo, under a new `packages/` directory,
using npm workspaces locally for this repo's own development — it is not moved to a new dedicated repo, and it
is not consumed by `manage.cognitum.one` via a same-repo workspace reference.**

This is the third option from the checklist ("private registry"), deliberately choosing *inline-in-this-repo,
published externally* over *carve out a brand-new repo per package*:

- **Why not an npm workspace package (option 1):** a plain `workspaces` reference only resolves a dependency
  that lives in the *same* repository tree. `manage.cognitum.one` is confirmed to be a separate deployable
  application with (per the domain map) no runtime relationship to this repo, and no plan document proposes
  merging the two repos. Workspace-only packaging is not just suboptimal here, it is structurally infeasible
  for the actual cross-repo consumer this ADR must support.
- **Why not a dedicated separate repo per package (option 2):** two small component packages do not yet
  justify the overhead of a new repo each (its own CI pipeline, issue tracker, branch protection, release
  process, and cross-repo coordination for every change). This repo's own CI (`../ci.yml`) already
  builds/lints/tests `frontend/`; adding a publish step for `packages/*` on version bump is materially less
  overhead than standing up new repos, and the packages can be given their own repos later without breaking
  consumers — a private-registry package's location is transparent to `npm install`.
- **Package boundary matches what's real today**, not what PROMPT-42 speculated: `@cognitum/design-system`
  absorbs `frontend/src/components/` wholesale (`Alert`, `Button`, `Card`, `CardGrid`, `Dialog`, `Header`,
  `Layout`, `Sidebar`, `TextInput`) since these are already the shared shell PROMPT-17 established.
  `@cognitum/dashboard-components` gets exactly the two confirmed-duplicated domain patterns: a
  `ListDetailPanel` (built on `Card`) replacing the row/detail idiom duplicated across
  customer/products/execution/commit/edu/landscape/legal/notifications, and a `CapabilityForm` wrapper
  (built on `TextInput`/`Button`/`Alert`) replacing the form-plus-error-alert idiom duplicated across
  capacity/commit/landscape/sales. Filter/search and dialog-usage abstractions are explicitly **not**
  extracted now — there is no real call site for either, and PROMPT-42's mention of them is prospective, not
  evidence of present duplication. Extract them when a third real usage appears, not preemptively.
- **Versioning/publishing:** standard semver on each package (`0.1.0` initial). CI publishes `packages/*` to
  the registry when a package's version is bumped on `main` (tag-triggered or a `changesets`-style
  version-bump gate — either is acceptable at implementation time; the requirement fixed by this ADR is
  "publish on version bump," not the specific tool). Consumers (`frontend/` in this repo, and eventually
  `manage.cognitum.one`) install by version like any other npm dependency — no git submodules, no
  filesystem-path dependencies across repos.
- **Local dev ergonomics:** within *this* repo, a root `package.json` with
  `"workspaces": ["frontend", "packages/*"]` lets `frontend/` consume `packages/design-system` and
  `packages/dashboard-components` by local workspace symlink during development, while the same packages are
  independently published for `manage.cognitum.one` (a different repo) to consume as an ordinary external
  dependency. Workspace-local linking and cross-repo registry publishing are not mutually exclusive — this ADR
  uses both, one for this repo's dev loop, the other for cross-repo distribution.

**What "done" looks like for PROMPT-42, once this ADR unblocks it:**
1. `packages/design-system` and `packages/dashboard-components` exist in this repo, each published at `>=0.1.0`
   to the chosen private registry.
2. This repo's `frontend/` imports both packages instead of the local `frontend/src/components/` copies and
   the four-plus duplicated card/detail and form call sites identified above.
3. The registry URL and package names are documented (e.g. in this repo's `docs/`) so `manage.cognitum.one`'s
   own team can add them as an external dependency in their repo — actually doing so is out of this repo's
   control and therefore out of PROMPT-42's acceptance criteria, but the contract (package names, registry,
   versioning scheme) this ADR fixes is what makes that adoption possible without further negotiation.
4. Once `manage.cognitum.one` adopts the packages, the one-time source-code borrow noted in
   `../ddd/domain-map.md` §3 stops being the operative relationship between the two codebases, replaced by an
   ordinary versioned-package dependency — the outcome the domain map anticipated.

## Consequences
**Positive**
- Unblocks PROMPT-42 with a concrete, verifiable scope (two named packages, two confirmed extraction targets)
  instead of the speculative four PROMPT-42's prose listed.
- No repo restructuring or cross-team migration is required to start — package source stays in this repo;
  only a registry and a publish step are added.
- `manage.cognitum.one` can adopt the packages on its own schedule, from its own repo, without this repo
  needing write access to it or vice versa — consistent with the "peer application, not a dependency" framing
  in `../ddd/domain-map.md` §3.
- Deferring filter/search and dialog extraction avoids building abstractions with zero real call sites, per
  this repo's own anti-premature-abstraction convention.

**Negative / Trade-offs**
- Two packages living inside this repo's tree but published for consumption by a repo that doesn't share this
  tree is a slightly unusual shape — most engineers expect "shared package" to imply "separate repo." This is
  accepted as the pragmatic middle ground for two thin packages; nothing here prevents splitting either
  package into its own repo later if it grows enough to justify independent governance.
- This repo cannot guarantee or enforce `manage.cognitum.one`'s adoption of the packages — the domain map's
  "borrow stops being relevant" outcome only happens once manage's own team acts on it, which is outside this
  repo's scope and this ADR's authority.
- The exact registry host is left open (mirrors ADR-014's treatment of the deployment target) because no
  existing document fixes one; a follow-up, lower-stakes decision (which specific registry) is needed before
  the first `npm publish`, but does not block writing `packages/*` source or this ADR's acceptance.

## Alternatives Considered
- **npm workspace package only (no external registry).** Rejected — infeasible for the actual cross-repo
  consumer (`manage.cognitum.one`), which does not share this repo's tree and has no plan to merge into it.
- **Dedicated separate repo per package, published via private registry.** Rejected for now as unwarranted
  overhead (new CI, new repo governance, cross-repo PR coordination) for two component packages at this size;
  revisit if either package's scope grows enough to need independent versioning cadence or ownership from this
  repo's frontend.
- **Publish to the public npm registry.** Rejected — these are internal Cognitum One implementation details
  with no external audience, and claiming the `@cognitum` scope publicly risks squatting/collision with an
  unrelated party; a private registry keeps them inside the organization's control.
- **Git dependency (npm `install` from a git URL/subdirectory) instead of a registry.** Rejected — no proper
  semver resolution, worse install ergonomics for consumers, and no clean way to publish pre-built artifacts
  (types, compiled CSS) versus raw source.
- **Do nothing (leave duplication as-is).** Rejected — this is the status quo PROMPT-42 exists to fix, and the
  duplication (card/detail layout, forms) is now independently confirmed to be real, not hypothetical.

## Relationships
- Depends on: ADR-005 (React/Tailwind as the frontend stack these packages are built on), ADR-004 (this repo's
  existing workspace conventions, extended here to npm), ADR-013 (CI gates that the new `packages/*` publish
  step extends).
- Unblocks: `../implementation-prompts.md` PROMPT-42 (design-system extraction), the last remaining unit in
  the implementation plan.
- Source docs: `../implementation-plan.md` §3.4 (this ADR's checklist entry) and §5 Phase 5; `../research.md`
  §"Dashboard Relationship" (package names, long-term intent); `../ddd/domain-map.md` §3 (the one-time
  Manage borrow and when it stops being relevant); `../implementation-prompts.md` PROMPT-17 (origin of
  `frontend/src/components/`) and PROMPT-34–41 (origin of the ten feature modules whose duplication this ADR
  verified).
