import { useState } from 'react'
import type { ChangeEvent, FormEvent } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Alert, TextInput } from '@cognitum/design-system'
import { CapabilityForm, ListDetailPanel } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-40: Landscape feature module — an intelligence digest card (read
 * approved items) plus an observation-submission form (write), following
 * `LearningDashboard.tsx`'s `useQuery` list pattern for the read half and
 * `LeadConflictCheck.tsx`'s `useMutation`-with-reset pattern for the write
 * half (`docs/SALES_FLOW_PATTERN.md` §4).
 *
 * Mirrors `crates/nexus-client/src/landscape.rs`'s `IntelligenceDigestItem`/
 * `FieldObservationSubmission` verbatim — `crates/bff-api/src/landscape.rs`
 * relays the digest unshaped, same convention as every other Phase 4 feature
 * module's read DTO.
 *
 * # No auto-retry on a failed submission (ADR-016)
 * `POST /api/landscape/observations` maps to
 * `LandscapeGateway::submit_field_observation`, a non-idempotent command —
 * per ADR-016 this repo never auto-retries it. TanStack Query's own default
 * for `useMutation` (unlike `useQuery`, which defaults to 3 retries) is no
 * retry at all, and this repo's shared `QueryClient` (`lib/queryClient.ts`,
 * provided at the root in `main.tsx`) sets no `mutations.retry` override, so
 * that default already covers this; a failed submission surfaces an error
 * alert and leaves the form's text intact so the consultant can consciously
 * re-click "Submit Observation" — a fresh, deliberate re-submission, never
 * an automatic one.
 */
export interface IntelligenceDigestItem {
  intel_id: string
  topic: string
  summary: string
  published_at: string
  deep_link: string | null
}

async function fetchIntelligenceDigest(): Promise<IntelligenceDigestItem[]> {
  const response = await fetch('/api/landscape/intelligence', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/landscape/intelligence failed: ${response.status}`)
  }

  return (await response.json()) as IntelligenceDigestItem[]
}

interface SubmitObservationInput {
  observationText: string
  relatedCompanyReference: string
}

/**
 * `POST /api/landscape/observations`. The BFF derives `submitted_by` from
 * the authenticated session — this request body never carries a consultant
 * id (`crates/bff-api/src/landscape.rs`'s module docs).
 */
async function submitObservation(input: SubmitObservationInput): Promise<void> {
  const response = await fetch('/api/landscape/observations', {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      observation_text: input.observationText,
      related_company_reference: input.relatedCompanyReference.trim().length > 0 ? input.relatedCompanyReference.trim() : undefined,
    }),
  })

  if (!response.ok) {
    throw new Error(`POST /api/landscape/observations failed: ${response.status}`)
  }
}

export function LandscapeWorkspace() {
  return (
    <div className="flex flex-col gap-4">
      <IntelligenceDigestCard />
      <ObservationSubmissionForm />
    </div>
  )
}

function IntelligenceDigestCard() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const digestQuery = useQuery({
    queryKey: queryKeys.landscape.digest(consultantId ?? ''),
    queryFn: fetchIntelligenceDigest,
    enabled: session.status === 'authenticated',
  })

  if (digestQuery.isPending) {
    return <p className="text-sm text-muted-foreground">Loading the intelligence digest…</p>
  }

  if (digestQuery.isError) {
    return <Alert variant="error">Failed to load the intelligence digest.</Alert>
  }

  const items = digestQuery.data ?? []

  return (
    <section>
      <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Intelligence Digest</h4>
      {items.length === 0 ? (
        <p className="text-xs text-muted-foreground">No approved intelligence items yet.</p>
      ) : (
        <ListDetailPanel
          items={items}
          getKey={(item) => item.intel_id}
          listClassName="mt-1 flex flex-col gap-2"
          renderRow={(item) => (
            <div className="rounded border border-border p-3">
              <div className="flex items-center justify-between gap-2">
                <p className="text-sm font-semibold text-foreground">{item.topic}</p>
                <span className="rounded bg-secondary px-2 py-0.5 text-xs text-card-foreground">
                  {new Date(item.published_at).toLocaleDateString()}
                </span>
              </div>
              <p className="mt-1 text-xs text-card-foreground">{item.summary}</p>
              {item.deep_link !== null ? (
                <a href={item.deep_link} className="text-xs text-primary hover:underline" target="_blank" rel="noreferrer">
                  Open in Landscape
                </a>
              ) : null}
            </div>
          )}
        />
      )}
    </section>
  )
}

const EMPTY_OBSERVATION_FIELDS: SubmitObservationInput = { observationText: '', relatedCompanyReference: '' }

function ObservationSubmissionForm() {
  const [fields, setFields] = useState<SubmitObservationInput>(EMPTY_OBSERVATION_FIELDS)

  const submitMutation = useMutation({
    mutationFn: submitObservation,
    onSuccess: () => {
      setFields(EMPTY_OBSERVATION_FIELDS)
    },
  })

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    // A fresh submission should never show a stale prior result while the
    // new one is in flight — same "reset before mutate" rule
    // `LeadConflictCheck.tsx`'s/`ProfileEditForm.tsx`'s `handleSubmit`
    // follow for ADR-015.
    submitMutation.reset()
    submitMutation.mutate(fields)
  }

  function handleFieldChange(key: keyof SubmitObservationInput) {
    return (event: ChangeEvent<HTMLInputElement>) => {
      setFields((current) => ({ ...current, [key]: event.target.value }))
    }
  }

  const alerts = [
    ...(submitMutation.isSuccess ? [{ variant: 'info' as const, message: 'Observation submitted.' }] : []),
    ...(submitMutation.isError
      ? [{ variant: 'error' as const, message: 'Failed to submit your observation. Please try again.' }]
      : []),
  ]

  return (
    <section>
      <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Submit a Field Observation</h4>

      <CapabilityForm
        alerts={alerts}
        onSubmit={handleSubmit}
        submitLabel="Submit Observation"
        pendingLabel="Submitting…"
        isPending={submitMutation.isPending}
        isSubmitDisabled={fields.observationText.trim().length === 0}
        className="mt-2 flex flex-col gap-3"
      >
        <TextInput
          label="Observation"
          value={fields.observationText}
          onChange={handleFieldChange('observationText')}
          required
        />
        <TextInput
          label="Related Company Reference (optional)"
          value={fields.relatedCompanyReference}
          onChange={handleFieldChange('relatedCompanyReference')}
        />
      </CapabilityForm>
    </section>
  )
}
