/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    // Bind on all interfaces (not just the IPv6 loopback Vite defaults to)
    // so the dev server is reachable at all through a forwarded port —
    // GitHub Codespaces' (and most devcontainer) port forwarding connects
    // from outside the container's loopback interface.
    host: true,
    // Vite's DNS-rebinding protection rejects any request whose Host header
    // isn't localhost/127.0.0.1/an explicit allowlist entry. A forwarded
    // dev-container port (Codespaces' `*.app.github.dev`, Gitpod, etc.)
    // arrives with that public hostname as the Host header, so without this
    // every request 403s with a plain-text "Blocked request" page — this is
    // a local dev-only server behind the platform's own authenticated port
    // forwarding, not a publicly exposed one, so trusting any forwarded host
    // here is safe.
    allowedHosts: true,
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
