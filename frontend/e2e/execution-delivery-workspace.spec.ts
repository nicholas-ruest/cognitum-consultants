import { test, expect } from '@playwright/test'
import { MOCK_NEXUS_BASE_URL_ENV } from './support/constants'

/**
 * PROMPT-38 e2e (ADR-013 layer 5): drives the Execution delivery-workspace
 * flow through the full real stack — real Vite-served frontend, real
 * `bff-api`, real Postgres — with Nexus mocked at the HTTP boundary,
 * following `customer-context-cards.spec.ts`'s established pattern
 * (`docs/SALES_FLOW_PATTERN.md` §5/§9).
 *
 * `"execution"` is one of `DEFAULT_CARD_MODULE_IDS`
 * (`crates/bff-core/src/dashboard_configuration.rs`), so — unlike Customer —
 * no `PUT /api/dashboard` dance is needed before the card can be asserted on
 * screen; granting the `execution` capability (`mock-nexus-server.ts`'s
 * armor fixture) is enough for it to appear.
 *
 * Three flows are covered in one spec, all against the same fixture task
 * (`task-1`, `mock-nexus-server.ts`'s `ENGAGEMENT_SNAPSHOT_FIXTURE`):
 * 1. The read-only delivery workspace (`GET /api/execution/engagements`) and
 *    its "Request Completion" affordance, which forwards to Execution via
 *    the BFF without flipping any local state (PROMPT-38's core ACL flow).
 * 2. A `TaskAssigned` event, ingested via Nexus polling exactly like
 *    `notifications-sse.spec.ts`'s `referral_submitted` case, landing in the
 *    Action Queue card with confirmed-completion semantics: it can be
 *    "started" (a bare consultant click) but this repo exposes no route
 *    that ever locally completes it.
 * 3. The confirmation half of that same invariant, proven end-to-end rather
 *    than just asserted by absence: a `task_completed` event carrying
 *    `related_origin_event_id` back to step 2's `TaskAssigned` event is
 *    ingested the same way, and *that* — not any button in this UI — is what
 *    moves the entry to "Completed" (`crates/bff-core/src/event_ingestion.rs`'s
 *    `ingest_confirmation`).
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

test('logs in, sees the delivery workspace, requests task completion through the BFF, and sees an assigned task land in the action queue with confirmed-completion semantics', async ({
  page,
  request,
}) => {
  // 1. Load the app unauthenticated, log in (PROMPT-18 flow).
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page.getByRole('heading', { name: 'Cognitum Consultants', level: 1 })).toBeVisible()

  // 2. The Execution card renders by default (it's in DEFAULT_CARD_MODULE_IDS),
  // listing the mock Nexus server's fixed `ENGAGEMENT_SNAPSHOT_FIXTURE`.
  await expect(page.getByRole('heading', { name: 'Execution', level: 3 })).toBeVisible()
  await expect(page.getByText('engagement-1')).toBeVisible()
  await expect(page.getByText('on_track')).toBeVisible()

  // No detail card until the engagement is selected.
  await expect(page.getByText('Draft delivery plan')).not.toBeVisible()

  // 3. Selecting the engagement reveals its workstreams, milestones, tasks,
  // and deep link.
  await page.getByRole('button', { name: /engagement-1/ }).click()
  await expect(page.getByText('Discovery')).toBeVisible()
  await expect(page.getByText('Kickoff complete')).toBeVisible()
  await expect(page.getByText('Draft delivery plan')).toBeVisible()
  await expect(page.getByText('Status: assigned')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Open in Execution' })).toHaveAttribute(
    'href',
    'https://execution.cognitum.one/engagements/engagement-1',
  )

  // 4. Requesting completion on the fixture task calls
  // `POST /api/execution/tasks/task-1/complete`, which the BFF forwards to
  // Execution via `ExecutionGateway::confirm_task_completion` — never a
  // local state flip (the task's displayed status stays "assigned").
  const [completionResponse] = await Promise.all([
    page.waitForResponse(
      (res) => res.url().includes('/api/execution/tasks/task-1/complete') && res.request().method() === 'POST',
    ),
    page.getByRole('button', { name: 'Request Completion' }).click(),
  ])
  expect(completionResponse.status()).toBe(200)
  await expect(page.getByText('Status: assigned')).toBeVisible()

  const taskCompletionRequestsResponse = await request.get(`${mockNexusBaseUrl()}/_test/task-completion-requests`)
  expect(taskCompletionRequestsResponse.ok()).toBe(true)
  const taskCompletionRequests = (await taskCompletionRequestsResponse.json()) as Array<{
    body: { task_id: string; consultant_id: string }
  }>
  expect(taskCompletionRequests).toHaveLength(1)
  expect(taskCompletionRequests[0].body).toEqual({ task_id: 'task-1', consultant_id: 'dev-consultant-001' })

  // 5. Queue a `TaskAssigned` event for the mock Nexus server's *next*
  // `events/v1/poll` response, exactly like `notifications-sse.spec.ts`
  // does for `referral_submitted` — `bff-api`'s background polling loop
  // picks it up on its own schedule.
  const originEventId = `e2e-execution-${Date.now()}`
  const enqueueResponse = await request.post(`${mockNexusBaseUrl()}/_test/enqueue-event`, {
    data: {
      origin_capability: 'execution',
      origin_event_id: originEventId,
      event_type: 'task_assigned',
      summary: 'You have been assigned a new delivery task (e2e check).',
      deep_link: 'https://execution.cognitum.one/engagements/engagement-1/tasks/task-1',
      received_at: new Date().toISOString(),
      consultant_id: 'dev-consultant-001',
    },
  })
  expect(enqueueResponse.ok()).toBe(true)

  // 6. Wait for it to appear in the Action Queue card — same SSE-driven
  // chain `notifications-sse.spec.ts` proves, this time landing as an
  // `ActionQueueEntry` (per PROMPT-38's classifier wiring) rather than a
  // `NotificationItem`.
  //
  // Scoped to the Action Queue card's own list item (not a page-wide
  // `page.getByText(...)`) from this point on: when this spec runs as part
  // of the full suite (not in isolation), sibling dashboard cards other
  // specs have already added to this same shared dev consultant's layout —
  // e.g. Edu's own per-course status badges, one of which literally reads
  // "completed" — can otherwise collide with a page-wide "Completed"/"In
  // Progress" text lookup and trip Playwright's strict-mode check.
  const actionQueueEntry = page.getByRole('listitem').filter({ hasText: 'Task Assigned' })
  await expect(actionQueueEntry).toBeVisible({ timeout: 20_000 })
  await expect(actionQueueEntry.getByText('You have been assigned a new delivery task (e2e check).')).toBeVisible()

  // 7. Confirmed-completion semantics: a "Take Action" button exists (the
  // bare consultant click, `Pending -> InProgress`), but no button on this
  // card ever completes the entry directly — completion only ever arrives
  // via a confirmation event through the ingestion pipeline, which this
  // spec's fixture never sends.
  const takeActionButton = actionQueueEntry.getByRole('button', { name: 'Take Action' })
  await expect(takeActionButton).toBeVisible()
  await expect(actionQueueEntry.getByRole('button', { name: /complete/i })).toHaveCount(0)

  const [startResponse] = await Promise.all([
    page.waitForResponse((res) => /\/api\/action-queue\/.+\/start$/.test(res.url()) && res.request().method() === 'POST'),
    takeActionButton.click(),
  ])
  expect(startResponse.status()).toBe(200)

  await expect(actionQueueEntry.getByText('In Progress')).toBeVisible()
  await expect(actionQueueEntry.getByRole('button', { name: 'Take Action' })).toHaveCount(0)
  await expect(actionQueueEntry.getByRole('button', { name: /complete/i })).toHaveCount(0)

  // 8. Confirmation, proven end-to-end (not just "no button completes it"):
  // queue a `task_completed` event referencing step 5's `TaskAssigned`
  // event via `related_origin_event_id`. `bff-api`'s polling loop ingests
  // it into `bff_core::event_ingestion::ingest_confirmation`, which is the
  // *only* thing in this repo that ever moves the entry to `Completed`
  // (`ActionQueueEntry::complete`, invariant 3).
  const confirmationEnqueueResponse = await request.post(`${mockNexusBaseUrl()}/_test/enqueue-event`, {
    data: {
      origin_capability: 'execution',
      origin_event_id: `e2e-execution-confirmation-${Date.now()}`,
      event_type: 'task_completed',
      summary: 'Execution confirmed your task is complete (e2e check).',
      deep_link: null,
      received_at: new Date().toISOString(),
      consultant_id: 'dev-consultant-001',
      related_origin_event_id: originEventId,
    },
  })
  expect(confirmationEnqueueResponse.ok()).toBe(true)

  await expect(actionQueueEntry.getByText('Completed')).toBeVisible({ timeout: 20_000 })
  await expect(actionQueueEntry.getByText('In Progress')).not.toBeVisible()
})
