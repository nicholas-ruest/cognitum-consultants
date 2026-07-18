# The Sales Flow Pattern (reference template for PROMPT-34–41)

The lead-conflict-warning flow (Sales ACL) is the first capability built end-to-end in this repo — DTO
through gateway through BFF handler through frontend feature module through every test layer, including a
full Playwright e2e run against the real stack. Phase 4 (`PROMPT-34` onward: Commit, Edu, Capacity, Customer,
Execution, Products, Landscape, Legal — the remaining eight ACL integrations, per
[`.plans/implementation-plan.md`](../.plans/implementation-plan.md)) replicates this same five-layer shape
for each capability. This document is the checklist and file-path map for doing that.

Governing sources: [`.plans/ddd/anti-corruption-layers.md`](../.plans/ddd/anti-corruption-layers.md) §1 (the
worked example every layer below implements), [ADR-016](../.plans/adr/ADR-016-resilience-partial-failure-nexus-aggregation.md)
(resilience/timeout budgets), [ADR-009](../.plans/adr/ADR-009-authorization-permission-aware-presentation.md)
(the permission gate), [ADR-013](../.plans/adr/ADR-013-testing-strategy.md) (the five test layers this
document's §5 maps to).

---

## 1. DTO shape

**File**: [`crates/nexus-client/src/sales.rs`](../crates/nexus-client/src/sales.rs)

Three pieces, all `pub` on the gateway module:

- **The verdict DTO** — `AccountClaimResult { match_status, creation_allowed, display_message,
  permitted_actions }`. This is Sales' *opaque policy verdict*, never re-derived. It derives both
  `Serialize` and `Deserialize` deliberately: `Deserialize` to decode Sales' response, `Serialize` so
  `bff-api` can relay the exact same struct back out to the frontend with zero re-shaping (see §3).
- **The outbound query-shaped command** — `CheckAccountClaimCommand { company_name, normalized_domain?,
  consultant_id }`. A query in DDD terms (no side effect), but requires the write-timeout budget because it's
  a synchronous, user-blocking call (see §2).
- **The outbound side-effecting commands** — `RequestCollaborationCommand { company_reference,
  consultant_id, message? }` and `SubmitReferralCommand { company_reference, consultant_id, notes? }`. Both
  are **not** idempotent-safe to retry (each creates a record in Sales).

Match this shape exactly for a new capability by reading its own entry in `anti-corruption-layers.md` (§2
Commit, §3 Edu, §4 Capacity, etc.) — the "Crosses the boundary as" / "Outbound" / "Inbound" bullets there are
the DTO fields, command names, and event names to define, in the same "one query-shaped read + N
side-effecting commands" shape where applicable. Not every capability has this exact shape (Capacity's ACL,
for example, is deliberately write-heavy/read-narrow — see its `anti-corruption-layers.md` §4 entry) — read
the entry before assuming a 1:1 template match.

**Critical invariant to replicate**: never add a field, method, or branch that inspects or overrides the
verdict DTO's policy fields (here, `creation_allowed`). The owning capability's decision crosses the boundary
once and is relayed, not re-computed, at every downstream layer (gateway → BFF → frontend).

---

## 2. Gateway trait + implementation pattern

**File**: [`crates/nexus-client/src/sales.rs`](../crates/nexus-client/src/sales.rs) (trait `SalesGateway` +
impl `NexusSalesGateway`)

- One `#[async_trait] pub trait <Capability>Gateway: Send + Sync` per capability, with one method per
  outbound command/query from §1.
- One `Nexus<Capability>Gateway` struct implementing it, holding a single `transport: Arc<dyn
  NexusTransport>` field. **The constructor (`::new`) does not assemble the timeout/retry/circuit-breaker
  stack itself** — see `NexusArmorGateway::new`'s doc comment in
  [`crates/nexus-client/src/armor.rs`](../crates/nexus-client/src/armor.rs) for the convention this follows;
  composition happens once, at the call site in `main.rs` (§3).
- **Two-gateway-instances-for-different-retry-profiles convention**: if the capability has both a read (safe
  to retry) and side-effecting commands (not safe to retry) sharing one gateway struct with one `transport`
  field, you cannot give both the retry behavior they each need from a single instance. `sales.rs`'s module
  doc comment (top of the file) spells out why two `NexusSalesGateway` instances are constructed in `main.rs`
  rather than one: `sales_query_gateway` (retry-wrapped, `check_account_claim` only) and
  `sales_command_gateway` (no retry, `request_collaboration`/`submit_referral`). Replicate this whenever a
  new capability's ACL mixes idempotent reads and non-idempotent commands.
