import { useId } from 'react'
import type { InputHTMLAttributes } from 'react'

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
 * Pure presentational text input with a properly associated `<label>`
 * (via `useId`) and `aria-invalid`/`aria-describedby` wiring for an
 * optional error message. No form/business logic.
 */

export interface TextInputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'id'> {
  label: string
  errorMessage?: string
}

export function TextInput({ label, errorMessage, ...inputProps }: TextInputProps) {
  const id = useId()
  const errorId = errorMessage ? `${id}-error` : undefined

  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-sm font-medium text-gray-700">
        {label}
      </label>
      <input
        id={id}
        aria-invalid={errorMessage ? true : undefined}
        aria-describedby={errorId}
        className="rounded border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none"
        {...inputProps}
      />
      {errorMessage ? (
        <p id={errorId} className="text-sm text-red-600">
          {errorMessage}
        </p>
      ) : null}
    </div>
  )
}
