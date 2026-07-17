import { defineConfig, devices } from '@playwright/test'

// ADR-013 layer 5: Playwright e2e config. The Sales lead-conflict flow (U27)
// will extend this into the reusable e2e template referenced by ADR-013 §6;
// for now this only proves the harness runs against a live dev server.
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  reporter: 'list',
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
