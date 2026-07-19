import '@testing-library/jest-dom/vitest'
import { afterEach, vi } from 'vitest'
import { cleanup } from '@testing-library/react'

/**
 * Real login (`LoginPage.tsx`) initializes the Firebase JS SDK at module
 * import time (`../lib/firebase.ts`'s `initializeApp`/`getAuth`), using
 * `VITE_FIREBASE_*` env vars that are unset in this test harness — any
 * test that renders `LoginPage` (directly, or transitively via `App`'s
 * unauthenticated branch) would otherwise fail on a real SDK call.
 * Stubbed harness-wide, same "belongs in setup, not per-test-file"
 * rationale as the `EventSource` double below. Tests that need to control
 * `signInWithPopup`'s resolved/rejected value mock `firebase/auth`
 * themselves (see `LoginPage.test.tsx`) — this only stubs the app/auth
 * *instances* `../lib/firebase` exports, not sign-in behavior.
 */
vi.mock('../lib/firebase', () => ({
  firebaseAuth: {},
  googleAuthProvider: {},
}))

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
