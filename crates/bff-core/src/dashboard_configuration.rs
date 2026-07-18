//! `DashboardConfiguration` aggregate (`consultant-experience-context.md`
//! §1.2) and its repository port (`DashboardConfigurationRepository`,
//! implemented against Postgres in `persistence`, ADR-010).
//!
//! Invariants enforced here:
//! 1. Every [`CardPlacement::module_id`] must reference a capability the
//!    consultant currently holds a Permission Assertion for. This aggregate
//!    cannot depend on `bff-api`'s concrete `PermissionCache` (ADR-004:
//!    `bff-core` depends only on `nexus-client`'s trait interfaces, never on
//!    `bff-api`, which is the crate that depends on everything else — the
//!    dependency only ever points inward). Instead, every constructor/
//!    mutator that needs to check this invariant takes the check as an
//!    injected `&dyn Fn(&str) -> bool` predicate over a `module_id`. The
//!    real, authoritative check
//!    (`bff_api::permissions::PermissionCache::is_permitted`) is `async`
//!    (it may hit Armor over the network on a cache miss), so `bff-api` is
//!    expected to resolve the *full* permission set for the consultant
//!    first (`PermissionCache::assertions_for`, already `await`ed) and then
//!    hand this aggregate a synchronous closure over that already-resolved,
//!    in-memory set — e.g. `|module_id: &str|
//!    assertions.iter().any(|a| a.capability == module_id)`. That keeps
//!    `bff-core` itself fully synchronous and framework/infra-agnostic
//!    (no `async_trait`, no knowledge of Armor/Nexus/HTTP), while the real
//!    permission decision still always comes from `bff-api`'s
//!    `PermissionCache`, never a fabricated/local one.
//! 2. Card positions are unique within one configuration — enforced by
//!    [`DashboardConfiguration::add_card`] rejecting a duplicate position,
//!    and re-checked by [`DashboardConfiguration::from_parts`] so a
//!    repository can never reconstruct an aggregate that violates it either
//!    (defense in depth alongside `persistence`'s
//!    `UNIQUE (consultant_id, card_position)` constraint).
//! 3. Exactly one aggregate per consultant — like `ConsultantPreferences`,
//!    not enforceable by this crate alone; satisfied by
//!    [`DashboardConfigurationRepository::save`]'s upsert-on-`consultant_id`
//!    semantics at the persistence boundary.
//! 4. A zero-card configuration is valid, but [`DashboardConfiguration::new`]
//!    applies a default card set at creation, itself filtered through the
//!    same permission check as invariant 1 (a default card the consultant
//!    isn't permitted to see must not appear either). See
//!    [`DEFAULT_CARD_MODULE_IDS`]'s doc comment for which capabilities were
//!    chosen and why — research.md does not specify default-card behavior,
//!    so this is a documented assumption, not a requirement traced to a
//!    spec line.

use std::fmt;

/// Default card set applied by [`DashboardConfiguration::new`], filtered
/// through the caller's permission check like any other card (invariant 1
/// applies to defaults too — see the module docs).
///
/// **Assumption** (`consultant-experience-context.md` §1.2 invariant 4 flags
/// this as unspecified by research.md): these three module ids —
/// `sales`, `commit`, `execution` — are the capabilities `domain-map.md`
/// describes as the consultant's primary, ongoing, transactional
/// workspaces (Sales: lead/customer ownership; Commit: proposals the
/// consultant "create[s] and manage[s] centrally"; Execution: "the
/// consultant's assigned delivery workspace"), as opposed to capabilities
/// `domain-map.md` characterizes as read-heavy/catalog/reference-only
/// (Edu, Products, Landscape, Legal, Customer) or deliberately
/// narrow-access (Capacity). This is a deliberately small, easily-revisited
/// default, not an attempt to invent business meaning beyond what
/// `domain-map.md` already states about each capability's relationship
/// shape.
pub const DEFAULT_CARD_MODULE_IDS: [&str; 3] = ["sales", "commit", "execution"];

/// A single dashboard card, placed at a fixed `position` and pointing at a
/// `module_id` (a capability name, e.g. `"sales"`, `"commit"` — matching the
/// capability names used elsewhere for Permission Assertions). Deliberately
/// a plain `String`, not a fixed Rust enum: new capabilities may be added
/// over time, and which ones exist is data-driven by Armor assertions, not
/// a closed set this crate should hardcode (unlike [`crate::PreferenceKey`],
/// which genuinely is a small, closed set).
///
/// Child entity of [`DashboardConfiguration`] — no independent identity or
/// lifecycle outside the configuration that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPlacement {
    module_id: String,
    position: u32,
}

impl CardPlacement {
    pub fn new(module_id: impl Into<String>, position: u32) -> Self {
        Self { module_id: module_id.into(), position }
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn position(&self) -> u32 {
        self.position
    }
}

/// A single consultant's dashboard composition. Root of its own aggregate;
/// contains [`CardPlacement`] child entities (`consultant-experience-context.md`
/// §1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardConfiguration {
    consultant_id: String,
    cards: Vec<CardPlacement>,
}

