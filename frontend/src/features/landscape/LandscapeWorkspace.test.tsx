import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { LandscapeWorkspace } from './LandscapeWorkspace'

// PROMPT-40: `LandscapeWorkspace` loads `GET /api/landscape/intelligence`
// (an intelligence digest card) and submits field observations via
// `POST /api/landscape/observations` (never re-auto-retried on failure, per
// ADR-016). No backend runs here — `fetch` is mocked per-URL/method, the
// same pattern as `ProfileEditForm.test.tsx`.

const DIGEST = [
  {
    intel_id: 'intel-1',
    topic: 'Cloud Migration Trends',
    summary: 'Enterprises are accelerating multi-cloud adoption.',
    published_at: '2026-01-01T00:00:00Z',
    deep_link: 'https://landscape.cognitum.one/intel/intel-1',
  },
  {
    intel_id: 'intel-2',
    topic: 'Regulatory Shifts',
    summary: 'New data residency requirements in EMEA.',
    published_at: '2026-01-02T00:00:00Z',
    deep_link: null,
  },
]

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <LandscapeWorkspace />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('LandscapeWorkspace', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders the fetched intelligence digest, including an item with no deep_link', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/landscape/intelligence') return { ok: true, status: 200, json: async () => DIGEST }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Trends')).toBeInTheDocument()
    })
    expect(screen.getByText('Enterprises are accelerating multi-cloud adoption.')).toBeInTheDocument()
    expect(screen.getByText('Regulatory Shifts')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Open in Landscape' })).toHaveAttribute(
      'href',
      'https://landscape.cognitum.one/intel/intel-1',
    )
  })

  it('renders an error alert when the digest fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/landscape/intelligence') {
          return { ok: false, status: 502, json: async () => ({ error: 'landscape service unavailable' }) }
        }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load the intelligence digest.')).toBeInTheDocument()
    })
  })

  it('shows a message when the digest is empty', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/landscape/intelligence') return { ok: true, status: 200, json: async () => [] }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No approved intelligence items yet.')).toBeInTheDocument()
    })
  })

  it('submits an observation via POST, omitting related_company_reference when blank, and clears the form on success', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/landscape/intelligence' && method === 'GET') {
        return { ok: true, status: 200, json: async () => DIGEST }
      }
      if (url === '/api/landscape/observations' && method === 'POST') {
        return { ok: true, status: 200, json: async () => ({ status: 'ok' }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Trends')).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Observation'), {
      target: { value: 'Client mentioned a competitor launch.' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Submit Observation' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/landscape/observations',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({
            observation_text: 'Client mentioned a competitor launch.',
            related_company_reference: undefined,
          }),
        }),
      )
    })

    await waitFor(() => {
      expect(screen.getByText('Observation submitted.')).toBeInTheDocument()
    })
    expect(screen.getByLabelText('Observation')).toHaveValue('')
  })

  it('includes a trimmed related_company_reference when provided', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/landscape/intelligence' && method === 'GET') {
        return { ok: true, status: 200, json: async () => DIGEST }
      }
      if (url === '/api/landscape/observations' && method === 'POST') {
        return { ok: true, status: 200, json: async () => ({ status: 'ok' }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Trends')).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Observation'), { target: { value: 'Something noteworthy.' } })
    fireEvent.change(screen.getByLabelText('Related Company Reference (optional)'), {
      target: { value: '  acme-corp  ' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Submit Observation' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/landscape/observations',
        expect.objectContaining({
          body: JSON.stringify({ observation_text: 'Something noteworthy.', related_company_reference: 'acme-corp' }),
        }),
      )
    })
  })

  it('shows an error alert and does not clear the form when submission fails', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/landscape/intelligence' && method === 'GET') {
        return { ok: true, status: 200, json: async () => DIGEST }
      }
      if (url === '/api/landscape/observations' && method === 'POST') {
        return { ok: false, status: 502, json: async () => ({ error: 'landscape service unavailable' }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Trends')).toBeInTheDocument()
    })

    fireEvent.change(screen.getByLabelText('Observation'), { target: { value: 'Something noteworthy.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Submit Observation' }))

    await waitFor(() => {
      expect(screen.getByText('Failed to submit your observation. Please try again.')).toBeInTheDocument()
    })
    // Not auto-retried, and the consultant's typed text is preserved so they
    // can consciously re-submit (ADR-016).
    expect(screen.getByLabelText('Observation')).toHaveValue('Something noteworthy.')
    expect(fetchMock.mock.calls.filter(([, init]) => (init as RequestInit | undefined)?.method === 'POST')).toHaveLength(1)
  })
})
