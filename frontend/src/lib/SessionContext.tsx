import { createContext, useContext } from 'react'
import type { ReactNode } from 'react'
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
 */
export type SessionState =
  | { status: 'loading' }
  | { status: 'unauthenticated' }
  | { status: 'authenticated'; consultantId: string }
  | { status: 'error'; error: unknown }

const SessionContext = createContext<SessionState | undefined>(undefined)

export function SessionProvider({ children }: { children: ReactNode }) {
  const { data, isPending, isError, error } = useSessionQuery()

  const value = toSessionState({ data, isPending, isError, error })

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>
}

function toSessionState(query: {
  data: { consultant_id: string } | undefined
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

  return { status: 'authenticated', consultantId: query.data.consultant_id }
}

/** Reads the current `SessionState`. Must be called under `<SessionProvider>`. */
export function useSession(): SessionState {
  const context = useContext(SessionContext)

  if (context === undefined) {
    throw new Error('useSession must be used within a SessionProvider')
  }

  return context
}
