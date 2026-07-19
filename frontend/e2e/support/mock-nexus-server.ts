import { createServer } from 'node:http'
import type { IncomingMessage, Server, ServerResponse } from 'node:http'

/**
 * Mock Nexus HTTP server (PROMPT-27) — stands in for `nexus.cognitum.one`
 * at the HTTP boundary, per ADR-007: `nexus-client`'s `ReqwestNexusTransport`
 * only needs `NEXUS_ENDPOINT_URL` pointed at *something* that speaks the
 * provisional `armor/v1/...` / `sales/v1/...` request shapes documented in
 * `crates/nexus-client/src/armor.rs` and `crates/nexus-client/src/sales.rs`
 * — it has no idea (and no way to tell) whether that's the real Nexus or
 * this stand-in. This is the same pattern manual verification used in
 * PROMPT-18/19/23, formalized here as a reusable module so Phase 4
 * (PROMPT-34+) can start one of these per capability without re-deriving
 * the wiring.
 *
 * Deliberately plain `node:http` (no Express/msw dependency) — the surface
 * this test needs is four routes with fixed response shapes, which plain
 * `http.createServer` covers with zero added dependencies.
 *
 * # Fixtures are fixed, not dynamically configurable
 * This server always grants the `"sales"`, `"commit"`, `"edu"`, `"capacity"`,
 * `"customer"`, `"execution"`, and `"products"` capabilities (for whichever
 * `consultant_id` is asked about) and always answers a company-claim check
 * for most company names with the `active_owned_account` fixture —
 * `.plans/ddd/anti-corruption-layers.md` §1's worked example, and the same
 * fixture `crates/bff-api/src/sales.rs`'s own integration tests use
 * (`active_owned_account_fixture()`). Edu's `GET /edu/v1/catalog`
 * (PROMPT-35) always answers with the fixed `LEARNING_CATALOG_FIXTURE`
 * below, matching that module's own inline doc comment. Capacity's
 * `GET`/`POST /capacity/v1/profile` (PROMPT-36) serve a single, stateful
 * `PROFILE_FIXTURE` per `consultant_id`: `GET` returns whatever is currently
 * stored (seeded from `PROFILE_FIXTURE`), and `POST` always accepts the
 * update and overwrites the stored profile with it — see the module docs
 * for why a stateful round-trip was chosen over a fixed
 * always-the-same-response fixture for this one capability. Customer's
 * `GET /customer/v1/context` (PROMPT-37) always answers with the fixed
 * `CUSTOMER_CONTEXT_FIXTURE` below, matching Edu's "fixed, not dynamically
 * configurable" pattern. Execution's `GET /execution/v1/engagements`
 * (PROMPT-38) always answers with the fixed `ENGAGEMENT_SNAPSHOT_FIXTURE`
 * below, same pattern; `POST /execution/v1/task-completions` records every
 * request it receives (mirroring `proposalActions`/`referrals` below) so a
 * spec can assert the BFF forwarded the expected `task_id`/`consultant_id`.
 * Products' `GET /products/v1/catalog` (PROMPT-39) always answers with the
 * fixed `PRODUCT_CATALOG_FIXTURE` below, same "fixed, not dynamically
 * configurable" pattern as Edu/Customer/Execution. Landscape's
 * `GET /landscape/v1/intelligence` (PROMPT-40) always answers with the fixed
 * `INTELLIGENCE_DIGEST_FIXTURE` below, same pattern; `POST
 * /landscape/v1/observations` records every request it receives (mirroring
 * `taskCompletionRequests`/`capacityProfileUpdates` above) so a spec can
 * assert the BFF forwarded the expected `submitted_by`/`observation_text`.
 * Legal's `GET /legal/v1/clauses` (PROMPT-41) always answers with the fixed
 * `APPROVED_LEGAL_SNIPPET_FIXTURE` below regardless of `proposal_id`/`topic`
 * (this server has no real clause library to filter against), and records
 * every request it receives (mirroring `edu-catalog-requests`/
 * `customer-context-requests` above) so a spec can assert which
 * `proposal_id`/`topic` query param the BFF actually forwarded.
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
 * Fixed `GET /edu/v1/catalog` fixture (PROMPT-35): one completed/certified
 * course, one in-progress course with no certification, and one
 * `not_started` course with a `required` certification (the
 * `LearningDashboard.tsx` "Training Due" heuristic's worked example) —
 * proving the frontend's three-section partition against more than one
 * status combination, mirroring `ACCOUNT_CLAIM_FIXTURE`'s "fixed, not
 * dynamically configurable" rationale above.
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
 * Fixed initial `GET /capacity/v1/profile` fixture (PROMPT-36) — seeds the
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
 * Fixed `GET /customer/v1/context` fixture (PROMPT-37): one healthy customer
 * and one at-risk customer, proving the frontend's list-plus-detail
 * rendering against more than one `health_status` value — mirroring
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
 * Fixed `GET /products/v1/catalog` fixture (PROMPT-39): one product with a
 * demo asset and one without, proving the frontend's list-plus-detail
 * rendering against both shapes — mirroring `ENGAGEMENT_SNAPSHOT_FIXTURE`'s
 * "fixed, not dynamically configurable" rationale above.
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
 * Fixed `GET /execution/v1/engagements` fixture (PROMPT-38): one on-track
 * engagement with an assigned task, proving the frontend's
 * list-plus-detail-plus-tasks rendering — mirroring
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
 * Fixed `GET /landscape/v1/intelligence` fixture (PROMPT-40): one item with
 * a deep link, one without, proving the frontend's digest rendering against
 * both shapes — mirroring `ENGAGEMENT_SNAPSHOT_FIXTURE`'s "fixed, not
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
 * Fixed `GET /legal/v1/clauses` fixture (PROMPT-41): two approved clauses,
 * proving the frontend's list rendering — mirroring
 * `CUSTOMER_CONTEXT_FIXTURE`'s "fixed, not dynamically configurable"
 * rationale above.
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
  // PROMPT-34: proposals created via `POST /commit/v1/proposals`, keyed by
  // `proposal_id`, so `GET /commit/v1/proposals` (list) and
  // `POST /commit/v1/proposal-actions` can look them up.
  const proposals = new Map<string, ProposalSummary>()
  const proposalActions: RecordedCommand[] = []
  let nextProposalNumber = 1
  // PROMPT-35: `GET /edu/v1/catalog` requests, recorded so a spec can
  // confirm which `consultant_id` the BFF forwarded.
  const catalogRequests: RecordedCommand[] = []
  // PROMPT-36: per-`consultant_id` Capacity profile store, seeded lazily
  // from `PROFILE_FIXTURE` on first `GET`/`POST`, plus a record of every
  // `POST /capacity/v1/profile` this server received.
  const capacityProfiles = new Map<string, ConsultantProfileIntake>()
  const capacityProfileUpdates: RecordedCommand[] = []
  // PROMPT-37: `GET /customer/v1/context` requests, recorded so a spec can
  // confirm which `consultant_id` the BFF forwarded.
  const customerContextRequests: RecordedCommand[] = []
  // PROMPT-38: `POST /execution/v1/task-completions` requests, recorded so a
  // spec can confirm the BFF forwarded the expected `task_id`/`consultant_id`.
  const taskCompletionRequests: RecordedCommand[] = []
  // PROMPT-39: `GET /products/v1/catalog` requests, recorded so a spec can
  // confirm the BFF actually called through to Products (there is no
  // `consultant_id` to assert on this one — see `products.rs`'s module docs
  // for why — so this just counts/records hits).
  const productCatalogRequests: RecordedCommand[] = []
  // PROMPT-40: `POST /landscape/v1/observations` requests, recorded so a
  // spec can confirm the BFF forwarded the expected `submitted_by`/
  // `observation_text`.
  const fieldObservations: RecordedCommand[] = []
  // PROMPT-41: `GET /legal/v1/clauses` requests, recorded so a spec can
  // confirm which `proposal_id`/`topic` query param the BFF forwarded.
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

  async function handleRequest(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const url = new URL(request.url ?? '/', `http://127.0.0.1:${port}`)
    const method = request.method ?? 'GET'

    // Armor ACL (ADR-009, PROMPT-14): grants the "sales" and "commit"
    // capabilities for whichever consultant_id is asked about, matching
    // `crates/nexus-client/src/armor.rs`'s `{"assertions": [...]}` envelope.
    if (method === 'GET' && url.pathname === '/armor/v1/assertions') {
      const consultantId = url.searchParams.get('consultant_id') ?? 'unknown-consultant'
      const expiresAt = new Date(Date.now() + 60 * 60 * 1000).toISOString()
      sendJson(response, 200, {
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
    if (method === 'POST' && url.pathname === '/sales/v1/account-claims') {
      const body = (await readJsonBody(request)) as { company_name?: string }
      const fixture = body.company_name === NO_MATCH_COMPANY_NAME ? NO_MATCH_ACCOUNT_CLAIM_FIXTURE : ACCOUNT_CLAIM_FIXTURE
      sendJson(response, 200, fixture)
      return
    }

    if (method === 'POST' && url.pathname === '/sales/v1/collaboration-requests') {
      const body = await readJsonBody(request)
      collaborationRequests.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, {})
      return
    }

    if (method === 'POST' && url.pathname === '/sales/v1/referrals') {
      const body = await readJsonBody(request)
      referrals.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, {})
      return
    }

    // Commit proposal creation (ADR-016, PROMPT-34): creates and stores a
    // `ProposalSummary` keyed by a freshly minted `proposal_id`, matching
    // `crates/nexus-client/src/commit.rs`'s `CreateProposalCommand`/
    // `ProposalSummary` shapes.
    if (method === 'POST' && url.pathname === '/commit/v1/proposals') {
      const body = (await readJsonBody(request)) as { origin_reference: string; consultant_id: string }
      const proposalId = `proposal-${nextProposalNumber}`
      nextProposalNumber += 1
      const proposal: ProposalSummary = {
        proposal_id: proposalId,
        title: `${body.origin_reference} Engagement Proposal`,
        status: 'draft',
        stage: 'drafting',
        last_updated_at: new Date().toISOString(),
        deep_link: `https://commit.cognitum.one/proposals/${proposalId}`,
      }
      proposals.set(proposalId, proposal)
      sendJson(response, 200, proposal)
      return
    }

    // Commit proposal list (PROMPT-34): `{"proposals": [...]}` envelope,
    // matching `crates/nexus-client/src/commit.rs`'s `ProposalsEnvelope`.
    if (method === 'GET' && url.pathname === '/commit/v1/proposals') {
      sendJson(response, 200, { proposals: [...proposals.values()] })
      return
    }

    if (method === 'POST' && url.pathname === '/commit/v1/proposal-actions') {
      const body = await readJsonBody(request)
      proposalActions.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, {})
      return
    }

    // Edu learning-catalog query (ADR-016, PROMPT-35): always answers with
    // the fixed `LEARNING_CATALOG_FIXTURE`, matching
    // `crates/nexus-client/src/edu.rs`'s `LearningCatalogEnvelope`
    // (`{"snapshots": [...]}`) convention. `consultant_id` is recorded so a
    // spec can assert the BFF forwarded the authenticated consultant, the
    // same shape `_test/proposals` proves for Commit.
    if (method === 'GET' && url.pathname === '/edu/v1/catalog') {
      catalogRequests.push({
        path: url.pathname,
        body: { consultant_id: url.searchParams.get('consultant_id') },
        receivedAt: new Date().toISOString(),
      })
      sendJson(response, 200, { snapshots: LEARNING_CATALOG_FIXTURE })
      return
    }

    // Capacity own-profile fetch (ADR-016, PROMPT-36): returns whatever is
    // currently stored for `consultant_id`, seeding it from `PROFILE_FIXTURE`
    // on first access — matches
    // `crates/nexus-client/src/capacity.rs`'s bare-object (no envelope)
    // response shape.
    if (method === 'GET' && url.pathname === '/capacity/v1/profile') {
      const consultantId = url.searchParams.get('consultant_id') ?? 'unknown-consultant'
      const profile = capacityProfiles.get(consultantId) ?? PROFILE_FIXTURE
      capacityProfiles.set(consultantId, profile)
      sendJson(response, 200, profile)
      return
    }

    // Capacity own-profile update (ADR-016, PROMPT-36): always accepts and
    // overwrites the stored profile with `profile_fields`, matching
    // `crates/nexus-client/src/capacity.rs`'s `UpdateOwnProfileCommand`/
    // `ProfileUpdateResult` shapes.
    if (method === 'POST' && url.pathname === '/capacity/v1/profile') {
      const body = (await readJsonBody(request)) as { consultant_id: string; profile_fields: ConsultantProfileIntake }
      capacityProfiles.set(body.consultant_id, body.profile_fields)
      capacityProfileUpdates.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, { accepted: true })
      return
    }

    // Customer assigned-context query (ADR-016, PROMPT-37): always answers
    // with the fixed `CUSTOMER_CONTEXT_FIXTURE`, matching
    // `crates/nexus-client/src/customer.rs`'s `CustomerContextEnvelope`
    // (`{"contexts": [...]}`) convention. `consultant_id` is recorded so a
    // spec can assert the BFF forwarded the authenticated consultant, the
    // same shape `_test/edu-catalog-requests` proves for Edu.
    if (method === 'GET' && url.pathname === '/customer/v1/context') {
      customerContextRequests.push({
        path: url.pathname,
        body: { consultant_id: url.searchParams.get('consultant_id') },
        receivedAt: new Date().toISOString(),
      })
      sendJson(response, 200, { contexts: CUSTOMER_CONTEXT_FIXTURE })
      return
    }

    // Execution assigned-engagements query (ADR-016, PROMPT-38): always
    // answers with the fixed `ENGAGEMENT_SNAPSHOT_FIXTURE`, matching
    // `crates/nexus-client/src/execution.rs`'s `EngagementsEnvelope`
    // (`{"engagements": [...]}`) convention.
    if (method === 'GET' && url.pathname === '/execution/v1/engagements') {
      sendJson(response, 200, { engagements: ENGAGEMENT_SNAPSHOT_FIXTURE })
      return
    }

    // Execution task-completion-request command (ADR-016, PROMPT-38): always
    // accepts and records the request, matching
    // `crates/nexus-client/src/execution.rs`'s `ConfirmTaskCompletionCommand`
    // shape — fire-and-confirm, same "no documented ack body" convention as
    // Sales' `collaboration-requests`/`referrals` above.
    if (method === 'POST' && url.pathname === '/execution/v1/task-completions') {
      const body = await readJsonBody(request)
      taskCompletionRequests.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, { accepted: true })
      return
    }

    // Products catalog query (ADR-016, PROMPT-39): always answers with the
    // fixed `PRODUCT_CATALOG_FIXTURE`, matching
    // `crates/nexus-client/src/products.rs`'s `ProductCatalogEnvelope`
    // (`{"cards": [...]}`) convention.
    if (method === 'GET' && url.pathname === '/products/v1/catalog') {
      productCatalogRequests.push({ path: url.pathname, body: null, receivedAt: new Date().toISOString() })
      sendJson(response, 200, { cards: PRODUCT_CATALOG_FIXTURE })
      return
    }

    // Landscape intelligence-digest query (ADR-016, PROMPT-40): always
    // answers with the fixed `INTELLIGENCE_DIGEST_FIXTURE`, matching
    // `crates/nexus-client/src/landscape.rs`'s `IntelligenceDigestEnvelope`
    // (`{"items": [...]}`) convention.
    if (method === 'GET' && url.pathname === '/landscape/v1/intelligence') {
      sendJson(response, 200, { items: INTELLIGENCE_DIGEST_FIXTURE })
      return
    }

    // Landscape field-observation submission command (ADR-016, PROMPT-40):
    // always accepts and records the request, matching
    // `crates/nexus-client/src/landscape.rs`'s `FieldObservationSubmission`
    // shape — fire-and-confirm, same "no documented ack body" convention as
    // Sales' `collaboration-requests`/`referrals` above.
    if (method === 'POST' && url.pathname === '/landscape/v1/observations') {
      const body = await readJsonBody(request)
      fieldObservations.push({ path: url.pathname, body, receivedAt: new Date().toISOString() })
      sendJson(response, 200, {})
      return
    }

    // Legal approved-clauses query (ADR-007, PROMPT-41): always answers with
    // the fixed `APPROVED_LEGAL_SNIPPET_FIXTURE`, matching
    // `crates/nexus-client/src/legal.rs`'s `ClausesEnvelope`
    // (`{"clauses": [...]}`) convention, regardless of which of
    // `proposal_id`/`topic` was sent.
    if (method === 'GET' && url.pathname === '/legal/v1/clauses') {
      legalClauseRequests.push({
        path: url.pathname,
        body: { proposal_id: url.searchParams.get('proposal_id'), topic: url.searchParams.get('topic') },
        receivedAt: new Date().toISOString(),
      })
      sendJson(response, 200, { clauses: APPROVED_LEGAL_SNIPPET_FIXTURE })
      return
    }

    // Events poll (PROMPT-30/PROMPT-33): `bff-api`'s ingestion polling loop
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
