import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-34 e2e (ADR-013 layer 5): drives the Sales -> Commit deep link
 * through the full real stack — real Vite-served frontend, real `bff-api`,
 * real Postgres — with Nexus mocked at the HTTP boundary, following
 * `sales-lead-conflict.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * Flow: Sales lead-conflict check with a `no_match`/`creation_allowed: true`
 * result (`mock-nexus-server.ts`'s `NO_MATCH_COMPANY_NAME` carve-out) ->
 * click "Start Proposal in Commit" -> `POST /api/workflow-sessions` starts a
 * `CrossCapabilityWorkflowSession` -> full navigation to
 * `/?workflow_session_id=...` -> `ProposalWorkspace` consumes it and calls
 * `POST /api/commit/proposals` -> the new proposal is visible in the Commit
 * feature module. No new orchestration files needed — reuses
 * `playwright.config.ts`'s existing `globalSetup`.
 */

const NO_MATCH_COMPANY_NAME = 'Nova Ventures'

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

test('drives the Sales conflict check (no_match) through the Commit deep link to a created proposal', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in.
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()

  // 2. Dashboard renders both the Sales and Commit cards — proves the mock
  // Armor endpoint granted both capabilities and `GET /api/dashboard`
  // included both in its default card set.
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Sales', level: 3 })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Commit', level: 3 })).toBeVisible()
  await expect(page.getByText('No proposals yet.')).toBeVisible()

  // 3. Check a company name the mock Nexus server answers with the
  // no_match/creation_allowed:true fixture.
  await page.getByLabel('Company Name').fill(NO_MATCH_COMPANY_NAME)
  await page.getByRole('button', { name: 'Check for Conflicts' }).click()

  await expect(page.getByRole('alert').or(page.getByRole('status'))).toHaveText('No matching company found in Sales.')

  // 4. No Sales `permitted_actions` buttons (the fixture's list is empty),
  // but the PROMPT-34 deep-link affordance renders, since creation_allowed
  // is true.
  const startProposalButton = page.getByRole('button', { name: 'Start Proposal in Commit' })
  await expect(startProposalButton).toBeVisible()

  // 5. Click it: `POST /api/workflow-sessions` starts the hand-off session,
  // then a full navigation to `/?workflow_session_id=...` follows.
  await Promise.all([page.waitForURL(/workflow_session_id=/), startProposalButton.click()])

  // 6. The app reloads (session cookie persists across the full
  // navigation — no re-login needed) and `ProposalWorkspace` consumes the
  // query param, firing `POST /api/commit/proposals` with
  // `origin_workflow_session_id`. The resulting proposal appears in the
  // Commit card's list.
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()
  // `ProposalWorkspace` selects the freshly created proposal automatically
  // (see its `createMutation.onSuccess`), so its title appears twice: once
  // in the list entry, once as the detail view's own heading.
  await expect(page.getByRole('heading', { name: 'Nova Ventures Engagement Proposal' })).toBeVisible()

  // The query param must have been stripped, not left dangling.
  await expect(page).toHaveURL(/^[^?]*$/)

  // 7. Confirm the mock Nexus server actually received the
  // `CreateProposalCommand` with the workflow session's origin_reference
  // (the company name checked in step 3), not a fabricated one.
  const proposalsResponse = await request.get(`${mockNexusBaseUrl()}/_test/proposals`)
  expect(proposalsResponse.ok()).toBe(true)
  const proposals = (await proposalsResponse.json()) as Array<{ title: string; proposal_id: string }>
  expect(proposals).toHaveLength(1)
  expect(proposals[0].title).toBe('Nova Ventures Engagement Proposal')

  // 8. The created proposal's detail view and action buttons are reachable
  // from the Commit card — proves the deep link lands on a genuinely
  // functional feature module, not a decorative redirect. Already
  // auto-selected (see above), so the action buttons are visible without
  // an extra click.
  await expect(page.getByRole('button', { name: 'Resend' })).toBeVisible()

  const [actionResponse] = await Promise.all([
    page.waitForResponse(
      (res) => res.url().includes(`/api/commit/proposals/${proposals[0].proposal_id}/actions`) && res.request().method() === 'POST',
    ),
    page.getByRole('button', { name: 'Resend' }).click(),
  ])
  expect(actionResponse.status()).toBe(200)

  const actionsResponse = await request.get(`${mockNexusBaseUrl()}/_test/proposal-actions`)
  expect(actionsResponse.ok()).toBe(true)
  const actions = (await actionsResponse.json()) as Array<{ body: { proposal_id: string; action: string } }>
  expect(actions).toHaveLength(1)
  expect(actions[0].body).toEqual({ proposal_id: proposals[0].proposal_id, action: 'resend' })
})
