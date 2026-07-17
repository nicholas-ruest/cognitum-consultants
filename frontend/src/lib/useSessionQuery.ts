import { useQuery } from '@tanstack/react-query'

export interface SessionResponse {
  consultant_id: string
}

async function fetchSession(): Promise<SessionResponse> {
  const response = await fetch('/api/session', { credentials: 'include' })

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
 * This hook is not wired into any login flow, redirect, or protected route
 * yet — that's PROMPT-18's job. It exists purely to demonstrate the
 * `QueryClientProvider` + `useQuery` setup works against a live endpoint.
 */
export function useSessionQuery() {
  return useQuery({
    queryKey: ['session'],
    queryFn: fetchSession,
  })
}
