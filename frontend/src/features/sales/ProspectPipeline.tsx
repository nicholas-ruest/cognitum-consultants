import { useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, Button, TextInput } from '@cognitum/design-system'
import { CapabilityForm } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * ADR-020 part A: the sales prospect pipeline — a Kanban-style,
 * stage-grouped board over `crates/bff-api/src/sales.rs`'s
 * `/sales/prospects*` routes. Follows `ProposalWorkspace.tsx`'s TanStack
 * Query conventions (`docs/SALES_FLOW_PATTERN.md` §4).
 *
 * # Columns, not drag-and-drop
 * Stage changes go through an explicit `<select>` per card (posted to
 * `/sales/prospects/{id}/stage`), not drag-and-drop between columns. This
 * still satisfies "see and change the status of prospects" — drag-and-drop
 * is a pure interaction-affordance upgrade on top of the same underlying
 * mutation, addable later without changing the data model or API calls.
 */

/** Mirrors `crates/bff-api/src/sales.rs`'s `ProspectStage::as_str` values, in funnel order. */
const PROSPECT_STAGES = [
  'contacted',
  'appointment_scheduled',
  'nda_sent',
  'nda_signed',
  'rfp_sent',
  'rfp_signed',
  'proposal_sent',
  'proposal_signed',
  'sow_sent',
  'closed_won',
  'closed_lost',
] as const

type ProspectStage = (typeof PROSPECT_STAGES)[number]

const STAGE_LABELS: Record<ProspectStage, string> = {
  contacted: 'Contacted',
  appointment_scheduled: 'Appointment Scheduled',
  nda_sent: 'NDA Sent',
  nda_signed: 'NDA Signed',
  rfp_sent: 'RFP Sent',
  rfp_signed: 'RFP Signed',
  proposal_sent: 'Proposal Sent',
  proposal_signed: 'Proposal Signed',
  sow_sent: 'SOW Sent',
  closed_won: 'Closed (Won)',
  closed_lost: 'Closed (Lost)',
}

/** Mirrors `crates/bff-api/src/sales.rs`'s `ProspectNoteDto`. */
export interface ProspectNote {
  id: string
  body: string
  author_consultant_id: string
  created_at: string
}

/** Mirrors `crates/bff-api/src/sales.rs`'s `ProspectDto`. */
export interface Prospect {
  id: string
  company_name: string
  contact_name: string | null
  stage: ProspectStage
  notes: ProspectNote[]
  created_at: string
  updated_at: string
}

