import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Alert } from './Alert'

describe('Alert', () => {
  it('shows its message text and defaults to the info variant', () => {
    render(<Alert>Something informative</Alert>)

    const alert = screen.getByRole('status')
    expect(alert).toHaveTextContent('Something informative')
  })

  it('uses role="alert" for the warning and error variants', () => {
    render(<Alert variant="warning">Careful</Alert>)
    expect(screen.getByRole('alert')).toHaveTextContent('Careful')
  })

  it('renders the error variant', () => {
    render(<Alert variant="error">Something failed</Alert>)
    expect(screen.getByRole('alert')).toHaveTextContent('Something failed')
  })
})
