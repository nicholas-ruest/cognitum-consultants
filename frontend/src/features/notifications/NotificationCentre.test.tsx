import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { NotificationCentre } from './NotificationCentre'

// PROMPT-33: `NotificationCentre` renders `GET /api/notifications` and
// dismisses via `PATCH /api/notifications/:id/read`. No backend runs here —
// `fetch` is mocked per-URL/method, the same pattern as
// `LeadConflictCheck.test.tsx`/`DashboardPage.test.tsx`.

const NOTIFICATIONS = [
  {
    id: 'n-1',
    title: 'Referral submitted',
    body: 'A new referral was submitted for review.',
    deep_link: 'https://app.example.com/sales/referrals/1',
    read_state: 'unread' as const,
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'n-2',
    title: 'Already read',
    body: 'This one has already been read.',
    deep_link: null,
    read_state: 'read' as const,
    created_at: '2026-01-01T00:00:00Z',
  },
]

function mockFetch() {
  return vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input.toString()
    const method = init?.method ?? 'GET'

    if (url === '/api/session') {
      return {
        ok: true,
        status: 200,
        json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }),
      }
    }

    if (url === '/api/notifications' && method === 'GET') {
      return { ok: true, status: 200, json: async () => NOTIFICATIONS }
    }

    if (url === '/api/notifications/n-1/read' && method === 'PATCH') {
      return { ok: true, status: 200, json: async () => ({}) }
    }

    throw new Error(`unexpected fetch call: ${method} ${url}`)
  })
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <NotificationCentre />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('NotificationCentre', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders notification title/body and a deep-link anchor when present', async () => {
    vi.stubGlobal('fetch', mockFetch())
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Referral submitted')).toBeInTheDocument()
    })
    expect(screen.getByText('A new referral was submitted for review.')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'View details' })).toHaveAttribute(
      'href',
      'https://app.example.com/sales/referrals/1',
    )
  })

  it('calls the mark-read mutation against PATCH /api/notifications/:id/read when Dismiss is clicked', async () => {
    const fetchMock = mockFetch()
    vi.stubGlobal('fetch', fetchMock)
    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Referral submitted')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/notifications/n-1/read',
        expect.objectContaining({ method: 'PATCH' }),
      )
    })
  })

  /**
   * One-way read state (invariant 3): an already-read notification renders
   * a plain "Read" label, and — critically — there is no control anywhere
   * in the rendered output that could set it back to unread. Asserting
   * absence, not just that the dismiss flow works, per this unit's
   * acceptance criteria.
   */
  it('renders a Read label with no unread-toggle control for an already-read notification', async () => {
    vi.stubGlobal('fetch', mockFetch())
    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Already read')).toBeInTheDocument())

    expect(screen.getByText('Read')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /unread/i })).not.toBeInTheDocument()
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
    // Exactly one Dismiss button exists — the unread item's only, never one
    // rendered for the already-read item.
    expect(screen.getAllByRole('button', { name: 'Dismiss' })).toHaveLength(1)
  })
})
