import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider, useSession } from '../lib/SessionContext'
import { DashboardPage } from './DashboardPage'

// PROMPT-23: `DashboardPage` renders one `Card` per `GET /api/dashboard`
// entry, labeled by `module_id`. No backend runs here — `fetch` is mocked
// per-URL (session + dashboard), the same pattern as
// `useDashboardQuery.test.tsx`.

interface MockCard {
  module_id: string
  position: number
}

function mockFetch(cards: MockCard[]) {
  return vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString()

    if (url === '/api/session') {
      return {
        ok: true,
        status: 200,
        json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }),
      }
    }

    if (url === '/api/dashboard') {
      return {
        ok: true,
        status: 200,
        json: async () => ({ consultant_id: 'consultant-1', cards }),
      }
    }

    // PROMPT-33: `DashboardPage` now always renders `NotificationCentre`/
    // `ActionQueue`, each firing its own `GET` on mount — stubbed empty
    // here since this test suite is about the `GET /api/dashboard`-driven
    // cards, not notification/action-queue content (covered by those
    // components' own test files).
    if (url === '/api/notifications' || url === '/api/action-queue') {
      return { ok: true, status: 200, json: async () => [] }
    }

    throw new Error(`unexpected fetch call: ${url}`)
  })
}

/** Narrows `useSession()`'s result to the authenticated variant `DashboardPage` requires, the same way `App.tsx`'s `AppShell` does. */
function AuthenticatedDashboard() {
  const session = useSession()
  if (session.status !== 'authenticated') return null
  return <DashboardPage session={session} />
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <AuthenticatedDashboard />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('DashboardPage', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders one Card per dashboard cards entry with the module_id as its label', async () => {
    vi.stubGlobal(
      'fetch',
      mockFetch([
        { module_id: 'sales', position: 0 },
        { module_id: 'commit', position: 1 },
      ]),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Sales' })).toBeInTheDocument()
      expect(screen.getByRole('heading', { name: 'Commit' })).toBeInTheDocument()
    })

    // PROMPT-26: the "sales" card now hosts the real `LeadConflictCheck`
    // feature module, not the placeholder — only the remaining
    // (non-sales) card still renders it.
    expect(screen.getAllByText('no live data yet')).toHaveLength(1)
    expect(screen.getByLabelText('Company Name')).toBeInTheDocument()
  })

  it('renders zero cards when the dashboard has none', async () => {
    vi.stubGlobal('fetch', mockFetch([]))

    renderWithProviders()

    await waitFor(() => {
      expect(screen.queryByText('Loading dashboard…')).not.toBeInTheDocument()
    })

    expect(screen.queryByText('no live data yet')).not.toBeInTheDocument()
  })

  it('still renders the Layout/Header/Sidebar shell and the PROMPT-18 identity line', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'sales', position: 0 }]))

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Cognitum Consultants' })).toBeInTheDocument()
    })
    expect(screen.getByText('You are logged in as consultant-1')).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument()
  })
})
