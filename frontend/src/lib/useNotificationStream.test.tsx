import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { SessionProvider } from './SessionContext'
import { useNotificationStream } from './useNotificationStream'

// PROMPT-33: proves `useNotificationStream` (1) opens exactly one
// `EventSource` against `/api/notifications/stream` once a session exists,
// (2) invalidates the correct TanStack Query key per pushed `kind`
// (ADR-011's wire shape / ADR-015's invalidation contract), and (3) closes
// the connection on unmount. jsdom has no native `EventSource`
// (`src/test/setup.ts` installs a harness-wide no-op default for every
// other test); this suite installs its own controllable double instead, so
// individual tests can synthesize a pushed SSE frame.

class ControllableEventSource {
  static instances: ControllableEventSource[] = []
  onmessage: ((event: MessageEvent<string>) => void) | null = null
  closed = false
  readonly url: string

  constructor(url: string) {
    this.url = url
    ControllableEventSource.instances.push(this)
  }

  emit(data: string) {
    this.onmessage?.({ data } as MessageEvent<string>)
  }

  close() {
    this.closed = true
  }
}

function HookHarness() {
  useNotificationStream()
  return null
}

function renderWithProviders(client: QueryClient) {
  return render(
    <QueryClientProvider client={client}>
      <SessionProvider>
        <HookHarness />
      </SessionProvider>
    </QueryClientProvider>,
  )
}

function stubAuthenticatedSession() {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ consultant_id: 'consultant-1', permission_assertions: [] }),
    }),
  )
}

describe('useNotificationStream', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    ControllableEventSource.instances = []
  })

  it('opens exactly one EventSource against /api/notifications/stream once authenticated', async () => {
    stubAuthenticatedSession()
    vi.stubGlobal('EventSource', ControllableEventSource)

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    renderWithProviders(client)

    await waitFor(() => expect(ControllableEventSource.instances).toHaveLength(1))
    expect(ControllableEventSource.instances[0].url).toBe('/api/notifications/stream')
  })

  it('invalidates the notifications query key on a "notification" event', async () => {
    stubAuthenticatedSession()
    vi.stubGlobal('EventSource', ControllableEventSource)

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const invalidateSpy = vi.spyOn(client, 'invalidateQueries')
    renderWithProviders(client)

    await waitFor(() => expect(ControllableEventSource.instances).toHaveLength(1))
    ControllableEventSource.instances[0].emit(
      JSON.stringify({ kind: 'notification', notification_id: 'n-1', title: 't', body: 'b' }),
    )

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ['notifications', 'consultant-1'] })
  })

  it('invalidates the action-queue query key on an "action_queue_entry" event, not the notifications key', async () => {
    stubAuthenticatedSession()
    vi.stubGlobal('EventSource', ControllableEventSource)

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const invalidateSpy = vi.spyOn(client, 'invalidateQueries')
    renderWithProviders(client)

    await waitFor(() => expect(ControllableEventSource.instances).toHaveLength(1))
    ControllableEventSource.instances[0].emit(
      JSON.stringify({ kind: 'action_queue_entry', entry_id: 'e-1', action_state: 'pending' }),
    )

    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ['action-queue', 'consultant-1'] })
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ['notifications', 'consultant-1'] })
  })

  it('ignores a malformed frame instead of throwing', async () => {
    stubAuthenticatedSession()
    vi.stubGlobal('EventSource', ControllableEventSource)

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const invalidateSpy = vi.spyOn(client, 'invalidateQueries')
    renderWithProviders(client)

    await waitFor(() => expect(ControllableEventSource.instances).toHaveLength(1))

    expect(() => ControllableEventSource.instances[0].emit('not json')).not.toThrow()
    expect(invalidateSpy).not.toHaveBeenCalled()
  })

  it('closes the EventSource on unmount', async () => {
    stubAuthenticatedSession()
    vi.stubGlobal('EventSource', ControllableEventSource)

    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const { unmount } = renderWithProviders(client)

    await waitFor(() => expect(ControllableEventSource.instances).toHaveLength(1))
    expect(ControllableEventSource.instances[0].closed).toBe(false)

    unmount()

    expect(ControllableEventSource.instances[0].closed).toBe(true)
  })
})
