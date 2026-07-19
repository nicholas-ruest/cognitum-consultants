# ADR-019: Dashboard Permission Matching — Capability-Family Prefix, Not Exact Module Id

## Status
Proposed

## Context
With ADR-018's push receiver live and nexus's own identity-token signature bug fixed (both confirmed working
in production this session — a real reaction event was received and processed, and `armor.assertions` now
returns `200 success:true` for a real, validly-signed caller), the dashboard should finally have real
permission data to work with. It still renders zero cards. Root-caused live, not guessed:

`crates/bff-core/src/dashboard_configuration.rs` gates every card behind an **exact string match**:
```rust
let is_permitted = |module_id: &str| assertions.iter().any(|assertion| assertion.capability == module_id);
```
against a **bare capability-family** default card list:
```rust
pub const DEFAULT_CARD_MODULE_IDS: [&str; 3] = ["sales", "commit", "execution"];
```
But every `PermissionAssertion.capability` Armor actually issues — confirmed live
(`POST /api/v1/capabilities/armor.assertions` → `{"assertions":[{"capability":"sales.account_claims",...},
{"capability":"commit.proposals",...},{"capability":"customer.context",...}]}`) — is a **granular, dotted
`{family}.{action}` id**, matching the exact convention `nexus-client`'s own capability constants already use
everywhere else in this repo (`sales.account_claims`, `sales.collaboration_requests`, `sales.referrals`,
`commit.proposals`, `commit.proposal_actions`, `execution.task_completions`, `capacity.profile`,
`customer.context`, `edu.catalog`, `products.catalog`, `landscape.intelligence`, `landscape.observations`,
`legal.clauses`, `armor.assertions` — see each gateway's own `CAPABILITY_*` constants, and ADR-029's envelope
`capability_id` field these all populate).

`"sales.account_claims" == "sales"` is `false`. So even a consultant holding real, valid assertions for every
one of `DEFAULT_CARD_MODULE_IDS`'s three intended families gets zero matches, and — per invariant 4's own
"zero cards is valid" fallback (`dashboard_configuration.rs` module docs) — silently renders an empty,
valid-looking dashboard rather than an error. This is why the dashboard has been empty through every fix so
far: this bug sits *downstream* of everything already fixed (routing, auth, token signature) and was never
reachable until this session's other fixes made it possible for a real assertion to arrive at all.

Separately, `DEFAULT_CARD_MODULE_IDS`'s own doc comment documents a deliberate, narrow original scope — "sales,
commit, execution" chosen as `domain-map.md`'s "primary, ongoing, transactional workspaces," explicitly
*excluding* Edu/Products/Landscape/Legal/Customer as "read-heavy/catalog/reference-only" and Capacity as
"narrow-access." That was a reasonable default absent other guidance, but it now conflicts with an explicit,
direct requirement: consultants need Products, Legal, Sales, Commit, and Edu (at minimum) visible as tools on
first login, not opt-in via a manual dashboard edit — "tools... several repos need to train and do their jobs."

## Decision
Two changes, addressing the bug and the now-explicit scope requirement separately (the first is required
regardless; the second is a scope revision informed by direct product input, not a consequence of the first):

1. **Permission matching becomes a capability-family prefix match, not exact equality.** A `module_id` (e.g.
   `"sales"`) is permitted when the consultant holds *any* assertion whose `capability` equals the module id
   exactly **or** starts with `"{module_id}."` — covering both a hypothetical bare family-level grant and the
   real per-action granular grants Armor actually issues. Implemented once, at the single real call site
   (`bff_api::permissions::PermissionCache`'s `is_permitted`/the closure `dashboard.rs` builds from it — see
   `dashboard_configuration.rs` module docs invariant 1 for why the check itself must stay outside `bff-core`,
   injected as a closure); `bff-core`'s own `DEFAULT_CARD_MODULE_IDS`/`CardPlacement` model is unchanged, since
   `module_id` already meant "capability family," this only fixes how that family is matched against Armor's
   real grant granularity.
