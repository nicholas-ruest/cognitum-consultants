# Anti-Corruption Layers — External Context Adapters

Source of truth: `.plans/research.md`. Every call this repo makes to a sub-business capability crosses
`nexus.cognitum.one` (per `implementation-plan.md` §2.3: "the *only* integration point... must never call
sales/commit/edu/capacity/customer/execution/products/landscape/legal/armor/capital services directly").
Nexus already normalizes across services (it owns "API normalization"), but this repo additionally wraps
every Nexus-routed capability in its own ACL adapter — implemented as one module per capability inside the
`nexus-client` crate (`implementation-plan.md` §4) — so that external contract shapes never leak past that
boundary into this repo's own aggregates (`DashboardConfiguration`, `NotificationItem`, `ActionQueueEntry`,
etc., defined in `consultant-experience-context.md`).

**Explicit exclusion:** `capital.cognitum.one` and `verdict.cognitum.one` are consumed only by
`manage.cognitum.one`, per research.md's "Manage Consultants Section." This repo defines **no ACL adapter**
for either — they do not appear in `nexus-client` at all. Listed here only so the omission reads as
deliberate, not missed.

Each adapter below follows the same shape:

- **Our term** — the name this repo's ubiquitous language uses for the adapter/capability.
- **Crosses the boundary as** — the translated shape this repo's code actually works with (rough field
  list, not a schema — full DTOs are an implementation detail for later).
- **Outbound (consultants → Nexus → service)** — commands/queries this repo issues.
- **Inbound (service → Nexus → consultants)** — events/results this repo consumes.

---

## 1. Sales ACL — `SalesGateway`

- **Our term**: **Account Claim Check** (the lead-conflict capability) and **Collaboration Request**.
- **Crosses the boundary as**: `AccountClaimResult { match_status, creation_allowed: bool, display_message,
  permitted_actions: [...] }` — this repo never models Sales' internal Company/Lead/Contact/Opportunity
  graph; it only ever sees the verdict of a claim check plus a short list of actions it's allowed to render.
- **Outbound**: `CheckAccountClaimCommand { company_name, normalized_domain?, consultant_id }`,
  `RequestCollaborationCommand { company_reference, consultant_id, message? }`,
  `SubmitReferralCommand { company_reference, consultant_id, notes? }`.
- **Inbound**: `AccountClaimDetermined` (see worked example below), `CollaborationRequestAcknowledged`,
  `ReferralSubmitted`.

### Worked example: lead-conflict-warning flow (per research.md §"Lead Conflict Warning")

This is the reference flow `implementation-plan.md` Phase 2 builds first. Walked through as
commands/events crossing the Sales ACL:

1. **Consultant enters a company name** in the Consultants UI (Workspace context, not a business action —
   pure UI input, no aggregate yet).
2. BFF issues **`CheckAccountClaimCommand`** through `SalesGateway` → Nexus → Sales. This is a *query-shaped
   command*: it asks Sales to evaluate, it does not assert any fact.
3. Sales checks companies, leads, contacts, and opportunities (entirely inside Sales' own bounded context —
   opaque to this repo) and may consult Customer for existing account relationships (Sales↔Customer
   integration, also opaque to this repo — not our boundary to model).
4. Sales determines ownership/conflict status and Nexus returns it to the BFF as an **`AccountClaimDetermined`**
   event/result:
   ```text
   {
     match_status: "active_owned_account",
     creation_allowed: false,
     display_message: "This company is already being worked.",
     permitted_actions: ["request_collaboration", "submit_referral", "cancel"]
   }
   ```
5. The BFF **relays this verbatim** to the frontend — per research.md's explicit rule, "the Consultants
   frontend must not independently decide whether a competing lead may be created." No `AccountClaimResult`
   invariant in this repo re-derives or overrides `creation_allowed`; it is treated as an opaque policy
   verdict from Sales.
