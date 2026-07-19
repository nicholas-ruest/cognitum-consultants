import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// Mirrors frontend/src/test/setup.ts's harness registration (ADR-013 layer
// 4) for this package's own standalone Vitest run: jest-dom matchers plus
// unmounting rendered components after each test. Frontend's setup.ts also
// stubs a no-op `EventSource` for its SSE notification feature (PROMPT-33);
// that has no analog here since none of this package's presentational
// primitives touch `EventSource`.
afterEach(() => {
  cleanup()
})
