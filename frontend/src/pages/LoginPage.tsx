import { useMutation, useQueryClient } from '@tanstack/react-query'
import { signInWithPopup } from 'firebase/auth'
import { Alert, Button, Card } from '@cognitum/design-system'
import { firebaseAuth, googleAuthProvider } from '../lib/firebase'

/**
 * PROMPT-18 login page, restyled to match the rest of the Cognitum One
 * stack's shared sign-in look (badge / gradient wordmark / confidential
 * pill / single CTA composition).
 *
 * Real login: Google Sign-In via Firebase. `signInWithPopup` proves the
 * consultant's Google identity; the BFF (`POST /api/login/firebase`,
 * `crates/auth/src/firebase.rs`) independently verifies the resulting ID
 * token's signature and checks its email against the
 * `approved_consultants` allowlist before issuing a session — signing in
 * with Google here is proof of identity, not proof of authorization.
 *
 * The prior dev-stub route (`POST /api/login/dev`) still exists
 * server-side for local dev (`Config::is_dev()`), but this page always
 * uses the real flow now.
 */

interface LoginResponse {
  consultant_id: string
}

class NotApprovedError extends Error {}

async function loginWithGoogle(): Promise<LoginResponse> {
  const result = await signInWithPopup(firebaseAuth, googleAuthProvider)
  const idToken = await result.user.getIdToken()

  const response = await fetch('/api/login/firebase', {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ id_token: idToken }),
  })

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null
    if (response.status === 403) {
      throw new NotApprovedError(body?.error ?? 'not approved for access')
    }
    throw new Error(body?.error ?? `POST /api/login/firebase failed: ${response.status}`)
  }

  return (await response.json()) as LoginResponse
}

export function LoginPage() {
  const queryClient = useQueryClient()

  const mutation = useMutation({
    mutationFn: loginWithGoogle,
    onSuccess: () => {
      // Same bare `['session']` key `useSessionQuery` reads — invalidating
      // it refetches `GET /api/session`, which now resolves against the
      // cookie `POST /api/login/firebase` just set, flipping
      // `useSession()` to `'authenticated'` and swapping `LoginPage` for
      // the app shell.
      void queryClient.invalidateQueries({ queryKey: ['session'] })
    },
  })

  const isNotApproved = mutation.error instanceof NotApprovedError

  return (
    <div className="relative flex min-h-screen items-center justify-center overflow-hidden bg-background p-6">
      <div className="pointer-events-none absolute inset-0" style={{ backgroundImage: 'var(--gradient-glow)' }} />
      <div className="relative w-full max-w-md">
        <Card>
          <div className="flex flex-col gap-8 px-2 py-4">
            <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">
              <span className="h-2 w-2 rounded-full bg-accent shadow-[0_0_6px_hsl(142_70%_50%/0.8)]" />
              Secure Access
            </div>

            <div className="text-center">
              <h1 className="bg-gradient-to-br from-primary to-[hsl(185_70%_65%)] bg-clip-text text-5xl font-extrabold tracking-tight text-transparent">
                Cognitum.one
              </h1>
              <p className="mt-2 text-sm text-muted-foreground">Consultants</p>
            </div>

            <div className="flex justify-center">
              <span className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-secondary/40 px-4 py-1.5 text-xs font-medium text-muted-foreground">
                <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground" />
                Confidential — Internal Team Only
              </span>
            </div>

            {mutation.isError ? (
              <Alert variant={isNotApproved ? 'warning' : 'error'}>
                {isNotApproved
                  ? mutation.error.message
                  : `Sign-in failed: ${mutation.error.message}`}
              </Alert>
            ) : null}

            <Button
              onClick={() => mutation.mutate()}
              disabled={mutation.isPending}
              variant="secondary"
              className="h-14 w-full"
            >
              {mutation.isPending ? 'Signing in…' : 'Sign in with Google'}
            </Button>

            <p className="-mt-2 text-center text-xs text-muted-foreground">
              Access is limited to approved consultant accounts
            </p>
          </div>
        </Card>
      </div>
    </div>
  )
}
