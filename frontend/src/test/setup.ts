import '@testing-library/jest-dom/vitest'
import { afterEach } from 'vitest'
import { cleanup } from '@testing-library/react'

// ADR-013 layer 4: Vitest + React Testing Library harness setup.
// Registers jest-dom matchers and unmounts rendered components after each test.
afterEach(() => {
  cleanup()
})
