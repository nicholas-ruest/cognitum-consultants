import { useMutation, useQueryClient } from '@tanstack/react-query'
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
 * feature module, the `"products"` card (PROMPT-39) hosts the real
 * `ProductCatalog` feature module, and the `"landscape"` card (PROMPT-40)
 * hosts the real `LandscapeWorkspace` feature module; every other
 * `module_id` still renders a placeholder — their feature modules don't
 * exist yet (PROMPT-41+). Keeps the PROMPT-18 "You are logged in as ..." line too
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

  return (
    <Layout sidebar={<Sidebar items={navItemsFromAssertions(session.permissionAssertions)} />}>
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
        {/* Plain `<div>`s, not `<section>` -- feature modules (e.g. Edu's
            `LearningDashboard`) render their own `<section>`s for internal
            sub-partitions, and `<section>` wrapping *this* much of the page
            would make any `has: heading(...)` scoped query that matches one
            of those inner headings ambiguous between the module's own
            section and this outer wrapper (both contain the heading as a
            descendant), pulling in unrelated sibling content. */}
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

        <div>
          <h2 className="mb-3 text-[0.6875rem] font-semibold uppercase tracking-widest text-muted-foreground">
            Your Modules
          </h2>

          {isPending ? <p className="text-sm text-muted-foreground">Loading dashboard…</p> : null}
          {isError ? <p className="text-sm text-[hsl(0_70%_70%)]">Failed to load your dashboard.</p> : null}

          {!isPending && !isError ? (
            cards.length > 0 ? (
              <CardGrid>
                {cards.map((card) => (
                  <Card key={card.module_id} title={capitalize(card.module_id)}>
                    {card.module_id === 'sales' ? (
                      // ADR-020 part A: the prospect pipeline joins the
                      // existing conflict-check tool inside the same Sales
                      // card, rather than replacing it.
                      <div className="flex flex-col gap-4">
                        <LeadConflictCheck />
                        <div className="border-t border-border pt-4">
                          <ProspectPipeline />
                        </div>
                      </div>
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
                    ) : card.module_id === 'landscape' ? (
                      <LandscapeWorkspace />
                    ) : card.module_id === 'legal' ? (
                      // `ClauseContext` requires either a `proposalId` (see
                      // `ProposalWorkspace.tsx`'s in-context usage) or a
                      // `topic` -- there's no specific proposal on this
                      // general dashboard card, so `"general"` is a
                      // placeholder default pending a real topic-picker.
                      <ApprovedClauses context={{ topic: 'general' }} />
                    ) : (
                      <p className="text-xs text-muted-foreground">no live data yet</p>
                    )}
                  </Card>
                ))}
              </CardGrid>
            ) : (
              <div className="rounded-xl border border-dashed border-border bg-[image:var(--gradient-card)] p-8 text-center">
                <h3 className="mb-2 text-base font-semibold tracking-tight text-primary">
                  No modules assigned yet
                </h3>
                <p className="mx-auto max-w-md text-sm leading-relaxed text-muted-foreground">
                  Nexus hasn't granted this consultant access to any capability module. Once Armor
                  assigns permissions, the modules you're permitted to use will appear here.
                </p>
              </div>
            )
          ) : null}
        </div>
      </div>
    </Layout>
  )
}

function capitalize(value: string): string {
  return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1)
}
