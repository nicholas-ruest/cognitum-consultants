import { defineConfig, devices } from '@playwright/test'

// ADR-013 layer 5: Playwright e2e config. `globalSetup` (PROMPT-27) brings
// up the rest of the real stack this repo's e2e specs drive — real
// `bff-api`, real (throwaway, migrated) Postgres, and a mock Nexus HTTP
// server — around this pre-existing `webServer` (the real Vite-served
// frontend). See `e2e/support/global-setup.ts` for the orchestration.
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  // Every spec authenticates as the same fixed dev consultant identity
  // (`auth::dev_stub::DEV_CONSULTANT_ID`, ADR-008) against one shared
  // Postgres instance for the whole run — there is no per-test identity or
  // data isolation. Several specs (capacity/customer/edu/landscape/products)
  // read-modify-write that consultant's `DashboardConfiguration` (`GET
  // /api/dashboard` then `PUT` back with their own card appended) to add
  // their card before asserting on it. Run concurrently across workers,
  // that read-modify-write races: two specs can both `GET` the same
  // pre-append layout, and whichever `PUT`s last silently drops the other's
  // card, failing that spec's "card renders" assertion nondeterministically.
  // `workers: 1` serializes the whole run so no two specs ever interleave
  // their `GET`/`PUT` pair against this shared consultant record.
  workers: 1,
  // 'list' for readable console output either way; 'html' additionally
  // (written to playwright-report/, not opened automatically) so CI has
  // something to upload as an artifact on failure (.github/workflows/ci.yml
  // `e2e` job) — a bare console log doesn't survive past the run.
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  globalSetup: './e2e/support/global-setup.ts',
  // The stack `globalSetup` brings up (Postgres container start +
  // migrations + a `cargo build`) can comfortably take longer than
  // Playwright's default per-test timeout, which does not otherwise apply
  // to global setup/teardown but is worth raising explicitly for clarity.
  globalTimeout: 10 * 60 * 1000,
  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'on-first-retry',
  },
  webServer: {
    // Force IPv4 loopback explicitly: this sandbox's Vite dev server binds
    // only to the IPv6 loopback (::1) by default, which leaves 127.0.0.1
    // connection-refused.
    command: 'npm run dev -- --host 127.0.0.1',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})
