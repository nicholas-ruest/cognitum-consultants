import { useQuery } from '@tanstack/react-query'
import { Alert } from '@cognitum/design-system'
import { ListDetailPanel } from '@cognitum/dashboard-components'
import { queryKeys } from '../../lib/queryKeys'
import { useSession } from '../../lib/SessionContext'

/**
 * PROMPT-39: Products catalog feature module, following
 * `CustomerContextList.tsx`'s `useQuery` list-plus-selected-detail pattern
 * (`docs/SALES_FLOW_PATTERN.md` §4) — `GET /api/products/catalog` is an
 * ordinary cacheable, re-fetchable resource (not a per-submission verdict
 * like Sales' conflict check), so this uses `useQuery`, not `useMutation`.
 *
 * Mirrors `crates/nexus-client/src/products.rs`'s `ProductReferenceCard`
 * verbatim — `crates/bff-api/src/products.rs` relays it unshaped, same
 * convention as Sales' `AccountClaimResult`, Commit's `ProposalSummary`, and
 * every other Phase 4 feature module's DTO.
 */
export interface ProductReferenceCard {
  product_id: string
  name: string
  packaging_summary: string
  pricing_guidance: string
  demo_assets: string[]
}

async function fetchProductCatalog(): Promise<ProductReferenceCard[]> {
  const response = await fetch('/api/products/catalog', { credentials: 'include' })

  if (!response.ok) {
    throw new Error(`GET /api/products/catalog failed: ${response.status}`)
  }

  return (await response.json()) as ProductReferenceCard[]
}

/**
 * Aggressive client-side caching (this unit's own explicit acceptance
 * criterion): the approved product catalog changes rarely
 * (`anti-corruption-layers.md` §7), and ADR-015 names `ProductReferenceCard`
 * data directly as the motivating example for tuning a query's
 * `staleTime`/`gcTime` more generously than TanStack Query's defaults (which
 * treat data stale immediately). 15 minutes is a placeholder — same
 * "generous but bounded, not yet tuned against real usage data" reasoning
 * every other untuned constant in this repo documents — chosen to be
 * meaningfully longer than the implicit zero-staleTime default every other
 * feature module in this repo relies on, without being so long a
 * `ProductCatalogUpdated`-triggered `invalidateQueries` (ADR-011/ADR-015's
 * SSE -> cache-invalidation path) would go unnoticed for an unreasonable
 * window. `gcTime` (how long an unused cache entry is kept before eviction)
 * is set even longer, since a rarely-changing catalog is exactly the case
 * where keeping a stale-but-still-useful cached copy around after the
 * consultant navigates away is a pure win, not a staleness risk.
 */
const PRODUCT_CATALOG_STALE_TIME_MS = 15 * 60 * 1000
const PRODUCT_CATALOG_GC_TIME_MS = 60 * 60 * 1000

export function ProductCatalog() {
  const session = useSession()
  const consultantId = session.status === 'authenticated' ? session.consultantId : undefined

  const catalogQuery = useQuery({
    queryKey: queryKeys.products.catalog(consultantId ?? ''),
    queryFn: fetchProductCatalog,
    enabled: session.status === 'authenticated',
    staleTime: PRODUCT_CATALOG_STALE_TIME_MS,
    gcTime: PRODUCT_CATALOG_GC_TIME_MS,
  })

  if (catalogQuery.isPending) {
    return <p className="text-sm text-gray-500">Loading the product catalog…</p>
  }

  if (catalogQuery.isError) {
    return <Alert variant="error">Failed to load the product catalog.</Alert>
  }

  const cards = catalogQuery.data ?? []

  if (cards.length === 0) {
    return <p className="text-xs text-gray-500">No approved products yet.</p>
  }

  return (
    <ListDetailPanel
      items={cards}
      getKey={(card) => card.product_id}
      renderRow={(card, { select }) => (
        <button
          type="button"
          onClick={select}
          className="w-full rounded border border-gray-200 p-3 text-left hover:bg-gray-50"
        >
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-semibold text-gray-900">{card.name}</p>
            <span className="rounded bg-gray-100 px-2 py-0.5 text-xs text-gray-700">{card.pricing_guidance}</span>
          </div>
        </button>
      )}
      renderDetail={(card) => <ProductReferenceDetail card={card} />}
    />
  )
}

interface ProductReferenceDetailProps {
  card: ProductReferenceCard
}

function ProductReferenceDetail({ card }: ProductReferenceDetailProps) {
  return (
    <div>
      <h4 className="text-sm font-semibold text-gray-900">{card.name}</h4>
      <p className="text-xs text-gray-500">Pricing: {card.pricing_guidance}</p>
      <p className="mt-1 text-xs text-gray-700">{card.packaging_summary}</p>

      {card.demo_assets.length > 0 ? (
        <ul className="mt-2 flex flex-col gap-1">
          {card.demo_assets.map((assetUrl) => (
            <li key={assetUrl}>
              <a href={assetUrl} className="text-xs text-blue-600 hover:underline" target="_blank" rel="noreferrer">
                View demo asset
              </a>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}
