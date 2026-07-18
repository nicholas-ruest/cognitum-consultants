import { useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { TextInput } from '../../components/TextInput'
import { Button } from '../../components/Button'
import { Alert } from '../../components/Alert'
import type { AlertVariant } from '../../components/Alert'

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
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <TextInput
          label="Company Name"
          value={companyName}
          onChange={(event) => setCompanyName(event.target.value)}
          required
        />
        <Button type="submit" disabled={checkMutation.isPending}>
          {checkMutation.isPending ? 'Checking…' : 'Check for Conflicts'}
        </Button>
      </form>

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
