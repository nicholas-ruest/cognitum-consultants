import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'

/**
 * Mock Nexus HTTP server (PROMPT-27) — stands in for `nexus.cognitum.one`
 * at the HTTP boundary, per ADR-007: `nexus-client`'s `ReqwestNexusTransport`
 * only needs `NEXUS_ENDPOINT_URL` pointed at *something* that speaks the
 * real capability envelope `crates/nexus-client/src/transport.rs`'s
 * `CapabilityCaller` issues (ADR-029: `POST capabilities/:capability_id`,
 * body a `CapabilityRequest` envelope, response a `CapabilityResponse`
 * envelope) — it has no idea (and no way to tell) whether that's the real
 * Nexus or this stand-in. This is the same pattern manual verification used
 * in PROMPT-18/19/23, formalized here as a reusable module so Phase 4
 * (PROMPT-34+) can start one of these per capability without re-deriving
 * the wiring.
 *
 * Deliberately plain `node:http` (no Express/msw dependency) — the surface
 * this test needs is one route (plus a handful of test-only inspection
 * routes and the still-unmigrated events-poll endpoint — see below), which
 * plain `http.createServer` covers with zero added dependencies.
 *
 * # One route, all ten gateways (ADR-029)
 * `Implement ADR-029: wire consultants-portal off guessed REST paths onto
 * nexus's real capability envelope` (commit 8ea7a9b) migrated every
 * `nexus-client` gateway off its own provisional REST-ish path
 * (`armor/v1/assertions`, `sales/v1/account-claims`, etc.) onto one real
 * route nexus-server exposes: `POST capabilities/:capability_id`, with the
 * request/response bodies wrapped in the `CapabilityRequest`/
 * `CapabilityResponse` envelope `crates/nexus-client/src/transport.rs`
 * builds and unwraps. This mock was not updated in that commit, so it kept
 * answering the old per-capability REST paths — invisible until this
 * harness's own login/e2e regressions were fixed enough to reach an actual
 * `GET`/`PUT /api/dashboard` call, at which point every dashboard-gating
 * `is_permitted` check failed (the real `bff-api` was issuing
 * `POST capabilities/armor.assertions`, this mock 404is on it, `bff-api`
 * fell back to an empty assertion set) — see this repo's e2e-fix commit
 * history for the full diagnosis. This file now answers the one real route
 * instead, dispatching on `capability_id` (and, for the three ids two
 * gateway methods share — `commit.proposals`, `capacity.profile`,
 * `execution.task_completions` — on the request payload's shape, exactly
 * the way each gateway's own module docs describe disambiguating them).
 *
 * # Fixtures are fixed, not dynamically configurable
 * This server always grants the `"sales"`, `"commit"`, `"edu"`, `"capacity"`,
 * `"customer"`, `"execution"`, and `"products"` capabilities (for whichever
 * `consultant_id` is asked about) and always answers a company-claim check
 * for most company names with the `active_owned_account` fixture —
 * `.plans/ddd/anti-corruption-layers.md` §1's worked example, and the same
 * fixture `crates/bff-api/src/sales.rs`'s own integration tests use
 * (`active_owned_account_fixture()`). Edu's catalog capability (PROMPT-35)
 * always answers with the fixed `LEARNING_CATALOG_FIXTURE` below, matching
 * that module's own inline doc comment. Capacity's profile capability
 * (PROMPT-36) serves a single, stateful `PROFILE_FIXTURE` per
 * `consultant_id`: a read returns whatever is currently stored (seeded from
 * `PROFILE_FIXTURE`), and a write always accepts the update and overwrites
 * the stored profile with it — see the module docs for why a stateful
 * round-trip was chosen over a fixed always-the-same-response fixture for
 * this one capability. Customer's context capability (PROMPT-37) always
 * answers with the fixed `CUSTOMER_CONTEXT_FIXTURE` below, matching Edu's
 * "fixed, not dynamically configurable" pattern. Execution's
 * `execution.task_completions` capability (PROMPT-38) always answers an
 * engagements-query payload with the fixed `ENGAGEMENT_SNAPSHOT_FIXTURE`
 * below, same pattern; a task-completion payload records every request it
 * receives (mirroring `proposalActions`/`referrals` below) so a spec can
 * assert the BFF forwarded the expected `task_id`/`consultant_id`.
 * Products' catalog capability (PROMPT-39) always answers with the fixed
 * `PRODUCT_CATALOG_FIXTURE` below, same "fixed, not dynamically
 * configurable" pattern as Edu/Customer/Execution. Landscape's intelligence
 * capability (PROMPT-40) always answers with the fixed
 * `INTELLIGENCE_DIGEST_FIXTURE` below, same pattern; its observations
 * capability records every request it receives (mirroring
 * `taskCompletionRequests`/`capacityProfileUpdates` above) so a spec can
 * assert the BFF forwarded the expected `submitted_by`/`observation_text`.
 * Legal's clauses capability (PROMPT-41) always answers with the fixed
 * `APPROVED_LEGAL_SNIPPET_FIXTURE` below regardless of `proposal_id`/`topic`
 * (this server has no real clause library to filter against), and records
 * every request it receives (mirroring `edu-catalog-requests`/
 * `customer-context-requests` above) so a spec can assert which
 * `proposal_id`/`topic` the BFF actually forwarded.
 *
 * # One exception: the `no_match` fixture, by company name (PROMPT-34)
 * `commit-sales-deeplink.spec.ts` needs a `creation_allowed: true` result to
 * drive the Sales -> Commit deep link (`LeadConflictCheck.tsx`'s "Start
 * Proposal in Commit" affordance only renders on that path). Rather than
 * making the *whole* server's fixture selection dynamic (a bigger change
 * than this one spec needs), a single company name —
 * [`NO_MATCH_COMPANY_NAME`] — is carved out to answer with the `no_match`
 * fixture instead; every other company name keeps answering with
 * `active_owned_account` exactly as before, so `sales-lead-conflict.spec.ts`
 * (which uses "Acme Corp") is unaffected. A future test that needs more than
 * these two fixed scenarios should extend this module with a fully
 * configurable fixture rather than adding more hardcoded exceptions.
 *
 * # Events-poll: not part of the ADR-029 capability envelope
 * `bff-api::event_ingestion`'s polling loop calls `events/v1/poll` directly
 * over a raw `NexusTransport` (`crates/bff-api/src/event_ingestion.rs`),
 * not through `CapabilityCaller` — ADR-029's migration only touched the ten
 * `nexus-client` gateways, not this separate polling mechanism. That route
 * (and the `/_test/...` inspection namespace) is untouched by the ADR-029
 * rewrite below.
 *
 * # Inspection, not shared JS state
 * Playwright test files run in a worker process separate from
 * `global-setup.ts`, so a spec cannot hold a reference to this server's
 * in-memory request log directly. Instead, received commands are recorded
 * in memory here and exposed over HTTP (`GET /_test/...`) so any process
 * that knows this server's base URL can inspect them.
 */

