import { useQuery } from '@tanstack/react-query'
import { Alert } from '@cognitum/design-system'
import { ListDetailPanel } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-41: Legal feature module — a read-only approved-clause list,
 * following `ProductCatalog.tsx`'s `useQuery` list pattern
 * (`docs/SALES_FLOW_PATTERN.md` §4). Legal is a "pure read-only, conformist
 * relationship" (this unit's own governing ADR-007 note) — unlike
 * Landscape's digest-plus-submission-form shape, there is no write half
 * here to pair this with.
 *
 * Mirrors `crates/nexus-client/src/legal.rs`'s `ApprovedLegalSnippet`
 * verbatim — `crates/bff-api/src/legal.rs` relays it unshaped, same
 * convention as every other Phase 4 feature module's read DTO.
 *
 * # Primary integration point: Commit's proposal-review flow
 * `anti-corruption-layers.md` §9 frames Legal's inbound event as "mostly
 * relevant to Commit's proposal flow", and this unit's own prompt text
 * asks for approved clauses to be "displayed/available when editing"
 * a proposal. `ProposalWorkspace.tsx` renders this component with
 * `{ proposalId: proposal.proposal_id }` inside its proposal-detail view —
 * see that file for the integration. This component is also exported
 * standalone (not just as an internal detail of `ProposalWorkspace`) so a
 * future capability needing a topic-scoped lookup (`{ topic: '...' }`) can
 * reuse it without duplicating the fetch/render logic.
 */
export interface ApprovedLegalSnippet {
  clause_id: string
  title: string
  approved_text: string
  policy_reference: string
}

/**
 * `RequestApprovedClausesQuery { context: proposal_id | topic }`
 * (`anti-corruption-layers.md` §9) — an either/or, mirrored here as a
 * discriminated union rather than two optional fields, matching
 * `nexus_client::legal::ClauseContext`'s own "let the type structurally
 * forbid an invalid shape" reasoning.
 */
export type ClauseContext = { proposalId: string } | { topic: string }

function clauseContextQueryParam(context: ClauseContext): string {
  return 'proposalId' in context
    ? `proposal_id=${encodeURIComponent(context.proposalId)}`
    : `topic=${encodeURIComponent(context.topic)}`
}

/** Cache-key-safe string encoding of `context`, for `queryKeys.legal.clauses`'s scoping segment. */
function clauseContextCacheKey(context: ClauseContext): string {
  return 'proposalId' in context ? `proposal:${context.proposalId}` : `topic:${context.topic}`
}

async function fetchApprovedClauses(context: ClauseContext): Promise<ApprovedLegalSnippet[]> {
  const response = await fetch(`/api/legal/clauses?${clauseContextQueryParam(context)}`, { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/legal/clauses failed: ${response.status}`)
  }

  return (await response.json()) as ApprovedLegalSnippet[]
}

export interface ApprovedClausesProps {
  context: ClauseContext
}

export function ApprovedClauses({ context }: ApprovedClausesProps) {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const clausesQuery = useQuery({
    queryKey: queryKeys.legal.clauses(consultantId ?? '', clauseContextCacheKey(context)),
    queryFn: () => fetchApprovedClauses(context),
    enabled: session.status === 'authenticated',
  })

  if (clausesQuery.isPending) {
    return <p className="text-xs text-gray-500">Loading approved clauses…</p>
  }

  if (clausesQuery.isError) {
    return <Alert variant="error">Failed to load approved legal clauses.</Alert>
  }

  const clauses = clausesQuery.data ?? []

  return (
    <section>
      <h4 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Approved Legal Clauses</h4>
      {clauses.length === 0 ? (
        <p className="mt-1 text-xs text-gray-500">No approved clauses found.</p>
      ) : (
        <ListDetailPanel
          items={clauses}
          getKey={(clause) => clause.clause_id}
          listClassName="mt-1 flex flex-col gap-2"
          renderRow={(clause) => (
            <div className="rounded border border-gray-200 p-3">
              <div className="flex items-center justify-between gap-2">
                <p className="text-sm font-semibold text-gray-900">{clause.title}</p>
                <span className="rounded bg-gray-100 px-2 py-0.5 text-xs text-gray-700">{clause.policy_reference}</span>
              </div>
              <p className="mt-1 text-xs text-gray-700">{clause.approved_text}</p>
            </div>
          )}
        />
      )}
    </section>
  )
}
