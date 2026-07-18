import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert } from '../../components/Alert'
import { Button } from '../../components/Button'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-38: Execution delivery-workspace feature module, following
 * `CustomerContextList.tsx`'s `useQuery` list-plus-selected-detail pattern
 * (`docs/SALES_FLOW_PATTERN.md` §4) — `GET /api/execution/engagements` is an
 * ordinary cacheable, re-fetchable resource, not a per-submission verdict
 * like Sales' conflict check, so this uses `useQuery`, not `useMutation`.
 *
 * Mirrors `crates/nexus-client/src/execution.rs`'s `EngagementSnapshot`
 * verbatim — `crates/bff-api/src/execution.rs` relays it unshaped, same
 * convention as Commit's `ProposalSummary`/Customer's `CustomerContextCard`.
 */
export interface EngagementTaskSummary {
  task_id: string
  title: string
  status: string
}

export interface EngagementSnapshot {
  engagement_id: string
  workstreams: string[]
  milestones: string[]
  tasks: EngagementTaskSummary[]
  delivery_status: string
  deep_link: string | null
}

async function fetchAssignedEngagements(): Promise<EngagementSnapshot[]> {
  const response = await fetch('/api/execution/engagements', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/execution/engagements failed: ${response.status}`)
  }

  return (await response.json()) as EngagementSnapshot[]
}

/**
 * `POST /api/execution/tasks/:id/complete` — requests completion through the
 * BFF back to Execution (`crates/bff-api/src/execution.rs`'s `complete_task`).
 *
 * # This never marks the task done locally
 * There is no client-side state flip here: this only fires the request. The
 * task keeps showing whatever `status` the next `GET /api/execution/engagements`
 * re-fetch reports, and any `ActionQueueEntry` this task's `TaskAssigned`
 * event produced (see `ActionQueue.tsx`, rendered elsewhere on the
 * dashboard) only ever reaches `completed` once Execution's own confirmation
 * event arrives through the notification/action-queue ingestion pipeline —
 * never as a consequence of this mutation succeeding. This mirrors
 * `ActionQueue.tsx`'s own "Completion is never triggered from this UI"
 * documented rule.
 */
async function requestTaskCompletion(taskId: string): Promise<void> {
  const response = await fetch(`/api/execution/tasks/${taskId}/complete`, {
    method: 'POST',
    credentials: 'include',
  })

  if (!response.ok) {
    throw new Error(`POST /api/execution/tasks/${taskId}/complete failed: ${response.status}`)
  }
}

/**
 * `delivery_status` badge color, purely a rendering choice — the underlying
 * text (displayed verbatim, never re-derived) is always `delivery_status`
 * itself. **Provisional, documented assumption**: `EngagementSnapshot`
 * (`anti-corruption-layers.md` §6) names no fixed `delivery_status`
 * vocabulary, so this recognizes the conventional values and falls back to a
 * neutral style for anything else — same reasoning
 * `CustomerContextList.tsx`'s `HEALTH_BADGE_CLASSES` used for its own
 * provisional vocabulary.
 */
const DELIVERY_STATUS_BADGE_CLASSES: Record<string, string> = {
  on_track: 'bg-green-100 text-green-800',
  at_risk: 'bg-yellow-100 text-yellow-800',
  delayed: 'bg-red-100 text-red-800',
}

function deliveryStatusBadgeClass(deliveryStatus: string): string {
  return DELIVERY_STATUS_BADGE_CLASSES[deliveryStatus.toLowerCase()] ?? 'bg-gray-100 text-gray-700'
}

export function ExecutionWorkspace() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const [selectedEngagementId, setSelectedEngagementId] = useState<string | null>(null)
  const queryClient = useQueryClient()

  const engagementsQuery = useQuery({
    queryKey: queryKeys.execution.engagements(consultantId ?? ''),
    queryFn: fetchAssignedEngagements,
    enabled: session.status === 'authenticated',
  })

  const completionMutation = useMutation({
    mutationFn: requestTaskCompletion,
    onSuccess: () => {
      // Re-fetches Execution's own current view (which may now report a
      // "completion requested" task status) — this is Execution's data
      // reflecting back, not this repo deciding the task is done. See the
      // module docs: the `ActionQueueEntry` this task's `TaskAssigned` event
      // produced still only completes via the ingestion pipeline.
      if (consultantId !== undefined) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.execution.engagements(consultantId) })
      }
    },
  })

  if (engagementsQuery.isPending) {
    return <p className="text-sm text-gray-500">Loading your delivery workspace…</p>
  }

  if (engagementsQuery.isError) {
    return <Alert variant="error">Failed to load your delivery workspace.</Alert>
  }

  const engagements = engagementsQuery.data ?? []
  const selectedEngagement = engagements.find((engagement) => engagement.engagement_id === selectedEngagementId) ?? null

  if (engagements.length === 0) {
    return <p className="text-xs text-gray-500">No assigned engagements yet.</p>
  }

  return (
    <div className="flex flex-col gap-4">
      {completionMutation.isError ? (
        <Alert variant="error">Failed to request task completion. Please try again.</Alert>
      ) : null}

      <ul className="flex flex-col gap-2">
        {engagements.map((engagement) => (
          <li key={engagement.engagement_id}>
            <button
              type="button"
              onClick={() => setSelectedEngagementId(engagement.engagement_id)}
              className="w-full rounded border border-gray-200 p-3 text-left hover:bg-gray-50"
            >
              <div className="flex items-center justify-between gap-2">
                <p className="text-sm font-semibold text-gray-900">{engagement.engagement_id}</p>
                <span className={`rounded px-2 py-0.5 text-xs ${deliveryStatusBadgeClass(engagement.delivery_status)}`}>
                  {engagement.delivery_status}
                </span>
              </div>
            </button>
          </li>
        ))}
      </ul>

      {selectedEngagement ? (
        <EngagementDetail
          engagement={selectedEngagement}
          onRequestCompletion={(taskId) => completionMutation.mutate(taskId)}
          isRequestingCompletion={completionMutation.isPending}
          requestingTaskId={completionMutation.isPending ? completionMutation.variables : undefined}
        />
      ) : null}
    </div>
  )
}

interface EngagementDetailProps {
  engagement: EngagementSnapshot
  onRequestCompletion: (taskId: string) => void
  isRequestingCompletion: boolean
  requestingTaskId: string | undefined
}

function EngagementDetail({ engagement, onRequestCompletion, isRequestingCompletion, requestingTaskId }: EngagementDetailProps) {
  return (
    <div className="rounded border border-gray-300 p-3">
      <h4 className="text-sm font-semibold text-gray-900">{engagement.engagement_id}</h4>
      <p className="text-xs text-gray-500">
        Delivery status:{' '}
        <span className={`rounded px-2 py-0.5 ${deliveryStatusBadgeClass(engagement.delivery_status)}`}>
          {engagement.delivery_status}
        </span>
      </p>

      {engagement.deep_link !== null ? (
        <a href={engagement.deep_link} className="text-xs text-blue-600 hover:underline" target="_blank" rel="noreferrer">
          Open in Execution
        </a>
      ) : null}

      <WorkstreamsAndMilestones engagement={engagement} />

      <div className="mt-2">
        <h5 className="text-xs font-semibold text-gray-700">Tasks</h5>
        {engagement.tasks.length === 0 ? (
          <p className="text-xs text-gray-500">No tasks assigned.</p>
        ) : (
          <ul className="mt-1 flex flex-col gap-2">
            {engagement.tasks.map((task) => (
              <li key={task.task_id} className="flex items-center justify-between gap-2 rounded border border-gray-100 p-2">
                <div>
                  <p className="text-xs font-medium text-gray-900">{task.title}</p>
                  <p className="text-xs text-gray-500">Status: {task.status}</p>
                </div>
                {/* Requests completion through the BFF back to Execution — see
                    `requestTaskCompletion`'s doc comment: this never marks
                    the task complete locally. Any real, confirmed completion
                    only ever shows up via the Action Queue card once
                    Execution's own confirmation event arrives. */}
                <Button
                  variant="secondary"
                  disabled={isRequestingCompletion && requestingTaskId === task.task_id}
                  onClick={() => onRequestCompletion(task.task_id)}
                >
                  {isRequestingCompletion && requestingTaskId === task.task_id ? 'Requesting…' : 'Request Completion'}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

function WorkstreamsAndMilestones({ engagement }: { engagement: EngagementSnapshot }) {
  return (
    <div className="mt-2 flex flex-col gap-2">
      <div>
        <h5 className="text-xs font-semibold text-gray-700">Workstreams</h5>
        {engagement.workstreams.length === 0 ? (
          <p className="text-xs text-gray-500">None.</p>
        ) : (
          <ul className="list-inside list-disc text-xs text-gray-700">
            {engagement.workstreams.map((workstream) => (
              <li key={workstream}>{workstream}</li>
            ))}
          </ul>
        )}
      </div>

      <div>
        <h5 className="text-xs font-semibold text-gray-700">Milestones</h5>
        {engagement.milestones.length === 0 ? (
          <p className="text-xs text-gray-500">None.</p>
        ) : (
          <ul className="list-inside list-disc text-xs text-gray-700">
            {engagement.milestones.map((milestone) => (
              <li key={milestone}>{milestone}</li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
