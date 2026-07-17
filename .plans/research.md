Correct. The ownership hierarchy should be:

1. **`manage.cognitum.one`** — highest-level enterprise oversight
2. **Sub-business stack projects** — own the actual business capabilities, records, and workflows
3. **`consultants.cognitum.one`** — central consultant-facing workspace that composes those capabilities

The consultants repo is primarily an **experience and orchestration layer**, not the owner of the underlying business functions.

# Corrected Cognitum One Consultants Architecture

## Architectural Hierarchy

```text
manage.cognitum.one
Highest-level oversight, governance, administration, and performance visibility
    │
    ▼
Cognitum One sub-business stack
Authoritative capabilities, records, rules, workflows, and intelligence
    │
    ▼
consultants.cognitum.one
Unified consultant-facing workspace
```

`manage.cognitum.one` sits above the entire business stack.

It should provide executive and administrative oversight across:

- Consultants
- Sales
- Customers
- Proposals and commitments
- Delivery and execution
- Products
- Education
- Capacity
- Legal
- Capital
- Security
- Market intelligence
- Business performance

It is not the source of every underlying record or workflow.

---

# Role of `consultants.cognitum.one`

`consultants.cognitum.one` is the single interface consultants use to perform their work.

It should provide:

- One login
- One dashboard
- One navigation system
- One activity feed
- One notification centre
- One task list
- One unified workflow experience

However, it should not become the authoritative owner of leads, proposals, courses, products, customers, engagements, or capacity records.

Its responsibilities are:

- Consultant-facing application shell
- Consultant-specific navigation
- Dashboard composition
- Cross-capability workflow coordination
- Permission-aware presentation
- Aggregated consultant views
- Notifications and action queues
- Frontend routing
- Backend-for-frontend aggregation
- Consultant-specific preferences
- Deep links and transitions between capabilities

---

# Authoritative Ownership

## Sales

Owned by:

```text sales.cognitum.one ```

Sales owns:

- Companies
- Leads
- Contacts
- Opportunities
- Pipeline stages
- Sales activities
- Outreach tools
- Discovery tools
- Account ownership
- Lead protection
- Duplicate detection
- Collaboration requests
- Referral attribution

The consultants portal displays and operates these capabilities through the Sales service.

```text
consultants.cognitum.one/sales
        ↓
sales.cognitum.one
```

The warning that another consultant is already working a company or lead is therefore a **Sales business rule**, displayed inside the Consultants workspace.

---

## Proposals and Commitments

Owned by:

```text commit.cognitum.one ```

Commit owns:

- Proposal templates
- Proposal generation
- Scope
- Deliverables
- Timelines
- Pricing workflows
- Approval routing
- Proposal revisions
- Proposal sending
- Acceptance
- Commitments
- Statement-of-work generation

The consultants portal provides the consultant-facing proposal experience.

```text
consultants.cognitum.one/proposals
        ↓
commit.cognitum.one
```

Consultants create and manage proposals centrally, but Commit remains authoritative.

---

## Education

Owned by:

```text edu.cognitum.one ```

Edu owns:

- Courses
- Learning paths
- Assessments
- Certifications
- Completion records
- Training requirements
- Consultant playbooks
- Education content
- Credential validation

The education experience appears inside:

```text consultants.cognitum.one/education ```

but is powered by the Edu repository and services.

---

## Capacity and Consultant Expertise

Owned by:

```text capacity.cognitum.one ```

Capacity owns:

- Consultant skills
- Expertise
- Industries
- Certifications
- Languages
- Availability
- Geographic coverage
- Experience
- Internal capacity analysis
- Resource matching
- Staffing intelligence

Consultants must not receive internal Capacity access.

The Consultants workspace presents only a restricted consultant profile form:

```text
consultants.cognitum.one/profile
        ↓
capacity.cognitum.one consultant intake capability
```

Consultants can update their own information but cannot see internal capacity planning or other consultants.

---

## Customer Records

Owned by:

```text customer.cognitum.one ```

Customer owns:

- Customer accounts
- Stakeholders
- Relationship history
- Customer health
- Customer interactions
- Customer outcomes
- Account context

The consultants portal displays only assigned or permitted customer information.

---

## Engagement Delivery

Owned by:

```text execution.cognitum.one ```

Execution owns:

- Engagements
- Workstreams
- Tasks
- Milestones
- Deliverables
- Risks
- Issues
- Dependencies
- Delivery status
- Completion evidence

The consultants portal provides the consultant’s assigned delivery workspace.

---

## Products and Services

Owned by:

```text products.cognitum.one ```

Products owns:

- Product catalogue
- Services catalogue
- Capabilities
- Approved use cases
- Packaging
- Pricing guidance
- Product limitations
- Demo assets
- Product requirements

Consultants receive approved product information for selling and proposal generation.

---

## Market and Competitive Intelligence

Owned by:

```text landscape.cognitum.one ```

Landscape owns:

- Market research
- Industry intelligence
- Competitive intelligence
- Regulatory intelligence
- Strategic signals
- Buyer intelligence
- Research sources

Consultants consume approved intelligence and may submit field observations.

---

## Legal

Owned by:

```text legal.cognitum.one ```

Legal owns:

