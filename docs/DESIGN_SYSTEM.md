# Design System & Dashboard Components (PROMPT-42, ADR-017)

This repo's shared frontend UI lives in two packages under `packages/`, per
[ADR-017](../.plans/adr/ADR-017-design-system-packaging-strategy.md). This
doc summarizes their scope, current status, and how `frontend/` consumes
them; see the ADR itself for the full rationale.

## The two packages

**`@cognitum/design-system`** (`packages/design-system/`) — foundational
presentational primitives. Absorbs `frontend/src/components/` wholesale
(that directory no longer exists): `Alert`, `Button`, `Card`, `CardGrid`,
`Dialog`, `Header`, `Layout`, `Sidebar`, `TextInput`.

**`@cognitum/dashboard-components`** (`packages/dashboard-components/`) —
domain-specific dashboard patterns, built on top of `@cognitum/design-system`.
Exactly two components, matching what ADR-017's own investigation confirmed
as real, duplicated call sites (deliberately **not** filter/search or
dialog-usage abstractions — ADR-017 found no real call sites for either, and
building them now would be premature abstraction):

- **`ListDetailPanel`** — a generic list-of-items + optional
  selected-item-detail-panel component, built on `Card`. Replaces the
  hand-duplicated clickable-row-list idiom previously repeated across
  `frontend/src/features/{customer,products,execution,commit,edu,landscape,
  legal,notifications}/*`.
- **`CapabilityForm`** — a form wrapper built on `TextInput`/`Button`/`Alert`.
  Replaces the hand-duplicated "form + submit `Button` + mutation-error
  `Alert`" idiom previously repeated across
  `frontend/src/features/{capacity,commit,landscape,sales}/*`.

## Status: 0.1.0, workspace-linked, not yet published

Both packages are versioned `0.1.0` and currently consumed **only** via npm
workspace linking within this repo — they are not published to any external
registry, and no registry credentials exist yet. Per ADR-017, publishing to
a private registry (so `manage.cognitum.one`, a separate peer application,
can eventually adopt them as an ordinary versioned dependency) is an
explicitly deferred follow-up, not part of this unit of work.

Both packages ship TypeScript source directly — `main`/`types`/`exports` all
point at `./src/index.ts(x)`, with no bundler/dist build step. This is
sufficient for local workspace consumption via Vite + tsc project
references; a real bundled-publish pipeline is also deferred, alongside the
registry choice itself.

## How `frontend/` consumes them

The repo root `package.json` declares npm workspaces:

```json
{
  "private": true,
  "workspaces": ["frontend", "packages/*"]
}
```

`frontend/package.json` depends on both packages by ordinary semver range
(`"^0.1.0"`, not a workspace-protocol specifier — npm workspaces resolve a
dependency name matching a workspace member from the local tree
automatically):

```json
{
  "dependencies": {
    "@cognitum/dashboard-components": "^0.1.0",
    "@cognitum/design-system": "^0.1.0"
  }
}
```

Running `npm install` at the repo root links both packages into
`frontend/`'s dependency resolution via symlinks under the root
`node_modules/@cognitum/`. Feature code imports them like any other npm
package, e.g.:

```ts
import { Alert, Button, TextInput } from '@cognitum/design-system'
import { CapabilityForm, ListDetailPanel } from '@cognitum/dashboard-components'
```
