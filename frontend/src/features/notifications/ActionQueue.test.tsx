import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ActionQueue } from './ActionQueue'

// PROMPT-33: `ActionQueue` renders `GET /api/action-queue` and starts an
// entry via `POST /api/action-queue/:id/start`. No backend runs here —
// `fetch` is mocked per-URL/method, the same pattern as
// `NotificationCentre.test.tsx`/`LeadConflictCheck.test.tsx`.

const FAR_FUTURE = '2099-01-01T00:00:00Z'

const ENTRIES = [
  {
    id: 'e-1',
    title: 'Collaboration request',
    body: 'A collaboration request needs your response.',
    deep_link: 'https://app.example.com/sales/collab/1',
    action_state: 'pending' as const,
    expires_at: FAR_FUTURE,
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'e-2',
    title: 'Already started',
    body: 'This entry is already in progress.',
    deep_link: null,
    action_state: 'in_progress' as const,
    expires_at: FAR_FUTURE,
    created_at: '2026-01-01T00:00:00Z',
  },
  {
    id: 'e-3',
    title: 'Long done',
    body: 'This entry was already completed.',
    deep_link: null,
    action_state: 'completed' as const,
    expires_at: FAR_FUTURE,
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

    if (url === '/api/action-queue' && method === 'GET') {
      return { ok: true, status: 200, json: async () => ENTRIES }
    }

    if (url === '/api/action-queue/e-1/start' && method === 'POST') {
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
        <ActionQueue />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ActionQueue', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders each entry with its title/body and an action_state badge', async () => {
    vi.stubGlobal('fetch', mockFetch())
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Collaboration request')).toBeInTheDocument()
    })
    expect(screen.getByText('A collaboration request needs your response.')).toBeInTheDocument()
    expect(screen.getByText('Pending')).toBeInTheDocument()
    expect(screen.getByText('In Progress')).toBeInTheDocument()
    expect(screen.getByText('Completed')).toBeInTheDocument()
  })

  it('renders the Take Action button only for the pending entry, not in_progress/completed ones', async () => {
    vi.stubGlobal('fetch', mockFetch())
    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Collaboration request')).toBeInTheDocument())

    // Exactly one "Take Action" button — the pending entry's — even though
    // three entries are rendered.
    expect(screen.getAllByRole('button', { name: 'Take Action' })).toHaveLength(1)
  })

  it('calls POST /api/action-queue/:id/start when Take Action is clicked', async () => {
    const fetchMock = mockFetch()
    vi.stubGlobal('fetch', fetchMock)
    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Collaboration request')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: 'Take Action' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/action-queue/e-1/start',
        expect.objectContaining({ method: 'POST' }),
      )
    })
  })

  it('renders an expiry indicator for each entry', async () => {
    vi.stubGlobal('fetch', mockFetch())
    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Collaboration request')).toBeInTheDocument())

    expect(screen.getAllByText(/^Expires /)).toHaveLength(3)
  })
})
