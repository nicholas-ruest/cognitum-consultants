import type { ReactNode } from 'react'

/**
 * PROMPT-17 dashboard shell primitive.
 *
 * Provenance note: `research.md`'s "Dashboard Relationship" section
 * describes a *future* one-time borrow of manage.cognitum.one's
 * shell/layout components (Layout, Sidebar, Header, cards, forms, dialogs,
 * alerts) once that application exists. manage.cognitum.one's React
 * codebase is not accessible from this environment or any network this
 * sandbox can reach — there was no real source to port from at the time
 * this file was written. This component is built FRESH, from scratch, to
 * match the shape research.md describes; it is not ported or copied from
 * any actual manage.cognitum.one source.
 *
 * Pure presentational shell: a sidebar slot plus a main content area. No
 * API calls, no global state, no capability-specific logic (ADR-006). The
 * sidebar collapses on narrow viewports via Tailwind's `md:` breakpoint
 * only — no JS breakpoint logic.
 */

export interface LayoutProps {
  /** Sidebar slot — typically a `<Sidebar />` instance, but any node works. */
  sidebar?: ReactNode
  /** Main content area. */
  children?: ReactNode
}

export function Layout({ sidebar, children }: LayoutProps) {
  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      {sidebar ? (
        <aside className="hidden w-64 shrink-0 border-r border-gray-200 md:block">
          {sidebar}
        </aside>
      ) : null}
      <main className="flex-1 overflow-y-auto p-4">{children}</main>
    </div>
  )
}
