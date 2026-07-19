# ADR-020: Sales Prospect Pipeline, Consultant Action List, Real Navigation, and the Commit Simulator

## Status
Proposed

## Context
With ADR-019 live, the dashboard now genuinely renders permission-gated capability cards for the first time.
Direct product feedback against that live state names four gaps:

1. **No prospect pipeline.** Sales today only has `LeadConflictCheck` (`frontend/src/features/sales/`) — a
   single "check for conflicts" form. There is no way for a consultant to track a prospect through a deal
   lifecycle (contacted → NDA → RFP → proposal → SOW → closed), attach notes, or see their own pipeline state.
2. **No consultant-owned action list.** `bff_core::ActionQueueEntry` already exists, but its own invariant 3
   ("No local-only completion — this is the critical invariant") makes it structurally the wrong aggregate for
   this: it exists exclusively as a normalized projection of Nexus-sourced events, `complete` requires a
   non-empty `confirmation_event_id` from a real upstream confirmation, and "there is no method, and no
   combination of calls, that reaches `Completed`" without one. A consultant typing "call Acme back Thursday"
   has no Nexus event to confirm against — this is not a gap in that aggregate, it is that aggregate correctly
   refusing to be something it was never designed to be.
3. **Every "tab" looks identical.** Not a bug — confirmed by reading `DashboardPage.tsx`: there are no tabs.
   `PROMPT-18` explicitly deferred real client-side routing ("even though there's no real client-side router
   yet"), so Overview/Notifications/Action Queue/Your Modules are all sections on one continuously-scrolling
   page. That was a reasonable simplification for 3 cards; it stops being one once Sales alone gains a
   pipeline, an action list, and (per point 4) a simulator — the page needs real navigation.
4. **Edu/Products (course catalog, product info) — already built, not a gap here.** Checked directly:
   `LearningDashboard.tsx`, `ProductCatalog.tsx`, and `ApprovedClauses.tsx` (Legal) all exist as real feature
   components, and `DashboardPage.tsx`'s card-rendering switch already maps `edu`/`products`/`capacity`/
   `execution`/`landscape` module ids to them — only `legal` is missing its case (a one-line gap, fixed as part
   of this ADR, not a reason for new design). Backend routes (`GET /api/edu/catalog`, `GET /api/products/catalog`)
   already exist too. The only reason these aren't visible today is that Armor's current grant set for real
   consultants doesn't include those capabilities yet — an operational/nexus-side data gap, not a
   consultants-side code gap. Nothing in this ADR is needed to "build" Edu/Products visibility; it already
   works the moment Armor grants it.
5. **Commit "simulator"** — consultants feed in meeting transcripts and get back an implementation plan,
   "powered by commit.cognitum.one." No such capability exists anywhere in this repo's `nexus-client::CommitGateway`
   (`create_proposal`, `list_proposals`, `request_proposal_action` only) or, as far as this repo can determine,
   in nexus's own capability registry (`capabilities.json` had zero entries the one time this session read it
   live). This is the one item here this repo cannot deliver alone.

## Decision

### A. `Prospect` — a new, consultants-owned aggregate (not Nexus-sourced)
A `Prospect` is created, read, updated, and deleted entirely within `cognitum-consultants` — unlike every
other aggregate in `bff-core`, it has no upstream Nexus event driving it and no ACL boundary to respect,
because tracking *your own* prospecting notes is not a decision or a record any of the ten external
capabilities owns. Fields: `id`, `consultant_id` (owner), `company_name`, `contact_name` (optional),
`notes` (freeform, append-only history — see below), `stage`, `created_at`, `updated_at`.

**`ProspectStage`, a linear-with-branch state machine**, ordered by deal progression, ending in one of two
terminal states (a documented interpretation, not literally specified — see Alternatives):
```
Contacted -> AppointmentScheduled -> NdaSent -> NdaSigned -> RfpSent -> RfpSigned
          -> ProposalSent -> ProposalSigned -> SowSent -> ClosedWon
```
with `ClosedLost` reachable from *any* non-terminal stage (a deal can die at any point, not only at the end) —
**not** the flat, unordered list the request named (which put "closed" third); ordering deals lexically by
pipeline progression, and splitting "closed" into won/lost, is necessary for the stage to mean anything as a
funnel rather than an arbitrary tag. `notes` is a `Vec<ProspectNote { body, author_consultant_id, created_at }>`
— append-only, never edited/deleted in place, so a prospect's history stays a true history, matching this
repo's existing "audit trail, not mutable scratch space" convention (`ActionQueueEntry`'s own
`confirmation_event_id` non-mutation).

New: `bff_core::Prospect`/`ProspectRepository` (ADR-010 Postgres-backed, own migration), `bff-api::sales`
gains `GET/POST /api/sales/prospects`, `GET/PATCH /api/sales/prospects/{id}`,
`POST /api/sales/prospects/{id}/notes`, `POST /api/sales/prospects/{id}/stage` (a dedicated transition
endpoint, not a generic PATCH — so stage changes go through the state machine's own validation, not an
arbitrary field write). Frontend: a new `features/sales/ProspectPipeline.tsx` (a real Kanban/stage-grouped
board, not a flat list — the request explicitly wants to "see" and "change the status of" prospects), replacing
`LeadConflictCheck`'s current role as the sole Sales-card content (conflict-check becomes one tool *within* the
expanded Sales module, not the whole of it).

