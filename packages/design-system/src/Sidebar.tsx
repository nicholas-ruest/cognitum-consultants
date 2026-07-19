import type { ReactNode } from 'react'

/**
 * PROMPT-17 dashboard shell primitive.
 *
 * Provenance note: `research.md`'s "Dashboard Relationship" section
 * describes a *future* one-time borrow of manage.cognitum.one's
 * shell/layout components once that application exists.
 * manage.cognitum.one's React codebase is not accessible from this
 * environment or any network this sandbox can reach — there was no real
 * source to port from at the time this file was written. This component is
 * built FRESH, from scratch, to match the shape research.md describes; it
 * is not ported or copied from any actual manage.cognitum.one source.
 *
 * Pure presentational nav list. Deliberately takes `items` as a prop rather
 * than hardcoding any business navigation — [`navItemsFromAssertions`]
 * below is what builds real, capability-based nav items (PROMPT-19).
 */

export interface SidebarNavItem {
  label: string
  href: string
  icon?: ReactNode
}

/**
 * Builds `SidebarNavItem`s from the consultant's current Permission
 * Assertions (`GET /api/session`'s `permission_assertions`, ADR-009).
 *
 * ============================================================
 * UX ONLY — THIS IS NOT AN ENFORCEMENT MECHANISM (ADR-009 layer 2 of 3)
 * ============================================================
 * This function decides what to *render* as a shortcut, nothing more.
 * Omitting a nav item here just means the frontend chose not to show a
 * link to that capability — it does NOT mean the consultant is blocked
 * from it. A consultant (or a compromised/modified client that skips this
 * function entirely) can still call any backend route directly, regardless
 * of what this function produces. The real authorization checks live
 * server-side, and only there:
 *   - `PermissionCache`/`RequirePermission` in `bff-api` (PROMPT-15) —
 *     short-circuits with `403` before a Nexus call is even attempted.
 *   - The owning capability itself, downstream via Nexus/Armor — always
 *     re-checked, never assumed satisfied by this filtering.
 * Never write frontend logic anywhere that treats "item not rendered" as a
 * substitute for a real `401`/`403` response.
 *
 * Capabilities are deduplicated — a consultant may hold more than one
 * assertion for the same `capability` under different `scope`s, but this
 * is a nav-*presence* decision, not a scope-aware one (no real
 * per-capability route exists yet for scope to matter here; see
 * `PermissionCache::is_permitted`'s "capability name only" doc comment in
 * `crates/bff-api/src/permissions.rs`).
 *
 * `href` points at a stub route (`/{capability}`) since no real
 * per-capability pages exist yet beyond the `features/` stubs — this
 * proves the conditional-rendering *mechanism*, not real navigation
 * destinations.
 *
 * `assertions` is typed structurally by `CapabilityAssertion` below rather
 * than importing a consuming app's own permission-assertion type: this
 * package (`@cognitum/design-system`) cannot depend on `frontend/src/lib`
 * (or any other app's source) across the package boundary (ADR-017). Only
 * the `capability` field is ever read here, so any consumer's richer
 * assertion type (e.g. frontend's `PermissionAssertion` in
 * `frontend/src/lib/useSessionQuery.ts`, which also carries
 * `consultant_id`/`scope`/`expires_at`) already satisfies this shape
 * structurally and can be passed straight through with no adapter.
 */
export interface CapabilityAssertion {
  capability: string
}

export function navItemsFromAssertions(assertions: CapabilityAssertion[]): SidebarNavItem[] {
  const uniqueCapabilities = [...new Set(assertions.map((assertion) => assertion.capability))]

  return uniqueCapabilities.map((capability) => ({
    label: capitalize(capability),
    href: `/${capability}`,
  }))
}

function capitalize(value: string): string {
  return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1)
}

export interface SidebarProps {
  items: SidebarNavItem[]
}

export function Sidebar({ items }: SidebarProps) {
  return (
    <nav aria-label="Primary" className="flex flex-col gap-1 p-4">
      <h2 className="mb-1 px-3 text-[0.6875rem] font-semibold uppercase tracking-widest text-muted-foreground/70">
        Modules
      </h2>
      {items.length === 0 ? (
        <p className="rounded-md border border-dashed border-border/60 px-3 py-3 text-xs leading-relaxed text-muted-foreground">
          No capability modules assigned yet.
        </p>
      ) : (
        <ul className="flex flex-col gap-1">
          {items.map((item) => (
            <li key={item.href}>
              <a
                href={item.href}
                className="flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
              >
                {item.icon}
                <span>{item.label}</span>
              </a>
            </li>
          ))}
        </ul>
      )}
    </nav>
  )
}