/// Errors constructing/mutating a [`DashboardConfiguration`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DashboardConfigurationError {
    /// `consultant_id` was empty/blank — every aggregate must reference a
    /// real consultant (input validation at the aggregate boundary).
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
    /// Invariant 1: the consultant has no Permission Assertion for this
    /// card's `module_id`.
    #[error("consultant is not permitted to place a card for module {0:?}")]
    ModuleNotPermitted(String),
    /// Invariant 2: another card already occupies this position.
    #[error("position {0} is already occupied by another card")]
    PositionAlreadyOccupied(u32),
}

impl fmt::Display for CardPlacement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.module_id, self.position)
    }
}

impl DashboardConfiguration {
    /// Constructs a fresh configuration for `consultant_id`, applying
    /// [`DEFAULT_CARD_MODULE_IDS`] filtered through `is_permitted` (invariant
    /// 4, itself subject to invariant 1 — see the module docs). A consultant
    /// permitted for none of the defaults ends up with a valid, empty
    /// configuration (invariant 4's "zero cards is valid" half), not an
    /// error.
    pub fn new(
        consultant_id: impl Into<String>,
        is_permitted: &dyn Fn(&str) -> bool,
    ) -> Result<Self, DashboardConfigurationError> {
        let consultant_id = consultant_id.into();
        if consultant_id.trim().is_empty() {
            return Err(DashboardConfigurationError::EmptyConsultantId);
        }

        let cards = DEFAULT_CARD_MODULE_IDS
            .iter()
            .filter(|module_id| is_permitted(module_id))
            .enumerate()
            .map(|(position, module_id)| CardPlacement::new(*module_id, position as u32))
            .collect();

        Ok(Self { consultant_id, cards })
    }

    /// Reconstructs an aggregate from already-known parts (e.g. a repository
    /// loading a persisted row). Re-validates `consultant_id` and invariant 2
    /// (unique positions) the same as construction would. Deliberately does
    /// **not** take an `is_permitted` check and so does not re-validate
    /// invariant 1 on every read: permissions can change after a card was
    /// legitimately placed, and re-validating invariant 1 for *already
    /// persisted* data is `consultant-experience-context.md` §1.3's job for
    /// the `PermissionAssertionChanged` consumer (a future event-driven
    /// sweep), not something every read should pay for or reject on.
    pub fn from_parts(
        consultant_id: String,
        cards: Vec<CardPlacement>,
    ) -> Result<Self, DashboardConfigurationError> {
        if consultant_id.trim().is_empty() {
            return Err(DashboardConfigurationError::EmptyConsultantId);
        }

        let mut seen_positions = std::collections::HashSet::new();
        for card in &cards {
            if !seen_positions.insert(card.position) {
                return Err(DashboardConfigurationError::PositionAlreadyOccupied(card.position));
            }
        }

        Ok(Self { consultant_id, cards })
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    pub fn cards(&self) -> &[CardPlacement] {
        &self.cards
    }

    /// Adds one card, enforcing invariants 1 and 2. Rejects (without
    /// mutating `self`) a `module_id` the consultant has no Permission
    /// Assertion for, or a `position` another card already occupies.
    pub fn add_card(
        &mut self,
        card: CardPlacement,
        is_permitted: &dyn Fn(&str) -> bool,
    ) -> Result<(), DashboardConfigurationError> {
        if !is_permitted(&card.module_id) {
            return Err(DashboardConfigurationError::ModuleNotPermitted(card.module_id));
        }
        if self.cards.iter().any(|existing| existing.position == card.position) {
            return Err(DashboardConfigurationError::PositionAlreadyOccupied(card.position));
        }

        self.cards.push(card);
        Ok(())
    }

    /// Removes the card at `position`, if any. Returns whether a card was
    /// removed. No permission check needed to remove a card (invariant 1
    /// only constrains what may be *added*).
    pub fn remove_card(&mut self, position: u32) -> bool {
        let before = self.cards.len();
        self.cards.retain(|card| card.position != position);
        self.cards.len() != before
    }
}

/// Repository port for [`DashboardConfiguration`]
/// (`consultant-experience-context.md` §1.4). Implemented against Postgres
/// in `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn DashboardConfigurationRepository>` in Axum application state,
/// matching `ConsultantPreferencesRepository`'s convention (PROMPT-20).
#[async_trait::async_trait]
pub trait DashboardConfigurationRepository: Send + Sync {
    /// Looks up a consultant's dashboard configuration. `Ok(None)` means no
    /// configuration has been saved yet (not an error — a freshly onboarded
    /// consultant has none until one is created via [`DashboardConfiguration::new`]
    /// and saved).
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Option<DashboardConfiguration>, crate::RepoError>;

    /// Persists the full aggregate. Upsert semantics on `consultant_id` —
    /// this is how invariant 3 ("exactly one configuration per consultant")
    /// is satisfied at the storage boundary: saving twice for the same
    /// consultant replaces the row (and its cards) rather than creating a
    /// second configuration.
    async fn save(&self, config: &DashboardConfiguration) -> Result<(), crate::RepoError>;

