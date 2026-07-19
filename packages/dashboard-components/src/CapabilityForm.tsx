import type { FormEvent, ReactNode } from 'react'
import { Alert, Button } from '@cognitum/design-system'
import type { AlertVariant, ButtonVariant } from '@cognitum/design-system'

/**
 * `@cognitum/dashboard-components` (ADR-017): a form wrapper built on
 * `@cognitum/design-system`'s `TextInput`/`Button`/`Alert`. Replaces the
 * hand-duplicated "`<form className=\"flex flex-col gap-3\">` wrapping
 * `TextInput` fields plus a submit `Button`, with hand-rolled
 * mutation-result/error `Alert` boilerplate above it" idiom independently
 * confirmed (ADR-017's own investigation) across
 * `frontend/src/features/{capacity,commit,landscape,sales}/*`.
 *
 * # Deliberately field-shape-agnostic
 * This does not try to generically model arbitrary field shapes. The actual
 * `TextInput` fields are passed as `children` — same as an ordinary `<form>`
 * — so each call site keeps full control of its own field list, `value`/
 * `onChange` wiring, and validation.
 *
 * # Alert region
 * `alerts` is an ordered list of `{ variant, message }` entries rendered
 * above the form, one `Alert` per entry; omit (or pass `[]`) for none. A
 * list (not a single optional alert) because at least one call site
 * (`ProfileEditForm.tsx`) renders up to two independent alerts at once (an
 * accepted/rejected result alert, and a separate submission-failure alert).
 */
export interface CapabilityFormAlert {
  variant: AlertVariant
  message: ReactNode
}

export interface CapabilityFormProps {
  /** Rendered above the form, in order, one `Alert` per entry. Omit or pass `[]` for no alert region. */
  alerts?: CapabilityFormAlert[]
  /** Forwarded verbatim to the underlying `<form onSubmit>`. */
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  /** The form's fields (typically one or more `TextInput`s) and any other form content. */
  children: ReactNode
  /** Submit button label shown while not pending. */
  submitLabel: string
  /** Submit button label shown while `isPending` is true. Defaults to `submitLabel` if omitted. */
  pendingLabel?: string
  /** Whether the form's submission is currently in flight. Disables the submit button and swaps in `pendingLabel`. */
  isPending?: boolean
  /** Additional condition (beyond `isPending`) that disables the submit button, e.g. client-side validation. */
  isSubmitDisabled?: boolean
  /** Overrides the submit `Button`'s variant. Defaults to `Button`'s own default (`primary`). */
  submitVariant?: ButtonVariant
  /** Overrides the `<form>`'s className. Defaults to `"flex flex-col gap-3"`. */
  className?: string
}

const DEFAULT_FORM_CLASS_NAME = 'flex flex-col gap-3'

export function CapabilityForm({
  alerts,
  onSubmit,
  children,
  submitLabel,
  pendingLabel,
  isPending = false,
  isSubmitDisabled = false,
  submitVariant,
  className,
}: CapabilityFormProps) {
  return (
    <div className="flex flex-col gap-3">
      {alerts?.map((alert, index) => (
        // Alerts have no stable id of their own -- position in the caller-supplied,
        // render-order-stable list is the only ordering that exists.
        <Alert key={index} variant={alert.variant}>
          {alert.message}
        </Alert>
      ))}

      <form onSubmit={onSubmit} className={className ?? DEFAULT_FORM_CLASS_NAME}>
        {children}
        <Button type="submit" variant={submitVariant} disabled={isPending || isSubmitDisabled}>
          {isPending ? (pendingLabel ?? submitLabel) : submitLabel}
        </Button>
      </form>
    </div>
  )
}
