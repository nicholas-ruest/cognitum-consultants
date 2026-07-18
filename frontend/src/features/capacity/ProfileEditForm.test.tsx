import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ProfileEditForm } from './ProfileEditForm'

// PROMPT-36: `ProfileEditForm` loads `GET /api/capacity/profile`, renders it
// as an editable, comma-separated-list form, and submits changes via
// `PATCH /api/capacity/profile`, rendering Capacity's accepted/rejected
// verdict verbatim. No backend runs here — `fetch` is mocked per-URL/method,
// the same pattern as `ProposalWorkspace.test.tsx`.

const PROFILE = {
  skills: ['Rust', 'Cloud Architecture'],
  certifications: ['AWS Solutions Architect'],
  languages: ['English', 'French'],
  availability_window: '2026-08-01/2026-12-31',
  geographic_coverage: ['EMEA'],
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <ProfileEditForm />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

describe('ProfileEditForm', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders the fetched profile as comma-separated form fields', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/capacity/profile') return { ok: true, status: 200, json: async () => PROFILE }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByLabelText('Skills (comma-separated)')).toHaveValue('Rust, Cloud Architecture')
    })
    expect(screen.getByLabelText('Certifications (comma-separated)')).toHaveValue('AWS Solutions Architect')
    expect(screen.getByLabelText('Languages (comma-separated)')).toHaveValue('English, French')
    expect(screen.getByLabelText('Availability Window')).toHaveValue('2026-08-01/2026-12-31')
    expect(screen.getByLabelText('Geographic Coverage (comma-separated)')).toHaveValue('EMEA')
  })

  it('renders an error alert when the profile fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/capacity/profile') return { ok: false, status: 502, json: async () => ({ error: 'capacity service unavailable' }) }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load your profile.')).toBeInTheDocument()
    })
  })

  it('submitting the form calls PATCH /api/capacity/profile with the parsed lists and shows an accepted verdict', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/capacity/profile' && method === 'GET') {
        return { ok: true, status: 200, json: async () => PROFILE }
      }
      if (url === '/api/capacity/profile' && method === 'PATCH') {
        return { ok: true, status: 200, json: async () => ({ accepted: true }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByLabelText('Skills (comma-separated)')).toHaveValue('Rust, Cloud Architecture')
    })

    fireEvent.change(screen.getByLabelText('Skills (comma-separated)'), { target: { value: 'Rust, Kubernetes' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save Profile' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/capacity/profile',
        expect.objectContaining({
          method: 'PATCH',
          body: JSON.stringify({
            skills: ['Rust', 'Kubernetes'],
            certifications: ['AWS Solutions Architect'],
            languages: ['English', 'French'],
            availability_window: '2026-08-01/2026-12-31',
            geographic_coverage: ['EMEA'],
          }),
        }),
      )
    })

    await waitFor(() => {
      expect(screen.getByText('Profile update accepted.')).toBeInTheDocument()
    })
  })

  it('shows a rejected verdict with the reason, verbatim, without crashing', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/capacity/profile' && method === 'GET') {
        return { ok: true, status: 200, json: async () => PROFILE }
      }
      if (url === '/api/capacity/profile' && method === 'PATCH') {
        return {
          ok: true,
          status: 200,
          json: async () => ({ accepted: false, reason: 'availability_window overlaps an existing commitment' }),
        }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByLabelText('Skills (comma-separated)')).toHaveValue('Rust, Cloud Architecture')
    })

    fireEvent.click(screen.getByRole('button', { name: 'Save Profile' }))

    await waitFor(() => {
      expect(
        screen.getByText('Profile update rejected: availability_window overlaps an existing commitment'),
      ).toBeInTheDocument()
    })
  })

  it('does not crash when the PATCH response omits `reason` entirely (the real skip_serializing_if wire shape)', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      const method = init?.method ?? 'GET'
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/capacity/profile' && method === 'GET') {
        return { ok: true, status: 200, json: async () => PROFILE }
      }
      if (url === '/api/capacity/profile' && method === 'PATCH') {
        return { ok: true, status: 200, json: async () => ({ accepted: true }) }
      }
      throw new Error(`unexpected fetch call: ${method} ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByLabelText('Skills (comma-separated)')).toHaveValue('Rust, Cloud Architecture')
    })

    fireEvent.click(screen.getByRole('button', { name: 'Save Profile' }))

    await waitFor(() => {
      expect(screen.getByText('Profile update accepted.')).toBeInTheDocument()
    })
  })
})