- **Timeout budget choice**: `crates/nexus-client/src/timeout.rs` defines `DEFAULT_READ_TIMEOUT` (5s) and
  `DEFAULT_WRITE_TIMEOUT` (3s). Armor's `fetch_assertions` uses the read budget (a background lookup). Sales'
  `check_account_claim` uses the **write** budget even though it's a DDD-level read, because it is a
  synchronous call the consultant is actively waiting on in the UI — see `NexusSalesGateway`'s doc comment
  for the exact reasoning. Pick per-method based on *is a human blocked on this call right now*, not on
  whether it's a read or a write in DDD terms.

Wiring in `main.rs`: see [`crates/bff-api/src/main.rs`](../crates/bff-api/src/main.rs) lines ~84–105
(`sales_base_transport` → two decorated transports → two `NexusSalesGateway` instances) as the literal
template — copy this block, rename the capability, and adjust which methods go on which gateway per your
capability's own idempotency split.

---

## 3. BFF handler pattern

**File**: [`crates/bff-api/src/sales.rs`](../crates/bff-api/src/sales.rs)

Every route handler follows this exact shape, in this order:

1. **Permission short-circuit before any gateway call.** `state.permission_cache.is_permitted(&session
   .consultant_id, "<capability>").await` — if `false`, return `403` immediately (`forbidden()` helper) and
   *never* call the gateway. `sales.rs`'s tests prove this with an explicit gateway call-count assertion
   (`assert_eq!(mock_gateway.calls(), 0, ...)`, e.g.
   `lead_conflict_check_returns_403_and_never_calls_the_gateway_when_unpermitted`) — write the equivalent
   test for every new route.
2. **Verbatim relay, no re-adjudication.** The query handler (`lead_conflict_check`) calls the gateway and,
   on success, does `Json(result).into_response()` directly on the gateway's own DTO — no parallel "BFF
   response" struct that copies fields one at a time (a copy step is exactly where an accidental
   re-derivation could sneak in). Command handlers (`request_collaboration`, `submit_referral`) return a
   minimal `{"status": "ok"}` ack, since neither has a real acked-response DTO yet (see `sales.rs`'s module
   docs, "Ack response shape" section, for why).
3. **Error handling: `502`, never a coerced success.** A gateway error (timeout, transport failure, malformed
   response) maps to `502 Bad Gateway` via `sales_unavailable()`. It is never turned into a synthetic
   `200`/`creation_allowed: true` — "Sales is unavailable" and "Sales says this is fine" must stay
   distinguishable outcomes. `sales.rs`'s
   `lead_conflict_check_never_returns_creation_allowed_true_when_the_gateway_errors` test is the template for
   proving this per-capability.
4. **Router**: one `<capability>_router(state: AppState) -> Router<AppState>` function per capability module,
   with `session::require_session` layered on (same pattern `sales_router` uses), then `.merge`d into
   `api_router` in `main.rs`.

`AppState` (`crates/bff-api/src/session.rs`) needs one `Arc<dyn <Capability>Gateway>` field per gateway
instance from §2 (e.g. `sales_query_gateway`, `sales_command_gateway`) — add them there, thread them through
every existing test's `AppState { ... }` construction (grep for `sales_query_gateway:` across `crates/bff-api/src`
to find every call site that needs the new fields added, including other modules' tests that just need an
`Unused<Capability>Gateway` stub — see `dashboard.rs`'s `UnusedSalesGateway` test double for that pattern).

---

## 4. Frontend feature module pattern

**File**: [`frontend/src/features/sales/LeadConflictCheck.tsx`](../frontend/src/features/sales/LeadConflictCheck.tsx)

- **`useMutation`, never `useQuery`, for the verdict check.** ADR-015's explicit rule: a conflict-check
  result must never be cached/reused across a different company entry — every submission is a fresh
  mutation, not a query keyed for reuse. `checkMutation.reset()` is called *before* `mutate()` fires a new
  check, so a stale previous result can never linger visually while the new one is in flight.
- **Action-id-driven button rendering, never hardcoded.** The component maps over `result.permitted_actions`
  (an array of ids from the DTO) and renders one button per entry via an `ActionButton` switch — it does not
  hardcode "always show these three buttons." `KNOWN_ACTION_LABELS` maps known ids to human labels.
