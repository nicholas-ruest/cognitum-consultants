import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider, useSession } from './SessionContext'

// PROMPT-18: `useSession()`/`SessionProvider` must reflect loading,
// unauthenticated (401 from `GET /api/session`), authenticated (200), and
// real-error states without callers reaching into TanStack Query directly.

function Probe() {
  const session = useSession()

  switch (session.status) {
    case 'loading':
      return <p>loading</p>
    case 'unauthenticated':
      return <p>unauthenticated</p>
    case 'authenticated':
      return <p>authenticated as {session.consultantId}</p>
    case 'error':
      return <p>error</p>
  }
}

function renderWithProviders(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>{ui}</SessionProvider>
    </QueryClientProvider>,
  )
}

describe('SessionContext', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('starts in the loading state', () => {
    vi.stubGlobal('fetch', vi.fn().mockReturnValue(new Promise(() => {})))

    renderWithProviders(<Probe />)

    expect(screen.getByText('loading')).toBeInTheDocument()
  })

  it('reflects unauthenticated when GET /api/session returns 401', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 401, json: async () => ({}) }),
    )

    renderWithProviders(<Probe />)

    await waitFor(() => {
      expect(screen.getByText('unauthenticated')).toBeInTheDocument()
    })
  })

  it('reflects authenticated with the consultant id when GET /api/session returns 200', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({ consultant_id: 'dev-consultant-001' }),
      }),
    )

    renderWithProviders(<Probe />)

    await waitFor(() => {
      expect(screen.getByText('authenticated as dev-consultant-001')).toBeInTheDocument()
    })
  })

  it('reflects a real error for non-401 failures', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }),
    )

    renderWithProviders(<Probe />)

    await waitFor(() => {
      expect(screen.getByText('error')).toBeInTheDocument()
    })
  })

  it('throws when useSession is called outside a SessionProvider', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})

    expect(() => render(<Probe />)).toThrow('useSession must be used within a SessionProvider')

    consoleError.mockRestore()
  })
})