- Approved clauses
- Legal templates
- Contract policies
- Review workflows
- Exceptions
- NDAs
- Legal approvals
- Compliance requirements

Commit and Consultants consume approved legal capabilities without transferring legal ownership to either application.

---

## Security and Access

Owned by:

```text armor.cognitum.one ```

Armor owns:

- Authorization policy
- Access enforcement
- Data classification
- Security controls
- Access expiry
- Approval requirements
- Restricted-account rules
- Audit policy

Consultants do not need a general Armor interface.

Armor operates beneath the consultant experience.

---

## Integration and Routing

Owned by:

```text nexus.cognitum.one ```

Nexus owns:

- Unified service wrappers
- Service routing
- Cross-domain invocation
- Authentication propagation
- Workflow coordination
- Event routing
- **API** normalization

Consultants should not directly access Nexus.

---

# Role of `manage.cognitum.one`

`manage.cognitum.one` is the highest-level oversight and administrative control plane.

It should aggregate data and controls from the underlying business stack.

```text
manage.cognitum.one
│
├── Sales oversight
│     sales.cognitum.one
│
├── Proposal and commitment oversight
│     commit.cognitum.one
│
├── Consultant oversight
│     capacity, sales, edu, execution, commit
│
├── Customer oversight
│     customer.cognitum.one
│
├── Delivery oversight
│     execution.cognitum.one
│
├── Financial oversight
│     capital.cognitum.one
│
├── Legal oversight
│     legal.cognitum.one
│
├── Security oversight
│     armor.cognitum.one
│
└── Enterprise decision oversight
      verdict.cognitum.one
```

The future Consultants section inside Manage should therefore aggregate information from several sub-business systems.

---

# Manage Consultants Section

```text
manage.cognitum.one/consultants
│
├── Consultant Directory
│     capacity.cognitum.one
│
├── Skills and Expertise
│     capacity.cognitum.one
│
├── Certifications
│     edu.cognitum.one
│
├── Leads and Pipeline
│     sales.cognitum.one
│
├── Account Ownership
│     sales.cognitum.one
│
├── Proposals
│     commit.cognitum.one
│
├── Active Engagements
│     execution.cognitum.one
│
├── Customers
│     customer.cognitum.one
│
├── Earnings and Economics
│     capital.cognitum.one
│
├── Legal and Compliance
│     legal.cognitum.one
│
└── Access and Security
      armor.cognitum.one
```

The Consultants section in Manage does not need to treat `consultants.cognitum.one` as the system of record.

Instead, it uses the same underlying domain services while presenting higher-level oversight and administrative actions.

---

# Dashboard Relationship

The new Consultants application should receive a copy of the visual dashboard foundation currently used by `manage.cognitum.one`.

```text
manage dashboard code and design
    │
    ├── copy shell structure
    ├── copy layout patterns
    ├── copy reusable components
    └── remove Manage-specific business logic
    ↓
consultants.cognitum.one
independent consultant-facing application
```

This copy provides:

- Layout
- Sidebar
- Header
- Cards
- Tables
- Forms
- Search
- Filters
- Alerts
- Dialogs
- Responsive behaviour
- Cognitum One styling

It does not transfer ownership of Manage workflows or administrative data.

Long term, low-level reusable components may be extracted into:

```text @cognitum/design-system @cognitum/dashboard-components ```

---

# Correct Integration Model

```text
Consultant
    │
    ▼
consultants.cognitum.one
Unified consultant experience
    │
    ▼
nexus.cognitum.one
Capability routing
    │
    ├── sales.cognitum.one
    ├── commit.cognitum.one
    ├── edu.cognitum.one
    ├── customer.cognitum.one
    ├── execution.cognitum.one
    ├── products.cognitum.one
    ├── landscape.cognitum.one
    ├── capacity.cognitum.one
    ├── legal.cognitum.one
    └── armor.cognitum.one
```

Separately:

```text
Executive or administrator
    │
    ▼
manage.cognitum.one
Enterprise oversight
    │
    ▼
The same underlying sub-business stack
```

---

# Lead Conflict Warning

The lead ownership warning should be owned and enforced by Sales.

```text
Consultant enters company
        ↓
Consultants portal sends normalized information to Sales
        ↓
Sales checks companies, leads, contacts, and opportunities
        ↓
Customer may provide existing account relationships
        ↓
Sales determines ownership and conflict status
        ↓
Consultants portal displays the permitted warning and actions
```

Possible response:

```json
{
    *match_status*: *active_owned_account*,
    *creation_allowed*: false,
    *display_message*: *This company is already being worked.*,
    *permitted_actions*: [
    *request_collaboration*,
    *submit_referral*,
    *cancel*
    ]
}
```

The Consultants frontend must not independently decide whether a competing lead may be created.

That policy belongs to Sales and is enforced by the Sales **API**.

---

# Final Ownership Principle

> `manage.cognitum.one` is the enterprise oversight layer.

> The sub-business stack owns the actual business capabilities, records, policies, and workflows.

> `consultants.cognitum.one` is the unified consultant-facing operating workspace that composes those capabilities into one experience.

The Consultants repo should remain intentionally thin at the domain level. It owns the consultant experience, not the entire consultant business system.