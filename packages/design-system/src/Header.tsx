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
 * Pure presentational top bar: a title/breadcrumb slot and a right-hand
 * slot for user-identity display. No live session data is wired here —
 * that lands in PROMPT-18/19.
 */

export interface HeaderProps {
  title: string
  rightSlot?: ReactNode
}

export function Header({ title, rightSlot }: HeaderProps) {
  return (
    <header className="flex items-center justify-between border-b border-border/50 px-4 py-3.5">
      <h1 className="text-lg font-semibold tracking-tight text-foreground">{title}</h1>
      {rightSlot ? <div className="flex items-center gap-2">{rightSlot}</div> : null}
    </header>
  )
}