/** `AccountClaimResult` shape, mirrored from `crates/nexus-client/src/sales.rs`. */
interface AccountClaimResult {
  match_status: string
  creation_allowed: boolean
  display_message: string
  permitted_actions: string[]
}

/**
 * `active_owned_account` — the canonical worked example
 * (`anti-corruption-layers.md` §1) and the fixture
 * `crates/bff-api/src/sales.rs`'s tests already use. Chosen over any other
 * `match_status` so this e2e test exercises the exact reference scenario
 * the rest of the codebase's tests are already written against, rather
 * than inventing a parallel scenario with no other test coverage to
 * cross-check it.
 */
const ACCOUNT_CLAIM_FIXTURE: AccountClaimResult = {
  match_status: 'active_owned_account',
  creation_allowed: false,
  display_message: 'This company is already being worked.',
  permitted_actions: ['request_collaboration', 'submit_referral', 'cancel'],
}

/** The one company name (PROMPT-34) answered with the `no_match` fixture
 * below instead of `ACCOUNT_CLAIM_FIXTURE` — see the module docs. */
export const NO_MATCH_COMPANY_NAME = 'Nova Ventures'

const NO_MATCH_ACCOUNT_CLAIM_FIXTURE: AccountClaimResult = {
  match_status: 'no_match',
  creation_allowed: true,
  display_message: 'No matching company found in Sales.',
  permitted_actions: [],
}

export interface RecordedCommand {
  path: string
  body: unknown
  receivedAt: string
}

/** `ProposalSummary` shape, mirrored from `crates/nexus-client/src/commit.rs`. */
export interface ProposalSummary {
  proposal_id: string
  title: string
  status: string
  stage: string
  last_updated_at: string
  deep_link: string | null
}

