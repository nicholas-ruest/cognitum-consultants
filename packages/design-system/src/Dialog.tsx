import { useEffect, useRef } from 'react'
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
 * Controlled modal/dialog primitive: open/close state lives in the
 * caller (`open` + `onClose` props, not internal state). Closes on
 * backdrop click and on Escape. Basic ARIA: `role="dialog"`,
 * `aria-modal="true"`, and focus is moved onto the dialog when it opens.
 */

export interface DialogProps {
  open: boolean
  onClose: () => void
  title?: string
  children?: ReactNode
}

export function Dialog({ open, onClose, title, children }: DialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return

    dialogRef.current?.focus()

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [open, onClose])

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/70 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        className="max-w-md rounded-xl border border-border bg-card p-6 shadow-card outline-none"
        onClick={(event) => event.stopPropagation()}
      >
        {title ? <h2 className="mb-4 text-lg font-semibold text-foreground">{title}</h2> : null}
        {children}
      </div>
    </div>
  )
}