6. Frontend renders `display_message` and only the buttons listed in `permitted_actions`. If the consultant
   clicks "request collaboration," the BFF issues **`RequestCollaborationCommand`** back through the same
   gateway — a brand-new outbound command, not a mutation of anything stored locally.
7. Optionally, this exchange can also seed a `CrossCapabilityWorkflowSession` (Workspace context) if the
   consultant is mid-flow moving toward proposal creation in Commit — that session references the Sales
   company by opaque id only.

This flow demonstrates the general shape every other ACL below follows: **outbound command → opaque
external decision → inbound result relayed, never re-adjudicated.**

---

## 2. Commit ACL — `CommitGateway`

- **Our term**: **Proposal Workspace Handle**.
- **Crosses the boundary as**: `ProposalSummary { proposal_id, title, status, stage, last_updated_at,
  deep_link }` for listing/dashboard purposes; full proposal editing is presented via Commit-hosted
  UI/flows this repo only frames and deep-links into, not a form this repo re-implements over raw Commit
  data.
- **Outbound**: `CreateProposalCommand { origin_reference (e.g. Sales company/lead id), consultant_id }`,
  `RequestProposalActionCommand { proposal_id, action }` (e.g. resend, request revision).
- **Inbound**: `ProposalCreated`, `ProposalStatusChanged`, `ProposalAccepted` — each becomes a
  `NotificationItem` or `ActionQueueEntry` (Notification & Action Queue context) as appropriate.

---

## 3. Edu ACL — `EduGateway`

- **Our term**: **Learning Snapshot**.
- **Crosses the boundary as**: `LearningSnapshot { course_id, title, progress_status, certification_status,
  deep_link }` — a read-mostly projection; this repo never stores assessment content or certification
  criteria.
- **Outbound**: `RequestLearningCatalogQuery { consultant_id, filters? }`.
- **Inbound**: `CourseCompleted`, `CertificationIssued`, `TrainingRequirementDue` — feed
  Notification/Action Queue.

---

## 4. Capacity ACL — `CapacityGateway`

- **Our term**: **Consultant Profile Intake** (deliberately narrow — see relationship type in
  `domain-map.md`, "Conformist, via a deliberately restricted ACL").
- **Crosses the boundary as**: `ConsultantProfileIntake { skills[], certifications[], languages[],
  availability_window, geographic_coverage }` — this repo's ACL is intentionally **write-heavy and
  read-narrow**: it can submit the consultant's own updates but the read side returns only what that same
  consultant is permitted to see about themself. Per research.md, this repo must never expose "internal
  capacity planning or other consultants" — so this ACL has no query shape for cross-consultant data at all;
  that omission is structural, not a filtering afterthought.
- **Outbound**: `UpdateOwnProfileCommand { consultant_id, profile_fields }`.
- **Inbound**: `ProfileUpdateAccepted`, `ProfileUpdateRejected { reason }`.

---

## 5. Customer ACL — `CustomerGateway`

- **Our term**: **Customer Context Card**.
- **Crosses the boundary as**: `CustomerContextCard { customer_id, name, health_status,
  relationship_summary, deep_link }` — permission-filtered by construction: the query itself is scoped to
  "assigned or permitted" per research.md, not filtered client-side after a broader fetch.
- **Outbound**: `RequestAssignedCustomerContextQuery { consultant_id, customer_id? }`.
- **Inbound**: `CustomerHealthChanged`, `CustomerInteractionLogged` — feed dashboard cards / notifications.

---

## 6. Execution ACL — `ExecutionGateway`

- **Our term**: **Engagement Workspace Snapshot**.
- **Crosses the boundary as**: `EngagementSnapshot { engagement_id, workstreams[], milestones[], tasks[],
  delivery_status, deep_link }` — a read projection of "the consultant's assigned delivery workspace"; this
  repo never becomes a second store of tasks/milestones.
