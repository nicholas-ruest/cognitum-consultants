import { useQuery } from '@tanstack/react-query'
import { Alert } from '../../components/Alert'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-35: Edu learning-dashboard feature module, following
 * `ProposalWorkspace.tsx`'s `useQuery` patterns (`docs/SALES_FLOW_PATTERN.md`
 * §4) — `GET /api/edu/catalog` is an ordinary cacheable, re-fetchable
 * resource (not a per-submission verdict like Sales' conflict check), so
 * this uses `useQuery`, not `useMutation`, the same choice
 * `ProposalWorkspace.tsx`'s `fetchProposals` makes for the same reason.
 *
 * Mirrors `crates/nexus-client/src/edu.rs`'s `LearningSnapshot` verbatim —
 * `crates/bff-api/src/edu.rs` relays it unshaped, same convention as
 * Sales' `AccountClaimResult` and Commit's `ProposalSummary`.
 */
export interface LearningSnapshot {
  course_id: string
  title: string
  progress_status: string
  certification_status: string | null
  deep_link: string | null
}

async function fetchLearningCatalog(): Promise<LearningSnapshot[]> {
  const response = await fetch('/api/edu/catalog', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/edu/catalog failed: ${response.status}`)
  }

  return (await response.json()) as LearningSnapshot[]
}

/**
 * Which `progress_status` values are treated as "training due" for the
 * section below. **Provisional, documented assumption**: `LearningSnapshot`
 * (`anti-corruption-layers.md` §3) carries no field naming "this is a
 * training requirement", only `progress_status`/`certification_status` per
 * course — same "no worked example to match, don't invent DTO fields, but
 * must render *something* reasonable" reasoning `crate::sales`'s module
 * docs used for its own ack-response gap, and `ProposalWorkspace.tsx`'s
 * `KNOWN_PROPOSAL_ACTIONS` used for its own fixed local vocabulary. Update
 * once Edu's real contract names a `progress_status` vocabulary.
 */
const TRAINING_DUE_PROGRESS_STATUSES = new Set(['not_started', 'overdue'])

function isTrainingDue(item: LearningSnapshot): boolean {
  return TRAINING_DUE_PROGRESS_STATUSES.has(item.progress_status)
}

/**
 * `item.certification_status` truthy-checked, not `!== null`-checked: the
 * backend DTO (`crates/nexus-client/src/edu.rs`'s `LearningSnapshot`)
 * derives `#[serde(skip_serializing_if = "Option::is_none")]` on this
 * field, so a `None` value is **omitted from the JSON entirely**, not sent
 * as an explicit `null` — the wire value this component actually observes
 * for "no certification" is `undefined`, not `null`. A strict `!== null`
 * check would treat that omitted key as present and crash on
 * `.length` (exactly the failure mode this doc comment now protects
 * against — see this file's history). Same reasoning applies to
 * `deep_link`'s render check below.
 */
function hasCertification(item: LearningSnapshot): boolean {
  return typeof item.certification_status === 'string' && item.certification_status.length > 0
}

export function LearningDashboard() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const catalogQuery = useQuery({
    queryKey: queryKeys.edu.catalog(consultantId ?? ''),
    queryFn: fetchLearningCatalog,
    enabled: session.status === 'authenticated',
  })

  if (catalogQuery.isPending) {
    return <p className="text-sm text-gray-500">Loading your learning catalog…</p>
  }

  if (catalogQuery.isError) {
    return <Alert variant="error">Failed to load your learning catalog.</Alert>
  }

  const snapshots = catalogQuery.data ?? []
  const certifications = snapshots.filter(hasCertification)
  const trainingDue = snapshots.filter(isTrainingDue)

  if (snapshots.length === 0) {
    return <p className="text-xs text-gray-500">No courses yet.</p>
  }

  return (
    <div className="flex flex-col gap-4">
      <LearningSection title="Courses" items={snapshots} emptyMessage="No courses yet." />
      <LearningSection title="Certifications" items={certifications} emptyMessage="No certifications yet." />
      <LearningSection title="Training Due" items={trainingDue} emptyMessage="Nothing due right now." />
    </div>
  )
}

interface LearningSectionProps {
  title: string
  items: LearningSnapshot[]
  emptyMessage: string
}

function LearningSection({ title, items, emptyMessage }: LearningSectionProps) {
  return (
    <section>
      <h4 className="text-xs font-semibold uppercase tracking-wide text-gray-500">{title}</h4>
      {items.length === 0 ? (
        <p className="text-xs text-gray-500">{emptyMessage}</p>
      ) : (
        <ul className="mt-1 flex flex-col gap-2">
          {items.map((item) => (
            <LearningSnapshotRow key={`${title}-${item.course_id}`} item={item} />
          ))}
        </ul>
      )}
    </section>
  )
}

function LearningSnapshotRow({ item }: { item: LearningSnapshot }) {
  return (
    <li className="rounded border border-gray-200 p-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm font-semibold text-gray-900">{item.title}</p>
        <span className="rounded bg-gray-100 px-2 py-0.5 text-xs text-gray-700">{item.progress_status}</span>
      </div>
      {/* Truthy-checked, not `!== null`-checked — see `hasCertification`'s
          doc comment above for why. */}
      {item.certification_status ? (
        <p className="text-xs text-gray-500">Certification: {item.certification_status}</p>
      ) : null}
      {item.deep_link ? (
        <a href={item.deep_link} className="text-xs text-blue-600 hover:underline" target="_blank" rel="noreferrer">
          Open in Edu
        </a>
      ) : null}
    </li>
  )
}
