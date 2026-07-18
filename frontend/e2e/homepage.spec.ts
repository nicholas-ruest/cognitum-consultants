import { test, expect } from '@playwright/test'

// PROMPT-05 smoke test (ADR-013 layer 5): proves the Playwright harness runs
// end-to-end against a live dev server. Now runs in CI as part of the
// `e2e` job (see docs/ci.md) alongside `sales-lead-conflict.spec.ts`
// (PROMPT-27, which brought up the rest of the stack via `globalSetup` —
// this test doesn't need any of it, but pays the cost of it running
// regardless since `globalSetup` is process-wide, not per-spec-file).
//
// PROMPT-27 fix: the original assertion here (`getByRole('heading', {
// name: 'Cognitum Consultants' })`) predates PROMPT-18's auth gating in
// `App.tsx`. An unauthenticated load now always renders `LoginPage`
// ("Sign in") — the "Cognitum Consultants" header only exists inside
// `DashboardPage`, unreachable without a session. This assertion was
// stale (never caught, since this test "was not yet wired into CI" per
// this comment's own prior text) and is corrected here to what a fresh,
// unauthenticated load actually renders, while still proving the same
// thing PROMPT-05 wanted: the harness serves the real, live app.
test('homepage renders the login page for an unauthenticated visitor', async ({ page }) => {
  await page.goto('/')

  await expect(page.getByRole('heading', { name: 'Sign in', level: 3 })).toBeVisible()
})
