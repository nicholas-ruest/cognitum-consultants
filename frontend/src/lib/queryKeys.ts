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
 * `sales`, `commit`, `edu`, `capacity`, `customer`, `execution`, `products`,
 * `landscape`, and `legal` all have concrete example keys below
 * (`conflicts`/`proposals`/`catalog`/`profile`/`assigned`/`engagements`/
 * `catalog`/`digest`/`clauses`) — those mirror the actual reference flow in
 * `../ddd/anti-corruption-layers.md` §1–§9 and the literal examples in
 * PROMPT-16/ADR-015.
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
    /** `GET /api/edu/catalog` (PROMPT-35) query key. */
    catalog: (consultantId: string) => capabilityKey('edu', 'catalog', consultantId),
  },
  capacity: {
    all: capabilityRoot('capacity'),
    /** `GET`/`PATCH /api/capacity/profile` (PROMPT-36) query key. */
    profile: (consultantId: string) => capabilityKey('capacity', 'profile', consultantId),
  },
  customer: {
    all: capabilityRoot('customer'),
    /** `GET /api/customer/assigned` (PROMPT-37) query key. */
    assigned: (consultantId: string) => capabilityKey('customer', 'assigned', consultantId),
  },
  execution: {
    all: capabilityRoot('execution'),
    /** `GET /api/execution/engagements` (PROMPT-38) query key. */
    engagements: (consultantId: string) => capabilityKey('execution', 'engagements', consultantId),
  },
  products: {
    all: capabilityRoot('products'),
    /** `GET /api/products/catalog` (PROMPT-39) query key. */
    catalog: (consultantId: string) => capabilityKey('products', 'catalog', consultantId),
  },
  landscape: {
    all: capabilityRoot('landscape'),
    /** `GET /api/landscape/intelligence` (PROMPT-40) query key. */
    digest: (consultantId: string) => capabilityKey('landscape', 'digest', consultantId),
  },
  legal: {
    all: capabilityRoot('legal'),
    /**
     * `GET /api/legal/clauses` (PROMPT-41) query key. Scoped by `context`
     * beyond `consultantId` (e.g. `proposal:proposal-1` or
     * `topic:data-residency`) — unlike every other capability's single,
     * page-load-scoped read above, a consultant can look up clauses for
     * many different proposals/topics across a session, so a bare
     * `[legal, clauses, consultantId]` key would incorrectly collapse
     * distinct lookups into one cache entry.
     */
    clauses: (consultantId: string, context: string) => capabilityKey('legal', 'clauses', consultantId, context),
  },
} as const

/**
 * `GET`/`PUT /api/dashboard` (PROMPT-23) query key: `['dashboard', consultantId]`.
 *
 * Deliberately **not** routed through `capabilityKey`/the `queryKeys`
 * capability namespace above. `DashboardConfiguration` is this repo's own
 * aggregate (`consultant-experience-context.md` §1.2) — it composes
 * *across* capabilities (a dashboard's cards can reference `sales`,
 * `commit`, etc. all at once), it isn't itself one capability's data. That
 * makes it structurally the same case as `useSessionQuery.ts`'s bare
 * `['session']` key (see that file's comment): a cross-cutting BFF
 * aggregate, not a `features/<capability>` resource, so forcing it under
 * one arbitrary capability slot would misrepresent what it is and would
 * make `queryKeys.sales.all`-style capability-wide invalidation
 * (incorrectly) sweep up dashboard state too. Unlike `['session']` (which
 * has no per-consultant variants to distinguish — there is only ever "the
 * current session"), the dashboard cache is still explicitly scoped by
 * `consultantId`, since a consultant's dashboard is meaningfully
 * per-consultant data.
 */
export function dashboardQueryKey(consultantId: string) {
  return ['dashboard', consultantId] as const
}

/**
 * `GET /api/notifications` (PROMPT-33) query key: `['notifications', consultantId]`.
 *
 * Same "cross-cutting BFF aggregate, not a `features/<capability>` resource"
 * rationale as {@link dashboardQueryKey} above — `NotificationItem` is the
 * Notification & Action Queue context's own aggregate
 * (`consultant-experience-context.md` §2.2), sourced from events that
 * originate across every capability, not owned by any single one of the
 * nine `queryKeys` capability slots. ADR-011/ADR-015: this is exactly the
 * key `useNotificationStream`'s SSE handler invalidates on every pushed
 * event, so `useNotificationsQuery` re-fetches.
 */
export function notificationsQueryKey(consultantId: string) {
  return ['notifications', consultantId] as const
}

/**
 * `GET /api/action-queue` (PROMPT-33) query key: `['action-queue', consultantId]`.
 * Same rationale as {@link notificationsQueryKey} above, for
 * `ActionQueueEntry` instead of `NotificationItem`.
 */
export function actionQueueQueryKey(consultantId: string) {
  return ['action-queue', consultantId] as const
}
