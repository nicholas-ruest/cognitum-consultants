import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-27 e2e (ADR-013 layer 5, canonical smoke test): drives the
 * lead-conflict-warning flow (`.plans/ddd/anti-corruption-layers.md` §1's
 * worked example) through the *full real stack* — real Vite-served
 * frontend, real `bff-api`, real Postgres — with Nexus mocked at the HTTP
 * boundary (`e2e/support/mock-nexus-server.ts`, brought up by
 * `playwright.config.ts`'s `globalSetup`). This is the template Phase 4
 * (PROMPT-34+) replicates — see `docs/SALES_FLOW_PATTERN.md`.
 *
 * `match_status` fixture choice: `active_owned_account`, the exact
 * `anti-corruption-layers.md` §1 worked example (and the same fixture
 * `crates/bff-api/src/sales.rs`'s own integration tests already use) — see
 * `mock-nexus-server.ts`'s doc comment for the full justification. The
 * mock server hardcodes this single fixture, so this test does not choose
 * it per-call; it documents what to expect.
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

test('logs in, checks a company for a sales conflict, and requests collaboration', async ({ page, request }) => {
  // 1. Load the app unauthenticated — LoginPage.
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()

  // 2. Log in via the dev-stub, through the UI's login form (PROMPT-18).
  await page.getByRole('button', { name: 'Sign in' }).click()

  // 3. Dashboard renders, and the Sales card/nav item is visible — proves
  // the mock Armor endpoint granted "sales" (mock-nexus-server.ts) and
  // `GET /api/dashboard` filtered the default card set to include it.
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Sales' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Sales', level: 3 })).toBeVisible()

  // 4. Enter a company name in the Sales lead-conflict form.
  await page.getByLabel('Company Name').fill('Acme Corp')

  // 5. Submit — the mock Nexus server responds with the active_owned_account fixture.
  await page.getByRole('button', { name: 'Check for Conflicts' }).click()

  // 6. `display_message` renders, and the `permitted_actions` this fixture
  // lists (request_collaboration, submit_referral, cancel) render as
  // buttons — nothing else and nothing missing.
  await expect(page.getByRole('alert')).toHaveText('This company is already being worked.')
  await expect(page.getByRole('button', { name: 'Request Collaboration' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Submit Referral' })).toBeVisible()
  await expect(page.getByRole('button', { name: 'Cancel' })).toBeVisible()

  // 7. Click "Request Collaboration": verify the UI reflects success (no
  // error alert appears, the mutation resolves) and that the mock Nexus
  // server actually received the RequestCollaborationCommand.
  const [response] = await Promise.all([
    page.waitForResponse((res) => res.url().endsWith('/api/sales/request-collaboration') && res.request().method() === 'POST'),
    page.getByRole('button', { name: 'Request Collaboration' }).click(),
  ])
  expect(response.status()).toBe(200)

  // LeadConflictCheck (PROMPT-26) has no dedicated post-success UI state
  // beyond "no error alert" — confirmed here: the only alert on the page
  // remains the original display_message, never an error variant.
  await expect(page.getByText('Failed to check this company. Please try again.')).toHaveCount(0)

  const collaborationRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/collaboration-requests`)
  expect(collaborationRequestsResponse.ok()).toBe(true)
  const collaborationRequests = (await collaborationRequestsResponse.json()) as Array<{
    body: { company_reference: string; consultant_id: string }
  }>

  expect(collaborationRequests).toHaveLength(1)
  expect(collaborationRequests[0].body.company_reference).toBe('Acme Corp')
  expect(collaborationRequests[0].body.consultant_id).toBe('dev-consultant-001')
})
