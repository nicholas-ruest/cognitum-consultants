import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { LoginPage } from './LoginPage'

// PROMPT-18: LoginPage renders a form and, on submit, POSTs to the real
// dev-stub endpoint (`POST /api/login/dev`, PROMPT-11) via a `useMutation`.
// No backend runs in this test — `fetch` is mocked.

function renderWithClient(ui: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

describe('LoginPage', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders a form with a dev-identifier input and a submit button', () => {
    renderWithClient(<LoginPage />)

    expect(
      screen.getByLabelText(/Dev consultant ID.*unused.*dev-consultant-001/i),
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
  })

  it('POSTs to /api/login/dev on submit', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ consultant_id: 'dev-consultant-001' }),
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: /sign in/i }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/login/dev',
        expect.objectContaining({ method: 'POST', credentials: 'include' }),
      )
    })
  })

  it('shows an error alert when the login request fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }),
    )

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: /sign in/i }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(/sign-in failed/i)
    })
  })
})
