import type { ReactElement } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { LeadConflictCheck } from './LeadConflictCheck'

// PROMPT-26: `LeadConflictCheck` drives the Sales lead-conflict-warning
// flow (`.plans/ddd/anti-corruption-layers.md` §1 worked example) via a
// `useMutation` (never `useQuery` — ADR-015's explicit rule that a
// conflict-check result must never be cached/reused across a different
// company entry). No backend runs here — `fetch` is mocked per-URL, the
// same `fireEvent`-based pattern as `LoginPage.test.tsx`.

const ACTIVE_OWNED_ACCOUNT_RESPONSE = {
  match_status: 'active_owned_account',
  creation_allowed: false,
  display_message: 'This company is already being worked.',
  permitted_actions: ['request_collaboration', 'submit_referral', 'cancel'],
}

function renderWithProviders(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>)
}

function submitCompanyName(name: string) {
  fireEvent.change(screen.getByLabelText('Company Name'), { target: { value: name } })
  fireEvent.click(screen.getByRole('button', { name: /check for conflicts/i }))
}

describe('LeadConflictCheck', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders the form and calls the check mutation with the typed company name on submit', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ACTIVE_OWNED_ACCOUNT_RESPONSE,
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('Acme Corp')

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/sales/lead-conflict-check',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({ company_name: 'Acme Corp' }),
        }),
      )
    })
  })

  it('renders display_message and only the permitted_actions buttons for the active_owned_account fixture', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ACTIVE_OWNED_ACCOUNT_RESPONSE }),
    )

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('Acme Corp')

    await waitFor(() => {
      expect(screen.getByText('This company is already being worked.')).toBeInTheDocument()
    })

    expect(screen.getByRole('button', { name: 'Request Collaboration' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Submit Referral' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument()

    // Not hardcoded: exactly the three buttons from `permitted_actions`,
    // plus the form's own submit button — no extra action button appears.
    expect(screen.getAllByRole('button')).toHaveLength(4)
  })

  it('renders no action buttons when permitted_actions is empty', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          match_status: 'no_conflict',
          creation_allowed: true,
          display_message: 'No existing owner found — you may proceed.',
          permitted_actions: [],
        }),
      }),
    )

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('Brand New Co')

    await waitFor(() => {
      expect(screen.getByText('No existing owner found — you may proceed.')).toBeInTheDocument()
    })

    // Only the form's submit button remains — no action buttons rendered.
    expect(screen.getAllByRole('button')).toHaveLength(1)
  })

  it('renders an unrecognized action id generically instead of crashing', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          match_status: 'active_owned_account',
          creation_allowed: false,
          display_message: 'This company is already being worked.',
          permitted_actions: ['escalate_to_manager'],
        }),
      }),
    )

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('Acme Corp')

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Escalate To Manager' })).toBeInTheDocument()
    })
  })

  it('clicking "Request Collaboration" fires the request-collaboration mutation to the correct endpoint', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()

      if (url === '/api/sales/lead-conflict-check') {
        return { ok: true, status: 200, json: async () => ACTIVE_OWNED_ACCOUNT_RESPONSE }
      }
      if (url === '/api/sales/request-collaboration') {
        return { ok: true, status: 200, json: async () => ({ status: 'ok' }) }
      }
      throw new Error(`unexpected fetch call: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('Acme Corp')

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Request Collaboration' })).toBeInTheDocument()
    })

    fireEvent.click(screen.getByRole('button', { name: 'Request Collaboration' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/sales/request-collaboration',
        expect.objectContaining({
          method: 'POST',
          body: JSON.stringify({ company_reference: 'Acme Corp' }),
        }),
      )
    })
  })

  it('clears the previous result before a new company-name submission resolves', async () => {
    let resolveSecondCheck: ((value: unknown) => void) | undefined
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url !== '/api/sales/lead-conflict-check') throw new Error(`unexpected fetch call: ${url}`)

      const body = JSON.parse(String(init?.body)) as { company_name: string }
      if (body.company_name === 'First Co') {
        return { ok: true, status: 200, json: async () => ACTIVE_OWNED_ACCOUNT_RESPONSE }
      }

      // Second company's check deliberately hangs until the test resolves
      // it, so we can assert the first result is gone *before* the second
      // arrives.
      return new Promise((resolve) => {
        resolveSecondCheck = () =>
          resolve({
            ok: true,
            status: 200,
            json: async () => ({
              match_status: 'no_conflict',
              creation_allowed: true,
              display_message: 'No existing owner found — you may proceed.',
              permitted_actions: [],
            }),
          })
      })
    })
    vi.stubGlobal('fetch', fetchMock)

    renderWithProviders(<LeadConflictCheck />)
    submitCompanyName('First Co')

    await waitFor(() => {
      expect(screen.getByText('This company is already being worked.')).toBeInTheDocument()
    })

    submitCompanyName('Second Co')

    // The stale first result must be gone while the second check is still
    // in flight (the second fetch call is deliberately left unresolved
    // above), not linger until the second one resolves.
    await waitFor(() => {
      expect(screen.queryByText('This company is already being worked.')).not.toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: /checking/i })).toBeInTheDocument()

    resolveSecondCheck?.(undefined)

    await waitFor(() => {
      expect(screen.getByText('No existing owner found — you may proceed.')).toBeInTheDocument()
    })
  })
})
