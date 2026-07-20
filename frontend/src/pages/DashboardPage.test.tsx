import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { SessionProvider, useSession } from '../lib/SessionContext'
import { DashboardPage } from './DashboardPage'

// PROMPT-23/ADR-020 part C: `DashboardPage` now renders real routes -- `/`
// (Overview: Notifications/Action Queue/My Action List) and
// `/modules/:moduleId` (the one `GET /api/dashboard` card matching that id,
// replacing the old everything-on-one-scroll `CardGrid`). No backend runs
// here -- `fetch` is mocked per-URL, the same pattern as
// `useDashboardQuery.test.tsx`.

interface MockCard {
  module_id: string
  position: number
}

function mockFetch(cards: MockCard[], grantedCapabilities: string[] = []) {
  return vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString()

    if (url === '/api/session') {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          consultant_id: 'consultant-1',
          permission_assertions: grantedCapabilities.map((capability) => ({
            consultant_id: 'consultant-1',
            capability,
            scope: 'default',
            expires_at: '2099-01-01T00:00:00Z',
          })),
        }),
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
    // here since this test suite is about routing, not notification/
    // action-queue content (covered by those components' own test files).
    if (url === '/api/notifications' || url === '/api/action-queue') {
      return { ok: true, status: 200, json: async () => [] }
    }

    // PROMPT-34: the "commit" card now hosts the real `ProposalWorkspace`
    // feature module, which fires its own `GET` on mount — stubbed empty
    // here for the same "not what this suite is about" reason as above
    // (covered by `ProposalWorkspace.test.tsx`).
    if (url === '/api/commit/proposals') {
      return { ok: true, status: 200, json: async () => [] }
    }

    if (url === '/api/logout') {
      return { ok: true, status: 200, json: async () => ({ ok: true }) }
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

function renderWithProviders(initialPath = '/') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[initialPath]}>
        <SessionProvider>
          <AuthenticatedDashboard />
        </SessionProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('DashboardPage', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders the Overview cards at "/", with no module content', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'sales', position: 0 }]))

    renderWithProviders('/')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Overview' })).toBeInTheDocument()
    })
    expect(screen.getByRole('heading', { name: 'Notifications' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Action Queue' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'My Action List' })).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Sales' })).not.toBeInTheDocument()
  })

  it('renders the one dashboard card matching the :moduleId route param, with its real feature module', async () => {
    vi.stubGlobal(
      'fetch',
      mockFetch([
        { module_id: 'sales', position: 0 },
        { module_id: 'commit', position: 1 },
      ]),
    )

    renderWithProviders('/modules/sales')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Sales' })).toBeInTheDocument()
    })
    expect(screen.getByLabelText('Company Name')).toBeInTheDocument()

    // Only the routed-to card renders -- Commit's card and the Overview
    // content are both absent from this route.
    expect(screen.queryByRole('heading', { name: 'Commit' })).not.toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Overview' })).not.toBeInTheDocument()
  })

  it('renders the placeholder for a dashboard card whose module_id has no feature module wired up yet', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'staffing', position: 0 }]))

    renderWithProviders('/modules/staffing')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Staffing' })).toBeInTheDocument()
    })
    expect(screen.getByText('no live data yet')).toBeInTheDocument()
  })

  it('renders a "Module not available" fallback for a moduleId with no matching dashboard card', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'sales', position: 0 }]))

    renderWithProviders('/modules/commit')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Module not available' })).toBeInTheDocument()
    })
  })

  it('still renders the Layout/Header/Sidebar shell and the PROMPT-18 identity line on every route', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'sales', position: 0 }]))

    renderWithProviders('/modules/sales')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Cognitum Consultants' })).toBeInTheDocument()
    })
    expect(screen.getByText('You are logged in as consultant-1')).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument()
  })

  it('navigates from Overview to a module route by clicking its Sidebar link', async () => {
    vi.stubGlobal('fetch', mockFetch([{ module_id: 'sales', position: 0 }], ['sales']))

    renderWithProviders('/')

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Overview' })).toBeInTheDocument()
    })

    fireEvent.click(screen.getByRole('link', { name: 'Sales' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Sales' })).toBeInTheDocument()
    })
    expect(screen.getByLabelText('Company Name')).toBeInTheDocument()
    expect(screen.queryByRole('heading', { name: 'Overview' })).not.toBeInTheDocument()
  })

  it('POSTs to /api/logout when "Sign out" is clicked', async () => {
    const fetchMock = mockFetch([])
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders('/')

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Sign out' })).toBeInTheDocument()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Sign out' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith('/api/logout', expect.objectContaining({ method: 'POST' }))
    })
  })
})
