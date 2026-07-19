import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-35 e2e (ADR-013 layer 5): drives the Edu learning-catalog flow
 * through the full real stack — real Vite-served frontend, real `bff-api`,
 * real Postgres — with Nexus mocked at the HTTP boundary, following
 * `sales-lead-conflict.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"edu"` is deliberately excluded from `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`'s own documented
 * assumption: Edu is "read-heavy/catalog/reference-only", not one of the
 * three default transactional-workspace cards), so this spec first `GET`s
 * the dev consultant's current dashboard layout and `PUT`s it back with an
 * `"edu"` card appended — via `page.request`, which shares the logged-in
 * page's session cookie (unlike the top-level `request` fixture used below
 * purely to inspect the mock Nexus server) — before the card can be
 * asserted on screen.
 *
 * # Why append rather than replace outright
 * `DashboardConfiguration` is one persisted aggregate **per consultant**
 * (`consultant-experience-context.md` §1.2 invariant 3), and every e2e spec
 * in this suite runs against the same fixed dev consultant
 * (`auth::dev_stub::DEV_CONSULTANT_ID`) over one shared throwaway Postgres,
 * with `playwright.config.ts`'s `fullyParallel: true` running spec *files*
 * concurrently. A `PUT` with only `{module_id: 'edu', ...}` would silently
 * replace (not merge with) whatever `sales-lead-conflict.spec.ts`/
 * `commit-sales-deeplink.spec.ts` rely on being present (invariant 4's
 * "PUT replaces the full layout" semantics, `crates/bff-api/src/dashboard.rs`)
 * — fetching first and appending keeps those siblings' expectations intact
 * regardless of run order.
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

test('logs in, adds the Edu card, and sees the learning catalog partitioned into Courses/Certifications/Training Due', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Fetch the current layout (`page.request` shares the browser
  // context's session cookie) — `edu` isn't in it, since it isn't one of
  // the three defaults.
  const getResponse = await page.request.get('/api/dashboard')
  expect(getResponse.ok()).toBe(true)
  const currentDashboard = (await getResponse.json()) as { cards: Array<{ module_id: string; position: number }> }
  expect(currentDashboard.cards.some((card) => card.module_id === 'edu')).toBe(false)

  // 3. `PUT` the existing layout back with an `edu` card appended (see the
  // module docs for why this appends rather than replacing outright), then
  // reload so `DashboardPage` re-fetches the merged layout.
  const nextPosition = currentDashboard.cards.reduce((max, card) => Math.max(max, card.position), -1) + 1
  const putResponse = await page.request.put('/api/dashboard', {
    data: { cards: [...currentDashboard.cards, { module_id: 'edu', position: nextPosition }] },
  })
  expect(putResponse.ok()).toBe(true)
  await page.reload()

  // 4. The Edu card renders, and the mock Nexus server's fixed
  // `LEARNING_CATALOG_FIXTURE` (`mock-nexus-server.ts`) is partitioned into
  // the three documented sections.
  await expect(page.getByRole('heading', { name: 'Edu', level: 3 })).toBeVisible()
  // `getByText(...).first()`: "Cloud Security Fundamentals" renders in both
  // the Courses and Certifications sections (it has a certification), so a
  // page-wide text lookup here is deliberately non-strict — the
  // section-scoped locators below assert the actual partitioning.
  await expect(page.getByText('Cloud Security Fundamentals').first()).toBeVisible()
  await expect(page.getByText('Advanced Negotiation')).toBeVisible()
  // Also renders twice: Courses and Training Due (its `not_started`
  // progress_status also makes it `required`-certified, so Certifications
  // too) — see the section-scoped assertions below for the real proof.
  await expect(page.getByText('Annual Compliance Refresher').first()).toBeVisible()

  const coursesSection = page.locator('section', { has: page.getByRole('heading', { name: 'Courses', level: 4 }) })
  await expect(coursesSection.getByText('Cloud Security Fundamentals')).toBeVisible()
  await expect(coursesSection.getByText('Advanced Negotiation')).toBeVisible()

  const certificationsSection = page.locator('section', {
    has: page.getByRole('heading', { name: 'Certifications', level: 4 }),
  })
  await expect(certificationsSection.getByText('Cloud Security Fundamentals')).toBeVisible()
  await expect(certificationsSection.getByText('Advanced Negotiation')).toHaveCount(0)

  const trainingDueSection = page.locator('section', {
    has: page.getByRole('heading', { name: 'Training Due', level: 4 }),
  })
  await expect(trainingDueSection.getByText('Annual Compliance Refresher')).toBeVisible()
  await expect(trainingDueSection.getByText('Cloud Security Fundamentals')).toHaveCount(0)

  // A deep link renders for the one fixture item that carries one.
  await expect(page.getByRole('link', { name: 'Open in Edu' }).first()).toHaveAttribute(
    'href',
    'https://edu.cognitum.one/courses/course-1',
  )

  // 5. Confirm the mock Nexus server actually received the
  // `RequestLearningCatalogQuery` for the authenticated dev consultant.
  const catalogRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/edu-catalog-requests`)
  expect(catalogRequestsResponse.ok()).toBe(true)
  const catalogRequests = (await catalogRequestsResponse.json()) as Array<{ body: { consultant_id: string } }>
  expect(catalogRequests.length).toBeGreaterThan(0)
  expect(catalogRequests[catalogRequests.length - 1].body.consultant_id).toBe('dev-consultant-001')
})
