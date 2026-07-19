import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Alert, Button } from '@cognitum/design-system'
import { ListDetailPanel } from '@cognitum/dashboard-components'
import { notificationsQueryKey } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'
import type { Notification } from '../../lib/useNotificationsQuery'
import { useNotificationsQuery } from '../../lib/useNotificationsQuery'

/**
 * PROMPT-33: renders the consultant's live notification list
 * (`GET /api/notifications`, kept fresh by `useNotificationStream`'s SSE-
 * triggered invalidation).
 *
 * # One-way read state — no "mark unread" control anywhere
 * Mirrors `bff_core::NotificationItem`'s invariant 3
 * (`consultant-experience-context.md` §2.2): once a notification is read,
 * this component renders a plain "Read" label for it, never a
 * toggle/checkbox/button that could set it back to unread. There is no
 * `PATCH .../unread` endpoint to call even if a control existed — the
 * absence here is a direct reflection of the backend's structural
 * guarantee, not an independent UI choice that could drift from it.
 */
async function markNotificationRead(id: string): Promise<void> {
  const response = await fetch(`/api/notifications/${id}/read`, {
    method: 'PATCH',
    credentials: 'include',
  })

  if (!response.ok) {
    throw new Error(`PATCH /api/notifications/${id}/read failed: ${response.status}`)
  }
}

export function NotificationCentre() {
  const session = useSession()
  const queryClient = useQueryClient()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const { data, isPending, isError } = useNotificationsQuery()
  const notifications = data ?? []

  const markReadMutation = useMutation({
    mutationFn: markNotificationRead,
    onSuccess: () => {
      if (consultantId !== undefined) {
        void queryClient.invalidateQueries({ queryKey: notificationsQueryKey(consultantId) })
      }
    },
  })

  if (isPending) return <p className="text-sm text-muted-foreground">Loading notifications…</p>
  if (isError) return <Alert variant="error">Failed to load notifications.</Alert>

  if (notifications.length === 0) {
    return <p className="text-xs text-muted-foreground">No notifications.</p>
  }

  return (
    <ListDetailPanel
      items={notifications}
      getKey={(notification) => notification.id}
      listClassName="flex flex-col gap-3"
      renderRow={(notification) => (
        <NotificationRow
          notification={notification}
          onMarkRead={() => markReadMutation.mutate(notification.id)}
          isMarkingRead={markReadMutation.isPending && markReadMutation.variables === notification.id}
        />
      )}
    />
  )
}

interface NotificationRowProps {
  notification: Notification
  onMarkRead: () => void
  isMarkingRead: boolean
}

function NotificationRow({ notification, onMarkRead, isMarkingRead }: NotificationRowProps) {
  const isRead = notification.read_state === 'read'

  return (
    <div className="rounded border border-border p-3">
      <p className="text-sm font-semibold text-foreground">{notification.title}</p>
      <p className="text-sm text-card-foreground">{notification.body}</p>

      {notification.deep_link !== null ? (
        <a
          href={notification.deep_link}
          className="text-xs text-primary hover:underline"
          target="_blank"
          rel="noreferrer"
        >
          View details
        </a>
      ) : null}

      <div className="mt-2">
        {isRead ? (
          <span className="text-xs text-muted-foreground">Read</span>
        ) : (
          <Button variant="secondary" disabled={isMarkingRead} onClick={onMarkRead}>
            {isMarkingRead ? 'Marking…' : 'Dismiss'}
          </Button>
        )}
      </div>
    </div>
  )
}
