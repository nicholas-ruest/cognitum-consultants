import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Alert, Button } from '@cognitum/design-system'
import { ListDetailPanel } from '@cognitum/dashboard-components'
import { actionQueueQueryKey } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'
import type { ActionQueueEntry as ActionQueueEntryData } from '../../lib/useActionQueueQuery'
import { useActionQueueQuery } from '../../lib/useActionQueueQuery'

/**
 * PROMPT-33: renders the consultant's action queue
 * (`GET /api/action-queue`, kept fresh by `useNotificationStream`'s SSE-
 * triggered invalidation).
 *
 * # Completion is never triggered from this UI
 * The "take action" button below calls only `POST
 * /api/action-queue/:id/start` (`crates/bff-api/src/notifications.rs`'s
 * `action_queue_start`), which is a bare consultant click — `Pending ->
 * InProgress`, per `bff_core::ActionQueueEntry::start`'s doc comment. There
 * is no `.../complete` endpoint for this component to call: completion
 * requires a `confirmation_event_id` that only PROMPT-30's event-ingestion
 * pipeline ever has (invariant 3, `consultant-experience-context.md` §2.2).
 * An entry's `action_state` only ever advances to `completed` here as a
 * *reflection* of that confirmation arriving — via `useNotificationStream`'s
 * SSE invalidation or the next `GET /api/action-queue` poll/re-fetch — never
 * as a local decision this component makes.
 */
async function startActionQueueEntry(id: string): Promise<void> {
  const response = await fetch(`/api/action-queue/${id}/start`, {
    method: 'POST',
    credentials: 'include',
  })

  if (!response.ok) {
    throw new Error(`POST /api/action-queue/${id}/start failed: ${response.status}`)
  }
}

/** Threshold below which an entry's expiry indicator switches to a warning style. */
const EXPIRY_WARNING_THRESHOLD_MS = 24 * 60 * 60 * 1000

export function ActionQueue() {
  const session = useSession()
  const queryClient = useQueryClient()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const { data, isPending, isError } = useActionQueueQuery()
  const entries = data ?? []

  const startMutation = useMutation({
    mutationFn: startActionQueueEntry,
    onSuccess: () => {
      if (consultantId !== undefined) {
        void queryClient.invalidateQueries({ queryKey: actionQueueQueryKey(consultantId) })
      }
    },
  })

  if (isPending) return <p className="text-sm text-gray-500">Loading action queue…</p>
  if (isError) return <Alert variant="error">Failed to load your action queue.</Alert>

  if (entries.length === 0) {
    return <p className="text-xs text-gray-500">Nothing needs your attention right now.</p>
  }

  return (
    <ListDetailPanel
      items={entries}
      getKey={(entry) => entry.id}
      listClassName="flex flex-col gap-3"
      renderRow={(entry) => (
        <ActionQueueRow
          entry={entry}
          onStart={() => startMutation.mutate(entry.id)}
          isStarting={startMutation.isPending && startMutation.variables === entry.id}
        />
      )}
    />
  )
}

interface ActionQueueRowProps {
  entry: ActionQueueEntryData
  onStart: () => void
  isStarting: boolean
}

function ActionQueueRow({ entry, onStart, isStarting }: ActionQueueRowProps) {
  return (
    <div className="rounded border border-gray-200 p-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm font-semibold text-gray-900">{entry.title}</p>
        <StateBadge state={entry.action_state} />
      </div>
      <p className="text-sm text-gray-700">{entry.body}</p>
      <ExpiryIndicator expiresAt={entry.expires_at} />

      {entry.deep_link !== null ? (
        <a
          href={entry.deep_link}
          className="text-xs text-blue-600 hover:underline"
          target="_blank"
          rel="noreferrer"
        >
          View details
        </a>
      ) : null}

      {/* "Take action" is visible/enabled only while `pending` — once it's
          `in_progress`/`completed`/`expired`, this component renders no
          button for it at all (not just disabled), since a second click
          on an already-started entry has nothing left for `start` to do. */}
      {entry.action_state === 'pending' ? (
        <div className="mt-2">
          <Button disabled={isStarting} onClick={onStart}>
            {isStarting ? 'Starting…' : 'Take Action'}
          </Button>
        </div>
      ) : null}
    </div>
  )
}

const STATE_LABELS: Record<ActionQueueEntryData['action_state'], string> = {
  pending: 'Pending',
  in_progress: 'In Progress',
  completed: 'Completed',
  expired: 'Expired',
}

const STATE_CLASSES: Record<ActionQueueEntryData['action_state'], string> = {
  pending: 'bg-yellow-50 text-yellow-800',
  in_progress: 'bg-blue-50 text-blue-800',
  completed: 'bg-green-50 text-green-800',
  expired: 'bg-gray-100 text-gray-600',
}

function StateBadge({ state }: { state: ActionQueueEntryData['action_state'] }) {
  return (
    <span className={`rounded px-2 py-0.5 text-xs font-medium ${STATE_CLASSES[state]}`}>
      {STATE_LABELS[state]}
    </span>
  )
}

/** Relative-time expiry indicator, switching to a warning style once inside `EXPIRY_WARNING_THRESHOLD_MS` of `expiresAt`. */
function ExpiryIndicator({ expiresAt }: { expiresAt: string }) {
  const remainingMs = new Date(expiresAt).getTime() - Date.now()
  const isSoon = remainingMs <= EXPIRY_WARNING_THRESHOLD_MS

  return (
    <p className={`text-xs ${isSoon ? 'font-semibold text-red-600' : 'text-gray-500'}`}>
      {remainingMs <= 0 ? 'Expired' : `Expires ${formatRelative(remainingMs)}`}
    </p>
  )
}

function formatRelative(remainingMs: number): string {
  const hours = Math.round(remainingMs / (60 * 60 * 1000))
  if (hours < 1) return 'soon'
  if (hours < 24) return `in ${hours}h`
  return `in ${Math.round(hours / 24)}d`
}
