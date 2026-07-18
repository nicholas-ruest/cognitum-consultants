/**
 * Shared ports/paths for the PROMPT-27 e2e orchestration (global-setup.ts,
 * test-stack.ts, mock-nexus-server.ts, and any spec that needs to reach the
 * mock Nexus server's inspection endpoints).
 *
 * Fixed (not dynamically allocated) ports are deliberate: this harness
 * only ever runs one Playwright project (`chromium`, `fullyParallel` test
 * *files*, not multiple concurrent stacks) against one stack per `npx
 * playwright test` invocation, so a port clash within a single run isn't a
 * concern. `BFF_PORT` in particular is NOT free to change independently —
 * it must match `frontend/vite.config.ts`'s hardcoded `/api` proxy target
 * (`http://127.0.0.1:3000`).
 */

/** Must match `vite.config.ts`'s proxy target for `/api/*`. */
export const BFF_PORT = 3000

/** Mock Nexus HTTP server (stands in for `nexus.cognitum.one`, ADR-007). */
export const MOCK_NEXUS_PORT = 4010
export const MOCK_NEXUS_BASE_URL = `http://127.0.0.1:${MOCK_NEXUS_PORT}`

/** Throwaway Postgres container, mapped to the host on this port. Not the
 * Postgres default (5432) or the equally-common 55432 — deliberately picked
 * to avoid clashing with an unrelated Postgres a dev machine or shared
 * sandbox may already have bound on one of those. */
export const POSTGRES_PORT = 57432
export const POSTGRES_DATABASE_URL = `postgres://postgres:postgres@127.0.0.1:${POSTGRES_PORT}/postgres`

/** Env var name the spec reads to find the mock Nexus server's base URL. */
export const MOCK_NEXUS_BASE_URL_ENV = 'E2E_MOCK_NEXUS_BASE_URL'
