import { useQuery } from '@tanstack/react-query'
import { notificationsQueryKey } from './queryKeys'
import { useSession } from './SessionContext'

/** Mirrors `crates/bff-api/src/notifications.rs`'s `NotificationDto`. */
export interface Notification {
  id: string
  title: string
  body: string
  deep_link: string | null
  read_state: 'unread' | 'read'
  created_at: string
}

async function fetchNotifications(): Promise<Notification[]> {
  const response = await fetch('/api/notifications', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/notifications failed: ${response.status}`)
  }

  return (await response.json()) as Notification[]
}

/**
 * PROMPT-33: fetches the authenticated consultant's notifications
 * (`GET /api/notifications`).
 *
 * `enabled: session.status === 'authenticated'`, same rationale as
 * `useDashboardQuery`'s identical guard — there is no `consultantId` to
 * scope the query key by until a session exists, and the BFF's
 * `require_session` gate would 401 the request anyway.
 *
 * This query's key (`notificationsQueryKey`) is exactly what
 * `useNotificationStream`'s SSE handler invalidates on every pushed
 * `"notification"` event (ADR-011/ADR-015), so a live push triggers a
 * re-fetch through this same hook rather than a separate code path.
 */
export function useNotificationsQuery() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  return useQuery({
    queryKey: notificationsQueryKey(consultantId ?? ''),
    queryFn: fetchNotifications,
    enabled: session.status === 'authenticated',
  })
}
