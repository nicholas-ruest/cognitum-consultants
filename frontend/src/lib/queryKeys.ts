/**
 * Query-key convention for TanStack Query (ADR-015).
 *
 * Every `/api/*` call made through `useQuery`/`useMutation` must build its
 * key through this module rather than hand-writing array literals. Keys are
 * namespaced by **capability** (mirroring the nine `frontend/src/features/`
 * directories, which themselves mirror `domain-map.md`'s bounded contexts)
 * and then by **consultant**, e.g. `['sales', 'conflicts', consultantId]` —
 * so a capability's whole cache slice can be invalidated in one call
 * (`queryClient.invalidateQueries({ queryKey: queryKeys.sales.all })`),
 * which is exactly the mechanism ADR-015 wires up to ADR-011's SSE-pushed
 * updates.
 *
 * Only `sales` and `commit` have concrete example keys below
 * (`conflicts`/`proposals`) — those mirror the actual reference flow in
 * `../ddd/anti-corruption-layers.md` §1 and the literal examples in
 * PROMPT-16/ADR-015. Real backend routes for Sales don't land until
 * PROMPT-24+, so even these are illustrative today, not wired to a live
 * endpoint.
 *
 * The other seven capabilities (`edu`, `capacity`, `customer`, `execution`,
 * `products`, `landscape`, `legal`) have no routes and no settled resource
 * shape yet (PROMPT-34+) — inventing named keys for them now would fabricate
 * business meaning that doesn't exist. Instead each exposes `all` (for
 * capability-wide invalidation) and a generic `resource()` builder that
 * follows the same [capability, resource, consultantId, ...rest] shape.
 * Once a capability's real routes land, replace its `resource()` calls with
 * named, typed methods the same way `sales.conflicts` and `commit.proposals`
 * are done, and keep `resource()` around only if it's still useful for
 * ad hoc/rare lookups.
 */

export const CAPABILITIES = [
  'sales',
  'commit',
  'edu',
  'capacity',
  'customer',
  'execution',
  'products',
  'landscape',
  'legal',
] as const

export type Capability = (typeof CAPABILITIES)[number]

/**
 * The enforced shape every capability-scoped query key must follow:
 * `[capability, resource, consultantId, ...rest]`. `rest` covers keys that
 * need extra scoping beyond consultant (e.g. ADR-015's
 * `['sales', 'account-claim', consultantId, companyRef]`).
 */
export function capabilityKey<C extends Capability>(
  capability: C,
  resource: string,
  consultantId: string,
  ...rest: readonly string[]
) {
  return [capability, resource, consultantId, ...rest] as const
}

/** Root key for a capability, for invalidating its entire cache slice. */
function capabilityRoot<C extends Capability>(capability: C) {
  return [capability] as const
}

/** Generic per-capability builder for capabilities with no settled routes yet. */
function genericResource<C extends Capability>(capability: C) {
  return (resource: string, consultantId: string, ...rest: readonly string[]) =>
    capabilityKey(capability, resource, consultantId, ...rest)
}

export const queryKeys = {
  sales: {
    all: capabilityRoot('sales'),
    /** Illustrative — mirrors ADR-015's example; no live route until PROMPT-24+. */
    conflicts: (consultantId: string) => capabilityKey('sales', 'conflicts', consultantId),
  },
  commit: {
    all: capabilityRoot('commit'),
    /** Illustrative — mirrors PROMPT-16's example; no live route until PROMPT-34+. */
    proposals: (consultantId: string) => capabilityKey('commit', 'proposals', consultantId),
  },
  edu: {
    all: capabilityRoot('edu'),
    resource: genericResource('edu'),
  },
  capacity: {
    all: capabilityRoot('capacity'),
    resource: genericResource('capacity'),
  },
  customer: {
    all: capabilityRoot('customer'),
    resource: genericResource('customer'),
  },
  execution: {
    all: capabilityRoot('execution'),
    resource: genericResource('execution'),
  },
  products: {
    all: capabilityRoot('products'),
    resource: genericResource('products'),
  },
  landscape: {
    all: capabilityRoot('landscape'),
    resource: genericResource('landscape'),
  },
  legal: {
    all: capabilityRoot('legal'),
    resource: genericResource('legal'),
  },
} as const
