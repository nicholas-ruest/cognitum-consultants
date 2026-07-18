import { useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { TextInput, Button, Alert } from '@cognitum/design-system'
import type { AlertVariant } from '@cognitum/design-system'
import { CapabilityForm } from '@cognitum/dashboard-components'

/**
 * PROMPT-26: Sales lead-conflict-warning flow, the Phase 2 reference
 * feature module (`../../../.plans/ddd/anti-corruption-layers.md` §1
 * worked example).
 *
 * Mirrors `crates/bff-api/src/sales.rs`'s `AccountClaimResult` verbatim —
 * this repo never re-derives or overrides `creation_allowed`
 * (`anti-corruption-layers.md` §1 step 5); it only renders what the BFF
 * relayed from Sales.
 */
export interface AccountClaimResult {
  match_status: string
  creation_allowed: boolean
  display_message: string
  permitted_actions: string[]
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

function checkLeadConflict(companyName: string): Promise<AccountClaimResult> {
  return postJson<AccountClaimResult>('/api/sales/lead-conflict-check', { company_name: companyName })
}

// PROMPT-26 provisional stand-in: `AccountClaimResult` (as defined in
// `anti-corruption-layers.md` §1 and mirrored by the BFF's DTO) carries no
// company reference/id field — only `match_status`/`creation_allowed`/
// `display_message`/`permitted_actions`. `request-collaboration` and
// `submit-referral` both require a `company_reference` per
// `crates/bff-api/src/sales.rs`'s request DTOs. Until Nexus's real contract
// defines an opaque `company_reference` for this flow, the company name the
// consultant typed is used as that reference. This is a deliberate,
// documented stand-in — not a fabricated id — and should be replaced with
// the real opaque reference once Sales/Nexus's contract provides one.
function requestCollaboration(companyReference: string): Promise<unknown> {
  return postJson('/api/sales/request-collaboration', { company_reference: companyReference })
}

function submitReferral(companyReference: string): Promise<unknown> {
  return postJson('/api/sales/submit-referral', { company_reference: companyReference })
}

/**
 * PROMPT-34: Sales -> Commit deep link. `POST /api/workflow-sessions`
 * response shape, mirrored from `crates/bff-api/src/workflow_sessions.rs`'s
 * `WorkflowSessionResponse`.
 */
interface WorkflowSessionResponse {
  session_id: string
  status: string
  expires_at: string
}

/**
 * Starts a `CrossCapabilityWorkflowSession` (`origin_capability: "sales"`,
 * `target_capability: "commit"`) hand-off. This is deliberately **not**
 * part of Sales' own `permitted_actions` vocabulary (`AccountClaimResult`
 * carries no such action id) — it's this module's own added affordance for
 * the "no conflict, consider starting a proposal" moment, per
 * `docs/SALES_FLOW_PATTERN.md` §4 / PROMPT-34's acceptance criteria. See
 * `frontend/src/features/commit/ProposalWorkspace.tsx` for the other half
 * of the hand-off (consuming the resulting `?workflow_session_id=...`).
 */
function startWorkflowSessionToCommit(companyReference: string): Promise<WorkflowSessionResponse> {
  return postJson<WorkflowSessionResponse>('/api/workflow-sessions', {
    origin_capability: 'sales',
    origin_reference: companyReference,
    target_capability: 'commit',
  })
}

/**
 * `creation_allowed` -> `Alert` variant mapping (the prompt leaves the
 * exact variant to this unit's judgment):
 * - `false` -> `warning`: a conflict exists and the consultant must choose
 *   one of the offered `permitted_actions` rather than proceed freely. Not
 *   `error` — this is an expected, recoverable business outcome (Sales
 *   found an existing owner), not a failure of the check itself.
 * - `true` -> `info`: no conflict; a neutral, non-alarming status message.
 */
function alertVariantFor(creationAllowed: boolean): AlertVariant {
  return creationAllowed ? 'info' : 'warning'
}

/** Known `permitted_actions` ids, per `anti-corruption-layers.md` §1's worked example. */
const KNOWN_ACTION_LABELS: Record<string, string> = {
  request_collaboration: 'Request Collaboration',
  submit_referral: 'Submit Referral',
  cancel: 'Cancel',
}

/**
 * Turns an unrecognized `permitted_actions` entry into a human label
 * (`"some_new_action"` -> `"Some New Action"`) instead of crashing or
 * silently dropping it. Defensive-by-construction: Sales/Nexus may add new
 * action ids over time, and hiding a permitted action from the consultant
 * would be worse than rendering a generically-labeled, inert button for it.
 */
function humanizeActionId(actionId: string): string {
  return actionId
    .split('_')
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ')
}

export function LeadConflictCheck() {
  const [companyName, setCompanyName] = useState('')

  const checkMutation = useMutation({ mutationFn: checkLeadConflict })
  const collaborationMutation = useMutation({ mutationFn: requestCollaboration })
  const referralMutation = useMutation({ mutationFn: submitReferral })
  // PROMPT-34 deep link: on success, navigate to the dashboard with the new
  // session's id as a query param — `ProposalWorkspace.tsx` consumes it and
  // fires the actual `create_proposal` call. A full navigation (not a
  // client-side router push, since this app has none — see `App.tsx`'s
  // doc comment) is safe here: the session cookie persists across it.
  const startProposalMutation = useMutation({
    mutationFn: startWorkflowSessionToCommit,
    onSuccess: (workflowSession) => {
      window.location.assign(`/?workflow_session_id=${encodeURIComponent(workflowSession.session_id)}`)
    },
  })

  // The reference used for follow-up commands must match the company the
  // rendered result actually pertains to, not whatever is currently typed
  // in the input (the consultant may have started editing again). TanStack
  // Query's `variables` on a mutation always reflects the arguments of its
  // most recent `mutate()` call, so it stays correctly paired with `data`.
  const companyReference = checkMutation.variables ?? companyName

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    // ADR-015: a conflict-check result must never be reused across a
    // different company entry. `reset()` clears any previous result/error
    // back to idle *before* `mutate()` fires the new check, so a stale
    // result can never linger (or be mistaken for the new company's
    // result) while the new request is in flight.
    checkMutation.reset()
    checkMutation.mutate(companyName)
  }

  function handleCancel() {
    setCompanyName('')
    checkMutation.reset()
  }

  const result = checkMutation.data

  return (
    <div className="flex flex-col gap-4">
      <CapabilityForm
        onSubmit={handleSubmit}
        submitLabel="Check for Conflicts"
        pendingLabel="Checking…"
        isPending={checkMutation.isPending}
      >
        <TextInput
          label="Company Name"
          value={companyName}
          onChange={(event) => setCompanyName(event.target.value)}
          required
        />
      </CapabilityForm>

      {checkMutation.isError ? (
        <Alert variant="error">Failed to check this company. Please try again.</Alert>
      ) : null}

      {result ? (
        <div className="flex flex-col gap-3">
          <Alert variant={alertVariantFor(result.creation_allowed)}>{result.display_message}</Alert>

          {result.permitted_actions.length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {result.permitted_actions.map((actionId) => (
                <ActionButton
                  key={actionId}
                  actionId={actionId}
                  companyReference={companyReference}
                  onCancel={handleCancel}
                  collaborationMutation={collaborationMutation}
                  referralMutation={referralMutation}
                />
              ))}
            </div>
          ) : null}

          {/* PROMPT-34 Sales -> Commit deep link: rendered on the
              `creation_allowed: true` success path, since that's when
              starting a *new* proposal makes sense — not on the
              already-owned conflict path, where `permitted_actions`
              (request_collaboration/submit_referral/cancel) already covers
              what the consultant can do. Not part of `permitted_actions` —
              see `startWorkflowSessionToCommit`'s doc comment. */}
          {result.creation_allowed ? (
            <div>
              <Button
                variant="secondary"
                disabled={startProposalMutation.isPending}
                onClick={() => startProposalMutation.mutate(companyReference)}
              >
                {startProposalMutation.isPending ? 'Starting…' : 'Start Proposal in Commit'}
              </Button>
              {startProposalMutation.isError ? (
                <p className="mt-1 text-sm text-red-600">Failed to start a proposal. Please try again.</p>
              ) : null}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

interface ActionButtonProps {
  actionId: string
  companyReference: string
  onCancel: () => void
  collaborationMutation: ReturnType<typeof useMutation<unknown, Error, string>>
  referralMutation: ReturnType<typeof useMutation<unknown, Error, string>>
}

/**
 * Maps one `permitted_actions` entry to a label + click behavior. Never
 * hardcodes *which* actions appear — the caller maps over
 * `result.permitted_actions` and renders one `ActionButton` per entry.
 */
function ActionButton({ actionId, companyReference, onCancel, collaborationMutation, referralMutation }: ActionButtonProps) {
  switch (actionId) {
    case 'request_collaboration':
      return (
        <Button
          variant="secondary"
          disabled={collaborationMutation.isPending}
          onClick={() => collaborationMutation.mutate(companyReference)}
        >
          {KNOWN_ACTION_LABELS.request_collaboration}
        </Button>
      )
    case 'submit_referral':
      return (
        <Button
          variant="secondary"
          disabled={referralMutation.isPending}
          onClick={() => referralMutation.mutate(companyReference)}
        >
          {KNOWN_ACTION_LABELS.submit_referral}
        </Button>
      )
    case 'cancel':
      return (
        <Button variant="secondary" onClick={onCancel}>
          {KNOWN_ACTION_LABELS.cancel}
        </Button>
      )
    default:
      // Unrecognized action id: render generically rather than crash or
      // drop it. See `humanizeActionId`'s doc comment.
      return (
        <Button variant="secondary" disabled>
          {humanizeActionId(actionId)}
        </Button>
      )
  }
}
