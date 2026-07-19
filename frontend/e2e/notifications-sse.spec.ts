import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-33 e2e: proves an SSE-delivered event actually triggers a UI
 * re-render against the real running stack — real Vite-served frontend,
 * real `bff-api` (real Postgres, real Postgres LISTEN/NOTIFY bridge,
 * PROMPT-32/ADR-014), real `GET /api/notifications/stream` SSE endpoint,
 * with only Nexus mocked at the HTTP boundary (`mock-nexus-server.ts`,
 * same pattern as `sales-lead-conflict.spec.ts`).
 *
 * No page reload/navigation happens anywhere after the initial `page.goto`
 * + login — the notification only becomes visible because
 * `useNotificationStream`'s real `EventSource` (native to the real browser
 * Playwright drives, not a jsdom stub) received a real pushed frame and
 * invalidated the real TanStack Query cache, which re-fetched
 * `GET /api/notifications` and re-rendered `NotificationCentre`.
 */

function mockNexusBaseUrl(): string {
  const url = process.env[MOCK_NEXUS_BASE_URL_ENV]
  if (!url) {
    throw new Error(
      `${MOCK_NEXUS_BASE_URL_ENV} is not set — this test must run under playwright.config.ts's globalSetup.`,
    )
  }
  return url
}

test('an event ingested via Nexus polling is pushed over SSE and appears in the Notification Centre with no page reload', async ({
  page,
  request,
}) => {
  // 1. Log in (PROMPT-18 flow, same as sales-lead-conflict.spec.ts).
  await page.goto('/')
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. Notification Centre card renders, starting empty — nothing has been
  // ingested for this consultant yet.
  await expect(page.getByRole('heading', { name: 'Notifications', level: 3 })).toBeVisible()
  await expect(page.getByText('No notifications.')).toBeVisible()

  // 3. Queue a fresh CapabilityEventReceived for the mock Nexus server's
  // *next* `events/v1/poll` response — `bff-api`'s background polling loop
  // (PROMPT-30) will pick this up on its own schedule (no manual trigger
  // from this test beyond queuing it).
  const originEventId = `e2e-sse-${Date.now()}`
  const enqueueResponse = await request.post(`${mockNexusBaseUrl()}/_test/enqueue-event`, {
    data: {
      origin_capability: 'sales',
      origin_event_id: originEventId,
      event_type: 'referral_submitted',
      summary: 'A new referral was submitted for review (e2e SSE check).',
      deep_link: 'https://app.example.com/sales/referrals/e2e-sse',
      received_at: new Date().toISOString(),
      consultant_id: 'dev-consultant-001',
    },
  })
  expect(enqueueResponse.ok()).toBe(true)

  // 4. Wait for it to appear — driven entirely by: bff-api's poll loop
  // fetching + ingesting it, Postgres NOTIFY/LISTEN bridging it to this
  // instance's EventBus, the SSE endpoint pushing it to this page's live
  // EventSource, `useNotificationStream` invalidating the notifications
  // query key, and TanStack Query re-fetching. Generous timeout to cover
  // the poll interval (default 5s) plus that whole chain.
  await expect(page.getByText('Referral Submitted')).toBeVisible({ timeout: 20_000 })
  await expect(page.getByText('A new referral was submitted for review (e2e SSE check).')).toBeVisible()
  await expect(page.getByText('No notifications.')).toHaveCount(0)

  // 5. One-way read state: a Dismiss button exists for the still-unread
  // item; clicking it calls the mark-read endpoint and the item flips to a
  // plain "Read" label with no unread-toggle control anywhere.
  const dismissButton = page.getByRole('button', { name: 'Dismiss' })
  await expect(dismissButton).toBeVisible()

  const [readResponse] = await Promise.all([
    page.waitForResponse((res) => /\/api\/notifications\/.+\/read$/.test(res.url()) && res.request().method() === 'PATCH'),
    dismissButton.click(),
  ])
  expect(readResponse.status()).toBe(200)

  await expect(page.getByText('Read')).toBeVisible()
  await expect(page.getByRole('button', { name: 'Dismiss' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /unread/i })).toHaveCount(0)
})
