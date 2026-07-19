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
  primary: 'bg-primary text-primary-foreground shadow-[0_4px_8px_-2px_hsl(185_80%_50%/0.3)] hover:bg-primary/90',
  secondary: 'bg-secondary text-foreground border border-border hover:bg-secondary/70 hover:border-primary/50',
}

export function Button({ variant = 'primary', className, ...buttonProps }: ButtonProps) {
  const classes =
    `rounded-lg px-4 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${VARIANT_CLASSES[variant]} ${className ?? ''}`.trim()

  return <button type="button" className={classes} {...buttonProps} />
}