2. **`DEFAULT_CARD_MODULE_IDS` expands from 3 to every capability family this repo actually integrates with**:
   `sales`, `commit`, `edu`, `capacity`, `customer`, `execution`, `products`, `landscape`, `legal` — every
   `nexus-client` gateway family except `armor` itself (a permission source, not a displayable capability
   surface). A first-login consultant now sees every card they're actually permitted for, not a hardcoded
   subset independent of their real grants; invariant 4's permission filter still applies per-card exactly as
   before, so a consultant genuinely un-permitted for a given family still won't see that card — this changes
   the *candidate* set offered by default, not the permission check itself.

## Consequences
**Positive**
- Unblocks the dashboard for real: this was the last of four independent, stacked failures found this session
  (wrong capability path → wrong events-poll model → nexus-side token-signature bug → this matching bug), each
  only reachable once the one before it was fixed.
- Matching logic now reads from the same capability-naming vocabulary this repo already depends on everywhere
  else (`nexus-client`'s `CAPABILITY_*` constants, ADR-029's envelope), rather than a second, disconnected
  naming scheme invented only for dashboard defaults.
- Consultants see their real permitted toolset on first login without needing to know a manual
  `PUT /api/dashboard` exists.

**Negative / Trade-offs**
- A consultant holding *any* single granular assertion in a family (e.g. only `sales.referrals`) now sees the
  whole family's card, even for actions they hold no assertion for — the card-level grant is coarser than the
  action-level assertion. Acceptable: ADR-009 already establishes this repo's filtering as presentation-layer
  defense-in-depth, never the real enforcement boundary — "every real mutation still goes through the BFF,
  which re-checks" against the actual per-action assertion before doing anything, so a coarser card-visibility
  grant cannot itself authorize an action it shouldn't.
- Expanding the default set revises `dashboard_configuration.rs`'s own documented "primary vs.
  catalog/reference" distinction. That categorization isn't wrong as a description of each capability's
  *relationship shape* (`domain-map.md`'s own framing) — it just turns out not to be the right basis for
  which cards a consultant should discover by default, now that there's explicit input on that question.

## Alternatives Considered
- **Fix this on Armor's/nexus's side instead — have Armor also grant bare family-level assertions
  (`"sales"`, not just `"sales.account_claims"`).** Rejected — directly conflicts with ADR-009's own
  established principle that this repo "is not permitted to have an opinion about what a consultant is
  allowed to do beyond relaying Armor's own assertions," including their granularity. Armor's capability
  vocabulary is not this repo's to redefine; the presentation layer must correctly interpret whatever
  granularity Armor actually issues, not ask Armor to accommodate this repo's own internal card model.
- **Keep exact matching; change `DEFAULT_CARD_MODULE_IDS` itself to the full granular action-id list instead
  of family names.** Rejected — a `CardPlacement` is meant to represent one capability *surface* (one card ⇒
  one tool a consultant opens), not one narrow action; a consultant permitted for `sales.referrals` but not
  `sales.account_claims` should still see (and be able to open) the Sales card, just with that action
  unavailable inside it — a family-level card with action-level enforcement inside it, not N micro-cards per
  family.
- **Leave the default set at 3 and rely on manual `PUT /api/dashboard` for the rest.** Rejected given the
  explicit, direct requirement that consultants see the full toolset without a manual setup step.

## Relationships
- Depends on: ADR-009 (permission-aware presentation — this ADR is a bug fix *within* that model, not a
  change to it), ADR-007/ADR-029 (the `{family}.{action}` capability-id vocabulary this now matches against),
  ADR-018 (the push-delivery + auth fixes that made a real assertion reachable in the first place).
- Informs: nothing downstream yet — this is presentation-layer only, touches no persisted shape
  (`CardPlacement.module_id` values already stored as bare family names remain valid; only the *matching*
  logic changes) and no wire contract the frontend depends on beyond what it already renders per card.
- Source docs: live production investigation this session (`armor.assertions` response, `dashboard_configuration.rs`
  invariants 1 and 4, `nexus-client`'s per-gateway `CAPABILITY_*` constants); `../ddd/domain-map.md` (the
  "primary vs. catalog/reference" categorization this ADR's point 2 revises); ADR-009 (the matching principle
  this ADR corrects an implementation gap in, not a policy this ADR changes).
