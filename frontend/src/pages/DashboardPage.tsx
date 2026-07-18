import { Card } from '../components/Card'
import { CardGrid } from '../components/CardGrid'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { navItemsFromAssertions, Sidebar } from '../components/Sidebar'
import type { SessionState } from '../lib/SessionContext'
import { useDashboardQuery } from '../lib/useDashboardQuery'

export interface DashboardPageProps {
  /** Narrowed by the caller (`App.tsx`) to the authenticated variant only — this page has nothing to render without a `consultantId`. */
  session: Extract<SessionState, { status: 'authenticated' }>
}

/**
 * PROMPT-23 dashboard shell: the authenticated app's home page. Owns the
 * `Layout`/`Header`/`Sidebar` shell itself (the same way `LoginPage` owns
 * its own layout for the unauthenticated case) rather than having `App.tsx`
 * build the shell around it, so there's exactly one place assembling the
 * authenticated shell.
 *
 * Renders one `Card` per entry in `GET /api/dashboard`'s `cards` array,
 * labeled by `module_id` (capitalized) — a placeholder only; no live
 * capability data is wired up yet (that's Phase 2/4, once Sales/Commit/etc.
 * gateways exist). Keeps the PROMPT-18 "You are logged in as ..." line too
 * (this unit's "replace or keep both" choice: keep both) since it's cheap,
 * still true, and no acceptance criterion asks for its removal.
 */
export function DashboardPage({ session }: DashboardPageProps) {
  const { data, isPending, isError } = useDashboardQuery()
  const cards = data?.cards ?? []

  return (
    <Layout sidebar={<Sidebar items={navItemsFromAssertions(session.permissionAssertions)} />}>
      <Header
        title="Cognitum Consultants"
        rightSlot={<span className="text-sm text-gray-600">{session.consultantId}</span>}
      />
      <p className="p-4 text-sm text-gray-700">You are logged in as {session.consultantId}</p>

      <div className="p-4">
        {isPending ? <p className="text-sm text-gray-500">Loading dashboard…</p> : null}
        {isError ? <p className="text-sm text-red-600">Failed to load your dashboard.</p> : null}

        {!isPending && !isError ? (
          <CardGrid>
            {cards.map((card) => (
              <Card key={card.module_id} title={capitalize(card.module_id)}>
                <p className="text-xs text-gray-500">no live data yet</p>
              </Card>
            ))}
          </CardGrid>
        ) : null}
      </div>
    </Layout>
  )
}

function capitalize(value: string): string {
  return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1)
}
