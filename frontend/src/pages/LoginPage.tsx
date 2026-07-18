import type { FormEvent } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Alert } from '../components/Alert'
import { Button } from '../components/Button'
import { Card } from '../components/Card'
import { TextInput } from '../components/TextInput'

/**
 * PROMPT-18 login page.
 *
 * Naming resolution: the prompt text (and PROMPT-18's own header in
 * `.plans/implementation-prompts.md`) says POST to `/api/login`, but the
 * BFF route that actually exists (PROMPT-11, `crates/bff-api/src/session.rs`)
 * is `POST /api/login/dev` — deliberately named to flag it as the dev-stub,
 * not a real login flow (ADR-008 "Interim dev-stub"). This page posts to the
 * real route rather than adding a redundant `/api/login` alias in the BFF.
 *
 * The dev-stub takes no request body and always logs in as the fixed
 * `dev-consultant-001` (`DevStubSessionProvider::create_dev_session`), so
 * this form has nothing for a real identifier input to do. It still
 * includes a text input per PROMPT-18's acceptance criteria ("a form with
 * an input") — labeled honestly as an inert placeholder, disabled so it
 * can't be mistaken for something the backend reads.
 */

interface LoginResponse {
  consultant_id: string
}

async function loginDev(): Promise<LoginResponse> {
  const response = await fetch('/api/login/dev', {
    method: 'POST',
    credentials: 'include',
  })

  if (!response.ok) {
    throw new Error(`POST /api/login/dev failed: ${response.status}`)
  }

  return (await response.json()) as LoginResponse
}

export function LoginPage() {
  const queryClient = useQueryClient()

  const mutation = useMutation({
    mutationFn: loginDev,
    onSuccess: () => {
      // Same bare `['session']` key `useSessionQuery` reads — invalidating
      // it refetches `GET /api/session`, which now resolves against the
      // cookie `POST /api/login/dev` just set, flipping `useSession()` to
      // `'authenticated'` and swapping `LoginPage` for the app shell.
      void queryClient.invalidateQueries({ queryKey: ['session'] })
    },
  })

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    mutation.mutate()
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-gray-50">
      <div className="w-full max-w-sm">
        <Card title="Sign in">
          <form onSubmit={handleSubmit} className="flex flex-col gap-4">
            <TextInput
              label="Dev consultant ID (unused — dev-stub always logs in as dev-consultant-001)"
              value="dev-consultant-001"
              disabled
              readOnly
            />
            {mutation.isError ? (
              <Alert variant="error">Sign-in failed. Please try again.</Alert>
            ) : null}
            <Button type="submit" disabled={mutation.isPending}>
              {mutation.isPending ? 'Signing in…' : 'Sign in'}
            </Button>
          </form>
        </Card>
      </div>
    </div>
  )
}
