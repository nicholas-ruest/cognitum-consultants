import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-37 e2e (ADR-013 layer 5): drives the Customer assigned-context flow
 * through the full real stack — real Vite-served frontend, real `bff-api`,
 * real Postgres — with Nexus mocked at the HTTP boundary, following
 * `edu-learning-dashboard.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"customer"` is deliberately excluded from `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`'s own documented
 * assumption: Customer is "read-heavy/catalog/reference-only", not one of
 * the three default transactional-workspace cards), so this spec first
 * `GET`s the dev consultant's current dashboard layout and `PUT`s it back
 * with a `"customer"` card appended — via `page.request`, which shares the
 * logged-in page's session cookie (unlike the top-level `request` fixture
 * used below purely to inspect the mock Nexus server) — before the card can
 * be asserted on screen. See `edu-learning-dashboard.spec.ts`'s module docs
 * for why this appends rather than replacing outright (the same shared
 * dev-consultant-dashboard concurrency concern applies here).
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

test('logs in, adds the Customer card, sees the assigned customer list, and can view a health/interaction detail card', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Fetch the current layout (`page.request` shares the browser
  // context's session cookie) — `customer` isn't in it, since it isn't one
  // of the three defaults.
  const getResponse = await page.request.get('/api/dashboard')
  expect(getResponse.ok()).toBe(true)
  const currentDashboard = (await getResponse.json()) as { cards: Array<{ module_id: string; position: number }> }
  expect(currentDashboard.cards.some((card) => card.module_id === 'customer')).toBe(false)

  // 3. `PUT` the existing layout back with a `customer` card appended, then
  // reload so `DashboardPage` re-fetches the merged layout.
  const nextPosition = currentDashboard.cards.reduce((max, card) => Math.max(max, card.position), -1) + 1
  const putResponse = await page.request.put('/api/dashboard', {
    data: { cards: [...currentDashboard.cards, { module_id: 'customer', position: nextPosition }] },
  })
  expect(putResponse.ok()).toBe(true)
  await page.reload()

  // 4. The Customer card renders, listing the mock Nexus server's fixed
  // `CUSTOMER_CONTEXT_FIXTURE` (`mock-nexus-server.ts`) — one healthy, one
  // at-risk customer.
  await expect(page.getByRole('heading', { name: 'Customer', level: 3 })).toBeVisible()
  await expect(page.getByText('Acme Corp')).toBeVisible()
  await expect(page.getByText('Beta LLC')).toBeVisible()
  await expect(page.getByText('green')).toBeVisible()
  await expect(page.getByText('red')).toBeVisible()

  // No detail card until a customer is selected.
  await expect(page.getByText('Healthy, quarterly business review scheduled.')).not.toBeVisible()

  // 5. Selecting the healthy customer reveals its health/interaction
  // (relationship) summary and deep link.
  await page.getByRole('button', { name: /Acme Corp/ }).click()
  await expect(page.getByText('Healthy, quarterly business review scheduled.')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Open in Customer' })).toHaveAttribute(
    'href',
    'https://customer.cognitum.one/customers/customer-1',
  )

  // 6. Selecting the at-risk customer swaps the detail card and shows no
  // deep link (the fixture carries none for this customer).
  await page.getByRole('button', { name: /Beta LLC/ }).click()
  await expect(page.getByText('At risk — escalation in progress.')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Open in Customer' })).toHaveCount(0)

  // 7. Confirm the mock Nexus server actually received the
  // `RequestAssignedCustomerContextQuery` for the authenticated dev
  // consultant.
  const contextRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/customer-context-requests`)
  expect(contextRequestsResponse.ok()).toBe(true)
  const contextRequests = (await contextRequestsResponse.json()) as Array<{ body: { consultant_id: string } }>
  expect(contextRequests.length).toBeGreaterThan(0)
  expect(contextRequests[contextRequests.length - 1].body.consultant_id).toBe('dev-consultant-001')
})
