import { useMutation, useQueryClient } from '@tanstack/react-query'
import { Route, Routes, useParams } from 'react-router-dom'
import { Button, Card, CardGrid, Header, Layout, navItemsFromAssertions, Sidebar } from '@cognitum/design-system'
import { ActionList } from '../features/action-items/ActionList'
import { ProfileEditForm } from '../features/capacity/ProfileEditForm'
import { ProposalWorkspace } from '../features/commit/ProposalWorkspace'
import { CustomerContextList } from '../features/customer/CustomerContextList'
import { LearningDashboard } from '../features/edu/LearningDashboard'
import { ExecutionWorkspace } from '../features/execution/ExecutionWorkspace'
import { LandscapeWorkspace } from '../features/landscape/LandscapeWorkspace'
import { ApprovedClauses } from '../features/legal/ApprovedClauses'
import { ActionQueue } from '../features/notifications/ActionQueue'
import { NotificationCentre } from '../features/notifications/NotificationCentre'
import { ProductCatalog } from '../features/products/ProductCatalog'
import { LeadConflictCheck } from '../features/sales/LeadConflictCheck'
import { ProspectPipeline } from '../features/sales/ProspectPipeline'
import type { SessionState } from '../lib/SessionContext'
import type { DashboardCard } from '../lib/useDashboardQuery'
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
 * # ADR-020 part C: real client-side routing
 * The shell (`Layout`/`Header`/`Sidebar`/the PROMPT-18 identity line) stays
 * mounted across navigation; only the body switches, via `Routes`:
 * - `/` (`OverviewRoute`) — `NotificationCentre`/`ActionQueue`/`ActionList`,
 *   unconditional regardless of capability grants (PROMPT-33/ADR-020 part B).
 * - `/modules/:moduleId` (`ModuleRoute`) — the one `GET /api/dashboard` card
 *   matching `:moduleId`, replacing the old single everything-on-one-scroll
 *   `CardGrid` this page used to render every card into. `Sidebar`'s nav
 *   items (`navItemsFromAssertions`) already point at `/modules/{capability}`,
 *   so a sidebar click lands directly on that module's route.
 *
 * The `"sales"` card (PROMPT-26) hosts the real `LeadConflictCheck` feature
 * module, the `"commit"` card (PROMPT-34) hosts the real `ProposalWorkspace`
 * feature module, the `"edu"` card (PROMPT-35) hosts the real
 * `LearningDashboard` feature module, the `"capacity"` card (PROMPT-36)
 * hosts the real `ProfileEditForm` feature module, the `"customer"` card
 * (PROMPT-37) hosts the real `CustomerContextList` feature module, the
 * `"execution"` card (PROMPT-38) hosts the real `ExecutionWorkspace`
 * feature module, the `"products"` card (PROMPT-39) hosts the real
 * `ProductCatalog` feature module, and the `"landscape"` card (PROMPT-40)
 * hosts the real `LandscapeWorkspace` feature module; every other
 * `module_id` still renders a placeholder — their feature modules don't
 * exist yet (PROMPT-41+). Keeps the PROMPT-18 "You are logged in as ..." line too
 * (this unit's "replace or keep both" choice: keep both) since it's cheap,
 * still true, and no acceptance criterion asks for its removal.
 *
 * `useNotificationStream()` is called once here, unconditionally for the
 * page (not per-route), so exactly one `EventSource` connection exists per
 * authenticated session regardless of which route is active.
 */
export function DashboardPage({ session }: DashboardPageProps) {
  useNotificationStream()

  const { data, isPending, isError } = useDashboardQuery()
  const cards = data?.cards ?? []

  const queryClient = useQueryClient()
  const logoutMutation = useMutation({
    mutationFn: async () => {
      const response = await fetch('/api/logout', { method: 'POST', credentials: 'include' })
      if (!response.ok) {
        throw new Error(`POST /api/logout failed: ${response.status}`)
      }
    },
    onSuccess: () => {
      // Same bare `['session']` key `useSessionQuery` reads -- invalidating
      // it refetches `GET /api/session`, which now 401s against the
      // server-invalidated session, flipping `useSession()` back to
      // `'unauthenticated'` and swapping this page for `LoginPage`.
      void queryClient.invalidateQueries({ queryKey: ['session'] })
    },
  })

  // `Overview` is prepended ahead of the capability-derived items — it's
  // the one nav destination not gated by a Nexus/Armor grant (PROMPT-33/
  // ADR-020 part B's fixed cards), so `navItemsFromAssertions` itself
  // (which only ever knows about `permission_assertions`) can't produce it.
  // Without this, `/modules/:moduleId` routes would have no way back to `/`
  // once real routing (ADR-020 part C) replaced the old single scrolling
  // page.
  const sidebarItems = [{ label: 'Overview', href: '/' }, ...navItemsFromAssertions(session.permissionAssertions)]

  return (
    <Layout sidebar={<Sidebar items={sidebarItems} />}>
      <Header
        title="Cognitum Consultants"
        rightSlot={
          <div className="flex items-center gap-3">
            <span className="text-sm text-muted-foreground">{session.consultantId}</span>
            <Button
              variant="secondary"
              className="h-8 px-3 text-xs"
              disabled={logoutMutation.isPending}
              onClick={() => logoutMutation.mutate()}
            >
              {logoutMutation.isPending ? 'Signing out…' : 'Sign out'}
            </Button>
          </div>
        }
      />
      <p className="border-b border-border/50 px-4 py-3 text-sm text-muted-foreground">
        You are logged in as {session.consultantId}
      </p>

      <div className="mx-auto max-w-[1400px] p-4">
        <Routes>
          <Route path="/" element={<OverviewRoute cards={cards} isPending={isPending} isError={isError} />} />
          <Route
            path="/modules/:moduleId"
            element={<ModuleRoute cards={cards} isPending={isPending} isError={isError} />}
          />
        </Routes>
      </div>
    </Layout>
  )
}

interface OverviewRouteProps {
  cards: DashboardCard[]
  isPending: boolean
  isError: boolean
}

/**
 * ADR-020 part C `/` route: the fixed, capability-independent cards
 * (PROMPT-33/ADR-020 part B) are never capability-gated, so this route
 * itself never has a loading/error state to branch on. It does still take
 * `cards`/`isPending`/`isError`, though, for the "no modules assigned yet"
 * empty state below: `Sidebar`'s own version of that message
 * (`navItemsFromAssertions` returning zero items) is permanently
 * unreachable through this page now that `sidebarItems` above always
 * prepends "Overview" — this is the only remaining place a zero-capability
 * consultant would ever see that explained.
 */
function OverviewRoute({ cards, isPending, isError }: OverviewRouteProps) {
  return (
    <div>
      <div className="mb-8">
        <h2 className="mb-3 text-[0.6875rem] font-semibold uppercase tracking-widest text-muted-foreground">
          Overview
        </h2>
        <CardGrid>
          <Card title="Notifications">
            <NotificationCentre />
          </Card>
          <Card title="Action Queue">
            <ActionQueue />
          </Card>
          {/* ADR-020 part B: a consultant-authored checklist, not
              capability-gated (same as the two cards above) -- see
              `ActionList.tsx`'s module docs for why it's kept separate
              from `ActionQueue`. */}
          <Card title="My Action List">
            <ActionList />
          </Card>
        </CardGrid>
      </div>

      {!isPending && !isError && cards.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border bg-[image:var(--gradient-card)] p-8 text-center">
          <h3 className="mb-2 text-base font-semibold tracking-tight text-primary">No modules assigned yet</h3>
          <p className="mx-auto max-w-md text-sm leading-relaxed text-muted-foreground">
            Nexus hasn't granted this consultant access to any capability module. Once Armor
            assigns permissions, the modules you're permitted to use will appear in the sidebar.
          </p>
        </div>
      ) : null}
    </div>
  )
}

