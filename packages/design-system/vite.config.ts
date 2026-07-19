/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Standalone Vitest config for this package (ADR-017): mirrors
// frontend/vite.config.ts's `test` block (jsdom environment) so the
// components' moved test suites can run against this package on its own,
// independent of the frontend app that consumes it.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    exclude: ['**/node_modules/**', '**/dist/**'],
  },
})
