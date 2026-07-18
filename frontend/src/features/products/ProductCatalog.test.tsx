import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ProductCatalog } from './ProductCatalog'

// PROMPT-39: `ProductCatalog` renders `GET /api/products/catalog`'s
// `ProductReferenceCard[]` as a list, with a selectable detail card. No
// backend runs here — `fetch` is mocked per-URL, the same pattern as
// `CustomerContextList.test.tsx`/`LearningDashboard.test.tsx`.

const MIGRATION_PRODUCT = {
  product_id: 'product-1',
  name: 'Cloud Migration Accelerator',
  packaging_summary: '4-week fixed-scope engagement',
  pricing_guidance: 'Starting at $50,000',
  demo_assets: ['https://products.cognitum.one/demos/product-1.mp4'],
}

const SECURITY_PRODUCT = {
  product_id: 'product-2',
  name: 'Security Posture Review',
  packaging_summary: '2-week assessment',
  pricing_guidance: 'Starting at $20,000',
  demo_assets: [],
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <ProductCatalog />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

function stubFetch(cards: unknown[]) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/products/catalog') return { ok: true, status: 200, json: async () => cards }
      throw new Error(`unexpected fetch call: ${url}`)
    }),
  )
}

describe('ProductCatalog', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders "No approved products yet." when the catalog is empty', async () => {
    stubFetch([])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No approved products yet.')).toBeInTheDocument()
    })
  })

  it('renders every product with its pricing_guidance badge', async () => {
    stubFetch([MIGRATION_PRODUCT, SECURITY_PRODUCT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Accelerator')).toBeInTheDocument()
    })
    expect(screen.getByText('Security Posture Review')).toBeInTheDocument()
    expect(screen.getAllByText('Starting at $50,000')).toHaveLength(1)
    expect(screen.getAllByText('Starting at $20,000')).toHaveLength(1)
  })

  it('shows no detail card until a product is selected', async () => {
    stubFetch([MIGRATION_PRODUCT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Accelerator')).toBeInTheDocument()
    })
    expect(screen.queryByText('4-week fixed-scope engagement')).not.toBeInTheDocument()
  })

  it('renders the packaging_summary and demo asset links after selecting a product', async () => {
    stubFetch([MIGRATION_PRODUCT, SECURITY_PRODUCT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Cloud Migration Accelerator')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /Cloud Migration Accelerator/ }).click()

    await waitFor(() => {
      expect(screen.getByText('4-week fixed-scope engagement')).toBeInTheDocument()
    })
    expect(screen.getByRole('link', { name: 'View demo asset' })).toHaveAttribute(
      'href',
      'https://products.cognitum.one/demos/product-1.mp4',
    )
  })

  it('omits demo asset links for a product with none', async () => {
    stubFetch([SECURITY_PRODUCT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Security Posture Review')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /Security Posture Review/ }).click()

    await waitFor(() => {
      expect(screen.getByText('2-week assessment')).toBeInTheDocument()
    })
    expect(screen.queryByRole('link', { name: 'View demo asset' })).not.toBeInTheDocument()
  })

  it('renders an error alert when the catalog fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/products/catalog') {
          return { ok: false, status: 502, json: async () => ({ error: 'products service unavailable' }) }
        }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load the product catalog.')).toBeInTheDocument()
    })
  })
})
