# Domain Event Catalog — consultants.cognitum.one

Consolidates every domain event raised or consumed across this repo's two bounded contexts (Consultant
Workspace; Notification & Action Queue — see `consultant-experience-context.md`) and the ten external ACL
adapters (see `anti-corruption-layers.md`). All cross-context traffic with external services flows through
`nexus.cognitum.one`; "Source" / "Destination" below name the *business* context on the other end of that
routing, not Nexus itself (Nexus is transport/normalization, not a party to the domain conversation).

Legend for **Direction**: `raises` = this repo is the origin; `consumes` = this repo reacts to an event
whose origin is external (or, for internal cross-context events, the other internal context).

## 1. Consultant Workspace context

| Event | Direction | One-line description | Rough payload | Source → Destination |
|---|---|---|---|---|
| `DashboardConfigurationUpdated` | raises | A consultant's dashboard card layout changed. | `consultant_id, card_placements[], updated_at` | Consultant Workspace → (internal only; no external subscriber known) |
| `ConsultantPreferencesUpdated` | raises | A consultant changed one or more UI preferences. | `consultant_id, changed_keys[], updated_at` | Consultant Workspace → (internal only) |
| `WorkflowSessionStarted` | raises | A cross-capability deep-link/workflow session began. | `session_id, consultant_id, origin_capability, origin_reference, started_at` | Consultant Workspace → Notification & Action Queue (potential future trigger — flagged as assumption in `consultant-experience-context.md` §2.3) |
| `WorkflowSessionCompleted` | raises | A cross-capability workflow session reached a successful handoff. | `session_id, target_capability, target_reference, completed_at` | Consultant Workspace → Notification & Action Queue (same assumption) |
| `WorkflowSessionExpired` | raises | A workflow session's TTL elapsed without completion. | `session_id, expired_at` | Consultant Workspace → (internal only) |
| `PermissionAssertionChanged` | consumes | The consultant's capability/scope grants changed. | `consultant_id, capability, scope, expires_at` | Armor (via Nexus) → Consultant Workspace |

## 2. Notification & Action Queue context

