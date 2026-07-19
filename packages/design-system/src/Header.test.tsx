import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Header } from './Header'

describe('Header', () => {
  it('renders the title and an optional right slot', () => {
    render(<Header title="Dashboard" rightSlot={<span>consultant-42</span>} />)

    expect(screen.getByRole('heading', { name: 'Dashboard' })).toBeInTheDocument()
    expect(screen.getByText('consultant-42')).toBeInTheDocument()
  })

  it('does not crash without a right slot', () => {
    render(<Header title="Dashboard" />)

    expect(screen.getByRole('heading', { name: 'Dashboard' })).toBeInTheDocument()
  })
})