async function fetchProspects(): Promise<Prospect[]> {
  const response = await fetch('/api/sales/prospects', { credentials: 'include' })
  if (!response.ok) {
    throw new Error(`GET /api/sales/prospects failed: ${response.status}`)
  }
  return (await response.json()) as Prospect[]
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

function createProspect(input: { companyName: string; contactName?: string }): Promise<Prospect> {
  const body: Record<string, string> = { company_name: input.companyName }
  if (input.contactName !== undefined && input.contactName.length > 0) body.contact_name = input.contactName
  return postJson<Prospect>('/api/sales/prospects', body)
}

function transitionStage(input: { prospectId: string; stage: ProspectStage }): Promise<Prospect> {
  return postJson<Prospect>(`/api/sales/prospects/${input.prospectId}/stage`, { stage: input.stage })
}

function addNote(input: { prospectId: string; body: string }): Promise<Prospect> {
  return postJson<Prospect>(`/api/sales/prospects/${input.prospectId}/notes`, { body: input.body })
}

export function ProspectPipeline() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined
  const queryClient = useQueryClient()

  const [companyName, setCompanyName] = useState('')
  const [contactName, setContactName] = useState('')

  const prospectsQuery = useQuery({
    queryKey: queryKeys.sales.prospects(consultantId ?? ''),
    queryFn: fetchProspects,
    enabled: session.status === 'authenticated',
  })

  function invalidateProspects() {
    if (consultantId !== undefined) {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sales.prospects(consultantId) })
    }
  }

  const createMutation = useMutation({ mutationFn: createProspect, onSuccess: invalidateProspects })
  const stageMutation = useMutation({ mutationFn: transitionStage, onSuccess: invalidateProspects })
  const noteMutation = useMutation({ mutationFn: addNote, onSuccess: invalidateProspects })

  function handleCreateSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmed = companyName.trim()
    if (trimmed.length === 0) return
    createMutation.mutate({ companyName: trimmed, contactName: contactName.trim() })
    setCompanyName('')
    setContactName('')
  }

  const prospects = prospectsQuery.data ?? []

  return (
    <div className="flex flex-col gap-4">
      <CapabilityForm
        alerts={createMutation.isError ? [{ variant: 'error', message: 'Failed to create prospect. Please try again.' }] : []}
        onSubmit={handleCreateSubmit}
        submitLabel="Add Prospect"
        pendingLabel="Adding…"
        isPending={createMutation.isPending}
        className="flex flex-col gap-2 sm:flex-row sm:items-end sm:gap-3"
      >
        <TextInput
          label="Prospect Company Name"
          value={companyName}
          onChange={(event) => setCompanyName(event.target.value)}
          placeholder="e.g. Acme Corp"
        />
        <TextInput
          label="Prospect Contact Name"
          value={contactName}
          onChange={(event) => setContactName(event.target.value)}
          placeholder="Optional"
        />
      </CapabilityForm>

      {prospectsQuery.isPending ? <p className="text-sm text-muted-foreground">Loading prospects…</p> : null}
      {prospectsQuery.isError ? <Alert variant="error">Failed to load your prospects.</Alert> : null}

      {!prospectsQuery.isPending && !prospectsQuery.isError ? (
        prospects.length === 0 ? (
          <p className="text-xs text-muted-foreground">No prospects yet — add one above to start tracking it.</p>
        ) : (
          <div className="flex gap-3 overflow-x-auto pb-2">
            {PROSPECT_STAGES.map((stage) => {
              const prospectsAtStage = prospects.filter((prospect) => prospect.stage === stage)
              if (prospectsAtStage.length === 0) return null
              return (
                <div key={stage} className="flex w-64 shrink-0 flex-col gap-2">
                  <h4 className="text-[0.6875rem] font-semibold uppercase tracking-wide text-muted-foreground">
                    {STAGE_LABELS[stage]} ({prospectsAtStage.length})
                  </h4>
                  <div className="flex flex-col gap-2">
                    {prospectsAtStage.map((prospect) => (
                      <ProspectCard
                        key={prospect.id}
                        prospect={prospect}
                        onStageChange={(newStage) => stageMutation.mutate({ prospectId: prospect.id, stage: newStage })}
                        onAddNote={(body) => noteMutation.mutate({ prospectId: prospect.id, body })}
                        isMutating={stageMutation.isPending || noteMutation.isPending}
                      />
                    ))}
                  </div>
                </div>
              )
            })}
          </div>
        )
      ) : null}
    </div>
  )
}

interface ProspectCardProps {
  prospect: Prospect
  onStageChange: (stage: ProspectStage) => void
  onAddNote: (body: string) => void
  isMutating: boolean
}

function ProspectCard({ prospect, onStageChange, onAddNote, isMutating }: ProspectCardProps) {
  const [noteBody, setNoteBody] = useState('')
  const latestNote = prospect.notes.length > 0 ? prospect.notes[prospect.notes.length - 1] : null

  function handleAddNote(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmed = noteBody.trim()
    if (trimmed.length === 0) return
    onAddNote(trimmed)
    setNoteBody('')
  }

  return (
    <div className="rounded-lg border border-border bg-card p-3">
      <p className="text-sm font-semibold text-foreground">{prospect.company_name}</p>
      {prospect.contact_name !== null ? <p className="text-xs text-muted-foreground">{prospect.contact_name}</p> : null}

      <select
        aria-label={`Stage for ${prospect.company_name}`}
        value={prospect.stage}
        disabled={isMutating}
        onChange={(event) => onStageChange(event.target.value as ProspectStage)}
        className="mt-2 w-full rounded border border-input bg-background px-2 py-1 text-xs text-foreground"
      >
        {PROSPECT_STAGES.map((stage) => (
          <option key={stage} value={stage}>
            {STAGE_LABELS[stage]}
          </option>
        ))}
      </select>

      {latestNote !== null ? (
        <p className="mt-2 line-clamp-2 text-xs text-muted-foreground">{latestNote.body}</p>
      ) : null}

      <form onSubmit={handleAddNote} className="mt-2 flex gap-1">
        <input
          aria-label={`Add a note for ${prospect.company_name}`}
          value={noteBody}
          onChange={(event) => setNoteBody(event.target.value)}
          placeholder="Add a note…"
          className="min-w-0 flex-1 rounded border border-input bg-background px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground"
        />
        <Button type="submit" variant="secondary" disabled={isMutating} className="px-2 py-1 text-xs">
          Add
        </Button>
      </form>
    </div>
  )
}
