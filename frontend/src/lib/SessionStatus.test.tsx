import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionStatus } from './SessionStatus'

// Proves the TanStack Query wiring itself works in a component (PROMPT-16):
// a real `useQuery` call, driven by a mocked `fetch`, resolves through
// QueryClientProvider and renders. No backend is running in this test —
// that's what `vi.stubGlobal('fetch', ...)` replaces.

function renderWithClient(ui: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

describe('SessionStatus', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders the consultant id once the session query resolves', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ consultant_id: 'consultant-42' }),
      }),
    )

    renderWithClient(<SessionStatus />)

    expect(screen.getByText('Loading session…')).toBeInTheDocument()

    await waitFor(() => {
      expect(screen.getByText('Signed in as consultant-42')).toBeInTheDocument()
    })
  })

  it('renders a fallback when the session request fails (e.g. 401 unauthenticated)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: false,
        status: 401,
        json: async () => ({ error: 'unauthorized' }),
      }),
    )

    renderWithClient(<SessionStatus />)

    await waitFor(() => {
      expect(screen.getByText('No active session')).toBeInTheDocument()
    })
  })
})
