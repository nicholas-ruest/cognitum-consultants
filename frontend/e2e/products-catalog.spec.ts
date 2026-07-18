import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-39 e2e (ADR-013 layer 5): drives the Products catalog flow through
 * the full real stack — real Vite-served frontend, real `bff-api`, real
 * Postgres — with Nexus mocked at the HTTP boundary, following
 * `customer-context-cards.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"products"` is deliberately excluded from `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`'s own documented
 * assumption: Products is "read-heavy/catalog/reference-only", not one of
 * the three default transactional-workspace cards), so this spec first
 * `GET`s the dev consultant's current dashboard layout and `PUT`s it back
 * with a `"products"` card appended — via `page.request`, which shares the
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

test('logs in, adds the Products card, sees the approved catalog, and can view a product detail card', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Fetch the current layout (`page.request` shares the browser
  // context's session cookie) — `products` isn't in it, since it isn't one
  // of the three defaults.
  const getResponse = await page.request.get('/api/dashboard')
  expect(getResponse.ok()).toBe(true)
  const currentDashboard = (await getResponse.json()) as { cards: Array<{ module_id: string; position: number }> }
  expect(currentDashboard.cards.some((card) => card.module_id === 'products')).toBe(false)

  // 3. `PUT` the existing layout back with a `products` card appended, then
  // reload so `DashboardPage` re-fetches the merged layout.
  const nextPosition = currentDashboard.cards.reduce((max, card) => Math.max(max, card.position), -1) + 1
  const putResponse = await page.request.put('/api/dashboard', {
    data: { cards: [...currentDashboard.cards, { module_id: 'products', position: nextPosition }] },
  })
  expect(putResponse.ok()).toBe(true)
  await page.reload()

  // 4. The Products card renders, listing the mock Nexus server's fixed
  // `PRODUCT_CATALOG_FIXTURE` (`mock-nexus-server.ts`) — one product with a
  // demo asset, one without.
  await expect(page.getByRole('heading', { name: 'Products', level: 3 })).toBeVisible()
  await expect(page.getByText('Cloud Migration Accelerator')).toBeVisible()
  await expect(page.getByText('Security Posture Review')).toBeVisible()
  await expect(page.getByText('Starting at $50,000')).toBeVisible()
  await expect(page.getByText('Starting at $20,000')).toBeVisible()

  // No detail card until a product is selected.
  await expect(page.getByText('4-week fixed-scope engagement')).not.toBeVisible()

  // 5. Selecting the first product reveals its packaging summary and demo
  // asset link.
  await page.getByRole('button', { name: /Cloud Migration Accelerator/ }).click()
  await expect(page.getByText('4-week fixed-scope engagement')).toBeVisible()
  await expect(page.getByRole('link', { name: 'View demo asset' })).toHaveAttribute(
    'href',
    'https://products.cognitum.one/demos/product-1.mp4',
  )

  // 6. Selecting the second product swaps the detail card and shows no demo
  // asset link (the fixture carries none for this product).
  await page.getByRole('button', { name: /Security Posture Review/ }).click()
  await expect(page.getByText('2-week assessment')).toBeVisible()
  await expect(page.getByRole('link', { name: 'View demo asset' })).toHaveCount(0)

  // 7. Confirm the mock Nexus server actually received the
  // `RequestProductCatalogQuery`.
  const catalogRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/product-catalog-requests`)
  expect(catalogRequestsResponse.ok()).toBe(true)
  const catalogRequests = (await catalogRequestsResponse.json()) as unknown[]
  expect(catalogRequests.length).toBeGreaterThan(0)
})
