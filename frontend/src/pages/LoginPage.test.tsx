import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { LoginPage } from './LoginPage'

// PROMPT-18 (real-login revision): LoginPage renders a "Sign in with
// Google" button that, on click, runs Firebase's `signInWithPopup` and
// POSTs the resulting ID token to `POST /api/login/firebase`
// (`crates/auth/src/firebase.rs`). Neither Firebase nor the backend run in
// this test — `firebase/auth`'s `signInWithPopup` and the global `fetch`
// are both mocked.

const signInWithPopupMock = vi.fn()

vi.mock('firebase/auth', async () => {
  const actual = await vi.importActual<typeof import('firebase/auth')>('firebase/auth')
  return { ...actual, signInWithPopup: (...args: unknown[]) => signInWithPopupMock(...args) }
})

// `../lib/firebase` (the app/auth instances) is stubbed harness-wide in
// `src/test/setup.ts` -- this file only needs to control `signInWithPopup`'s
// resolved/rejected value, above.

function renderWithClient(ui: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

function mockSuccessfulGoogleSignIn() {
  signInWithPopupMock.mockResolvedValue({
    user: { getIdToken: async () => 'fake-id-token' },
  })
}

describe('LoginPage', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    signInWithPopupMock.mockReset()
  })

  it('renders the sign-in button with an access-restriction caption', () => {
    renderWithClient(<LoginPage />)

    expect(screen.getByRole('button', { name: /sign in with google/i })).toBeInTheDocument()
    expect(screen.getByText(/approved consultant accounts/i)).toBeInTheDocument()
  })

  it('signs in with Google then POSTs the ID token to /api/login/firebase', async () => {
    mockSuccessfulGoogleSignIn()
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ consultant_id: 'nick@example.com' }),
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: /sign in with google/i }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/login/firebase',
        expect.objectContaining({
          method: 'POST',
          credentials: 'include',
          body: JSON.stringify({ id_token: 'fake-id-token' }),
        }),
      )
    })
  })

  it('shows a warning alert (not a generic error) when the email is not approved', async () => {
    mockSuccessfulGoogleSignIn()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: false,
        status: 403,
        json: async () => ({ error: 'someone@example.com is not approved for access' }),
      }),
    )

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: /sign in with google/i }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(/not approved for access/i)
    })
  })

  it('shows a generic error alert when the login request fails for another reason', async () => {
    mockSuccessfulGoogleSignIn()
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) }),
    )

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: /sign in with google/i }))

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(/sign-in failed/i)
    })
  })
})