    /// Deletes a consultant's configuration entirely (e.g. on offboarding).
    /// Not an error if no configuration existed.
    async fn delete_by_consultant_id(&self, consultant_id: &str) -> Result<(), crate::RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_permitted(_module_id: &str) -> bool {
        true
    }

    fn none_permitted(_module_id: &str) -> bool {
        false
    }

    fn only(allowed: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |module_id: &str| allowed.contains(&module_id)
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err = DashboardConfiguration::new("", &all_permitted).unwrap_err();
        assert_eq!(err, DashboardConfigurationError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_blank_consultant_id() {
        let err = DashboardConfiguration::new("   ", &all_permitted).unwrap_err();
        assert_eq!(err, DashboardConfigurationError::EmptyConsultantId);
    }

    /// Invariant 1 applies even to defaults: an all-false permission check
    /// must produce zero cards, not an error and not the defaults anyway.
    #[test]
    fn new_with_no_permissions_produces_zero_cards() {
        let config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();
        assert_eq!(config.consultant_id(), "consultant-1");
        assert!(config.cards().is_empty());
    }

    /// Invariant 4: an all-true permission check produces the chosen
    /// default card set.
    #[test]
    fn new_with_all_permissions_produces_the_default_card_set() {
        let config = DashboardConfiguration::new("consultant-1", &all_permitted).unwrap();

        let module_ids: Vec<&str> = config.cards().iter().map(CardPlacement::module_id).collect();
        assert_eq!(module_ids, DEFAULT_CARD_MODULE_IDS.to_vec());

        let positions: Vec<u32> = config.cards().iter().map(CardPlacement::position).collect();
        assert_eq!(positions, vec![0, 1, 2]);
    }

    /// A partial permission set only yields the permitted defaults.
    #[test]
    fn new_with_partial_permissions_filters_defaults() {
        let config = DashboardConfiguration::new("consultant-1", &only(&["commit"])).unwrap();

        let module_ids: Vec<&str> = config.cards().iter().map(CardPlacement::module_id).collect();
        assert_eq!(module_ids, vec!["commit"]);
    }

    /// The acceptance-criteria-required test: adding a card for a module the
    /// consultant has no Permission Assertion for must be rejected.
    #[test]
    fn add_card_rejects_a_card_without_permission() {
        let mut config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();

        let err = config
            .add_card(CardPlacement::new("sales", 0), &none_permitted)
            .unwrap_err();

        assert_eq!(err, DashboardConfigurationError::ModuleNotPermitted("sales".to_string()));
        assert!(config.cards().is_empty());
    }

    #[test]
    fn add_card_accepts_a_permitted_module() {
        let mut config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();

        config.add_card(CardPlacement::new("sales", 0), &all_permitted).unwrap();

        assert_eq!(config.cards().len(), 1);
        assert_eq!(config.cards()[0].module_id(), "sales");
    }

    /// Invariant 2: two cards cannot share a position.
    #[test]
    fn add_card_rejects_a_duplicate_position() {
        let mut config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();
        config.add_card(CardPlacement::new("sales", 0), &all_permitted).unwrap();

        let err = config
            .add_card(CardPlacement::new("commit", 0), &all_permitted)
            .unwrap_err();

        assert_eq!(err, DashboardConfigurationError::PositionAlreadyOccupied(0));
        assert_eq!(config.cards().len(), 1);
    }

    #[test]
    fn remove_card_removes_an_existing_position_and_reports_true() {
        let mut config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();
        config.add_card(CardPlacement::new("sales", 0), &all_permitted).unwrap();

        assert!(config.remove_card(0));
        assert!(config.cards().is_empty());
    }

    #[test]
    fn remove_card_reports_false_for_a_missing_position() {
        let mut config = DashboardConfiguration::new("consultant-1", &none_permitted).unwrap();
        assert!(!config.remove_card(0));
    }

    #[test]
    fn from_parts_reconstructs_and_revalidates_consultant_id() {
        let cards = vec![CardPlacement::new("sales", 0)];
        let config = DashboardConfiguration::from_parts("consultant-1".to_string(), cards).unwrap();
        assert_eq!(config.cards().len(), 1);

        let err =
            DashboardConfiguration::from_parts("".to_string(), Vec::new()).unwrap_err();
        assert_eq!(err, DashboardConfigurationError::EmptyConsultantId);
    }

    /// `from_parts` re-checks invariant 2 too — a repository must not be
    /// able to reconstruct an aggregate that violates it, even if
    /// (hypothetically) corrupt/pre-invariant data made it into storage.
    #[test]
    fn from_parts_rejects_duplicate_positions() {
        let cards = vec![CardPlacement::new("sales", 0), CardPlacement::new("commit", 0)];
        let err = DashboardConfiguration::from_parts("consultant-1".to_string(), cards).unwrap_err();
        assert_eq!(err, DashboardConfigurationError::PositionAlreadyOccupied(0));
    }
}