/**
 * `LearningSnapshot` shape, mirrored from `crates/nexus-client/src/edu.rs`
 * (PROMPT-35).
 */
export interface LearningSnapshot {
  course_id: string
  title: string
  progress_status: string
  certification_status: string | null
  deep_link: string | null
}

/**
 * Fixed `edu.catalog` fixture (PROMPT-35): one completed/certified course,
 * one in-progress course with no certification, and one `not_started`
 * course with a `required` certification (the `LearningDashboard.tsx`
 * "Training Due" heuristic's worked example) — proving the frontend's
 * three-section partition against more than one status combination,
 * mirroring `ACCOUNT_CLAIM_FIXTURE`'s "fixed, not dynamically configurable"
 * rationale above.
 */
const LEARNING_CATALOG_FIXTURE: LearningSnapshot[] = [
  {
    course_id: 'course-1',
    title: 'Cloud Security Fundamentals',
    progress_status: 'completed',
    certification_status: 'issued',
    deep_link: 'https://edu.cognitum.one/courses/course-1',
  },
  {
    course_id: 'course-2',
    title: 'Advanced Negotiation',
    progress_status: 'in_progress',
    certification_status: null,
    deep_link: null,
  },
  {
    course_id: 'course-3',
    title: 'Annual Compliance Refresher',
    progress_status: 'not_started',
    certification_status: 'required',
    deep_link: null,
  },
]

/**
 * `CapabilityEventReceived` shape, mirrored from
 * `crates/bff-core/src/event_ingestion.rs` — the provisional
 * `GET events/v1/poll` contract `bff-api`'s ingestion polling loop
 * (PROMPT-30) expects a bare JSON array of.
 */
/** `ConsultantProfileIntake` shape, mirrored from `crates/nexus-client/src/capacity.rs`. */
export interface ConsultantProfileIntake {
  skills: string[]
  certifications: string[]
  languages: string[]
  availability_window: string
  geographic_coverage: string[]
}

/**
 * Fixed initial `capacity.profile` fixture (PROMPT-36) — seeds the
 * per-`consultant_id` stateful store on first read/write. See the module
 * docs for why this one capability's fixture is stateful rather than fixed.
 */
const PROFILE_FIXTURE: ConsultantProfileIntake = {
  skills: ['Rust', 'Cloud Architecture'],
  certifications: ['AWS Solutions Architect'],
  languages: ['English', 'French'],
  availability_window: '2026-08-01/2026-12-31',
  geographic_coverage: ['EMEA'],
}

/** `CustomerContextCard` shape, mirrored from `crates/nexus-client/src/customer.rs` (PROMPT-37). */
export interface CustomerContextCard {
  customer_id: string
  name: string
  health_status: string
  relationship_summary: string
  deep_link: string | null
}

/**
 * Fixed `customer.context` fixture (PROMPT-37): one healthy customer and one
 * at-risk customer, proving the frontend's list-plus-detail rendering
 * against more than one `health_status` value — mirroring
 * `LEARNING_CATALOG_FIXTURE`'s "fixed, not dynamically configurable"
 * rationale above.
 */
const CUSTOMER_CONTEXT_FIXTURE: CustomerContextCard[] = [
  {
    customer_id: 'customer-1',
    name: 'Acme Corp',
    health_status: 'green',
    relationship_summary: 'Healthy, quarterly business review scheduled.',
    deep_link: 'https://customer.cognitum.one/customers/customer-1',
  },
  {
    customer_id: 'customer-2',
    name: 'Beta LLC',
    health_status: 'red',
    relationship_summary: 'At risk — escalation in progress.',
    deep_link: null,
  },
]

/** `ProductReferenceCard` shape, mirrored from `crates/nexus-client/src/products.rs` (PROMPT-39). */
export interface ProductReferenceCard {
  product_id: string
  name: string
  packaging_summary: string
  pricing_guidance: string
  demo_assets: string[]
}

/**
 * Fixed `products.catalog` fixture (PROMPT-39): one product with a demo
 * asset and one without, proving the frontend's list-plus-detail rendering
 * against both shapes — mirroring `ENGAGEMENT_SNAPSHOT_FIXTURE`'s "fixed,
 * not dynamically configurable" rationale above.
 */
