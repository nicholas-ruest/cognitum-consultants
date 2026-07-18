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
 * than hardcoding any business navigation — wiring real, capability-based
 * nav items is PROMPT-19's job.
 */

export interface SidebarNavItem {
  label: string
  href: string
  icon?: ReactNode
}

export interface SidebarProps {
  items: SidebarNavItem[]
}

export function Sidebar({ items }: SidebarProps) {
  return (
    <nav aria-label="Primary" className="flex flex-col gap-1 p-4">
      <ul className="flex flex-col gap-1">
        {items.map((item) => (
          <li key={item.href}>
            <a
              href={item.href}
              className="flex items-center gap-2 rounded px-3 py-2 text-sm text-gray-700 hover:bg-gray-100"
            >
              {item.icon}
              <span>{item.label}</span>
            </a>
          </li>
        ))}
      </ul>
    </nav>
  )
}
