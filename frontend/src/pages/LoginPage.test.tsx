import type { ReactElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { LoginPage } from './LoginPage'

// PROMPT-18 (real-login revision): when this build has a Firebase API key
// baked in (`VITE_FIREBASE_API_KEY`), LoginPage renders a "Sign in with
// Google" button that runs Firebase's `signInWithPopup` and POSTs the
// resulting ID token to `POST /api/login/firebase`
// (`crates/auth/src/firebase.rs`). Neither Firebase nor the backend run in
// this test — `firebase/auth`'s `signInWithPopup` and the global `fetch`
// are both mocked. Without that key (the default in this test
// environment, matching a plain `npm run dev`/e2e build -- see
// `LoginPage.tsx`'s own doc comment), it falls back to the dev-stub
// `POST /api/login/dev` flow -- covered by its own describe block below.

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

describe('LoginPage (Firebase configured)', () => {
  beforeEach(() => {
    vi.stubEnv('VITE_FIREBASE_API_KEY', 'test-api-key')
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.unstubAllEnvs()
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

describe('LoginPage (Firebase not configured -- dev-stub fallback)', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders a plain "Sign in" button with a dev-stub disclosure caption', () => {
    renderWithClient(<LoginPage />)

    expect(screen.getByRole('button', { name: 'Sign in' })).toBeInTheDocument()
    expect(screen.getByText(/dev-stub session.*dev-consultant-001/i)).toBeInTheDocument()
  })

  it('POSTs to /api/login/dev on click, without touching Firebase', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ consultant_id: 'dev-consultant-001' }),
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithClient(<LoginPage />)

    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/login/dev',
        expect.objectContaining({ method: 'POST', credentials: 'include' }),
      )
    })
    expect(signInWithPopupMock).not.toHaveBeenCalled()
  })
})
