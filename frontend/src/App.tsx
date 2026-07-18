import { Header } from './components/Header'
import { Layout } from './components/Layout'
import { Sidebar } from './components/Sidebar'
import { SessionProvider, useSession } from './lib/SessionContext'
import { LoginPage } from './pages/LoginPage'

/**
 * PROMPT-18 app shell wiring.
 *
 * "Redirect to dashboard" (PROMPT-18's acceptance criteria) is implemented
 * as a conditional render swap on `useSession()`'s status, not a router
 * navigation — no ADR mandates a client-side router yet for this unit, and
 * a real dashboard is PROMPT-23's job; this only has to prove the
 * authenticated-shell path renders. A router can replace this swap later
 * without changing `SessionContext`'s contract.
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

  return (
    <Layout sidebar={<Sidebar items={[]} />}>
      <Header
        title="Cognitum Consultants"
        rightSlot={<span className="text-sm text-gray-600">{session.consultantId}</span>}
      />
      <p className="p-4 text-sm text-gray-700">
        You are logged in as {session.consultantId}
      </p>
    </Layout>
  )
}

function App() {
  return (
    <SessionProvider>
      <AppShell />
    </SessionProvider>
  )
}

export default App
