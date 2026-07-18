import { SessionProvider, useSession } from './lib/SessionContext'
import { DashboardPage } from './pages/DashboardPage'
import { LoginPage } from './pages/LoginPage'

/**
 * PROMPT-18/23 app shell wiring.
 *
 * "Redirect to dashboard" (PROMPT-18's acceptance criteria) is implemented
 * as a conditional render swap on `useSession()`'s status, not a router
 * navigation — no ADR mandates a client-side router yet for this unit. A
 * router can replace this swap later without changing `SessionContext`'s
 * contract.
 *
 * The authenticated branch renders `DashboardPage` (PROMPT-23), which owns
 * the `Layout`/`Header`/`Sidebar` shell itself — `AppShell` just picks which
 * top-level page to render for the current session state, the same way it
 * already delegates the unauthenticated case to `LoginPage`.
 *
 * Split into `AppShell` (reads `useSession()`) and `App` (provides it) since
 * a component can't call a hook from the context it also renders.
 */
function AppShell() {
  const session = useSession()

  if (session.status === 'loading') {
    return <p className="p-4 text-sm text-gray-500">Loading…</p>
  }

  if (session.status === 'unauthenticated') {
    return <LoginPage />
  }

  if (session.status === 'error') {
    return (
      <p className="p-4 text-sm text-red-600">
        Something went wrong loading your session. Please refresh the page.
      </p>
    )
  }

  return <DashboardPage session={session} />
}

function App() {
  return (
    <SessionProvider>
      <AppShell />
    </SessionProvider>
  )
}

export default App
