/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    // Playwright owns everything under e2e/ (ADR-013 layer 5); keep it out of
    // Vitest's (layer 4) discovery so the two harnesses don't collide.
    exclude: ['**/node_modules/**', '**/dist/**', '**/e2e/**'],
  },
})
