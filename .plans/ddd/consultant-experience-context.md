# This Repo's Own Bounded Context(s) — Consultant Experience

Source of truth: `.plans/research.md` §"Role of consultants.cognitum.one" and §"Final Ownership Principle"
("The Consultants repo should remain intentionally thin at the domain level"); scope confirmed against
`.plans/implementation-plan.md` §2.1–2.2.

## 0. One context or two?

**Decision: split into two tightly-coupled bounded contexts**, both owned by this repo:

1. **Consultant Workspace** — session-scoped configuration and coordination: shell/navigation, dashboard
   composition, consultant preferences, cross-capability workflow/deep-link sessions.
2. **Notification & Action Queue** — the aggregated, event-driven feed of things happening across
   capabilities that need a consultant's attention.

**Justification:**

- **Different consistency/lifecycle shape.** Workspace aggregates are long-lived, low-churn, and edited
  directly by the consultant (a dashboard layout or a preference set changes rarely, on demand).
  Notification/Action Queue aggregates are short-lived, high-churn, and driven by *external* events arriving
  continuously from Nexus — the invariants that matter (idempotent ingestion, expiry, no duplicate
  notifications) are unrelated to the invariants that matter for a dashboard layout (valid card references,
  permission fit).
- **research.md itself lists them as separate line items** ("One dashboard... One notification centre...
  One task list") rather than describing notifications as a property of the dashboard.
- **The implementation plan schedules them in different phases** (Phase 1: shell/dashboard/preferences;
  Phase 3: notifications/action queue) with a distinct open question (§6.5: "Where does notification/
  action-queue state live?") that doesn't apply to the Workspace context — a sign they carry different
  design risk and deserve separate boundaries.
- They remain "tightly related" (not fully independent) because they share the same consultant identity,
  the same permission model, and the Workspace context is a natural *trigger* for entries in the
  Notification & Action Queue context (e.g. completing a cross-capability workflow session can raise a
  notification). They communicate via domain events, not shared tables/aggregates.

**Assumption (flagged):** research.md does not explicitly say these should be separate bounded contexts —
this is a modeling judgment call based on lifecycle/invariant differences and the plan's phase boundaries,
not a stated requirement.

Both contexts obey the repo-wide invariant from research.md: **they own zero business records**. Every
aggregate below stores only view-state, configuration, or transient coordination state — never a copy of a
lead, proposal, course, engagement, etc. Business facts are always referenced by opaque external ID plus a
display-safe summary, never duplicated as a system of record.

---

## 1. Context: Consultant Workspace

### 1.1 Ubiquitous language glossary

| Term | Meaning |
|---|---|
| **Consultant** | The authenticated user of this application. Identity and role data originate from Armor (via Nexus); this context only holds a reference (`consultant_id`) and presentation-relevant projections of it. |
| **Dashboard Configuration** | A consultant's personal arrangement of dashboard cards/modules — which capability widgets appear, in what order/layout. |
| **Card / Module** | A single dashboard widget bound to one external capability's data (e.g. "Open Proposals" card bound to Commit). Not a business object — a presentation slot. |
| **Navigation Entry** | One item in the consultant's nav menu, permission-gated. |
| **Preference** | A single named, typed, consultant-scoped UI setting (theme, density, default landing page, notification channel opt-in/out). |
| **Workflow Session** (a.k.a. cross-capability workflow / deep-link session) | Transient, correlation-tracked state describing an in-progress hop between capabilities (e.g. "consultant is mid-flow moving a Sales lead into a Commit proposal"). Holds only correlation/reference data, never the business payload itself. |
| **Deep Link** | A URL/route into this app that resolves to a specific capability + record reference + optional workflow session, used to resume a cross-capability flow. |
| **Permission Assertion** | A capability/scope grant sourced from Armor (via Nexus) that this context reads but never issues or overrides — used purely to decide what to render. |

### 1.2 Aggregate roots

#### `DashboardConfiguration`
- **Root of aggregate**: yes. Contains `CardPlacement` entities (module id, position, external capability
  reference) as child entities — no independent identity/lifecycle outside the configuration.
- **Invariants:**
  1. Every `CardPlacement.module_id` must reference a capability module the consultant currently holds a
     Permission Assertion for — a card the consultant isn't permitted to see cannot be persisted into their
     configuration (permission-aware presentation is enforced at the aggregate boundary, not just in the UI).
  2. Card positions within one configuration must be unique (no two cards claim the same slot).
  3. Exactly one `DashboardConfiguration` exists per consultant (one dashboard, per research.md's "One
     dashboard" requirement).
  4. A configuration referencing zero cards is valid (freshly onboarded consultant) but a default card set
     is applied at creation time — **assumption**: research.md doesn't specify default-card behavior; this
     is inferred from "One dashboard composition" being a baseline expectation, not something a consultant
     must build from a blank slate.

#### `ConsultantPreferences`
- **Root of aggregate**: yes. Simple key/typed-value bag, no meaningful child entities.
- **Invariants:**
  1. Every preference key must belong to a known, versioned allow-list of preference types (schema
     validated) — this context does not accept arbitrary key/value pairs, to keep it "thin, presentation-
     only state" per the implementation plan's explicit framing (§2.1).
  2. Exactly one `ConsultantPreferences` aggregate exists per consultant.
  3. Preference values never encode business data (e.g. a "default customer filter" preference stores a
     customer *reference id*, never a cached customer record).

#### `CrossCapabilityWorkflowSession`
- **Root of aggregate**: yes.
- **Invariants:**
  1. Must reference an origin capability + external record id, and (once resolved) a target capability +
     external record id — both by opaque reference only; the session never stores the underlying business
     entity.
  2. Has a bounded time-to-live; a session past its `expires_at` cannot be resumed or transitioned — it must
     be re-initiated. **Assumption**: research.md doesn't give a TTL value; a finite TTL is inferred from
     general BFF-session hygiene and to avoid this context silently becoming a long-term store of
     cross-capability state (which would violate "owns zero business records" if left unbounded).
  3. A session's status is a linear state machine: `started → in_progress → { completed | abandoned |
     expired }` — no transition out of a terminal state.
  4. Completion of a session does not itself mutate the target capability's data; it only records that the
     consultant was handed off successfully. The actual mutation (e.g. proposal created) is owned and
     confirmed by the target capability via Nexus.

### 1.3 Domain events (Workspace context)

See `domain-events.md` for the full catalog; summarized here for context-completeness:

- **Raises**: `DashboardConfigurationUpdated`, `ConsultantPreferencesUpdated`, `WorkflowSessionStarted`,
  `WorkflowSessionCompleted`, `WorkflowSessionExpired`.
- **Consumes**: `PermissionAssertionChanged` (from Armor, via Nexus — used to re-validate
  `DashboardConfiguration` invariant #1 when a consultant's access changes).

### 1.4 Repository interfaces

```text
DashboardConfigurationRepository
  - find_by_consultant_id(consultant_id) -> Option<DashboardConfiguration>
  - save(config: DashboardConfiguration) -> Result<()>
  - delete_by_consultant_id(consultant_id) -> Result<()>          # e.g. on offboarding

ConsultantPreferencesRepository
  - find_by_consultant_id(consultant_id) -> Option<ConsultantPreferences>
  - save(prefs: ConsultantPreferences) -> Result<()>
  - upsert_preference(consultant_id, key, value) -> Result<()>     # narrow, high-frequency path

CrossCapabilityWorkflowSessionRepository
  - find_by_id(session_id) -> Option<CrossCapabilityWorkflowSession>
  - find_active_by_consultant_id(consultant_id) -> Vec<CrossCapabilityWorkflowSession>
  - save(session: CrossCapabilityWorkflowSession) -> Result<()>
  - expire_older_than(cutoff: Timestamp) -> Result<u64>           # housekeeping sweep
```

---

## 2. Context: Notification & Action Queue

### 2.1 Ubiquitous language glossary

| Term | Meaning |
|---|---|
| **Notification Item** | A single, display-ready, read-tracked entry summarizing an event that occurred in an external capability, aggregated here for the consultant's attention. |
| **Action Queue Entry** | A notification-like item that additionally represents something the consultant is expected to *act on* (approve, respond, review) — carries a status beyond read/unread. |
| **Origin Capability** | Which external context (Sales, Commit, Edu, ...) produced the underlying event. |
| **Origin Event Id** | The idempotency key from the origin capability/Nexus used to prevent duplicate ingestion of the same upstream event. |
| **Read State** | Local, presentation-only tracking of whether the consultant has seen a Notification Item. |
| **Action State** | Local tracking of an Action Queue Entry's progress (`pending`, `in_progress`, `completed`, `expired`) — a *mirror* of the real action's status, never the authoritative record of it. |
| **Deep Link Reference** | The pointer (capability + record id + optional workflow session id) a Notification Item or Action Queue Entry carries so the consultant can jump to the source. |

### 2.2 Aggregate roots

#### `NotificationItem`
- **Root of aggregate**: yes.
- **Invariants:**
  1. `(origin_capability, origin_event_id)` is unique — re-delivery of the same upstream event from Nexus
     must not create a duplicate `NotificationItem` (idempotent ingestion).
  2. Payload is limited to a display-safe summary (title, short body, deep link reference, severity/category)
     — never the full business object. This directly enforces the "owns zero business records" rule at the
     aggregate boundary, since notifications are the highest-risk place for a full payload to leak in by
     accident.
  3. `Read State` transitions only `unread → read`; there is no "unread again" transition (matches ordinary
     inbox semantics). **Assumption**: research.md doesn't specify this; inferred from standard notification-
     center behavior since it isn't a detail research.md addresses.
  4. Belongs to exactly one consultant; never shared/broadcast as a single row (a capability-level event that
     is relevant to multiple consultants fans out into multiple `NotificationItem` aggregates).

#### `ActionQueueEntry`
- **Root of aggregate**: yes.
- **Invariants:**
  1. Same idempotent-ingestion rule as `NotificationItem` via `(origin_capability, origin_event_id)`.
  2. `Action State` is a linear state machine: `pending → in_progress → { completed | expired }` — no
     regression from a terminal state.
  3. **This context cannot locally mark the underlying business action complete.** `completed` may only be
     set in response to a confirmation event routed back through Nexus from the owning capability (e.g.
     Sales confirms a collaboration request was resolved) — never by direct consultant click alone. The
     consultant's click *initiates* a command through Nexus; the entry's state changes only when the owning
     capability's event confirms it. This is the direct analog, for action items, of research.md's rule that
     "the Consultants frontend must not independently decide" business policy.
  4. Has an `expires_at` mirrored from (or defaulted relative to) the origin event, after which unresolved
     entries move to `expired` rather than lingering indefinitely.

### 2.3 Domain events (Notification & Action Queue context)

- **Consumes** (via Nexus, from any of the ten integrated external contexts): capability-specific events
  normalized into `CapabilityEventReceived` (envelope), which this context maps into either a
  `NotificationItem` or `ActionQueueEntry` depending on whether the event implies a required consultant
  action.
- **Raises**: `NotificationRead`, `NotificationDismissed`, `ActionQueueEntryStarted`,
  `ActionQueueEntryCompleted`, `ActionQueueEntryExpired`.
- **Consumes internally** (from the Workspace context): none required, but `WorkflowSessionCompleted` is a
  natural future trigger for a confirmation `NotificationItem` ("Your proposal handoff finished") —
  **assumption**: not required by research.md, flagged as a plausible future wiring, not a committed one.

### 2.4 Repository interfaces

```text
NotificationRepository
  - find_by_id(notification_id) -> Option<NotificationItem>
  - find_unread_by_consultant_id(consultant_id) -> Vec<NotificationItem>
  - find_by_origin_event(origin_capability, origin_event_id) -> Option<NotificationItem>  # idempotency check
  - save(item: NotificationItem) -> Result<()>
  - mark_read(notification_id) -> Result<()>
  - mark_dismissed(notification_id) -> Result<()>
  - purge_older_than(cutoff: Timestamp) -> Result<u64>

ActionQueueRepository
  - find_by_id(entry_id) -> Option<ActionQueueEntry>
  - find_pending_by_consultant_id(consultant_id) -> Vec<ActionQueueEntry>
  - find_by_origin_event(origin_capability, origin_event_id) -> Option<ActionQueueEntry>  # idempotency check
  - save(entry: ActionQueueEntry) -> Result<()>
  - update_action_state(entry_id, new_state) -> Result<()>
  - expire_older_than(cutoff: Timestamp) -> Result<u64>
```

### 2.5 Open persistence question (carried from implementation-plan.md §6.5)

Both repository interfaces above are written against an abstract store. The implementation plan flags that
*where* this view-state physically lives (embedded DB, Postgres, Redis, or something Nexus-provided) is an
open ADR decision, not resolved here. This DDD model is written to be indifferent to that choice — the
invariants and repository contracts hold regardless of backing store, which is deliberate: whichever storage
ADR is chosen should not require re-deriving the aggregate boundaries above.
