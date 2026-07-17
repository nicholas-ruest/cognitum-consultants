import { useSessionQuery } from './useSessionQuery'

/**
 * Minimal proof that `useQuery` + `QueryClientProvider` work end-to-end
 * against a real BFF route (`GET /api/session`). Not mounted in `<App>` and
 * not part of any login/redirect flow — that wiring is PROMPT-18's job.
 * This component exists solely so the pattern is exercised by a real
 * component (see its test), per PROMPT-16's acceptance criteria.
 */
export function SessionStatus() {
  const { data, isPending, isError } = useSessionQuery()

  if (isPending) return <p>Loading session…</p>
  if (isError) return <p>No active session</p>

  return <p>Signed in as {data.consultant_id}</p>
}
