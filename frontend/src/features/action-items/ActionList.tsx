import { useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert, Button, TextInput } from '@cognitum/design-system'
import { CapabilityForm } from '@cognitum/dashboard-components'
import { actionItemsQueryKey } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * ADR-020 part B: a consultant's own freeform checklist ("L10 type action
 * list") over `crates/bff-api/src/action_items.rs`'s `/api/action-items*`
 * routes — deliberately separate from `ActionQueue`/`ActionQueueEntry`
 * (Nexus-sourced, one-way completion) — see that crate's module docs.
 * Rendered as a third fixed "Overview" card in `DashboardPage`, alongside
 * `NotificationCentre`/`ActionQueue` — not capability-gated, same as those
 * two.
 */

/** Mirrors `crates/bff-api/src/action_items.rs`'s `ActionItemDto`. */
export interface ActionItem {
  id: string
  title: string
  notes: string | null
  done: boolean
  linked_prospect_id: string | null
  created_at: string
  updated_at: string
}

async function fetchActionItems(): Promise<ActionItem[]> {
  const response = await fetch('/api/action-items', { credentials: 'include' })
  if (!response.ok) {
    throw new Error(`GET /api/action-items failed: ${response.status}`)
  }
  return (await response.json()) as ActionItem[]
}

async function createActionItem(title: string): Promise<ActionItem> {
  const response = await fetch('/api/action-items', {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title }),
  })
  if (!response.ok) {
    throw new Error(`POST /api/action-items failed: ${response.status}`)
  }
  return (await response.json()) as ActionItem
}

async function setDone(input: { id: string; done: boolean }): Promise<ActionItem> {
  const response = await fetch(`/api/action-items/${input.id}`, {
    method: 'PATCH',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ done: input.done }),
  })
  if (!response.ok) {
    throw new Error(`PATCH /api/action-items/${input.id} failed: ${response.status}`)
  }
  return (await response.json()) as ActionItem
}

async function deleteActionItem(id: string): Promise<void> {
  const response = await fetch(`/api/action-items/${id}/delete`, { method: 'POST', credentials: 'include' })
  if (!response.ok) {
    throw new Error(`POST /api/action-items/${id}/delete failed: ${response.status}`)
  }
}

export function ActionList() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined
  const queryClient = useQueryClient()
  const [title, setTitle] = useState('')

  const itemsQuery = useQuery({
    queryKey: actionItemsQueryKey(consultantId ?? ''),
    queryFn: fetchActionItems,
    enabled: session.status === 'authenticated',
  })

  function invalidate() {
    if (consultantId !== undefined) {
      void queryClient.invalidateQueries({ queryKey: actionItemsQueryKey(consultantId) })
    }
  }

  const createMutation = useMutation({ mutationFn: createActionItem, onSuccess: invalidate })
  const toggleMutation = useMutation({ mutationFn: setDone, onSuccess: invalidate })
  const deleteMutation = useMutation({ mutationFn: deleteActionItem, onSuccess: invalidate })

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const trimmed = title.trim()
    if (trimmed.length === 0) return
    createMutation.mutate(trimmed)
    setTitle('')
  }

  const items = itemsQuery.data ?? []

  return (
    <div className="flex flex-col gap-3">
      <CapabilityForm
        alerts={createMutation.isError ? [{ variant: 'error', message: 'Failed to add item. Please try again.' }] : []}
        onSubmit={handleSubmit}
        submitLabel="Add"
        pendingLabel="Adding…"
        isPending={createMutation.isPending}
        className="flex items-end gap-2"
      >
        <TextInput label="New item" value={title} onChange={(event) => setTitle(event.target.value)} placeholder="e.g. Call Acme back" />
      </CapabilityForm>

      {itemsQuery.isPending ? <p className="text-sm text-muted-foreground">Loading…</p> : null}
      {itemsQuery.isError ? <Alert variant="error">Failed to load your action list.</Alert> : null}

      {!itemsQuery.isPending && !itemsQuery.isError ? (
        items.length === 0 ? (
          <p className="text-xs text-muted-foreground">Nothing on your list yet.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {items.map((item) => (
              <li key={item.id} className="flex items-center gap-2 rounded border border-border px-2 py-1.5">
                <input
                  type="checkbox"
                  checked={item.done}
                  aria-label={`Mark "${item.title}" ${item.done ? 'not done' : 'done'}`}
                  disabled={toggleMutation.isPending}
                  onChange={(event) => toggleMutation.mutate({ id: item.id, done: event.target.checked })}
                />
                <span className={`flex-1 text-sm ${item.done ? 'text-muted-foreground line-through' : 'text-foreground'}`}>
                  {item.title}
                </span>
                <Button
                  variant="secondary"
                  disabled={deleteMutation.isPending}
                  onClick={() => deleteMutation.mutate(item.id)}
                  className="px-2 py-1 text-xs"
                >
                  Remove
                </Button>
              </li>
            ))}
          </ul>
        )
      ) : null}
    </div>
  )
}
