import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { actionQueueQueryKey, notificationsQueryKey } from './queryKeys'
import { useSession } from './SessionContext'

/**
 * PROMPT-33: subscribes to `GET /api/notifications/stream`
 * (`crates/bff-api/src/notifications_sse.rs`) via the browser's native
 * `EventSource` — no extra dependency needed, `EventSource` already handles
 * the SSE framing and auto-reconnects on its own after a dropped
 * connection.
 *
 * On every pushed frame (ADR-011's wire shape: `{"kind": "notification" |
 * "action_queue_entry", ...}`), invalidates the matching TanStack Query key
 * (ADR-015) so `useNotificationsQuery`/`useActionQueueQuery` re-fetch —
 * this hook never writes the pushed payload into the cache directly, it
 * only triggers a re-fetch of the authoritative `GET` list, which also
 * naturally re-applies read-state/expiry that a bare SSE payload wouldn't
 * carry consistently.
 *
 * A malformed frame (JSON parse failure or an unrecognized `kind`) is
 * ignored rather than thrown — a single bad frame must not tear down the
 * whole subscription.
 *
 * Call this once near the top of the authenticated shell (e.g.
 * `DashboardPage`) — it has no return value; its only effect is keeping the
 * notification/action-queue caches fresh as events arrive.
 */
export function useNotificationStream(): void {
  const session = useSession()
  const queryClient = useQueryClient()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  useEffect(() => {
    if (consultantId === undefined) return

    const source = new EventSource('/api/notifications/stream')

    source.onmessage = (event: MessageEvent<string>) => {
      const kind = parseKind(event.data)

      if (kind === 'notification') {
        void queryClient.invalidateQueries({ queryKey: notificationsQueryKey(consultantId) })
      } else if (kind === 'action_queue_entry') {
        void queryClient.invalidateQueries({ queryKey: actionQueueQueryKey(consultantId) })
      }
    }

    // Cleanup on unmount (and before a new consultant re-subscribes): stop
    // the connection rather than leaking an open `EventSource` that keeps
    // trying to reconnect in the background.
    return () => {
      source.close()
    }
  }, [consultantId, queryClient])
}

/** Extracts `kind` from one SSE `data:` frame's JSON, or `undefined` if it isn't parseable JSON with a string `kind` field. */
function parseKind(data: string): string | undefined {
  try {
    const parsed: unknown = JSON.parse(data)
    if (typeof parsed === 'object' && parsed !== null && 'kind' in parsed) {
      const kind = (parsed as { kind: unknown }).kind
      return typeof kind === 'string' ? kind : undefined
    }
    return undefined
  } catch {
    return undefined
  }
}
