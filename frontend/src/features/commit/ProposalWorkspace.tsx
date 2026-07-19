import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, Button, TextInput } from '@cognitum/design-system'
import { CapabilityForm, ListDetailPanel } from '@cognitum/dashboard-components'
import { ApprovedClauses } from '../legal/ApprovedClauses'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-34: Commit proposal-workspace feature module, following
 * `LeadConflictCheck.tsx`'s TanStack Query patterns
 * (`docs/SALES_FLOW_PATTERN.md` §4).
 *
 * Mirrors `crates/nexus-client/src/commit.rs`'s `ProposalSummary` verbatim
 * — `crates/bff-api/src/commit.rs` relays it unshaped, same convention as
 * Sales' `AccountClaimResult`.
 */
export interface ProposalSummary {
  proposal_id: string
  title: string
  status: string
  stage: string
  last_updated_at: string
  deep_link: string | null
}

/**
 * `useQuery`, not `useMutation`, for the proposal list — unlike Sales'
 * per-submission conflict check (ADR-015's explicit "never cache a verdict"
 * rule), "my current proposals" is an ordinary cacheable, re-fetchable
 * resource: the same shape as `useActionQueueQuery`/`useNotificationsQuery`,
 * not a one-shot check tied to a single form submission.
 */
async function fetchProposals(): Promise<ProposalSummary[]> {
  const response = await fetch('/api/commit/proposals', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/commit/proposals failed: ${response.status}`)
  }

  return (await response.json()) as ProposalSummary[]
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  const response = await fetch(url, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })

  if (!response.ok) {
    throw new Error(`POST ${url} failed: ${response.status}`)
  }

  return (await response.json()) as T
}

interface CreateProposalInput {
  originReference?: string
  originWorkflowSessionId?: string
}

/**
 * `POST /api/commit/proposals`. Either `originReference` (a manually
 * entered origin id) or `originWorkflowSessionId` (the Sales -> Commit deep
 * link, see [`consumeWorkflowSessionIdFromUrl`]) must be set — the BFF
 * itself enforces this and 400s otherwise
 * (`crates/bff-api/src/commit.rs`'s `resolve_origin_reference`).
 */
function createProposal(input: CreateProposalInput): Promise<ProposalSummary> {
  const body: Record<string, string> = {}
  if (input.originWorkflowSessionId !== undefined) body.origin_workflow_session_id = input.originWorkflowSessionId
  if (input.originReference !== undefined) body.origin_reference = input.originReference
  return postJson<ProposalSummary>('/api/commit/proposals', body)
}

function requestProposalAction(input: { proposalId: string; action: string }): Promise<unknown> {
  return postJson(`/api/commit/proposals/${input.proposalId}/actions`, { action: input.action })
}

const WORKFLOW_SESSION_QUERY_PARAM = 'workflow_session_id'

/**
 * Reads and strips the `?workflow_session_id=...` query param the Sales
 * deep link (`LeadConflictCheck.tsx`'s "Start Proposal in Commit"
 * affordance) appends to the URL before navigating here — see that file's
 * module docs and `crates/bff-api/src/workflow_sessions.rs` for the other
 * half of the hand-off. Stripped immediately (not just read) via
 * `history.replaceState` so a page refresh after the deep link has already
 * been consumed doesn't fire `create_proposal` a second time.
 */
function consumeWorkflowSessionIdFromUrl(): string | null {
  const url = new URL(window.location.href)
  const sessionId = url.searchParams.get(WORKFLOW_SESSION_QUERY_PARAM)
  if (sessionId === null) return null

  url.searchParams.delete(WORKFLOW_SESSION_QUERY_PARAM)
  window.history.replaceState({}, '', url.toString())
  return sessionId
}

/**
 * Fixed local action vocabulary, unlike Sales' `permitted_actions`: unlike
 * `AccountClaimResult`, `ProposalSummary` (`anti-corruption-layers.md` §2)
 * carries no server-supplied action-id list to drive rendering from — unlike
 * Sales' worked example, this doc doesn't name Commit's ack-response shape
 * for `RequestProposalActionCommand`'s `action` values either. This is a
 * deliberate, documented stand-in (same "no worked example to match, don't
 * invent DTO fields" reasoning `crate::sales`'s module docs used for its own
 * ack-response gap) — replace with a server-driven action list once Commit's
 * real contract defines one.
 */
const KNOWN_PROPOSAL_ACTIONS: ReadonlyArray<{ id: string; label: string }> = [
  { id: 'resend', label: 'Resend' },
  { id: 'request_revision', label: 'Request Revision' },
]

export function ProposalWorkspace() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined
  const queryClient = useQueryClient()

  const [selectedProposalId, setSelectedProposalId] = useState<string | null>(null)
  const [originReference, setOriginReference] = useState('')

  const proposalsQuery = useQuery({
    queryKey: queryKeys.commit.proposals(consultantId ?? ''),
    queryFn: fetchProposals,
    enabled: session.status === 'authenticated',
  })

  function invalidateProposals() {
    if (consultantId !== undefined) {
      void queryClient.invalidateQueries({ queryKey: queryKeys.commit.proposals(consultantId) })
    }
  }

  const createMutation = useMutation({
    mutationFn: createProposal,
    onSuccess: (proposal) => {
      invalidateProposals()
      setSelectedProposalId(proposal.proposal_id)
    },
  })

  const actionMutation = useMutation({
    mutationFn: requestProposalAction,
    onSuccess: invalidateProposals,
  })

  // Sales -> Commit deep link (PROMPT-34, docs/SALES_FLOW_PATTERN.md §4):
  // consume the query param exactly once per page load and, if present,
  // fire `create_proposal` with it. The BFF resolves the real
  // `origin_reference` server-side from the workflow session itself, so
  // nothing else is needed here — this genuinely creates a proposal, it
  // isn't a decorative redirect.
  useEffect(() => {
    if (session.status !== 'authenticated') return
    const workflowSessionId = consumeWorkflowSessionIdFromUrl()
    if (workflowSessionId !== null) {
      createMutation.mutate({ originWorkflowSessionId: workflowSessionId })
    }
    // Deliberately run only once `session` becomes authenticated — this
    // reads `window.location` directly, not `createMutation`, which is
    // recreated every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.status])

  function handleCreateSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const reference = originReference.trim()
    if (reference.length === 0) return
    createMutation.mutate({ originReference: reference })
    setOriginReference('')
  }

  const proposals = proposalsQuery.data ?? []

  if (proposalsQuery.isPending) {
    return <p className="text-sm text-muted-foreground">Loading proposals…</p>
  }

  if (proposalsQuery.isError) {
    return <Alert variant="error">Failed to load your proposals.</Alert>
  }

  return (
    <div className="flex flex-col gap-4">
      <CapabilityForm
        alerts={createMutation.isError ? [{ variant: 'error', message: 'Failed to start a new proposal. Please try again.' }] : []}
        onSubmit={handleCreateSubmit}
        submitLabel="Start Proposal"
        pendingLabel="Starting…"
        isPending={createMutation.isPending}
        className="flex flex-col gap-2"
      >
        <TextInput
          label="Origin Reference"
          value={originReference}
          onChange={(event) => setOriginReference(event.target.value)}
          placeholder="e.g. a Sales company/lead id"
        />
      </CapabilityForm>

      {proposals.length === 0 ? (
        <p className="text-xs text-muted-foreground">No proposals yet.</p>
      ) : (
        <ListDetailPanel
          items={proposals}
          getKey={(proposal) => proposal.proposal_id}
          selectedKey={selectedProposalId}
          onSelectedKeyChange={setSelectedProposalId}
          renderRow={(proposal, { select }) => (
            <button
              type="button"
              onClick={select}
              className="w-full rounded border border-border p-3 text-left hover:bg-secondary/60"
            >
              <div className="flex items-center justify-between gap-2">
                <p className="text-sm font-semibold text-foreground">{proposal.title}</p>
                <span className="rounded bg-secondary px-2 py-0.5 text-xs text-card-foreground">{proposal.status}</span>
              </div>
              <p className="text-xs text-muted-foreground">Stage: {proposal.stage}</p>
            </button>
          )}
          renderDetail={(proposal) => (
            <ProposalDetail
              proposal={proposal}
              onAction={(action) => actionMutation.mutate({ proposalId: proposal.proposal_id, action })}
              isActionPending={actionMutation.isPending}
            />
          )}
        />
      )}
    </div>
  )
}

interface ProposalDetailProps {
  proposal: ProposalSummary
  onAction: (action: string) => void
  isActionPending: boolean
}

function ProposalDetail({ proposal, onAction, isActionPending }: ProposalDetailProps) {
  return (
    <div>
      <h4 className="text-sm font-semibold text-foreground">{proposal.title}</h4>
      <p className="text-xs text-muted-foreground">
        Status: {proposal.status} · Stage: {proposal.stage}
      </p>
      <p className="text-xs text-muted-foreground">Last updated: {new Date(proposal.last_updated_at).toLocaleString()}</p>

      {proposal.deep_link !== null ? (
        <a href={proposal.deep_link} className="text-xs text-primary hover:underline" target="_blank" rel="noreferrer">
          Open in Commit
        </a>
      ) : null}

      <div className="mt-2 flex flex-wrap gap-2">
        {KNOWN_PROPOSAL_ACTIONS.map((action) => (
          <Button key={action.id} variant="secondary" disabled={isActionPending} onClick={() => onAction(action.id)}>
            {action.label}
          </Button>
        ))}
      </div>

      {/* PROMPT-41: approved legal clauses for this proposal, read-only —
          see `ApprovedClauses.tsx`'s module docs for why this is Legal's
          primary integration point in this repo. */}
      <div className="mt-3 border-t border-border pt-3">
        <ApprovedClauses context={{ proposalId: proposal.proposal_id }} />
      </div>
    </div>
  )
}
