import { useQuery } from '@tanstack/react-query'
import { actionQueueQueryKey } from './queryKeys'
import { useSession } from './SessionContext'

/** Mirrors `crates/bff-api/src/notifications.rs`'s `ActionQueueEntryDto`. */
export interface ActionQueueEntry {
  id: string
  title: string
  body: string
  deep_link: string | null
  action_state: 'pending' | 'in_progress' | 'completed' | 'expired'
  expires_at: string
  created_at: string
}

async function fetchActionQueue(): Promise<ActionQueueEntry[]> {
  const response = await fetch('/api/action-queue', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/action-queue failed: ${response.status}`)
  }

  return (await response.json()) as ActionQueueEntry[]
}

/**
 * PROMPT-33: fetches the authenticated consultant's action-queue entries
 * (`GET /api/action-queue`). Same `enabled`/query-key-invalidation rationale
 * as `useNotificationsQuery` above — see that file's doc comment.
 */
export function useActionQueueQuery() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  return useQuery({
    queryKey: actionQueueQueryKey(consultantId ?? ''),
    queryFn: fetchActionQueue,
    enabled: session.status === 'authenticated',
  })
}
