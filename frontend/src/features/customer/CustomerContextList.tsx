import { useQuery } from '@tanstack/react-query'
import { Alert } from '@cognitum/design-system'
import { ListDetailPanel } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-37: Customer context-card feature module, following
 * `LearningDashboard.tsx`'s `useQuery` list pattern
 * (`docs/SALES_FLOW_PATTERN.md` §4) with `ProposalWorkspace.tsx`'s
 * list-plus-selected-detail shape — `GET /api/customer/assigned` is an
 * ordinary cacheable, re-fetchable resource (not a per-submission verdict
 * like Sales' conflict check), so this uses `useQuery`, not `useMutation`.
 *
 * Mirrors `crates/nexus-client/src/customer.rs`'s `CustomerContextCard`
 * verbatim — `crates/bff-api/src/customer.rs` relays it unshaped, same
 * convention as Sales' `AccountClaimResult`, Commit's `ProposalSummary`, and
 * Edu's `LearningSnapshot`.
 */
export interface CustomerContextCard {
  customer_id: string
  name: string
  health_status: string
  relationship_summary: string
  deep_link: string | null
}

async function fetchAssignedCustomers(): Promise<CustomerContextCard[]> {
  const response = await fetch('/api/customer/assigned', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/customer/assigned failed: ${response.status}`)
  }

  return (await response.json()) as CustomerContextCard[]
}

/**
 * `health_status` badge color, purely a rendering choice — the underlying
 * text (`display`-verbatim, never re-derived) is always `health_status`
 * itself, this only maps it to a visual accent. **Provisional, documented
 * assumption**: `CustomerContextCard` (`anti-corruption-layers.md` §5)
 * names no fixed `health_status` vocabulary, so this recognizes the
 * conventional traffic-light values and falls back to a neutral style for
 * anything else — same "no worked example to match, don't invent DTO
 * fields, but must render *something* reasonable" reasoning
 * `LearningDashboard.tsx`'s `TRAINING_DUE_PROGRESS_STATUSES` used for its
 * own provisional vocabulary.
 */
const HEALTH_BADGE_CLASSES: Record<string, string> = {
  green: 'bg-accent/10 text-[hsl(142_70%_65%)]',
  yellow: 'bg-warning/10 text-[hsl(35_85%_70%)]',
  red: 'bg-destructive/10 text-[hsl(0_70%_70%)]',
}

function healthBadgeClass(healthStatus: string): string {
  return HEALTH_BADGE_CLASSES[healthStatus.toLowerCase()] ?? 'bg-secondary text-card-foreground'
}

export function CustomerContextList() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const contextsQuery = useQuery({
    queryKey: queryKeys.customer.assigned(consultantId ?? ''),
    queryFn: fetchAssignedCustomers,
    enabled: session.status === 'authenticated',
  })

  if (contextsQuery.isPending) {
    return <p className="text-sm text-muted-foreground">Loading your assigned customers…</p>
  }

  if (contextsQuery.isError) {
    return <Alert variant="error">Failed to load your assigned customers.</Alert>
  }

  const contexts = contextsQuery.data ?? []

  if (contexts.length === 0) {
    return <p className="text-xs text-muted-foreground">No assigned customers yet.</p>
  }

  return (
    <ListDetailPanel
      items={contexts}
      getKey={(context) => context.customer_id}
      renderRow={(context, { select }) => (
        <button
          type="button"
          onClick={select}
          className="w-full rounded border border-border p-3 text-left hover:bg-secondary/60"
        >
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-semibold text-foreground">{context.name}</p>
            <span className={`rounded px-2 py-0.5 text-xs ${healthBadgeClass(context.health_status)}`}>
              {context.health_status}
            </span>
          </div>
        </button>
      )}
      renderDetail={(context) => <CustomerContextDetail context={context} />}
    />
  )
}

interface CustomerContextDetailProps {
  context: CustomerContextCard
}

function CustomerContextDetail({ context }: CustomerContextDetailProps) {
  return (
    <div>
      <h4 className="text-sm font-semibold text-foreground">{context.name}</h4>
      <p className="text-xs text-muted-foreground">
        Health: <span className={`rounded px-2 py-0.5 ${healthBadgeClass(context.health_status)}`}>{context.health_status}</span>
      </p>
      <p className="mt-1 text-xs text-card-foreground">{context.relationship_summary}</p>

      {context.deep_link !== null ? (
        <a href={context.deep_link} className="text-xs text-primary hover:underline" target="_blank" rel="noreferrer">
          Open in Customer
        </a>
      ) : null}
    </div>
  )
}
