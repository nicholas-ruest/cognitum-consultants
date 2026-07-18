import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// Mirrors packages/design-system/src/test/setup.ts's harness registration
// (ADR-013 layer 4) for this package's own standalone Vitest run: jest-dom
// matchers plus unmounting rendered components after each test. Neither of
// this package's two components touch `EventSource`, so (same as
// design-system's setup file) no SSE stub is registered here.
afterEach(() => {
  cleanup()
})
