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
 * Pure presentational responsive grid — intended to wrap `<Card>` children,
 * but accepts any nodes.
 */

export interface CardGridProps {
  children?: ReactNode
}

export function CardGrid({ children }: CardGridProps) {
  return <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">{children}</div>
}
