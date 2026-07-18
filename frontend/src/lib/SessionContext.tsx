import { createContext, useContext } from 'react'
import type { ReactNode } from 'react'
import type { PermissionAssertion, SessionResponse } from './useSessionQuery'
import { UnauthorizedError, useSessionQuery } from './useSessionQuery'

/**
 * PROMPT-18: session identity exposed via React context so the rest of the
 * app (starting with `App.tsx`'s authenticated/unauthenticated branch) can
 * read `consultant_id` without prop-drilling `useSessionQuery`'s result
 * through every component.
 *
 * Deliberately a discriminated union on `status` rather than exposing raw
 * TanStack Query flags (`isPending`/`isError`/...) — callers should branch
 * on "what to render" (loading / login / authenticated shell / real error),
 * not on query-internal state. A 401 (`UnauthorizedError`, see
 * `useSessionQuery.ts`) maps to `'unauthenticated'`, not `'error'` — per
 * PROMPT-18, "no session" is an expected state, not a failure to surface.
 *
 * PROMPT-19 (ADR-009): the authenticated variant also carries
 * `permissionAssertions`, the consultant's current Armor-granted
 * `PermissionAssertion` set, so `App.tsx`/`Sidebar.tsx` can build
 * permission-aware nav without a separate query. As with every other use of
 * this data (see `components/Sidebar.tsx`), this is UX-only — it is never
 * an authorization decision in its own right.
 */
export type SessionState =
  | { status: 'loading' }
  | { status: 'unauthenticated' }
  | { status: 'authenticated'; consultantId: string; permissionAssertions: PermissionAssertion[] }
  | { status: 'error'; error: unknown }

const SessionContext = createContext<SessionState | undefined>(undefined)

export function SessionProvider({ children }: { children: ReactNode }) {
  const { data, isPending, isError, error } = useSessionQuery()

  const value = toSessionState({ data, isPending, isError, error })

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>
}

function toSessionState(query: {
  data: SessionResponse | undefined
  isPending: boolean
  isError: boolean
  error: unknown
}): SessionState {
  if (query.isPending) return { status: 'loading' }

  if (query.isError || query.data === undefined) {
    return query.error instanceof UnauthorizedError
      ? { status: 'unauthenticated' }
      : { status: 'error', error: query.error }
  }

  return {
    status: 'authenticated',
    consultantId: query.data.consultant_id,
    // Defensive fallback: existing mocks/callers that predate PROMPT-19
    // may omit this field entirely; treat that as "no assertions" rather
    // than throwing downstream in `navItemsFromAssertions`.
    permissionAssertions: query.data.permission_assertions ?? [],
  }
}

/** Reads the current `SessionState`. Must be called under `<SessionProvider>`. */
export function useSession(): SessionState {
  const context = useContext(SessionContext)

  if (context === undefined) {
    throw new Error('useSession must be used within a SessionProvider')
  }

  return context
}
