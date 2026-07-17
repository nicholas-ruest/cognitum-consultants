import { test, expect } from '@playwright/test'

// PROMPT-05 smoke test (ADR-013 layer 5): proves the Playwright harness runs
// end-to-end against a live dev server. Not yet wired into CI (see docs/ci.md);
// U27 (Sales lead-conflict e2e) is the unit that establishes full Playwright CI.
test('homepage renders the Cognitum Consultants heading', async ({ page }) => {
  await page.goto('/')

  await expect(
    page.getByRole('heading', { name: 'Cognitum Consultants' }),
  ).toBeVisible()
})
