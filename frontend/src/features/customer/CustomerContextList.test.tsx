import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { CustomerContextList } from './CustomerContextList'

// PROMPT-37: `CustomerContextList` renders `GET /api/customer/assigned`'s
// `CustomerContextCard[]` as a list, with a selectable detail card. No
// backend runs here — `fetch` is mocked per-URL, the same pattern as
// `LearningDashboard.test.tsx`/`ProposalWorkspace.test.tsx`.

const HEALTHY_CUSTOMER = {
  customer_id: 'customer-1',
  name: 'Acme Corp',
  health_status: 'green',
  relationship_summary: 'Healthy, quarterly business review scheduled.',
  deep_link: 'https://customer.cognitum.one/customers/customer-1',
}

const AT_RISK_CUSTOMER = {
  customer_id: 'customer-2',
  name: 'Beta LLC',
  health_status: 'red',
  relationship_summary: 'At risk — escalation in progress.',
  deep_link: null,
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <CustomerContextList />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

function stubFetch(contexts: unknown[]) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/customer/assigned') return { ok: true, status: 200, json: async () => contexts }
      throw new Error(`unexpected fetch call: ${url}`)
    }),
  )
}

describe('CustomerContextList', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders "No assigned customers yet." when the list is empty', async () => {
    stubFetch([])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No assigned customers yet.')).toBeInTheDocument()
    })
  })

  it('renders every assigned customer with its health_status badge', async () => {
    stubFetch([HEALTHY_CUSTOMER, AT_RISK_CUSTOMER])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })
    expect(screen.getByText('Beta LLC')).toBeInTheDocument()
    expect(screen.getAllByText('green')).toHaveLength(1)
    expect(screen.getAllByText('red')).toHaveLength(1)
  })

  it('shows no detail card until a customer is selected', async () => {
    stubFetch([HEALTHY_CUSTOMER])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })
    expect(screen.queryByText('Healthy, quarterly business review scheduled.')).not.toBeInTheDocument()
  })

  it('renders the relationship_summary and deep link after selecting a customer', async () => {
    stubFetch([HEALTHY_CUSTOMER, AT_RISK_CUSTOMER])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Acme Corp')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /Acme Corp/ }).click()

    await waitFor(() => {
      expect(screen.getByText('Healthy, quarterly business review scheduled.')).toBeInTheDocument()
    })
    expect(screen.getByRole('link', { name: 'Open in Customer' })).toHaveAttribute(
      'href',
      'https://customer.cognitum.one/customers/customer-1',
    )
  })

  it('omits the deep link for a customer with none', async () => {
    stubFetch([AT_RISK_CUSTOMER])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Beta LLC')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /Beta LLC/ }).click()

    await waitFor(() => {
      expect(screen.getByText('At risk — escalation in progress.')).toBeInTheDocument()
    })
    expect(screen.queryByRole('link', { name: 'Open in Customer' })).not.toBeInTheDocument()
  })

  it('renders an error alert when the assigned-customers fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/customer/assigned') {
          return { ok: false, status: 502, json: async () => ({ error: 'customer service unavailable' }) }
        }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load your assigned customers.')).toBeInTheDocument()
    })
  })
})
