import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-36 e2e (ADR-013 layer 5): drives the Capacity own-profile-update
 * flow through the full real stack — real Vite-served frontend, real
 * `bff-api`, real Postgres — with Nexus mocked at the HTTP boundary,
 * following `edu-learning-dashboard.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"capacity"` is deliberately excluded from `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`), so — exactly like
 * `edu-learning-dashboard.spec.ts` — this spec first `GET`s the dev
 * consultant's current dashboard layout and `PUT`s it back with a
 * `"capacity"` card appended (via `page.request`, sharing the logged-in
 * page's session cookie) before the card can be asserted on screen. See
 * that spec's own doc comment for why this appends rather than replacing
 * the layout outright.
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

test('logs in, adds the Capacity card, edits the profile, and sees the accepted verdict', async ({ page, request }) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Fetch the current layout — `capacity` isn't in it, since it isn't
  // one of the three defaults.
  const getResponse = await page.request.get('/api/dashboard')
  expect(getResponse.ok()).toBe(true)
  const currentDashboard = (await getResponse.json()) as { cards: Array<{ module_id: string; position: number }> }
  expect(currentDashboard.cards.some((card) => card.module_id === 'capacity')).toBe(false)

  // 3. `PUT` the existing layout back with a `capacity` card appended, then
  // reload so `DashboardPage` re-fetches the merged layout.
  const nextPosition = currentDashboard.cards.reduce((max, card) => Math.max(max, card.position), -1) + 1
  const putResponse = await page.request.put('/api/dashboard', {
    data: { cards: [...currentDashboard.cards, { module_id: 'capacity', position: nextPosition }] },
  })
  expect(putResponse.ok()).toBe(true)
  await page.reload()

  // 4. The Capacity card renders, pre-populated from the mock Nexus
  // server's fixed `PROFILE_FIXTURE` (`mock-nexus-server.ts`).
  await expect(page.getByRole('heading', { name: 'Capacity', level: 3 })).toBeVisible()
  await expect(page.getByLabel('Skills (comma-separated)')).toHaveValue('Rust, Cloud Architecture')
  await expect(page.getByLabel('Availability Window')).toHaveValue('2026-08-01/2026-12-31')

  // 5. Edit the skills field and save — this repo never renders another
  // consultant's data, and this flow only ever touches the dev consultant's
  // own profile (`GET`/`PATCH /api/capacity/profile` take no other id).
  await page.getByLabel('Skills (comma-separated)').fill('Rust, Kubernetes')
  await page.getByRole('button', { name: 'Save Profile' }).click()

  // 6. Capacity's verdict (relayed verbatim, never re-adjudicated) renders.
  await expect(page.getByText('Profile update accepted.')).toBeVisible()

  // 7. Confirm the mock Nexus server actually received the
  // `UpdateOwnProfileCommand` for the authenticated dev consultant, with the
  // edited skill list.
  const updatesResponse = await request.get(`${mockNexusBaseUrl()}/_test/capacity-profile-updates`)
  expect(updatesResponse.ok()).toBe(true)
  const updates = (await updatesResponse.json()) as Array<{
    body: { consultant_id: string; profile_fields: { skills: string[] } }
  }>
  expect(updates.length).toBeGreaterThan(0)
  const lastUpdate = updates[updates.length - 1]
  expect(lastUpdate.body.consultant_id).toBe('dev-consultant-001')
  expect(lastUpdate.body.profile_fields.skills).toEqual(['Rust', 'Kubernetes'])

  // 8. A subsequent reload re-fetches the now-updated profile (the mock
  // server's stateful store, see the module docs) — proving the accepted
  // update actually round-trips, not just that the form's local state
  // changed.
  await page.reload()
  await expect(page.getByLabel('Skills (comma-separated)')).toHaveValue('Rust, Kubernetes')
})
