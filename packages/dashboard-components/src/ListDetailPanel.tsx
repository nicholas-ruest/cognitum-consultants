import { useState } from 'react'
import type { ReactNode } from 'react'
import { Card } from '@cognitum/design-system'

/**
 * `@cognitum/dashboard-components` (ADR-017): a generic list-of-items +
 * optional selected-item-detail-panel component, built on top of
 * `@cognitum/design-system`'s `Card`. Replaces the hand-duplicated
 * "clickable row list, click sets selected id, optional detail panel below"
 * idiom independently confirmed (ADR-017's own investigation) across
 * `frontend/src/features/{customer,products,execution,commit,edu,landscape,
 * legal,notifications}/*`.
 *
 * # Deliberately DTO-agnostic
 * This component knows nothing about any feature's item shape
 * (`CustomerContextCard`, `ProductReferenceCard`, `EngagementSnapshot`, ...).
 * It takes a plain `items: T[]` plus three render props (`getKey`,
 * `renderRow`, optional `renderDetail`) — same contract any generic list
 * component would have. All feature-specific rendering (badges, deep links,
 * nested action buttons, ...) stays in the caller's render-prop closures.
 *
 * # Row wrapping is the caller's choice, not this component's
 * `renderRow` returns the *entire* content placed inside this component's
 * `<li>` — it is NOT auto-wrapped in a `<button>`. Callers that want a
 * clickable row (the customer/products/execution/commit "full list+detail"
 * call sites) return their own `<button onClick={select} ...>` from
 * `renderRow`. Callers with a genuinely non-interactive row (e.g. Edu's
 * course list, Legal's read-only clause list) can ignore `select`/
 * `isSelected` entirely and just render static content. Callers whose row
 * already contains its own nested interactive control (Notifications'
 * "Dismiss" button, the Action Queue's "Take Action" button) can likewise
 * ignore `select` and render their existing markup unchanged — forcing an
 * outer `<button>` wrapper in every case would produce invalid
 * button-inside-button markup for those two call sites. This is a
 * deliberate design choice, not an oversight.
 *
 * # Selection: uncontrolled by default, controllable if needed
 * If `selectedKey` is omitted, this component tracks the selected key
 * itself (the common case — matches every current call site, none of which
 * need selection driven from outside). Pass `selectedKey` (and, to actually
 * react to changes, `onSelectedKeyChange`) to control it externally instead.
 */
export interface ListDetailPanelRowHelpers {
  /** Whether this row's item is the currently selected one. */
  isSelected: boolean
  /** Call to make this row's item the selected one. */
  select: () => void
}

export interface ListDetailPanelProps<T> {
  /** The items to render, in order. */
  items: T[]
  /** Extracts a stable, unique string key for one item. */
  getKey: (item: T) => string
  /**
   * Renders one item's row content. The returned node is placed directly
   * inside this component's own `<li>` — do not return another `<li>`.
   * Ignore `helpers` entirely for a non-interactive row (no selection).
   */
  renderRow: (item: T, helpers: ListDetailPanelRowHelpers) => ReactNode
  /**
   * Renders the selected item's detail content, wrapped in a `Card`. Omit
   * entirely for the row-only call sites — no detail panel (and no `Card`)
   * is rendered at all when this prop is not given.
   */
  renderDetail?: (item: T) => ReactNode
  /** Controlled selected key. Omit to let this component manage its own selection state. */
  selectedKey?: string | null
  /** Fired whenever a row's `select()` is invoked, controlled or not. */
  onSelectedKeyChange?: (key: string) => void
  /** Overrides the `<ul>`'s className. Defaults to `"flex flex-col gap-2"`. */
  listClassName?: string
  /** Overrides the outer wrapper's className. Defaults to `"flex flex-col gap-4"`. */
  className?: string
}

const DEFAULT_LIST_CLASS_NAME = 'flex flex-col gap-2'
const DEFAULT_CONTAINER_CLASS_NAME = 'flex flex-col gap-4'

export function ListDetailPanel<T>({
  items,
  getKey,
  renderRow,
  renderDetail,
  selectedKey,
  onSelectedKeyChange,
  listClassName,
  className,
}: ListDetailPanelProps<T>) {
  const [internalSelectedKey, setInternalSelectedKey] = useState<string | null>(null)
  const isControlled = selectedKey !== undefined
  const currentSelectedKey = isControlled ? selectedKey : internalSelectedKey

  const selectedItem =
    renderDetail && currentSelectedKey !== null
      ? (items.find((item) => getKey(item) === currentSelectedKey) ?? null)
      : null

  return (
    <div className={className ?? DEFAULT_CONTAINER_CLASS_NAME}>
      <ul className={listClassName ?? DEFAULT_LIST_CLASS_NAME}>
        {items.map((item) => {
          const key = getKey(item)

          const select = () => {
            if (!isControlled) {
              setInternalSelectedKey(key)
            }
            onSelectedKeyChange?.(key)
          }

          return <li key={key}>{renderRow(item, { isSelected: key === currentSelectedKey, select })}</li>
        })}
      </ul>

      {selectedItem ? <Card>{renderDetail?.(selectedItem)}</Card> : null}
    </div>
  )
}