- **Defensive handling of unrecognized action ids.** `ActionButton`'s `switch` has a `default` arm
  (`humanizeActionId`) that renders a generically-labeled, inert button for any id it doesn't recognize,
  rather than crashing or silently dropping a permitted action the backend actually granted. Copy this
  `switch`-with-default shape for a new capability's own action-id set.
- **Company/entity reference threading.** `companyReference = checkMutation.variables ?? companyName` — the
  reference used by follow-up commands must track the *result currently displayed*, not whatever is
  currently typed in the input, since the consultant may have started editing again before clicking an
  action button. TanStack Query's `variables` on a mutation always reflects its own most recent `mutate()`
  call, which is what makes this safe.
- **Alert variant mapping** (`alertVariantFor`): map the verdict's boolean/enum policy field to a semantic
  `Alert` variant (`info`/`warning`/`error`) — this is a *rendering* choice, not a re-adjudication of the
  verdict itself (the displayed text is always `display_message` verbatim regardless of variant chosen).

The module lives at `frontend/src/features/<capability>/`, one directory per capability (the `.gitkeep`
placeholders at `frontend/src/features/{commit,edu,capacity,customer,execution,products,landscape,legal}/`
mark where each Phase 4 module goes). Wire the finished component into `DashboardPage.tsx`'s
`card.module_id === '<capability>'` branch (see the existing `'sales'` branch) once it exists.

---

## 5. Testing pattern at each layer (ADR-013)

