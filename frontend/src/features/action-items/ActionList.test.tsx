import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ActionList } from './ActionList'

// ADR-020 part B: `ActionList` renders `GET /api/action-items`, creates
// items via `POST /api/action-items`, toggles `done` via
// `PATCH /api/action-items/:id`, and removes via
// `POST /api/action-items/:id/delete`. No backend runs here — `fetch` is
// mocked per-URL/method, the same pattern as `ProposalWorkspace.test.tsx`.

const ITEM = {
  id: 'item-1',
  title: 'Call Acme back',
  notes: null,
  done: false,
  linked_prospect_id: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <ActionList />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ActionList', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders an existing item', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/action-items') return { ok: true, status: 200, json: async () => [ITEM] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Call Acme back')).toBeInTheDocument()
    })
  })

  it('renders "nothing on your list yet" when empty', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/action-items') return { ok: true, status: 200, json: async () => [] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText(/nothing on your list yet/i)).toBeInTheDocument()
    })
  })

  it('submitting the form posts the typed title', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/action-items' && method === 'GET') return { ok: true, status: 200, json: async () => [] }
      if (url === '/api/action-items' && method === 'POST') return { ok: true, status: 201, json: async () => ITEM }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText(/nothing on your list yet/i)).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('New item'), { target: { value: 'Call Acme back' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/action-items',
        expect.objectContaining({ method: 'POST', body: JSON.stringify({ title: 'Call Acme back' }) }),
      )
    })
  })

  it('checking the checkbox PATCHes done: true', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/action-items' && method === 'GET') return { ok: true, status: 200, json: async () => [ITEM] }
      if (url === '/api/action-items/item-1' && method === 'PATCH') {
        return { ok: true, status: 200, json: async () => ({ ...ITEM, done: true }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Call Acme back')).toBeInTheDocument()
    })

    fireEvent.click(screen.getByLabelText('Mark "Call Acme back" done'))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/action-items/item-1',
        expect.objectContaining({ method: 'PATCH', body: JSON.stringify({ done: true }) }),
      )
    })
  })

  it('clicking Remove deletes the item', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/action-items' && method === 'GET') return { ok: true, status: 200, json: async () => [ITEM] }
      if (url === '/api/action-items/item-1/delete' && method === 'POST') {
        return { ok: true, status: 200, json: async () => ({}) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Call Acme back')).toBeInTheDocument()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith('/api/action-items/item-1/delete', expect.objectContaining({ method: 'POST' }))
    })
  })
})
