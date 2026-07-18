import type { ButtonHTMLAttributes } from 'react'

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
 * Pure presentational button with `primary`/`secondary` variants. No
 * business logic — `onClick` etc. are passed through via native button
 * props.
 */

export type ButtonVariant = 'primary' | 'secondary'

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
}

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  primary: 'bg-blue-600 text-white hover:bg-blue-700',
  secondary: 'bg-gray-100 text-gray-900 hover:bg-gray-200',
}

export function Button({ variant = 'primary', className, ...buttonProps }: ButtonProps) {
  const classes = `rounded px-4 py-2 text-sm font-medium ${VARIANT_CLASSES[variant]} ${className ?? ''}`.trim()

  return <button type="button" className={classes} {...buttonProps} />
}
