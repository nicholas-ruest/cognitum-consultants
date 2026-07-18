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
 * This server always grants the `"sales"` and `"commit"` capabilities (for
 * whichever `consultant_id` is asked about) and always answers a
 * company-claim check for most company names with the `active_owned_account`
 * fixture — `.plans/ddd/anti-corruption-layers.md` §1's worked example, and
 * the same fixture `crates/bff-api/src/sales.rs`'s own integration tests use
 * (`active_owned_account_fixture()`).
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
 * `CapabilityEventReceived` shape, mirrored from
 * `crates/bff-core/src/event_ingestion.rs` — the provisional
 * `GET events/v1/poll` contract `bff-api`'s ingestion polling loop
 * (PROMPT-30) expects a bare JSON array of.
 */
export interface CapabilityEventReceived {
  origin_capability: string
  origin_event_id: string
  event_type: string
  summary: string
  deep_link: string | null
  received_at: string
  consultant_id: string
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
      sendJson(response, 200, {})
      return
    }

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
