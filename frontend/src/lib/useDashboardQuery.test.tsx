import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from './SessionContext'
import { useDashboardQuery } from './useDashboardQuery'

// PROMPT-23: `useDashboardQuery` must fetch `GET /api/dashboard` and expose
// its result via TanStack Query, but only once `useSession()` reports
// `'authenticated'` (it needs a `consultant_id` to scope the query key by,
// and the BFF's `require_session` gate would 401 an unauthenticated call
// anyway). No backend runs here — `fetch` is mocked per-URL, the same
// pattern as `SessionContext.test.tsx`.

interface MockCard {
  module_id: string
  position: number
}

function mockFetch({ cards }: { cards: MockCard[] }) {
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

    throw new Error(`unexpected fetch call: ${url}`)
  })
}

function Probe() {
  const { data, isPending, isError } = useDashboardQuery()

  if (isPending) return <p>loading</p>
  if (isError) return <p>error</p>

  return (
    <ul>
      {data.cards.map((card) => (
        <li key={card.module_id}>{card.module_id}</li>
      ))}
    </ul>
  )
}

function renderWithProviders(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>{ui}</SessionProvider>
    </QueryClientProvider>,
  )
}

describe('useDashboardQuery', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('fetches and returns the mocked GET /api/dashboard response once authenticated', async () => {
    vi.stubGlobal(
      'fetch',
      mockFetch({
        cards: [
          { module_id: 'sales', position: 0 },
          { module_id: 'commit', position: 1 },
        ],
      }),
    )

    renderWithProviders(<Probe />)

    await waitFor(() => {
      expect(screen.getByText('sales')).toBeInTheDocument()
      expect(screen.getByText('commit')).toBeInTheDocument()
    })
  })

  it('does not call GET /api/dashboard while unauthenticated (401 from GET /api/session)', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 401, json: async () => ({}) })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders(<Probe />)

    // The query stays disabled/pending forever in this state — it never
    // fires, so there is nothing to `waitFor` resolving. Wait for the
    // session lookup itself to settle instead.
    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalled()
    })

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith('/api/session', expect.anything())
    expect(screen.getByText('loading')).toBeInTheDocument()
  })
})
