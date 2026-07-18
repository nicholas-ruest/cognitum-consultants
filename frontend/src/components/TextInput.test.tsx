import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { TextInput } from './TextInput'

describe('TextInput', () => {
  it('associates the label with the input', () => {
    render(<TextInput label="Email" />)

    const input = screen.getByLabelText('Email')
    expect(input).toBeInTheDocument()
  })

  it('marks the input as invalid and links the error message', () => {
    render(<TextInput label="Email" errorMessage="Email is required" />)

    const input = screen.getByLabelText('Email')
    expect(input).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByText('Email is required')).toBeInTheDocument()
  })
})
