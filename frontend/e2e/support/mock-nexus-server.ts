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
 * This server always grants the `"sales"` capability (for whichever
 * `consultant_id` is asked about) and always answers a company-claim check
 * with the `active_owned_account` fixture — `.plans/ddd/anti-corruption-
 * layers.md` §1's worked example, and the same fixture
 * `crates/bff-api/src/sales.rs`'s own integration tests use
 * (`active_owned_account_fixture()`). A future test that needs a different
 * `match_status` should extend this module with a configurable fixture
 * rather than hardcoding a second server.
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

export interface RecordedCommand {
  path: string
  body: unknown
  receivedAt: string
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

    // Armor ACL (ADR-009, PROMPT-14): grants the "sales" capability for
    // whichever consultant_id is asked about, matching
    // `crates/nexus-client/src/armor.rs`'s `{"assertions": [...]}` envelope.
    if (method === 'GET' && url.pathname === '/armor/v1/assertions') {
      const consultantId = url.searchParams.get('consultant_id') ?? 'unknown-consultant'
      sendJson(response, 200, {
        assertions: [
          {
            consultant_id: consultantId,
            capability: 'sales',
            scope: 'default',
            expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
          },
        ],
      })
      return
    }

    // Sales account-claim check (ADR-016, PROMPT-24): always answers with
    // the active_owned_account worked example.
    if (method === 'POST' && url.pathname === '/sales/v1/account-claims') {
      await readJsonBody(request)
      sendJson(response, 200, ACCOUNT_CLAIM_FIXTURE)
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

    if (method === 'POST' && url.pathname === '/_test/reset') {
      collaborationRequests.length = 0
      referrals.length = 0
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
