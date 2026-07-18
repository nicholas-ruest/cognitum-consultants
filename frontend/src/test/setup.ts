import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// ADR-013 layer 4: Vitest + React Testing Library harness setup.
// Registers jest-dom matchers and unmounts rendered components after each test.
afterEach(() => {
  cleanup()
})

/**
 * PROMPT-33: jsdom does not implement `EventSource` (confirmed — there is
 * no global constructor of that name in a jsdom environment), but
 * `useNotificationStream` (and anything that renders `DashboardPage`,
 * which calls it unconditionally) constructs one. Without a stub, every
 * such test would throw `ReferenceError: EventSource is not defined`
 * regardless of whether that individual test cares about SSE at all.
 *
 * This minimal no-op double is installed directly on `globalThis` (not via
 * `vi.stubGlobal`, so `vi.unstubAllGlobals()` in an individual test's
 * `afterEach` never removes it) as the harness-wide default. Tests that
 * specifically exercise SSE delivery (`useNotificationStream.test.ts`)
 * install their own more capable double via `vi.stubGlobal('EventSource',
 * ...)` for the duration of that test, which shadows this default and is
 * then cleanly reverted by that test's own `vi.unstubAllGlobals()`.
 */
class NoopEventSource {
  onmessage: ((event: MessageEvent<string>) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  readonly url: string

  constructor(url: string) {
    this.url = url
  }

  close(): void {
    // no-op: nothing to tear down for a connection that was never opened.
  }

  addEventListener(): void {
    // no-op default double.
  }

  removeEventListener(): void {
    // no-op default double.
  }
}

if (typeof globalThis.EventSource === 'undefined') {
  // @ts-expect-error -- assigning a minimal test double, not a spec-complete EventSource.
  globalThis.EventSource = NoopEventSource
}
