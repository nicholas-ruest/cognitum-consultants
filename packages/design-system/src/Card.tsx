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
 * Pure presentational card. Intended to be laid out via `<CardGrid>`.
 */

export interface CardProps {
  title?: string
  children?: ReactNode
}

export function Card({ title, children }: CardProps) {
  return (
    <div className="rounded-xl border border-border/50 bg-[image:var(--gradient-card)] p-4 shadow-card">
      {title ? <h3 className="mb-2 text-sm font-semibold text-foreground">{title}</h3> : null}
      <div className="text-sm text-card-foreground">{children}</div>
    </div>
  )
}
