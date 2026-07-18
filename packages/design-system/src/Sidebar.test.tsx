import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import type { PermissionAssertion } from '../lib/useSessionQuery'
import { navItemsFromAssertions, Sidebar } from './Sidebar'

function assertion(capability: string): PermissionAssertion {
  return { consultant_id: 'consultant-1', capability, scope: 'default', expires_at: '2099-01-01T00:00:00Z' }
}

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

// PROMPT-19 (ADR-009): `navItemsFromAssertions` builds nav items from
// Permission Assertions. This proves only the conditional-rendering
// mechanism (items appear/disappear based on assertions) — it is NOT
// proof of enforcement, which is a server-side concern (see the function's
// doc comment in `Sidebar.tsx`).
describe('navItemsFromAssertions', () => {
  it('builds one nav item per unique capability', () => {
    const items = navItemsFromAssertions([assertion('sales'), assertion('delivery')])

    expect(items).toEqual([
      { label: 'Sales', href: '/sales' },
      { label: 'Delivery', href: '/delivery' },
    ])
  })

  it('deduplicates repeated capabilities (e.g. distinct scopes)', () => {
    const items = navItemsFromAssertions([assertion('sales'), assertion('sales')])

    expect(items).toHaveLength(1)
    expect(items[0]).toEqual({ label: 'Sales', href: '/sales' })
  })

  it('returns no nav items for an empty assertion set', () => {
    expect(navItemsFromAssertions([])).toEqual([])
  })
})