const PRODUCT_CATALOG_FIXTURE: ProductReferenceCard[] = [
  {
    product_id: 'product-1',
    name: 'Cloud Migration Accelerator',
    packaging_summary: '4-week fixed-scope engagement',
    pricing_guidance: 'Starting at $50,000',
    demo_assets: ['https://products.cognitum.one/demos/product-1.mp4'],
  },
  {
    product_id: 'product-2',
    name: 'Security Posture Review',
    packaging_summary: '2-week assessment',
    pricing_guidance: 'Starting at $20,000',
    demo_assets: [],
  },
]

/** `EngagementTaskSummary` shape, mirrored from `crates/nexus-client/src/execution.rs` (PROMPT-38). */
export interface EngagementTaskSummary {
  task_id: string
  title: string
  status: string
}

/** `EngagementSnapshot` shape, mirrored from `crates/nexus-client/src/execution.rs` (PROMPT-38). */
export interface EngagementSnapshot {
  engagement_id: string
  workstreams: string[]
  milestones: string[]
  tasks: EngagementTaskSummary[]
  delivery_status: string
  deep_link: string | null
}

/**
 * Fixed `execution.task_completions` (engagements-query payload) fixture
 * (PROMPT-38): one on-track engagement with an assigned task, proving the
 * frontend's list-plus-detail-plus-tasks rendering — mirroring
 * `CUSTOMER_CONTEXT_FIXTURE`'s "fixed, not dynamically configurable"
 * rationale above.
 */
const ENGAGEMENT_SNAPSHOT_FIXTURE: EngagementSnapshot[] = [
  {
    engagement_id: 'engagement-1',
    workstreams: ['Discovery', 'Delivery'],
    milestones: ['Kickoff complete'],
    tasks: [{ task_id: 'task-1', title: 'Draft delivery plan', status: 'assigned' }],
    delivery_status: 'on_track',
    deep_link: 'https://execution.cognitum.one/engagements/engagement-1',
  },
]

/** `IntelligenceDigestItem` shape, mirrored from `crates/nexus-client/src/landscape.rs` (PROMPT-40). */
export interface IntelligenceDigestItem {
  intel_id: string
  topic: string
  summary: string
  published_at: string
  deep_link: string | null
}

/**
 * Fixed `landscape.intelligence` fixture (PROMPT-40): one item with a deep
 * link, one without, proving the frontend's digest rendering against both
 * shapes — mirroring `ENGAGEMENT_SNAPSHOT_FIXTURE`'s "fixed, not
 * dynamically configurable" rationale above.
 */
const INTELLIGENCE_DIGEST_FIXTURE: IntelligenceDigestItem[] = [
  {
    intel_id: 'intel-1',
    topic: 'Cloud Migration Trends',
    summary: 'Enterprises are accelerating multi-cloud adoption.',
    published_at: '2026-01-01T00:00:00Z',
    deep_link: 'https://landscape.cognitum.one/intel/intel-1',
  },
  {
    intel_id: 'intel-2',
    topic: 'Regulatory Shifts',
    summary: 'New data residency requirements in EMEA.',
    published_at: '2026-01-02T00:00:00Z',
    deep_link: null,
  },
]

/** `ApprovedLegalSnippet` shape, mirrored from `crates/nexus-client/src/legal.rs` (PROMPT-41). */
export interface ApprovedLegalSnippet {
  clause_id: string
  title: string
  approved_text: string
  policy_reference: string
}

/**
 * Fixed `legal.clauses` fixture (PROMPT-41): two approved clauses, proving
 * the frontend's list rendering — mirroring `CUSTOMER_CONTEXT_FIXTURE`'s
 * "fixed, not dynamically configurable" rationale above.
 */
const APPROVED_LEGAL_SNIPPET_FIXTURE: ApprovedLegalSnippet[] = [
  {
    clause_id: 'clause-1',
    title: 'Limitation of Liability',
    approved_text: 'Neither party shall be liable for indirect or consequential damages.',
    policy_reference: 'policy-2026-01',
  },
  {
    clause_id: 'clause-2',
    title: 'Confidentiality',
    approved_text: 'Each party agrees to keep the other party’s confidential information confidential.',
    policy_reference: 'policy-2025-11',
  },
]

