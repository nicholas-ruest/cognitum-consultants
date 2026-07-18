import { useQuery } from '@tanstack/react-query'
import { dashboardQueryKey } from './queryKeys'
import { useSession } from './SessionContext'

/**
 * Mirrors `crates/bff-api/src/dashboard.rs`'s `DashboardCardDto` — one card
 * placed at a fixed `position`, pointing at a `module_id` (a capability
 * name, e.g. `"sales"`).
 */
export interface DashboardCard {
  module_id: string
  position: number
}

/** Mirrors `crates/bff-api/src/dashboard.rs`'s `DashboardResponse`. */
export interface DashboardResponse {
  consultant_id: string
  cards: DashboardCard[]
}

async function fetchDashboard(): Promise<DashboardResponse> {
  const response = await fetch('/api/dashboard', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/dashboard failed: ${response.status}`)
  }

  return (await response.json()) as DashboardResponse
}

/**
 * PROMPT-23: fetches the authenticated consultant's dashboard composition
 * (`GET /api/dashboard`).
 *
 * `enabled: session.status === 'authenticated'` — this must never fire
 * while `useSession()` is still loading or has no session at all: there is
 * no `consultantId` to scope the query key by yet, and the BFF's
 * `require_session` gate would just 401 the request anyway (see
 * `crates/bff-api/src/dashboard.rs`'s module docs on that 401 gate).
 * `useSessionQuery`'s own loading/401/error states are what `App.tsx`/
 * `SessionContext` already branch on before anything reaches
 * `DashboardPage`, so this hook only ever runs once a `consultantId` is
 * known.
 */
export function useDashboardQuery() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  return useQuery({
    queryKey: dashboardQueryKey(consultantId ?? ''),
    queryFn: fetchDashboard,
    enabled: session.status === 'authenticated',
  })
}