- **Outbound**: `RequestAssignedEngagementsQuery { consultant_id }`.
- **Inbound**: `MilestoneCompleted`, `DeliveryRiskRaised`, `TaskAssigned` — the latter two are natural
  `ActionQueueEntry` sources.

---

## 7. Products ACL — `ProductsGateway`

- **Our term**: **Product Reference Card**.
- **Crosses the boundary as**: `ProductReferenceCard { product_id, name, packaging_summary,
  pricing_guidance, demo_assets[] }` — approved-for-selling snapshot only, per research.md ("Consultants
  receive approved product information").
- **Outbound**: `RequestProductCatalogQuery { filters? }`.
- **Inbound**: `ProductCatalogUpdated` (dashboard/reference refresh; low priority, unlikely to warrant an
  `ActionQueueEntry`).

---

## 8. Landscape ACL — `LandscapeGateway`

- **Our term**: **Market Intelligence Digest** (read) / **Field Observation** (write).
- **Crosses the boundary as**: `IntelligenceDigestItem { intel_id, topic, summary, published_at, deep_link }`
  inbound; `FieldObservationSubmission { observation_text, related_company_reference?, submitted_by }`
  outbound.
- **Outbound**: `SubmitFieldObservationCommand`.
- **Inbound**: `IntelligenceItemPublished` — feeds notifications; this is the one context where this repo
  is a minor upstream contributor as well as a consumer (see relationship note in `domain-map.md`), but
  Landscape still governs what counts as "approved" — this repo's ACL has no concept of publishing directly.

---

## 9. Legal ACL — `LegalGateway`

- **Our term**: **Approved Legal Snippet**.
- **Crosses the boundary as**: `ApprovedLegalSnippet { clause_id, title, approved_text, policy_reference }`
  — read-only, per research.md ("consume approved legal capabilities without transferring legal ownership").
- **Outbound**: `RequestApprovedClausesQuery { context: proposal_id | topic }`.
- **Inbound**: `LegalClauseUpdated` (rare; mostly relevant to Commit's proposal flow, surfaced here only if a
  proposal-in-progress references a now-stale clause — **assumption**: research.md doesn't detail this
  inbound path explicitly).

---

## 10. Armor ACL — `ArmorGateway`

- **Our term**: **Permission Assertion**. Not a UI-facing "capability" like the others — infrastructural,
  consumed to drive permission-aware presentation (`DashboardConfiguration` invariant #1, nav gating) and to
  propagate identity, per research.md's "Armor operates beneath the consultant experience."
- **Crosses the boundary as**: `PermissionAssertion { consultant_id, capability, scope, expires_at }` — a
  set of grants, never the underlying authorization policy/rules themselves (those stay inside Armor).
- **Outbound**: none from a business-command perspective; the BFF forwards the consultant's authenticated
  session/token for Nexus to propagate to Armor on every call (auth propagation, not a domain command).
- **Inbound**: `PermissionAssertionChanged` (consumed by the Workspace context — see
  `consultant-experience-context.md` §1.3) — the only Armor-originated signal this repo's domain model
  reacts to.

---

## 11. Cross-cutting notes

- **No adapter re-implements business policy.** Every gateway above is a pure translation boundary: request
  shape out, opaque decision/result shape in. None of these ACLs contain conditional business logic beyond
  "is this shape valid to render" — that mirrors research.md's central warning (illustrated in the Sales
  worked example) generalized to all ten integrated contexts.
- **Idempotency at the boundary.** Every inbound event referenced above and consumed into
  `NotificationItem`/`ActionQueueEntry` must carry an origin event id (see
  `consultant-experience-context.md` §2.2) — this is an ACL-level contract requirement, not just an
  aggregate-level one, since Nexus may redeliver events on retry.
- **Capital/Verdict absence is structural.** There is no `CapitalGateway` or `VerdictGateway` module planned
  anywhere in `nexus-client`; adding one would itself be a scope violation of research.md's ownership model
  for this repo.