export interface CapabilityEventReceived {
  origin_capability: string
  origin_event_id: string
  event_type: string
  summary: string
  deep_link: string | null
  received_at: string
  consultant_id: string
  /**
   * PROMPT-38: only set on confirmation events (e.g. `task_completed`) —
   * the `origin_event_id` of the original `task_assigned` event this
   * confirms, mirroring `crates/bff-core/src/event_ingestion.rs`'s
   * `CapabilityEventReceived::related_origin_event_id`.
   */
  related_origin_event_id?: string
}

/**
 * Inbound `nexus_contracts::CapabilityRequest` envelope, mirrored from
 * `crates/nexus-client/src/transport.rs`'s `CapabilityRequest`. This mock
 * only ever reads `request_id`/`payload` — the identity/bookkeeping fields
 * (`caller`, `organization_id`, `actor`, `correlation_id`, `metadata`) are
 * accepted but not asserted on here (no spec currently needs to).
 */
interface CapabilityRequestEnvelope {
  request_id?: string
  capability_id?: string
  payload?: Record<string, unknown>
}

export interface MockNexusServer {
  url: string
  close: () => Promise<void>
}

function readJsonBody(request: IncomingMessage): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on('data', (chunk: Buffer) => chunks.push(chunk))
    request.on('end', () => {
      if (chunks.length === 0) {
        resolve(undefined)
        return
      }
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')))
      } catch (err) {
        reject(err instanceof Error ? err : new Error(String(err)))
      }
    })
    request.on('error', reject)
  })
}

function sendJson(response: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body)
  response.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) })
  response.end(payload)
}

