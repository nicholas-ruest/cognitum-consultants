import { MOCK_NEXUS_BASE_URL_ENV, MOCK_NEXUS_PORT } from './constants'
import { startMockNexusServer } from './mock-nexus-server'
import { startTestStack } from './test-stack'

/**
 * Playwright `globalSetup` (PROMPT-27, ADR-013 layer 5): brings up the full
 * real stack this repo's e2e tests drive — real Vite-served frontend (via
 * `playwright.config.ts`'s pre-existing `webServer`), real `bff-api`, real
 * Postgres, with Nexus mocked at the HTTP boundary
 * (`mock-nexus-server.ts`). See that module's and `test-stack.ts`'s doc
 * comments for why each piece is built the way it is.
 *
 * Per Playwright's global-setup/teardown contract, returning a function
 * here registers it as the global teardown — no separate
 * `globalTeardown` config entry is needed, and (unlike writing
 * PIDs/container ids to a temp file) the returned closure can just capture
 * the handles this function already has in scope.
 *
 * `process.env[MOCK_NEXUS_BASE_URL_ENV]` is set so spec files — which run
 * in separate worker processes forked *after* this function resolves, and
 * so inherit this env var — can reach the mock Nexus server's `/_test/...`
 * inspection routes without needing their own reference to the running
 * server.
 */
export default async function globalSetup(): Promise<() => Promise<void>> {
  const mockNexus = await startMockNexusServer(MOCK_NEXUS_PORT)
  process.env[MOCK_NEXUS_BASE_URL_ENV] = mockNexus.url

  const stack = await startTestStack(mockNexus.url)

  return async function globalTeardown(): Promise<void> {
    await stack.stop()
    await mockNexus.close()
  }
}