| Layer | What | Where (Sales' concrete example) |
|---|---|---|
| 2 — gateway contract | `wiremock` fixtures, one per `match_status`/scenario, plus a request-body-shape assertion and a malformed-response-doesn't-panic case | [`crates/nexus-client/tests/sales_gateway.rs`](../crates/nexus-client/tests/sales_gateway.rs) — `parses_active_owned_account_worked_example`, `parses_available_claim_fixture`, `parses_no_match_fixture`, `check_account_claim_sends_correct_command_body`, `returns_gateway_error_not_panic_on_malformed_account_claim_response` |
| 3 — BFF integration | `tower::oneshot` against the real `Router`, with a **test-double gateway** (not wiremock — the gateway trait is mocked directly in-process) and a real, migrated `testcontainers` Postgres | [`crates/bff-api/src/sales.rs`](../crates/bff-api/src/sales.rs) `#[cfg(test)] mod tests` — `MockSalesGateway` with a shared `AtomicUsize` call counter, used to prove both "relays verbatim on success" and "never calls the gateway when unpermitted" |
| 4 — frontend component | Vitest + React Testing Library, `fetch` mocked per-URL (`vi.stubGlobal('fetch', ...)`), no real network or backend | [`frontend/src/features/sales/LeadConflictCheck.test.tsx`](../frontend/src/features/sales/LeadConflictCheck.test.tsx) — covers the mutation call shape, conditional button rendering (including the empty and unrecognized-action-id cases), and the stale-result-cleared-before-new-result assertion |
| 5 — e2e (top layer, canonical smoke test) | Playwright against the **full real stack** (real frontend, real `bff-api`, real Postgres), Nexus mocked at the HTTP boundary | [`frontend/e2e/sales-lead-conflict.spec.ts`](../frontend/e2e/sales-lead-conflict.spec.ts) + orchestration in `frontend/e2e/support/` (see below) |

### The e2e orchestration is reusable as-is

`frontend/e2e/support/` has three modules a new Phase 4 e2e spec can reuse **unchanged**:

- [`global-setup.ts`](../frontend/e2e/support/global-setup.ts) — Playwright's `globalSetup` entry point,
  already wired into [`playwright.config.ts`](../frontend/playwright.config.ts). Runs once for the whole
  `frontend/e2e/` suite, regardless of how many spec files exist.
- [`test-stack.ts`](../frontend/e2e/support/test-stack.ts) — starts a throwaway, migrated Postgres container
  and the real `bff-api` binary pointed at it and at a given Nexus base URL.
- [`mock-nexus-server.ts`](../frontend/e2e/support/mock-nexus-server.ts) — a plain `node:http` mock Nexus
  server. **This is the one file a new capability's e2e coverage needs to extend**: add the new capability's
  `armor/v1/assertions` grant (if a different capability string) and its own `<capability>/v1/...` routes
  with fixture responses, following the existing Sales routes as the template. Keep the `/_test/...`
  inspection-route convention (`GET /_test/<log-name>`) so a spec in a different worker process can assert
  what the mock server received, the same way `sales-lead-conflict.spec.ts` asserts against
  `/_test/collaboration-requests`.

A new e2e spec, in most cases, only needs a **new spec file** (`frontend/e2e/<capability>-<flow>.spec.ts`)
plus the mock-server route additions above — not new orchestration.

---

## 6. Checklist for a new capability (PROMPT-34–41)

Work top-to-bottom; each step names its template file(s) from this repo to copy/adapt:

1. **Read the capability's `anti-corruption-layers.md` entry** (§2–§9) for its DTO shape, outbound
   commands, and inbound events. Confirm which calls are idempotent reads vs. non-idempotent commands.
2. **Gateway module** — new file `crates/nexus-client/src/<capability>.rs`, modeled on
   [`sales.rs`](../crates/nexus-client/src/sales.rs): DTO struct(s), command struct(s), `<Capability>GatewayError`
   enum, `#[async_trait] pub trait <Capability>Gateway`, `Nexus<Capability>Gateway` impl. Register the module
   in [`crates/nexus-client/src/lib.rs`](../crates/nexus-client/src/lib.rs).
3. **Gateway contract tests** — new file `crates/nexus-client/tests/<capability>_gateway.rs`, modeled on
   [`sales_gateway.rs`](../crates/nexus-client/tests/sales_gateway.rs): one `wiremock` fixture per
   `match_status`/scenario the capability's worked example describes, a request-body-shape assertion, and a
   malformed-response error-not-panic case.
4. **`main.rs` wiring** — add the transport/gateway construction block (§2), following the `sales_*`
   block in [`crates/bff-api/src/main.rs`](../crates/bff-api/src/main.rs) lines ~84–105. Add the new
   `Arc<dyn <Capability>Gateway>` field(s) to `AppState` in
   [`crates/bff-api/src/session.rs`](../crates/bff-api/src/session.rs).
5. **BFF handler module** — new file `crates/bff-api/src/<capability>.rs`, modeled on
   [`sales.rs`](../crates/bff-api/src/sales.rs): permission short-circuit, verbatim relay, `502` on gateway
   error, `<capability>_router` function. `.merge()` it into `api_router` in `main.rs`.
6. **BFF integration tests** — in the same file's `#[cfg(test)] mod tests`, modeled on `sales.rs`'s tests:
   a `Mock<Capability>Gateway` test double with a call counter, permission-gated and verbatim-relay
   assertions, `testcontainers` Postgres for the real `AppState`. Update every *other* test module's
   `AppState { ... }` construction (e.g. `dashboard.rs`'s tests) to supply a stub for the new gateway
   field(s) — see `UnusedSalesGateway` there for the pattern.
7. **Frontend feature module** — new directory `frontend/src/features/<capability>/`, modeled on
   [`LeadConflictCheck.tsx`](../frontend/src/features/sales/LeadConflictCheck.tsx): `useMutation` for the
   non-cacheable verdict/action, action-id-driven rendering with a defensive `default` case. Wire it into
   `DashboardPage.tsx`'s `card.module_id === '<capability>'` branch.
8. **Frontend component tests** — modeled on
   [`LeadConflictCheck.test.tsx`](../frontend/src/features/sales/LeadConflictCheck.test.tsx): mocked-`fetch`
   coverage of the mutation call shape and conditional rendering, including empty and unrecognized-action-id
   cases.
9. **E2e spec** — new file `frontend/e2e/<capability>-<flow>.spec.ts`, modeled on
   [`sales-lead-conflict.spec.ts`](../frontend/e2e/sales-lead-conflict.spec.ts): login → dashboard renders
   the new card/nav item → drive the flow → assert the mock Nexus server (extended per §5) received the
   expected command(s). No new orchestration files needed — `playwright.config.ts`'s existing `globalSetup`
   covers it.
10. **Run the full local gate** before opening a PR: `cargo check --workspace && cargo clippy --workspace
    --all-targets -- -D warnings && cargo test --workspace`, then in `frontend/`: `npm run build && npm run
    lint && npm run test && npm run test:e2e`. See [`docs/ci.md`](ci.md) for what CI itself runs (`rust`,
    `frontend`, and `e2e` jobs).