interface ModuleRouteProps {
  cards: DashboardCard[]
  isPending: boolean
  isError: boolean
}

/**
 * ADR-020 part C `/modules/:moduleId` route: renders the single `GET
 * /api/dashboard` card matching the route param, replacing the old
 * everything-on-one-scroll `CardGrid` of every permitted card. A `moduleId`
 * with no matching card (a stale/typo'd link, or a capability Armor granted
 * but `GET /api/dashboard` hasn't surfaced as a card yet) gets the same
 * "not yet available" framing the old page used for a fully empty
 * dashboard, scoped to just this one module instead of the whole page.
 */
function ModuleRoute({ cards, isPending, isError }: ModuleRouteProps) {
  const { moduleId } = useParams<{ moduleId: string }>()

  if (isPending) return <p className="text-sm text-muted-foreground">Loading dashboard…</p>
  if (isError) return <p className="text-sm text-[hsl(0_70%_70%)]">Failed to load your dashboard.</p>

  const card = cards.find((candidate) => candidate.module_id === moduleId)

  if (card === undefined) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-[image:var(--gradient-card)] p-8 text-center">
        <h3 className="mb-2 text-base font-semibold tracking-tight text-primary">Module not available</h3>
        <p className="mx-auto max-w-md text-sm leading-relaxed text-muted-foreground">
          This isn't one of your dashboard modules yet. Once Armor grants it and it appears on your
          dashboard, it'll be reachable here.
        </p>
      </div>
    )
  }

  return (
    <CardGrid>
      <Card title={capitalize(card.module_id)}>{renderModuleContent(card.module_id)}</Card>
    </CardGrid>
  )
}

function renderModuleContent(moduleId: string) {
  switch (moduleId) {
    case 'sales':
      // ADR-020 part A: the prospect pipeline joins the existing
      // conflict-check tool inside the same Sales card, rather than
      // replacing it.
      return (
        <div className="flex flex-col gap-4">
          <LeadConflictCheck />
          <div className="border-t border-border pt-4">
            <ProspectPipeline />
          </div>
        </div>
      )
    case 'commit':
      return <ProposalWorkspace />
    case 'edu':
      return <LearningDashboard />
    case 'capacity':
      return <ProfileEditForm />
    case 'customer':
      return <CustomerContextList />
    case 'execution':
      return <ExecutionWorkspace />
    case 'products':
      return <ProductCatalog />
    case 'landscape':
      return <LandscapeWorkspace />
    case 'legal':
      // `ClauseContext` requires either a `proposalId` (see
      // `ProposalWorkspace.tsx`'s in-context usage) or a `topic` -- there's
      // no specific proposal on this general module route, so `"general"`
      // is a placeholder default pending a real topic-picker.
      return <ApprovedClauses context={{ topic: 'general' }} />
    default:
      return <p className="text-xs text-muted-foreground">no live data yet</p>
  }
}

function capitalize(value: string): string {
  return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1)
}
