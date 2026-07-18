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
 * Pure presentational banner/alert with `info`/`warning`/`error` variants.
 * `error`/`warning` use `role="alert"` (assertive); `info` uses
 * `role="status"` (polite) — a reasonable a11y default for a primitive
 * with no product-specific severity semantics yet.
 */

export type AlertVariant = 'info' | 'warning' | 'error'

export interface AlertProps {
  variant?: AlertVariant
  children: ReactNode
}

const VARIANT_CLASSES: Record<AlertVariant, string> = {
  info: 'bg-blue-50 text-blue-800 border-blue-200',
  warning: 'bg-yellow-50 text-yellow-800 border-yellow-200',
  error: 'bg-red-50 text-red-800 border-red-200',
}

const VARIANT_ROLES: Record<AlertVariant, 'status' | 'alert'> = {
  info: 'status',
  warning: 'alert',
  error: 'alert',
}

export function Alert({ variant = 'info', children }: AlertProps) {
  return (
    <div
      role={VARIANT_ROLES[variant]}
      className={`rounded border px-4 py-3 text-sm ${VARIANT_CLASSES[variant]}`}
    >
      {children}
    </div>
  )
}
