import { QueryClient } from '@tanstack/react-query'

// ADR-015: TanStack Query is the sole server-state library for all `/api/*`
// calls from the SPA. One shared `QueryClient` instance is created here and
// provided at the root (`main.tsx`) so every feature module's `useQuery`/
// `useMutation` hooks share the same cache. Per-query `staleTime`/`gcTime`
// tuning is left to each query (ADR-015 "Negative/Trade-offs": caching
// behavior is a per-query tuning responsibility, not a global default that
// fits every capability equally).
export const queryClient = new QueryClient()
