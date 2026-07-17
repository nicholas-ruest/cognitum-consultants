/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // The dev server (5173) and bff-api (3000, crates/bff-api) are
    // different origins; proxy `/api/*` so frontend code can call
    // `fetch('/api/session')` the same way it will in production, where
    // bff-api serves the SPA and its API from one origin (ADR-006).
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    // Playwright owns everything under e2e/ (ADR-013 layer 5); keep it out of
    // Vitest's (layer 4) discovery so the two harnesses don't collide.
    exclude: ['**/node_modules/**', '**/dist/**', '**/e2e/**'],
  },
})
