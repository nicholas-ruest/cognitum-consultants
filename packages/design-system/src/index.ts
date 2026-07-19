/**
 * `@cognitum/design-system` -- foundational presentational primitives
 * (ADR-017). Absorbed wholesale from the PROMPT-17 dashboard shell
 * (formerly `frontend/src/components/`): pure, presentational, no
 * business logic, no API calls.
 */

export { Alert } from './Alert'
export type { AlertProps, AlertVariant } from './Alert'

export { Button } from './Button'
export type { ButtonProps, ButtonVariant } from './Button'

export { Card } from './Card'
export type { CardProps } from './Card'

export { CardGrid } from './CardGrid'
export type { CardGridProps } from './CardGrid'

export { Dialog } from './Dialog'
export type { DialogProps } from './Dialog'

export { Header } from './Header'
export type { HeaderProps } from './Header'

export { Layout } from './Layout'
export type { LayoutProps } from './Layout'

export { Sidebar, navItemsFromAssertions } from './Sidebar'
export type { SidebarProps, SidebarNavItem, CapabilityAssertion } from './Sidebar'

export { TextInput } from './TextInput'
export type { TextInputProps } from './TextInput'
