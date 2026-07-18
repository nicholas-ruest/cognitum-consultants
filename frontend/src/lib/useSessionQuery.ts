import { useQuery } from '@tanstack/react-query'

/**
 * Mirrors `crates/nexus-client/src/armor.rs`'s `PermissionAssertion` (and
 * `crates/bff-api/src/session.rs`'s re-serving of it verbatim in
 * `GET /api/session`) — a single Armor-granted capability grant, never the
 * underlying authorization policy itself (ADR-009).
 */
export interface PermissionAssertion {
  consultant_id: string
  capability: string
  scope: string
  expires_at: string
}

export interface SessionResponse {
  consultant_id: string
  /** ADR-009, PROMPT-19 — see `components/Sidebar.tsx`'s `navItemsFromAssertions`
   * for the UX-only-never-enforcement framing that applies to any use of
   * this field. */
  permission_assertions: PermissionAssertion[]
}

/**
 * Thrown by `fetchSession` specifically for a 401 response. PROMPT-18:
 * a 401 from `GET /api/session` means "no session" (show `LoginPage`),
 * not a real error to surface to the user — `SessionContext` below
 * branches on this type to tell "unauthenticated" apart from an actual
 * fetch/network/server failure.
 */
export class UnauthorizedError extends Error {
  constructor() {
    super('GET /api/session failed: 401 (unauthenticated)')
    this.name = 'UnauthorizedError'
  }
}

async function fetchSession(): Promise<SessionResponse> {
  const response = await fetch('/api/session', { credentials: 'include' })

  if (response.status === 401) {
    throw new UnauthorizedError()
  }

  if (!response.ok) {
    throw new Error(`GET /api/session failed: ${response.status}`)
  }

  return (await response.json()) as SessionResponse
}

/**
 * Proves TanStack Query works end-to-end against the one real BFF route
 * that exists today (`GET /api/session`, `crates/bff-api/src/session.rs`).
 * Deliberately not namespaced through `queryKeys` (ADR-015's capability
 * convention) since session identity isn't scoped to a `features/<capability>`
 * module — it's a cross-cutting concern.
 *
 * Query key is the bare `['session']` tuple used throughout the app
 * (`LoginPage`'s post-login `invalidateQueries` call, `SessionContext`) —
 * keep any future change to this key in sync with those call sites.
 *
 * Retries are disabled outright: a 401 (`UnauthorizedError`) can't be
 * fixed by retrying without a new login, and for any other failure
 * `SessionContext` already surfaces an explicit `'error'` state rather
 * than silently retrying in the background — retrying would just delay
 * that signal reaching the user.
 */
export function useSessionQuery() {
  return useQuery({
    queryKey: ['session'],
    queryFn: fetchSession,
    retry: false,
  })
}
