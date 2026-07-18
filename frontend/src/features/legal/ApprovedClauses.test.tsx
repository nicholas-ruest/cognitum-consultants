import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ApprovedClauses } from './ApprovedClauses'

// PROMPT-41: `ApprovedClauses` loads `GET /api/legal/clauses`, scoped by
// either a `proposalId` or a `topic` — a read-only, "conformist relationship"
// per ADR-007, no write half to pair this with (unlike Landscape's digest
// card). No backend runs here — `fetch` is mocked per-URL, the same pattern
// as `LandscapeWorkspace.test.tsx`.

const CLAUSES = [
  {
    clause_id: 'clause-1',
    title: 'Limitation of Liability',
    approved_text: 'Neither party shall be liable for...',
    policy_reference: 'policy-2026-01',
  },
  {
    clause_id: 'clause-2',
    title: 'Confidentiality',
    approved_text: 'Each party agrees to keep confidential...',
    policy_reference: 'policy-2025-11',
  },
]

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders(context: { proposalId: string } | { topic: string }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <ApprovedClauses context={context} />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ApprovedClauses', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('fetches by proposal_id and renders the returned clauses', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/legal/clauses?proposal_id=proposal-1') return { ok: true, status: 200, json: async () => CLAUSES }
      throw new Error(`unexpected fetch call: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders({ proposalId: 'proposal-1' })

    await waitFor(() => {
      expect(screen.getByText('Limitation of Liability')).toBeInTheDocument()
    })
    expect(screen.getByText('Neither party shall be liable for...')).toBeInTheDocument()
    expect(screen.getByText('Confidentiality')).toBeInTheDocument()
    expect(screen.getByText('policy-2026-01')).toBeInTheDocument()
  })

  it('fetches by topic when given a topic context', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/legal/clauses?topic=data-residency') return { ok: true, status: 200, json: async () => [] }
      throw new Error(`unexpected fetch call: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders({ topic: 'data-residency' })

    await waitFor(() => {
      expect(screen.getByText('No approved clauses found.')).toBeInTheDocument()
    })
  })

  it('renders an error alert when the fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/legal/clauses?proposal_id=proposal-1') {
          return { ok: false, status: 502, json: async () => ({ error: 'legal service unavailable' }) }
        }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders({ proposalId: 'proposal-1' })

    await waitFor(() => {
      expect(screen.getByText('Failed to load approved legal clauses.')).toBeInTheDocument()
    })
  })

  it('shows a message when no clauses are found', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/legal/clauses?proposal_id=proposal-1') return { ok: true, status: 200, json: async () => [] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders({ proposalId: 'proposal-1' })

    await waitFor(() => {
      expect(screen.getByText('No approved clauses found.')).toBeInTheDocument()
    })
  })
})