### B. `ConsultantActionItem` — a new, separate aggregate from `ActionQueueEntry`
Deliberately not a variant, extension, or relaxation of `ActionQueueEntry`'s invariant 3 (see Context point 2)
— a new, structurally simpler aggregate: `id`, `consultant_id`, `title`, `notes`, `done: bool`,
`linked_prospect_id: Option<Uuid>` (optional soft link to a `Prospect`, not a hard foreign key requirement — a
consultant can have action items unrelated to any prospect), `created_at`. No state machine beyond
`done`/`not done` — this is a checklist ("L10 type action list"), not a workflow with confirmation semantics;
that distinction *is* the point of keeping it separate from `ActionQueueEntry`. New:
`bff_core::ConsultantActionItem`/`ConsultantActionItemRepository`, `bff-api` routes under
`/api/action-items` (deliberately a different path than the existing `/api/action-queue`, to keep the two
concepts visibly distinct in the API surface, not just internally), frontend `features/sales/ActionList.tsx`
rendered alongside the pipeline.

### C. Real client-side routing
Adds `react-router` (or equivalent — a decision for the implementing unit, not fixed here) and restructures
`DashboardPage` into real routes: `/` (Overview: Notifications + Action Queue, today's content),
`/modules/{module_id}` (one route per permitted card, replacing the single giant `CardGrid`), with `Sidebar`
navigating between them instead of everything rendering on one scroll. This is what actually resolves "every
tab looks the same" — there are real tabs after this, not before.

### D. Commit meeting-transcript simulator — plumbing only, blocked on an external capability
This repo can build: a `nexus_client::CommitGateway::simulate_implementation_plan` method, a
`POST /api/commit/simulate` route (accepting a transcript — plain text initially, file upload as a later
iteration), and a frontend transcript-input + rendered-plan-output UI under the Commit module. **None of that
has anything to call** until nexus/Commit's own side defines and exposes a real capability contract for it —
this doesn't exist in nexus's registries today, unlike Prospect/ConsultantActionItem which need nothing from
Nexus at all. Given "generates an implementation plan" is presumably a slower, AI-driven generation step, the
capability contract nexus/Commit exposes should very likely be **asynchronous** (submit → poll/job-id →
result), not a single synchronous `POST capabilities/commit.simulate` under this repo's existing ADR-016
read/write timeout budgets, which are tuned for fast reads/writes, not long-running generation — that's a
design question for whoever owns Commit's side to answer, not something this repo can decide unilaterally.
**Sequencing**: build A/B/C first (fully self-contained, no external dependency, deliverable now); scope D's
frontend/gateway plumbing once Commit's real contract is confirmed, the same "verify against the real service,
don't guess a contract" discipline this session's other fixes already established the cost of skipping.

## Consequences
**Positive**
- Prospect pipeline and action list are fully deliverable today — zero external dependency, unlike D.
- Keeping `ConsultantActionItem` separate from `ActionQueueEntry` preserves invariant 3 exactly as designed
  (Nexus-confirmed completion stays Nexus-confirmed) while still giving consultants the freeform list they
  asked for — no weakening of an existing aggregate's guarantees to make room for a different use case.
- Real routing pays for itself immediately (Sales alone gains two new views) and scales cleanly as Edu/
  Products cards actually start appearing once Armor grants them.

**Negative / Trade-offs**
- Four sub-decisions in one ADR is a wider scope than this repo's other ADRs; justified because the request
  itself was one connected ask and B/C are small enough not to warrant their own documents, but implementation
  should still land as separable units/PRs (A, then B, then C, then D once unblocked), not one monolithic
  change.
- `ProspectStage`'s ordering and the `ClosedWon`/`ClosedLost` split are this repo's own interpretation of an
  unordered request list — worth confirming against real sales-team usage before treating the exact stage set
  as final.
- D cannot be scheduled with confidence until nexus/Commit's contract exists — any estimate here would be a
  guess.

## Alternatives Considered
- **Model the prospect pipeline as an extension of `DashboardConfiguration` or `ActionQueueEntry` instead of a
  new aggregate.** Rejected — `DashboardConfiguration` is presentation layout, not business data; extending
  `ActionQueueEntry` directly conflicts with its own documented invariant 3, per Context point 2.
- **Skip real routing, keep stuffing more cards onto one page.** Rejected — already borderline before this
  ADR's own two new Sales sub-features; adding a simulator UI on top makes one continuous page actively hostile
  to use.
- **Guess Commit's simulator contract now and build against a mock, the way ADR-030's events-poll was
  guessed.** Rejected outright, explicitly, given tonight's own evidence: two independent guessed contracts
  (`events/v1/poll`, then `api/v1/events/poll`) both 404'd against the real service, undetected by this repo's
  own e2e suite because the mock always answers whatever it's given. D is scoped as plumbing-ready-to-wire, not
  guess-and-ship.

## Relationships
- Depends on: ADR-010 (new aggregates' persistence pattern), ADR-018 (this repo's now-working push-ingestion
  pipeline stays untouched — `Prospect`/`ConsultantActionItem` are consultant-authored, never Nexus-sourced, so
  neither needs a `reaction_handler`), ADR-019 (the permission-gating this ADR's new Sales sub-features inherit
  unchanged).
- Informs: any future ADR defining the real Commit simulator contract once nexus/Commit's side confirms it
  (tracked here as an explicit external blocker, not designed in advance of that confirmation).
- Source docs: direct product requirements (this session); live inspection of `frontend/src/pages/DashboardPage.tsx`,
  `frontend/src/features/*`, and `crates/bff-core/src/action_queue_entry.rs`'s invariant 3.