| Event | Direction | One-line description | Rough payload | Source → Destination |
|---|---|---|---|---|
| `CapabilityEventReceived` | consumes | Normalized envelope for any upstream capability event, prior to being classified as a notification or action item. | `origin_capability, origin_event_id, event_type, summary, deep_link, received_at` | Any of the 10 external contexts (via Nexus) → Notification & Action Queue |
| `NotificationRead` | raises | Consultant viewed a notification. | `notification_id, consultant_id, read_at` | Notification & Action Queue → (internal only) |
| `NotificationDismissed` | raises | Consultant dismissed a notification without treating it as an action. | `notification_id, consultant_id, dismissed_at` | Notification & Action Queue → (internal only) |
| `ActionQueueEntryStarted` | raises | Consultant began acting on a queue entry (command issued to the owning capability). | `entry_id, consultant_id, started_at` | Notification & Action Queue → owning capability (via Nexus) |
| `ActionQueueEntryCompleted` | consumes | Owning capability confirmed the underlying action finished. **Never raised locally without this confirmation** (see invariant #3 in `consultant-experience-context.md` §2.2). | `entry_id, origin_capability, confirmed_at` | Owning capability (via Nexus) → Notification & Action Queue |
| `ActionQueueEntryExpired` | raises | An unresolved queue entry passed its expiry. | `entry_id, expired_at` | Notification & Action Queue → (internal only) |

## 3. External ACL events, by context (detail in `anti-corruption-layers.md`)

### Sales
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `CheckAccountClaimCommand` | raises (outbound) | Ask Sales to evaluate a company for conflicts. | `company_name, normalized_domain?, consultant_id` |
| `AccountClaimDetermined` | consumes (inbound) | Sales' policy verdict on a company claim. | `match_status, creation_allowed, display_message, permitted_actions[]` |
| `RequestCollaborationCommand` | raises (outbound) | Consultant requests to collaborate on an owned account. | `company_reference, consultant_id, message?` |
| `CollaborationRequestAcknowledged` | consumes (inbound) | Sales acknowledged the collaboration request. | `company_reference, status` |
| `SubmitReferralCommand` | raises (outbound) | Consultant submits a referral instead of a direct claim. | `company_reference, consultant_id, notes?` |
| `ReferralSubmitted` | consumes (inbound) | Confirmation of referral receipt. | `referral_id, status` |

### Commit
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `CreateProposalCommand` | raises (outbound) | Start a proposal from a cross-capability origin. | `origin_reference, consultant_id` |
| `ProposalCreated` | consumes (inbound) | Commit confirms a proposal now exists. | `proposal_id, status` |
| `ProposalStatusChanged` | consumes (inbound) | Proposal moved stage (e.g. sent → accepted). | `proposal_id, old_status, new_status` |
| `ProposalAccepted` | consumes (inbound) | Terminal positive outcome. | `proposal_id, accepted_at` |

### Edu
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `RequestLearningCatalogQuery` | raises (outbound) | Fetch a consultant's learning snapshot. | `consultant_id, filters?` |
| `CourseCompleted` | consumes (inbound) | A course was finished. | `course_id, consultant_id, completed_at` |
| `CertificationIssued` | consumes (inbound) | A certification was granted. | `certification_id, consultant_id, issued_at` |
| `TrainingRequirementDue` | consumes (inbound) | A required training is approaching/past due. | `requirement_id, consultant_id, due_at` |

### Capacity
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `UpdateOwnProfileCommand` | raises (outbound) | Consultant updates their own restricted profile. | `consultant_id, profile_fields` |
| `ProfileUpdateAccepted` | consumes (inbound) | Update accepted. | `consultant_id, accepted_at` |
| `ProfileUpdateRejected` | consumes (inbound) | Update rejected (e.g. validation failure upstream). | `consultant_id, reason` |

### Customer
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `RequestAssignedCustomerContextQuery` | raises (outbound) | Fetch permitted customer context. | `consultant_id, customer_id?` |
| `CustomerHealthChanged` | consumes (inbound) | A customer's health status changed. | `customer_id, old_status, new_status` |
| `CustomerInteractionLogged` | consumes (inbound) | A new interaction was recorded. | `customer_id, interaction_summary, logged_at` |

### Execution
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `RequestAssignedEngagementsQuery` | raises (outbound) | Fetch the consultant's delivery workspace. | `consultant_id` |
| `MilestoneCompleted` | consumes (inbound) | A milestone finished. | `engagement_id, milestone_id, completed_at` |
| `DeliveryRiskRaised` | consumes (inbound) | A new risk/issue was flagged. | `engagement_id, risk_id, severity` |
| `TaskAssigned` | consumes (inbound) | A task was assigned to the consultant. | `engagement_id, task_id, assigned_at` |

### Products
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `RequestProductCatalogQuery` | raises (outbound) | Fetch approved product/service reference data. | `filters?` |
| `ProductCatalogUpdated` | consumes (inbound) | Catalog content changed. | `product_id, changed_fields[]` |

### Landscape
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `SubmitFieldObservationCommand` | raises (outbound) | Consultant submits a field observation. | `observation_text, related_company_reference?, submitted_by` |
| `IntelligenceItemPublished` | consumes (inbound) | New approved intelligence item available. | `intel_id, topic, published_at` |

### Legal
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `RequestApprovedClausesQuery` | raises (outbound) | Fetch approved clause text for a proposal/topic. | `context (proposal_id \| topic)` |
| `LegalClauseUpdated` | consumes (inbound) | An approved clause changed (assumption — see `anti-corruption-layers.md` §9). | `clause_id, policy_reference` |

### Armor
| Event/Command | Direction | Description | Rough payload |
|---|---|---|---|
| `PermissionAssertionChanged` | consumes (inbound) | Consultant's grants changed (also listed in §1 as the Workspace context's consumed event — same event, single origin). | `consultant_id, capability, scope, expires_at` |

## 4. Explicitly out of scope

No events are cataloged for **Capital** or **Verdict** — this repo has no ACL adapter for either (see
`anti-corruption-layers.md` §11); any future event named e.g. `CapitalFigureUpdated` appearing in this
repo's code would itself indicate a scope violation against `research.md`.
