import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
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

/**
 * Session response is per-test (varies by consultant id / permission
 * assertions); every other endpoint `DashboardPage`'s always-rendered
 * `NotificationCentre`/`ActionQueue` (PROMPT-33) and `GET /api/dashboard`
 * fire on mount gets a URL-aware empty-shaped response here, matching the
 * pattern `DashboardPage.test.tsx`/`useDashboardQuery.test.tsx` already use
 * — a blanket `mockResolvedValue` answering every URL with the *session*
 * body would hand `NotificationCentre`/`ActionQueue` a non-array `data`,
 * which now crashes `ListDetailPanel` (PROMPT-42) instead of silently
 * rendering nothing.
 */
function mockFetch(sessionBody: unknown) {
  return vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString()

    if (url === '/api/session') {
      return { ok: true, status: 200, json: async () => sessionBody }
    }
    if (url === '/api/dashboard') {
      return { ok: true, status: 200, json: async () => ({ cards: [] }) }
    }
    if (url === '/api/notifications' || url === '/api/action-queue') {
      return { ok: true, status: 200, json: async () => [] }
    }

    throw new Error(`unexpected fetch call: ${url}`)
  })
}

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
    vi.stubGlobal('fetch', mockFetch({ consultant_id: 'dev-consultant-001' }))

    renderApp()

    await waitFor(() => {
      expect(
        screen.getByText('You are logged in as dev-consultant-001'),
      ).toBeInTheDocument()
    })
    expect(screen.getByRole('heading', { name: 'Cognitum Consultants' })).toBeInTheDocument()
  })

  // PROMPT-19 (ADR-009): the sidebar's nav items are built from
  // `GET /api/session`'s `permission_assertions` field. Each case mocks a
  // different assertion set and asserts the rendered Sidebar shows exactly
  // the matching nav items — proving the conditional-rendering mechanism,
  // not real navigation destinations (no per-capability pages exist yet).
  it.each([
    {
      description: 'three permitted capabilities',
      capabilities: ['sales', 'delivery', 'staffing'],
    },
    {
      description: 'one permitted capability',
      capabilities: ['sales'],
    },
    {
      description: 'zero permitted capabilities',
      capabilities: [],
    },
  ])('renders exactly the nav items matching $description', async ({ capabilities }) => {
    vi.stubGlobal(
      'fetch',
      mockFetch({
        consultant_id: 'dev-consultant-001',
        permission_assertions: capabilities.map((capability) => ({
          consultant_id: 'dev-consultant-001',
          capability,
          scope: 'default',
          expires_at: '2099-01-01T00:00:00Z',
        })),
      }),
    )

    renderApp()

    await waitFor(() => {
      expect(
        screen.getByText('You are logged in as dev-consultant-001'),
      ).toBeInTheDocument()
    })

    const nav = screen.getByRole('navigation', { name: 'Primary' })
    const links = within(nav).queryAllByRole('link')

    expect(links).toHaveLength(capabilities.length)
    capabilities.forEach((capability) => {
      expect(
        within(nav).getByRole('link', { name: new RegExp(`^${capability}$`, 'i') }),
      ).toHaveAttribute('href', `/${capability}`)
    })
  })
})
