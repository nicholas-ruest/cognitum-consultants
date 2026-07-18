import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Sidebar } from './Sidebar'

describe('Sidebar', () => {
  it('renders the nav items passed to it', () => {
    render(
      <Sidebar
        items={[
          { label: 'Home', href: '/home' },
          { label: 'Settings', href: '/settings' },
        ]}
      />,
    )

    const home = screen.getByRole('link', { name: 'Home' })
    const settings = screen.getByRole('link', { name: 'Settings' })
    expect(home).toHaveAttribute('href', '/home')
    expect(settings).toHaveAttribute('href', '/settings')
  })

  it('does not crash with an empty items list', () => {
    render(<Sidebar items={[]} />)

    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument()
  })
})
