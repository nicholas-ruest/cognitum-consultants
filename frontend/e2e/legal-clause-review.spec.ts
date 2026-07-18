import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-41 e2e (ADR-013 layer 5): drives the Legal read-only
 * approved-clauses flow through the full real stack — real Vite-served
 * frontend, real `bff-api`, real Postgres — with Nexus mocked at the HTTP
 * boundary, following `commit-sales-deeplink.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9). Focuses on the Commit integration,
 * per this unit's own prompt text: `ApprovedClauses` is exercised as it is
 * actually used in this repo — embedded in `ProposalWorkspace`'s
 * proposal-detail view, scoped by `proposal_id` — rather than in isolation.
 *
 * Flow: log in -> start a Commit proposal (`"commit"` is one of
 * `DEFAULT_CARD_MODULE_IDS`, so no dashboard-layout `PUT` is needed first,
 * unlike `landscape-intelligence-observation.spec.ts`) -> select it ->
 * `ApprovedClauses` fires `GET /api/legal/clauses?proposal_id=...` and
 * renders the mock Nexus server's fixed `APPROVED_LEGAL_SNIPPET_FIXTURE`.
 */

function mockNexusBaseUrl(): string {
  const url = process.env[MOCK_NEXUS_BASE_URL_ENV]
  if (!url) {
    throw new Error(
      `${MOCK_NEXUS_BASE_URL_ENV} is not set — this test must run under playwright.config.ts's globalSetup, ` +
        'which starts the mock Nexus server and exports this env var for spec processes to inherit.',
    )
  }
  return url
}

test('shows approved legal clauses for a Commit proposal under review', async ({ page, request }) => {
  // 1. Load the app unauthenticated, log in.
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()

  // 2. The Commit card renders (one of the three default dashboard cards).
  // Deliberately not asserting an empty "No proposals yet." state here —
  // unlike this spec's own proposal (identified by its own unique title
  // below), the Commit proposals list is shared mock-Nexus-server state
  // across every e2e spec in a full suite run (`mock-nexus-server.ts`'s
  // module docs), so another spec's proposal may already exist by the time
  // this one runs.
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Commit', level: 3 })).toBeVisible()

  // 3. Start a proposal directly (no Sales hand-off needed for this flow).
  await page.getByLabel('Origin Reference').fill('Acme Corp')
  await page.getByRole('button', { name: 'Start Proposal' }).click()

  // 4. `ProposalWorkspace` auto-selects the freshly created proposal
  // (`createMutation.onSuccess`), so its detail view — including the
  // embedded `ApprovedClauses` section — renders without an extra click.
  await expect(page.getByRole('heading', { name: 'Acme Corp Engagement Proposal' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Approved Legal Clauses', level: 4 })).toBeVisible()
  await expect(page.getByText('Limitation of Liability')).toBeVisible()
  await expect(page.getByText('Neither party shall be liable for indirect or consequential damages.')).toBeVisible()
  await expect(page.getByText('Confidentiality')).toBeVisible()
  await expect(page.getByText('policy-2026-01')).toBeVisible()

  // 5. Confirm the mock Nexus server actually received a
  // `RequestApprovedClausesQuery` scoped by this proposal's own id, not a
  // fabricated one — proves the BFF resolved `?proposal_id=` from the real
  // proposal the frontend just created, not a hardcoded/placeholder value.
  const proposalsResponse = await request.get(`${mockNexusBaseUrl()}/_test/proposals`)
  expect(proposalsResponse.ok()).toBe(true)
  const proposals = (await proposalsResponse.json()) as Array<{ proposal_id: string; title: string }>
  const thisProposal = proposals.find((proposal) => proposal.title === 'Acme Corp Engagement Proposal')
  expect(thisProposal).toBeDefined()

  const clauseRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/legal-clause-requests`)
  expect(clauseRequestsResponse.ok()).toBe(true)
  const clauseRequests = (await clauseRequestsResponse.json()) as Array<{
    body: { proposal_id: string | null; topic: string | null }
  }>
  const thisClauseRequest = clauseRequests.find((clauseRequest) => clauseRequest.body.proposal_id === thisProposal?.proposal_id)
  expect(thisClauseRequest).toBeDefined()
  expect(thisClauseRequest?.body.topic).toBeNull()
})
