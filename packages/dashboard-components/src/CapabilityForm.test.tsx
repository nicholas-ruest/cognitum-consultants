import type { FormEvent } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { TextInput } from '@cognitum/design-system'
import { CapabilityForm } from './CapabilityForm'

describe('CapabilityForm', () => {
  it('renders its children fields and a submit button', () => {
    render(
      <CapabilityForm onSubmit={vi.fn()} submitLabel="Save Profile">
        <TextInput label="Skills" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    expect(screen.getByLabelText('Skills')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save Profile' })).toBeInTheDocument()
  })

  it('calls onSubmit when the form is submitted', () => {
    const onSubmit = vi.fn((event: FormEvent<HTMLFormElement>) => event.preventDefault())

    render(
      <CapabilityForm onSubmit={onSubmit} submitLabel="Save Profile">
        <TextInput label="Skills" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Save Profile' }))

    expect(onSubmit).toHaveBeenCalledTimes(1)
  })

  it('shows the pending label and disables the submit button while isPending', () => {
    render(
      <CapabilityForm onSubmit={vi.fn()} submitLabel="Save Profile" pendingLabel="Saving…" isPending>
        <TextInput label="Skills" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    const button = screen.getByRole('button', { name: 'Saving…' })
    expect(button).toBeInTheDocument()
    expect(button).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'Save Profile' })).not.toBeInTheDocument()
  })

  it('disables the submit button when isSubmitDisabled, even while not pending', () => {
    render(
      <CapabilityForm onSubmit={vi.fn()} submitLabel="Submit Observation" isSubmitDisabled>
        <TextInput label="Observation" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    expect(screen.getByRole('button', { name: 'Submit Observation' })).toBeDisabled()
  })

  it('renders no alert region when alerts is omitted', () => {
    render(
      <CapabilityForm onSubmit={vi.fn()} submitLabel="Save Profile">
        <TextInput label="Skills" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('renders each alert in the alerts list, above the form', () => {
    render(
      <CapabilityForm
        onSubmit={vi.fn()}
        submitLabel="Save Profile"
        alerts={[
          { variant: 'info', message: 'Profile update accepted.' },
          { variant: 'error', message: 'Failed to submit your profile update. Please try again.' },
        ]}
      >
        <TextInput label="Skills" value="" onChange={vi.fn()} />
      </CapabilityForm>,
    )

    expect(screen.getByText('Profile update accepted.')).toBeInTheDocument()
    expect(screen.getByText('Failed to submit your profile update. Please try again.')).toBeInTheDocument()
  })
})
