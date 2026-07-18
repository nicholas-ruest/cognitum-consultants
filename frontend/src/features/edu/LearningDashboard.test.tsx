import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { LearningDashboard } from './LearningDashboard'

// PROMPT-35: `LearningDashboard` renders `GET /api/edu/catalog`'s
// `LearningSnapshot[]`, partitioned into Courses/Certifications/Training
// Due sections. No backend runs here — `fetch` is mocked per-URL, the same
// pattern as `ProposalWorkspace.test.tsx`.

const COMPLETED_COURSE = {
  course_id: 'course-1',
  title: 'Cloud Security Fundamentals',
  progress_status: 'completed',
  certification_status: 'issued',
  deep_link: 'https://edu.cognitum.one/courses/course-1',
}

const IN_PROGRESS_COURSE = {
  course_id: 'course-2',
  title: 'Advanced Negotiation',
  progress_status: 'in_progress',
  certification_status: null,
  deep_link: null,
}

const DUE_COURSE = {
  course_id: 'course-3',
  title: 'Annual Compliance Refresher',
  progress_status: 'not_started',
  certification_status: 'required',
  deep_link: null,
}

/**
 * The wire shape the real backend actually sends for a `None`
 * `certification_status`/`deep_link`: the Rust DTO
 * (`crates/nexus-client/src/edu.rs`'s `LearningSnapshot`) derives
 * `#[serde(skip_serializing_if = "Option::is_none")]` on both fields, so a
 * `None` value **omits the key from the JSON object entirely** rather than
 * serializing an explicit `null` — this course fixture has neither key at
 * all, proving the component doesn't crash on the real wire shape (not
 * just the more lenient `{ ..., certification_status: null }` shape the
 * other fixtures above use).
 */
const COURSE_WITH_OMITTED_OPTIONAL_FIELDS = {
  course_id: 'course-4',
  title: 'Introduction to Client Engagements',
  progress_status: 'in_progress',
}

function sessionResponse() {
  return { ok: true, status: 200, json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }) }
}

function renderWithProviders() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <LearningDashboard />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

function stubFetch(catalog: unknown[]) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/edu/catalog') return { ok: true, status: 200, json: async () => catalog }
      throw new Error(`unexpected fetch call: ${url}`)
    }),
  )
}

describe('LearningDashboard', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders "No courses yet." when the catalog is empty', async () => {
    stubFetch([])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No courses yet.')).toBeInTheDocument()
    })
  })

  function section(title: string): HTMLElement {
    const heading = screen.getByText(title)
    const el = heading.closest('section')
    expect(el).not.toBeNull()
    return el as HTMLElement
  }

  it('renders every course under Courses, with its progress_status badge', async () => {
    stubFetch([COMPLETED_COURSE, IN_PROGRESS_COURSE])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Courses')).toBeInTheDocument()
    })
    const courses = section('Courses')
    expect(within(courses).getByText('Cloud Security Fundamentals')).toBeInTheDocument()
    expect(within(courses).getByText('Advanced Negotiation')).toBeInTheDocument()
    expect(within(courses).getByText('completed')).toBeInTheDocument()
    expect(within(courses).getByText('in_progress')).toBeInTheDocument()
  })

  it('lists only courses with a non-null certification_status under Certifications', async () => {
    stubFetch([COMPLETED_COURSE, IN_PROGRESS_COURSE])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Certifications')).toBeInTheDocument()
    })

    const certificationsSection = section('Certifications')
    expect(certificationsSection).toHaveTextContent('Cloud Security Fundamentals')
    expect(certificationsSection).toHaveTextContent('Certification: issued')
    expect(certificationsSection).not.toHaveTextContent('Advanced Negotiation')
  })

  it('lists not_started/overdue courses under Training Due', async () => {
    stubFetch([COMPLETED_COURSE, DUE_COURSE])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Training Due')).toBeInTheDocument()
    })

    const trainingDueSection = section('Training Due')
    expect(trainingDueSection).toHaveTextContent('Annual Compliance Refresher')
    expect(trainingDueSection).not.toHaveTextContent('Cloud Security Fundamentals')
  })

  it('renders a deep link when present, and omits it when absent', async () => {
    stubFetch([COMPLETED_COURSE, IN_PROGRESS_COURSE])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Courses')).toBeInTheDocument()
    })

    // `COMPLETED_COURSE` has a deep link and a certification, so it renders
    // in both the Courses and Certifications sections — two links total.
    // `IN_PROGRESS_COURSE` has neither a deep link nor a certification, so
    // it contributes no link anywhere.
    const links = screen.getAllByRole('link', { name: 'Open in Edu' })
    expect(links).toHaveLength(2)
    for (const link of links) {
      expect(link).toHaveAttribute('href', 'https://edu.cognitum.one/courses/course-1')
    }
  })

  it('does not crash when certification_status/deep_link are omitted from the JSON entirely (the real skip_serializing_if wire shape)', async () => {
    stubFetch([COURSE_WITH_OMITTED_OPTIONAL_FIELDS])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Introduction to Client Engagements')).toBeInTheDocument()
    })

    const certificationsSection = section('Certifications')
    expect(certificationsSection).not.toHaveTextContent('Introduction to Client Engagements')
    expect(screen.queryByRole('link', { name: 'Open in Edu' })).not.toBeInTheDocument()
  })

  it('renders an error alert when the catalog fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/edu/catalog') return { ok: false, status: 502, json: async () => ({ error: 'edu service unavailable' }) }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load your learning catalog.')).toBeInTheDocument()
    })
  })
})
