import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ProposalWorkspace } from './ProposalWorkspace'

// PROMPT-34: `ProposalWorkspace` renders `GET /api/commit/proposals`,
// creates proposals via `POST /api/commit/proposals` (either a manually
// typed origin reference, or the Sales -> Commit deep link's
// `?workflow_session_id=...`), and requests proposal actions via
// `POST /api/commit/proposals/:id/actions`. No backend runs here — `fetch`
// is mocked per-URL/method, the same pattern as `ActionQueue.test.tsx`.

const PROPOSAL = {
  proposal_id: 'proposal-1',
  title: 'Acme Corp Engagement Proposal',
  status: 'draft',
  stage: 'drafting',
  last_updated_at: '2026-01-01T00:00:00Z',
  deep_link: 'https://commit.cognitum.one/proposals/proposal-1',
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <ProposalWorkspace />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ProposalWorkspace', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    window.history.replaceState({}, '', '/')
  })

  it('renders each proposal with its title, status, and stage', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input.toString()
        const method = init?.method ?? 'GET'
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/commit/proposals' && method === 'GET') {
          return { ok: true, status: 200, json: async () => [PROPOSAL] }
        }
        throw new Error(`unexpected fetch call: ${method} ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp Engagement Proposal')).toBeInTheDocument()
    })
    expect(screen.getByText('draft')).toBeInTheDocument()
    expect(screen.getByText('Stage: drafting')).toBeInTheDocument()
  })

  it('renders "No proposals yet." when the list is empty', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/commit/proposals') return { ok: true, status: 200, json: async () => [] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No proposals yet.')).toBeInTheDocument()
    })
  })

  it('submitting the origin-reference form calls POST /api/commit/proposals with origin_reference', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/commit/proposals' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [] }
      }
      if (url === '/api/commit/proposals' && method === 'POST') {
        return { ok: true, status: 200, json: async () => PROPOSAL }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => expect(screen.getByText('No proposals yet.')).toBeInTheDocument())

    fireEvent.change(screen.getByLabelText('Origin Reference'), { target: { value: 'acme-corp' } })
    fireEvent.click(screen.getByRole('button', { name: 'Start Proposal' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/commit/proposals',
        expect.objectContaining({ method: 'POST', body: JSON.stringify({ origin_reference: 'acme-corp' }) }),
      )
    })
  })

  it('consumes ?workflow_session_id from the URL on mount, creates a proposal from it, and strips the param', async () => {
    window.history.pushState({}, '', '/?workflow_session_id=session-abc')

    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/commit/proposals' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [] }
      }
      if (url === '/api/commit/proposals' && method === 'POST') {
        return { ok: true, status: 200, json: async () => PROPOSAL }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/commit/proposals',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({ origin_workflow_session_id: 'session-abc' }),
        }),
      )
    })

    // The query param must be stripped so a refresh doesn't re-fire.
    expect(window.location.search).toBe('')
  })

  it('selecting a proposal renders its detail view with action buttons that call POST /api/commit/proposals/:id/actions', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/commit/proposals' && method === 'GET') {
        return { ok: true, status: 200, json: async () => [PROPOSAL] }
      }
      if (url === '/api/commit/proposals/proposal-1/actions' && method === 'POST') {
        return { ok: true, status: 200, json: async () => ({ status: 'ok' }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => expect(screen.getByText('Acme Corp Engagement Proposal')).toBeInTheDocument())

    fireEvent.click(screen.getByRole('button', { name: /Acme Corp Engagement Proposal/ }))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Resend' })).toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: 'Request Revision' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Open in Commit' })).toHaveAttribute(
      'href',
      'https://commit.cognitum.one/proposals/proposal-1',
    )

    fireEvent.click(screen.getByRole('button', { name: 'Resend' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/commit/proposals/proposal-1/actions',
        expect.objectContaining({ method: 'POST', body: JSON.stringify({ action: 'resend' }) }),
      )
    })
  })
})
