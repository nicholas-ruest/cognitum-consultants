import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-40 e2e (ADR-013 layer 5): drives the Landscape intelligence-digest
 * (read) and field-observation-submission (write) flows through the full
 * real stack — real Vite-served frontend, real `bff-api`, real Postgres —
 * with Nexus mocked at the HTTP boundary, following
 * `capacity-profile-update.spec.ts`'s established read+write pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"landscape"` is deliberately excluded from `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`), so — exactly like
 * `capacity-profile-update.spec.ts` — this spec first `GET`s the dev
 * consultant's current dashboard layout and `PUT`s it back with a
 * `"landscape"` card appended before the card can be asserted on screen.
 * See that spec's own doc comment for why this appends rather than
 * replacing the layout outright.
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

test('logs in, adds the Landscape card, sees the intelligence digest, and submits a field observation', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Fetch the current layout — `landscape` isn't in it, since it isn't
  // one of the three defaults.
  const getResponse = await page.request.get('/api/dashboard')
  expect(getResponse.ok()).toBe(true)
  const currentDashboard = (await getResponse.json()) as { cards: Array<{ module_id: string; position: number }> }
  expect(currentDashboard.cards.some((card) => card.module_id === 'landscape')).toBe(false)

  // 3. `PUT` the existing layout back with a `landscape` card appended, then
  // reload so `DashboardPage` re-fetches the merged layout.
  const nextPosition = currentDashboard.cards.reduce((max, card) => Math.max(max, card.position), -1) + 1
  const putResponse = await page.request.put('/api/dashboard', {
    data: { cards: [...currentDashboard.cards, { module_id: 'landscape', position: nextPosition }] },
  })
  expect(putResponse.ok()).toBe(true)
  await page.reload()

  // 4. Navigate to the Landscape module route (ADR-020 part C; `exact:
  // true` since "Open in Landscape" — asserted below — would otherwise
  // substring-match the same nav link name). The card renders, listing the
  // mock Nexus server's fixed `INTELLIGENCE_DIGEST_FIXTURE`
  // (`mock-nexus-server.ts`) — one item with a deep link, one without.
  await page.getByRole('link', { name: 'Landscape', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'Landscape', level: 3 })).toBeVisible()
  await expect(page.getByText('Cloud Migration Trends')).toBeVisible()
  await expect(page.getByText('Regulatory Shifts')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Open in Landscape' })).toHaveAttribute(
    'href',
    'https://landscape.cognitum.one/intel/intel-1',
  )

  // 5. Submit a field observation.
  await page.getByLabel('Observation').fill('Client hinted at expanding into a new region next quarter.')
  await page.getByLabel('Related Company Reference (optional)').fill('acme-corp')
  await page.getByRole('button', { name: 'Submit Observation' }).click()

  // 6. The ack renders, and the form clears (never re-auto-retried; a fresh
  // submission would start from a blank form).
  await expect(page.getByText('Observation submitted.')).toBeVisible()
  await expect(page.getByLabel('Observation')).toHaveValue('')

  // 7. Confirm the mock Nexus server actually received the
  // `FieldObservationSubmission` for the authenticated dev consultant.
  const observationsResponse = await request.get(`${mockNexusBaseUrl()}/_test/field-observations`)
  expect(observationsResponse.ok()).toBe(true)
  const observations = (await observationsResponse.json()) as Array<{
    body: { observation_text: string; related_company_reference?: string; submitted_by: string }
  }>
  expect(observations.length).toBeGreaterThan(0)
  const lastObservation = observations[observations.length - 1]
  expect(lastObservation.body.observation_text).toBe('Client hinted at expanding into a new region next quarter.')
  expect(lastObservation.body.related_company_reference).toBe('acme-corp')
  expect(lastObservation.body.submitted_by).toBe('dev-consultant-001')
})
