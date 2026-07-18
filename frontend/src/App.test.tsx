import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import App from './App'

// PROMPT-18: App renders LoginPage while unauthenticated and the
// authenticated shell (Layout/Header/Sidebar) once `GET /api/session`
// resolves with a consultant. No backend runs here — `fetch` is mocked, the
// same pattern as `SessionStatus.test.tsx`/`SessionContext.test.tsx`.
//
// Supersedes the PROMPT-05 smoke test (App previously rendered a bare
// heading with no logic); that assertion no longer applies now that App is
// wired to real session state.

function renderApp() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  )
}

describe('App', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('shows a loading state before the session query resolves', () => {
    vi.stubGlobal('fetch', vi.fn().mockReturnValue(new Promise(() => {})))

    renderApp()

    expect(screen.getByText('Loading…')).toBeInTheDocument()
  })

  it('renders LoginPage when unauthenticated (401 from GET /api/session)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 401, json: async () => ({}) }),
    )

    renderApp()

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
    })
  })

  it('renders the authenticated app shell once a session resolves', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({ consultant_id: 'dev-consultant-001' }),
      }),
    )

    renderApp()

    await waitFor(() => {
      expect(
        screen.getByText('You are logged in as dev-consultant-001'),
      ).toBeInTheDocument()
    })
    expect(screen.getByRole('heading', { name: 'Cognitum Consultants' })).toBeInTheDocument()
  })
})
