import { BrowserRouter } from 'react-router-dom'
import { SessionProvider, useSession } from './lib/SessionContext'
import { DashboardPage } from './pages/DashboardPage'
import { LoginPage } from './pages/LoginPage'

/**
 * PROMPT-18/23 app shell wiring.
 *
 * "Redirect to dashboard" (PROMPT-18's acceptance criteria) is implemented
 * as a conditional render swap on `useSession()`'s status, not a router
 * navigation — `LoginPage`/`DashboardPage` aren't alternate routes of one
 * router, they're gated on auth state itself (there's no unauthenticated
 * URL a router could dispatch on). `BrowserRouter` (ADR-020 part C) wraps
 * both branches here regardless, since `DashboardPage`'s own `Routes` (and
 * `Sidebar`'s `Link`s) need a router context above them the moment
 * `AppShell` picks the authenticated branch.
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
    return <p className="p-4 text-sm text-muted-foreground">Loading…</p>
  }

  if (session.status === 'unauthenticated') {
    return <LoginPage />
  }

  if (session.status === 'error') {
    return (
      <p className="p-4 text-sm text-[hsl(0_70%_70%)]">
        Something went wrong loading your session. Please refresh the page.
      </p>
    )
  }

  return <DashboardPage session={session} />
}

function App() {
  return (
    <BrowserRouter>
      <SessionProvider>
        <AppShell />
      </SessionProvider>
    </BrowserRouter>
  )
}

export default App