/** Starts the mock Nexus server on `port`, resolving once it is listening. */
export function startMockNexusServer(port: number): Promise<MockNexusServer> {
  const collaborationRequests: RecordedCommand[] = []
  const referrals: RecordedCommand[] = []
  // PROMPT-34: proposals created via the `commit.proposals` capability
  // (create-shaped payload), keyed by `proposal_id`, so the same
  // capability's list-shaped payload and `commit.proposal_actions` can look
  // them up.
  const proposals = new Map<string, ProposalSummary>()
  const proposalActions: RecordedCommand[] = []
  let nextProposalNumber = 1
  // PROMPT-35: `edu.catalog` requests, recorded so a spec can confirm which
  // `consultant_id` the BFF forwarded.
  const catalogRequests: RecordedCommand[] = []
  // PROMPT-36: per-`consultant_id` Capacity profile store, seeded lazily
  // from `PROFILE_FIXTURE` on first read/write, plus a record of every
  // `capacity.profile` write this server received.
  const capacityProfiles = new Map<string, ConsultantProfileIntake>()
  const capacityProfileUpdates: RecordedCommand[] = []
  // PROMPT-37: `customer.context` requests, recorded so a spec can confirm
  // which `consultant_id` the BFF forwarded.
  const customerContextRequests: RecordedCommand[] = []
  // PROMPT-38: `execution.task_completions` (completion-shaped payload)
  // requests, recorded so a spec can confirm the BFF forwarded the expected
  // `task_id`/`consultant_id`.
  const taskCompletionRequests: RecordedCommand[] = []
  // PROMPT-39: `products.catalog` requests, recorded so a spec can confirm
  // the BFF actually called through to Products (there is no
  // `consultant_id` to assert on this one — see `products.rs`'s module docs
  // for why — so this just counts/records hits).
  const productCatalogRequests: RecordedCommand[] = []
  // PROMPT-40: `landscape.observations` requests, recorded so a spec can
  // confirm the BFF forwarded the expected `submitted_by`/`observation_text`.
  const fieldObservations: RecordedCommand[] = []
  // PROMPT-41: `legal.clauses` requests, recorded so a spec can confirm
  // which `proposal_id`/`topic` the BFF forwarded.
  const legalClauseRequests: RecordedCommand[] = []
  // PROMPT-33 e2e: events queued via `POST /_test/enqueue-event` and
  // drained (returned once, then cleared) on the next `GET
  // events/v1/poll` — see `notifications-sse.spec.ts`. Draining, not
  // repeat-serving, mirrors a well-behaved real Nexus honoring the
  // poller's cursor: once delivered, an event isn't handed out again.
  let queuedEvents: CapabilityEventReceived[] = []

  const server: Server = createServer((request, response) => {
    void handleRequest(request, response).catch((err: unknown) => {
      // A malformed request body (or any other handler-level failure)
      // should surface as a 500 from this test double, not crash the
      // whole mock server out from under a running test.
      // eslint-disable-next-line no-console
      console.error('[mock-nexus] request handling failed', err)
      sendJson(response, 500, { error: 'mock nexus server error' })
    })
  })

  const CAPABILITIES_PATH_PREFIX = '/capabilities/'

  async function handleRequest(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const url = new URL(request.url ?? '/', `http://127.0.0.1:${port}`)
    const method = request.method ?? 'GET'

    // ADR-029: the one real route every `nexus-client` gateway now calls
    // through (`CapabilityCaller::call`, `crates/nexus-client/src/transport.rs`)
    // — see the module docs' "One route, all ten gateways" section.
    if (method === 'POST' && url.pathname.startsWith(CAPABILITIES_PATH_PREFIX)) {
      const capabilityId = url.pathname.slice(CAPABILITIES_PATH_PREFIX.length)
      const envelope = (await readJsonBody(request)) as CapabilityRequestEnvelope
      const payload = envelope.payload ?? {}
      const requestId = envelope.request_id

      const respond = (responsePayload: unknown): void => {
        sendJson(response, 200, { request_id: requestId, success: true, payload: responsePayload })
      }
      const now = (): string => new Date().toISOString()

      // Armor ACL (ADR-009, PROMPT-14): grants every capability for
      // whichever `consultant_id` is asked about, matching
      // `crates/nexus-client/src/armor.rs`'s `{"assertions": [...]}`
      // envelope.
      if (capabilityId === 'armor.assertions') {
        const consultantId = (payload.consultant_id as string | undefined) ?? 'unknown-consultant'
        const expiresAt = new Date(Date.now() + 60 * 60 * 1000).toISOString()
        respond({
          assertions: [
            { consultant_id: consultantId, capability: 'sales', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'commit', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'edu', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'capacity', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'customer', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'execution', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'products', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'landscape', scope: 'default', expires_at: expiresAt },
            { consultant_id: consultantId, capability: 'legal', scope: 'default', expires_at: expiresAt },
          ],
        })
        return
      }

      // Sales account-claim check (ADR-016, PROMPT-24): answers with the
      // no_match fixture for `NO_MATCH_COMPANY_NAME`, and the
      // active_owned_account worked example for every other company name —
      // see the module docs.
      if (capabilityId === 'sales.account_claims') {
        const companyName = payload.company_name as string | undefined
        const fixture = companyName === NO_MATCH_COMPANY_NAME ? NO_MATCH_ACCOUNT_CLAIM_FIXTURE : ACCOUNT_CLAIM_FIXTURE
        respond(fixture)
        return
      }

      if (capabilityId === 'sales.collaboration_requests') {
        collaborationRequests.push({ path: capabilityId, body: payload, receivedAt: now() })
        respond({})
        return
      }

      if (capabilityId === 'sales.referrals') {
        referrals.push({ path: capabilityId, body: payload, receivedAt: now() })
        respond({})
        return
      }

      // Commit (ADR-016, PROMPT-34): one capability id serves both
      // `create_proposal` and `list_proposals` (they shared one REST path
      // historically — see `crates/nexus-client/src/commit.rs`'s module
      // docs) — disambiguated here by payload shape, exactly as that
      // module documents: an `origin_reference`-carrying payload creates,
      // a bare-`consultant_id` payload lists.
      if (capabilityId === 'commit.proposals') {
        if (typeof payload.origin_reference === 'string') {
          const originReference = payload.origin_reference
          const proposalId = `proposal-${nextProposalNumber}`
          nextProposalNumber += 1
          const proposal: ProposalSummary = {
            proposal_id: proposalId,
            title: `${originReference} Engagement Proposal`,
            status: 'draft',
            stage: 'drafting',
            last_updated_at: now(),
            deep_link: `https://commit.cognitum.one/proposals/${proposalId}`,
          }
          proposals.set(proposalId, proposal)
          respond(proposal)
          return
        }
        respond({ proposals: [...proposals.values()] })
        return
      }

      if (capabilityId === 'commit.proposal_actions') {
        proposalActions.push({ path: capabilityId, body: payload, receivedAt: now() })
        respond({})
        return
      }

      // Edu learning-catalog query (ADR-016, PROMPT-35): always answers
      // with the fixed `LEARNING_CATALOG_FIXTURE`, matching
      // `crates/nexus-client/src/edu.rs`'s `LearningCatalogEnvelope`
      // (`{"snapshots": [...]}`) convention. `consultant_id` is recorded so
      // a spec can assert the BFF forwarded the authenticated consultant,
      // the same shape `_test/proposals` proves for Commit.
      if (capabilityId === 'edu.catalog') {
        catalogRequests.push({ path: capabilityId, body: { consultant_id: payload.consultant_id ?? null }, receivedAt: now() })
        respond({ snapshots: LEARNING_CATALOG_FIXTURE })
        return
      }

      // Capacity (ADR-016, PROMPT-36): one capability id serves both
      // `get_own_profile` and `update_own_profile` (see
      // `crates/nexus-client/src/capacity.rs`'s module docs) —
      // disambiguated here by payload shape: a `profile_fields`-carrying
      // payload writes, a bare-`consultant_id` payload reads.
      if (capabilityId === 'capacity.profile') {
        if (payload.profile_fields !== undefined) {
          const consultantId = payload.consultant_id as string
          const profileFields = payload.profile_fields as ConsultantProfileIntake
          capacityProfiles.set(consultantId, profileFields)
          capacityProfileUpdates.push({ path: capabilityId, body: payload, receivedAt: now() })
          respond({ accepted: true })
          return
        }
        const consultantId = (payload.consultant_id as string | undefined) ?? 'unknown-consultant'
        const profile = capacityProfiles.get(consultantId) ?? PROFILE_FIXTURE
        capacityProfiles.set(consultantId, profile)
        respond(profile)
        return
      }

      // Customer assigned-context query (ADR-016, PROMPT-37): always
      // answers with the fixed `CUSTOMER_CONTEXT_FIXTURE`, matching
      // `crates/nexus-client/src/customer.rs`'s `CustomerContextEnvelope`
      // (`{"contexts": [...]}`) convention. `consultant_id` is recorded so
      // a spec can assert the BFF forwarded the authenticated consultant,
      // the same shape `_test/edu-catalog-requests` proves for Edu.
      if (capabilityId === 'customer.context') {
        customerContextRequests.push({
          path: capabilityId,
          body: { consultant_id: payload.consultant_id ?? null },
          receivedAt: now(),
        })
        respond({ contexts: CUSTOMER_CONTEXT_FIXTURE })
        return
      }

      // Execution (ADR-016, PROMPT-38): one capability id serves both
      // `request_assigned_engagements` and `confirm_task_completion` (see
      // `crates/nexus-client/src/execution.rs`'s module docs) —
      // disambiguated here by payload shape: a `task_id`-carrying payload
      // confirms completion, a bare-`consultant_id` payload queries
      // engagements.
      if (capabilityId === 'execution.task_completions') {
        if (typeof payload.task_id === 'string') {
          taskCompletionRequests.push({ path: capabilityId, body: payload, receivedAt: now() })
          respond({ accepted: true })
          return
        }
        respond({ engagements: ENGAGEMENT_SNAPSHOT_FIXTURE })
        return
      }

      // Products catalog query (ADR-016, PROMPT-39): always answers with
      // the fixed `PRODUCT_CATALOG_FIXTURE`, matching
      // `crates/nexus-client/src/products.rs`'s `ProductCatalogEnvelope`
      // (`{"cards": [...]}`) convention.
      if (capabilityId === 'products.catalog') {
        productCatalogRequests.push({ path: capabilityId, body: null, receivedAt: now() })
        respond({ cards: PRODUCT_CATALOG_FIXTURE })
        return
      }

      // Landscape intelligence-digest query (ADR-016, PROMPT-40): always
      // answers with the fixed `INTELLIGENCE_DIGEST_FIXTURE`, matching
      // `crates/nexus-client/src/landscape.rs`'s
      // `IntelligenceDigestEnvelope` (`{"items": [...]}`) convention.
      if (capabilityId === 'landscape.intelligence') {
        respond({ items: INTELLIGENCE_DIGEST_FIXTURE })
        return
      }

      // Landscape field-observation submission command (ADR-016,
      // PROMPT-40): always accepts and records the request, matching
      // `crates/nexus-client/src/landscape.rs`'s `FieldObservationSubmission`
      // shape — fire-and-confirm, same "no documented ack body" convention
      // as Sales' `collaboration-requests`/`referrals` above.
      if (capabilityId === 'landscape.observations') {
        fieldObservations.push({ path: capabilityId, body: payload, receivedAt: now() })
        respond({})
        return
      }

      // Legal approved-clauses query (ADR-007, PROMPT-41): always answers
      // with the fixed `APPROVED_LEGAL_SNIPPET_FIXTURE`, matching
      // `crates/nexus-client/src/legal.rs`'s `ClausesEnvelope`
      // (`{"clauses": [...]}`) convention, regardless of which of
      // `proposal_id`/`topic` was sent.
      if (capabilityId === 'legal.clauses') {
        legalClauseRequests.push({
          path: capabilityId,
          body: { proposal_id: payload.proposal_id ?? null, topic: payload.topic ?? null },
          receivedAt: now(),
        })
        respond({ clauses: APPROVED_LEGAL_SNIPPET_FIXTURE })
        return
      }

      console.error(`[mock-nexus] no capability handler for ${capabilityId}`)
      sendJson(response, 404, { error: `mock nexus has no route for capability ${capabilityId}` })
      return
    }

    // Events poll (PROMPT-30/PROMPT-33): not part of the ADR-029 capability
    // envelope — see the module docs. `bff-api`'s ingestion polling loop
    // (`crates/bff-api/src/event_ingestion.rs`) expects a bare JSON array
    // of `CapabilityEventReceived`. Drains whatever `_test/enqueue-event`
    // has queued so far, then clears it — see the `queuedEvents` doc
    // comment above.
    if (method === 'GET' && url.pathname === '/events/v1/poll') {
      const events = queuedEvents
      queuedEvents = []
      sendJson(response, 200, events)
      return
    }

    // Inspection routes (test-only, `/_test/...` namespace) — let a spec
    // running in a different process confirm what this server received.
    if (method === 'GET' && url.pathname === '/_test/collaboration-requests') {
      sendJson(response, 200, collaborationRequests)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/referrals') {
      sendJson(response, 200, referrals)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/proposals') {
      sendJson(response, 200, [...proposals.values()])
      return
    }

    if (method === 'GET' && url.pathname === '/_test/proposal-actions') {
      sendJson(response, 200, proposalActions)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/edu-catalog-requests') {
      sendJson(response, 200, catalogRequests)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/capacity-profile-updates') {
      sendJson(response, 200, capacityProfileUpdates)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/customer-context-requests') {
      sendJson(response, 200, customerContextRequests)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/task-completion-requests') {
      sendJson(response, 200, taskCompletionRequests)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/product-catalog-requests') {
      sendJson(response, 200, productCatalogRequests)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/field-observations') {
      sendJson(response, 200, fieldObservations)
      return
    }

    if (method === 'GET' && url.pathname === '/_test/legal-clause-requests') {
      sendJson(response, 200, legalClauseRequests)
      return
    }

    // Test-injection route (PROMPT-33): queues one `CapabilityEventReceived`
    // for the *next* `GET events/v1/poll` to pick up — how
    // `notifications-sse.spec.ts` drives a real Nexus->ingestion->NOTIFY/
    // LISTEN->SSE->browser push without needing a real Nexus.
    if (method === 'POST' && url.pathname === '/_test/enqueue-event') {
      const body = (await readJsonBody(request)) as CapabilityEventReceived
      queuedEvents.push(body)
      sendJson(response, 200, {})
      return
    }

    if (method === 'POST' && url.pathname === '/_test/reset') {
      collaborationRequests.length = 0
      referrals.length = 0
      queuedEvents = []
      proposals.clear()
      proposalActions.length = 0
      catalogRequests.length = 0
      capacityProfiles.clear()
      capacityProfileUpdates.length = 0
      customerContextRequests.length = 0
      taskCompletionRequests.length = 0
      productCatalogRequests.length = 0
      fieldObservations.length = 0
      legalClauseRequests.length = 0
      sendJson(response, 200, {})
      return
    }

    console.error(`[mock-nexus] no route matched for ${method} ${url.pathname}`)
    sendJson(response, 404, { error: `mock nexus has no route for ${method} ${url.pathname}` })
  }

  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', () => {
      resolve({
        url: `http://127.0.0.1:${port}`,
        close: () =>
          new Promise<void>((closeResolve, closeReject) => {
            server.close((err) => (err ? closeReject(err) : closeResolve()))
          }),
      })
    })
  })
}
