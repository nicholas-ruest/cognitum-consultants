import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from '../../lib/SessionContext'
import { ExecutionWorkspace } from './ExecutionWorkspace'

// PROMPT-38: `ExecutionWorkspace` renders `GET /api/execution/engagements`'s
// `EngagementSnapshot[]` as a list, with a selectable detail card showing
// workstreams/milestones/tasks, and a per-task "Request Completion" button
// that fires `POST /api/execution/tasks/:id/complete`. No backend runs
// here — `fetch` is mocked per-URL, the same pattern as
// `CustomerContextList.test.tsx`/`ProposalWorkspace.test.tsx`.

const ON_TRACK_ENGAGEMENT = {
  engagement_id: 'engagement-1',
  workstreams: ['Discovery', 'Delivery'],
  milestones: ['Kickoff complete'],
  tasks: [{ task_id: 'task-1', title: 'Draft delivery plan', status: 'assigned' }],
  delivery_status: 'on_track',
  deep_link: 'https://execution.cognitum.one/engagements/engagement-1',
}

const AT_RISK_ENGAGEMENT = {
  engagement_id: 'engagement-2',
  workstreams: ['Delivery'],
  milestones: [],
  tasks: [],
  delivery_status: 'at_risk',
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
        <ExecutionWorkspace />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

function stubFetch(engagements: unknown[], options: { onComplete?: (taskId: string) => void } = {}) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString()
      if (url === '/api/session') return sessionResponse()
      if (url === '/api/execution/engagements') return { ok: true, status: 200, json: async () => engagements }
      const completeMatch = /^\/api\/execution\/tasks\/(.+)\/complete$/.exec(url)
      if (completeMatch && init?.method === 'POST') {
        options.onComplete?.(completeMatch[1])
        return { ok: true, status: 200, json: async () => ({ status: 'ok' }) }
      }
      throw new Error(`unexpected fetch call: ${url}`)
    }),
  )
}

describe('ExecutionWorkspace', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('renders "No assigned engagements yet." when the list is empty', async () => {
    stubFetch([])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('No assigned engagements yet.')).toBeInTheDocument()
    })
  })

  it('renders every assigned engagement with its delivery_status badge', async () => {
    stubFetch([ON_TRACK_ENGAGEMENT, AT_RISK_ENGAGEMENT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('engagement-1')).toBeInTheDocument()
    })
    expect(screen.getByText('engagement-2')).toBeInTheDocument()
    expect(screen.getAllByText('on_track')).toHaveLength(1)
    expect(screen.getAllByText('at_risk')).toHaveLength(1)
  })

  it('shows no detail card until an engagement is selected', async () => {
    stubFetch([ON_TRACK_ENGAGEMENT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('engagement-1')).toBeInTheDocument()
    })
    expect(screen.queryByText('Draft delivery plan')).not.toBeInTheDocument()
  })

  it('renders workstreams, milestones, tasks, and the deep link after selecting an engagement', async () => {
    stubFetch([ON_TRACK_ENGAGEMENT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('engagement-1')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /engagement-1/ }).click()

    await waitFor(() => {
      expect(screen.getByText('Draft delivery plan')).toBeInTheDocument()
    })
    expect(screen.getByText('Discovery')).toBeInTheDocument()
    expect(screen.getByText('Delivery')).toBeInTheDocument()
    expect(screen.getByText('Kickoff complete')).toBeInTheDocument()
    expect(screen.getByText('Status: assigned')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Open in Execution' })).toHaveAttribute(
      'href',
      'https://execution.cognitum.one/engagements/engagement-1',
    )
  })

  it('omits the deep link and shows "No tasks assigned." for an engagement with none', async () => {
    stubFetch([AT_RISK_ENGAGEMENT])
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('engagement-2')).toBeInTheDocument()
    })

    screen.getByRole('button', { name: /engagement-2/ }).click()

    await waitFor(() => {
      expect(screen.getByText('No tasks assigned.')).toBeInTheDocument()
    })
    expect(screen.queryByRole('link', { name: 'Open in Execution' })).not.toBeInTheDocument()
  })

  it('requests completion through the BFF back to Execution without a local state flip', async () => {
    const completedTaskIds: string[] = []
    stubFetch([ON_TRACK_ENGAGEMENT], { onComplete: (taskId) => completedTaskIds.push(taskId) })
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('engagement-1')).toBeInTheDocument()
    })
    screen.getByRole('button', { name: /engagement-1/ }).click()

    const completeButton = await screen.findByRole('button', { name: 'Request Completion' })
    completeButton.click()

    await waitFor(() => {
      expect(completedTaskIds).toEqual(['task-1'])
    })
    // The task's own displayed `status` is unchanged by this click — it
    // only ever reflects the next `GET /api/execution/engagements` fetch,
    // never a local decision this component makes.
    expect(screen.getByText('Status: assigned')).toBeInTheDocument()
  })

  it('renders an error alert when the assigned-engagements fetch fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === '/api/session') return sessionResponse()
        if (url === '/api/execution/engagements') {
          return { ok: false, status: 502, json: async () => ({ error: 'execution service unavailable' }) }
        }
        throw new Error(`unexpected fetch call: ${url}`)
      }),
    )
    renderWithProviders()

    await waitFor(() => {
      expect(screen.getByText('Failed to load your delivery workspace.')).toBeInTheDocument()
    })
  })
})
