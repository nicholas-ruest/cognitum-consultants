import { Card } from '../components/Card'
import { CardGrid } from '../components/CardGrid'
import { Header } from '../components/Header'
import { Layout } from '../components/Layout'
import { navItemsFromAssertions, Sidebar } from '../components/Sidebar'
import { ProfileEditForm } from '../features/capacity/ProfileEditForm'
import { ProposalWorkspace } from '../features/commit/ProposalWorkspace'
import { CustomerContextList } from '../features/customer/CustomerContextList'
import { LearningDashboard } from '../features/edu/LearningDashboard'
import { ExecutionWorkspace } from '../features/execution/ExecutionWorkspace'
import { ActionQueue } from '../features/notifications/ActionQueue'
import { NotificationCentre } from '../features/notifications/NotificationCentre'
import { ProductCatalog } from '../features/products/ProductCatalog'
import { LeadConflictCheck } from '../features/sales/LeadConflictCheck'
import type { SessionState } from '../lib/SessionContext'
import { useDashboardQuery } from '../lib/useDashboardQuery'
import { useNotificationStream } from '../lib/useNotificationStream'

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
 * labeled by `module_id` (capitalized). The `"sales"` card (PROMPT-26) hosts
 * the real `LeadConflictCheck` feature module, the `"commit"` card
 * (PROMPT-34) hosts the real `ProposalWorkspace` feature module, the
 * `"edu"` card (PROMPT-35) hosts the real `LearningDashboard` feature
 * module, the `"capacity"` card (PROMPT-36) hosts the real
 * `ProfileEditForm` feature module, the `"customer"` card (PROMPT-37)
 * hosts the real `CustomerContextList` feature module, the
 * `"execution"` card (PROMPT-38) hosts the real `ExecutionWorkspace`
 * feature module, and the `"products"` card (PROMPT-39) hosts the real
 * `ProductCatalog` feature module; every other `module_id` still renders a
 * placeholder — their feature modules don't exist yet (PROMPT-40+). Keeps
 * the PROMPT-18 "You are logged in as ..." line too
 * (this unit's "replace or keep both" choice: keep both) since it's cheap,
 * still true, and no acceptance criterion asks for its removal.
 *
 * # PROMPT-33 placement: two fixed dashboard cards, not a sidebar/modal
 * `NotificationCentre`/`ActionQueue` are rendered as two additional
 * `Card`s prepended to the `CardGrid`, alongside — not inside — the
 * `GET /api/dashboard`-driven cards. Chosen over a persistent
 * sidebar/header slot (the prompt's other named option) because this page
 * already has exactly one shell-composition point (`DashboardPage` itself,
 * per this component's own doc comment above) and exactly one layout
 * primitive for "a titled box of content" (`Card`/`CardGrid`, PROMPT-17) —
 * adding a *second* layout mechanism (a fixed sidebar panel) for just these
 * two features would be a bigger structural change for no behavioral gain,
 * since a dashboard card is just as persistently visible on this page as a
 * sidebar slot would be. `useNotificationStream()` is called once here,
 * unconditionally for the page (not per-card), so exactly one `EventSource`
 * connection exists per authenticated session regardless of how many cards
 * end up rendered.
 */
export function DashboardPage({ session }: DashboardPageProps) {
  useNotificationStream()

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
        <CardGrid>
          <Card title="Notifications">
            <NotificationCentre />
          </Card>
          <Card title="Action Queue">
            <ActionQueue />
          </Card>
        </CardGrid>

        {isPending ? <p className="mt-4 text-sm text-gray-500">Loading dashboard…</p> : null}
        {isError ? <p className="mt-4 text-sm text-red-600">Failed to load your dashboard.</p> : null}

        {!isPending && !isError ? (
          <div className="mt-4">
            <CardGrid>
              {cards.map((card) => (
                <Card key={card.module_id} title={capitalize(card.module_id)}>
                  {card.module_id === 'sales' ? (
                    <LeadConflictCheck />
                  ) : card.module_id === 'commit' ? (
                    <ProposalWorkspace />
                  ) : card.module_id === 'edu' ? (
                    <LearningDashboard />
                  ) : card.module_id === 'capacity' ? (
                    <ProfileEditForm />
                  ) : card.module_id === 'customer' ? (
                    <CustomerContextList />
                  ) : card.module_id === 'execution' ? (
                    <ExecutionWorkspace />
                  ) : card.module_id === 'products' ? (
                    <ProductCatalog />
                  ) : (
                    <p className="text-xs text-gray-500">no live data yet</p>
                  )}
                </Card>
              ))}
            </CardGrid>
          </div>
        ) : null}
      </div>
    </Layout>
  )
}

function capitalize(value: string): string {
  return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1)
}
