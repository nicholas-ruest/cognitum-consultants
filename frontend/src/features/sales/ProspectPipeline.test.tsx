import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ProspectPipeline } from './ProspectPipeline'

// ADR-020 part A: `ProspectPipeline` renders `GET /api/sales/prospects`
// grouped by stage, creates prospects via `POST /api/sales/prospects`,
// changes stage via `POST /api/sales/prospects/:id/stage`, and adds notes
// via `POST /api/sales/prospects/:id/notes`. No backend runs here — `fetch`
// is mocked per-URL/method, the same pattern as `ProposalWorkspace.test.tsx`.

const PROSPECT = {
  id: 'prospect-1',
  company_name: 'Acme Corp',
  contact_name: 'Jane Doe',
  stage: 'contacted' as const,
  notes: [],
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
        <ProspectPipeline />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ProspectPipeline', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders a prospect grouped under its stage column', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input.toString()
        const method = init?.method ?? 'GET'
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/sales/prospects' && method === 'GET') {
          return { ok: true, status: 200, json: async () => [PROSPECT] }
        }
        throw new Error(`unexpected fetch call: ${method} ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })
    expect(screen.getByText(/Contacted \(1\)/)).toBeInTheDocument()
    expect(screen.getByText('Jane Doe')).toBeInTheDocument()
  })

  it('renders "no prospects yet" when the list is empty', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/sales/prospects') return { ok: true, status: 200, json: async () => [] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText(/no prospects yet/i)).toBeInTheDocument()
    })
  })

  it('submitting the create form posts the typed company/contact name', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/sales/prospects' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [] }
      }
      if (url === '/api/sales/prospects' && method === 'POST') {
        return { ok: true, status: 201, json: async () => PROSPECT }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText(/no prospects yet/i)).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Prospect Company Name'), { target: { value: 'Globex' } })
    fireEvent.change(screen.getByLabelText('Prospect Contact Name'), { target: { value: 'John Roe' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add Prospect' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/sales/prospects',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({ company_name: 'Globex', contact_name: 'John Roe' }),
        }),
      )
    })
  })

  it('changing the stage select posts the new stage', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/sales/prospects' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [PROSPECT] }
      }
      if (url === '/api/sales/prospects/prospect-1/stage' && method === 'POST') {
        return { ok: true, status: 200, json: async () => ({ ...PROSPECT, stage: 'appointment_scheduled' }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Stage for Acme Corp'), { target: { value: 'appointment_scheduled' } })

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/sales/prospects/prospect-1/stage',
        expect.objectContaining({ method: 'POST', body: JSON.stringify({ stage: 'appointment_scheduled' }) }),
      )
    })
  })

  it('submitting a note posts it to the notes endpoint', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/sales/prospects' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [PROSPECT] }
      }
      if (url === '/api/sales/prospects/prospect-1/notes' && method === 'POST') {
        return {
          ok: true,
          status: 200,
          json: async () => ({
            ...PROSPECT,
            notes: [{ id: 'note-1', body: 'First call went well.', author_consultant_id: 'consultant-1', created_at: '2026-01-01T00:00:00Z' }],
          }),
        }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Add a note for Acme Corp'), { target: { value: 'First call went well.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/sales/prospects/prospect-1/notes',
        expect.objectContaining({ method: 'POST', body: JSON.stringify({ body: 'First call went well.' }) }),
      )
    })
  })
})
