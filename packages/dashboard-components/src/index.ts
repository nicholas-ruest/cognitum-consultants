/**
 * `@cognitum/dashboard-components` -- domain-specific dashboard patterns
 * (ADR-017), built on top of `@cognitum/design-system`'s primitives.
 * Deliberately narrow scope: exactly the two confirmed-duplicated idioms
 * (list+detail, form+alert+submit) -- no filter/search or dialog-usage
 * abstractions, per ADR-017's own "no premature abstraction" finding.
 */

export { ListDetailPanel } from './ListDetailPanel'
export type { ListDetailPanelProps, ListDetailPanelRowHelpers } from './ListDetailPanel'

export { CapabilityForm } from './CapabilityForm'
export type { CapabilityFormProps, CapabilityFormAlert } from './CapabilityForm'
