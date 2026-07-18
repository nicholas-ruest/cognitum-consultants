/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Standalone Vitest config for this package (ADR-017): mirrors
// packages/design-system/vite.config.ts's `test` block (jsdom environment)
// so this package's own component tests can run independent of the
// frontend app that consumes it.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    exclude: ['**/node_modules/**', '**/dist/**'],
  },
})
