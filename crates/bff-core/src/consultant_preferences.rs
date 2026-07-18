//! `ConsultantPreferences` aggregate (`consultant-experience-context.md`
//! §1.2) and its repository port (`ConsultantPreferencesRepository`,
//! implemented against Postgres in `persistence`, ADR-010).
//!
//! Invariants enforced here:
//! 1. Every preference key belongs to the [`crate::PreferenceKey`]
//!    allow-list — structurally guaranteed by using it as the `HashMap`
//!    key type (see `preference_key.rs` for the data-entry-boundary half
//!    of this invariant).
//! 2. Exactly one aggregate per consultant — this crate cannot enforce
//!    that on its own (it requires a uniqueness check across storage), so
//!    it is satisfied by [`ConsultantPreferencesRepository::save`]'s
//!    upsert-on-`consultant_id` semantics at the persistence boundary.
//! 3. Preference values never encode business data (e.g. a cached record) —
//!    not mechanically enforceable in code; this is a modeling convention
//!    documented here and expected to hold by construction of what callers
//!    choose to store. Values are opaque `String`s to this aggregate: a
//!    richer, per-key typed value (e.g. an enum for `theme`) is a possible
//!    future refinement, not required for v1's three keys.

use std::collections::HashMap;

use crate::PreferenceKey;

/// A single consultant's preference bag. Root of its own aggregate — no
/// child entities (`consultant-experience-context.md` §1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsultantPreferences {
    consultant_id: String,
    preferences: HashMap<PreferenceKey, String>,
}

/// Errors constructing/mutating a [`ConsultantPreferences`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConsultantPreferencesError {
    /// `consultant_id` was empty/blank — every aggregate must reference a
    /// real consultant (input validation at the aggregate boundary).
    #[error("consultant_id must not be empty")]
    EmptyConsultantId,
}

impl ConsultantPreferences {
    /// Constructs a fresh, empty preference set for `consultant_id`.
    pub fn new(consultant_id: impl Into<String>) -> Result<Self, ConsultantPreferencesError> {
        Self::from_parts(consultant_id.into(), HashMap::new())
    }

    /// Reconstructs an aggregate from already-known parts (e.g. a
    /// repository loading a persisted row). Re-validates
    /// `consultant_id` the same as [`Self::new`] — a persisted row is not
    /// exempt from the aggregate's own invariants.
    pub fn from_parts(
        consultant_id: String,
        preferences: HashMap<PreferenceKey, String>,
    ) -> Result<Self, ConsultantPreferencesError> {
        if consultant_id.trim().is_empty() {
            return Err(ConsultantPreferencesError::EmptyConsultantId);
        }
        Ok(Self { consultant_id, preferences })
    }

    pub fn consultant_id(&self) -> &str {
        &self.consultant_id
    }

    /// Every preference currently set. `PreferenceKey` being the map key
    /// (rather than `String`) is what makes "only allow-listed keys" a
    /// property enforced by the type itself.
    pub fn preferences(&self) -> &HashMap<PreferenceKey, String> {
        &self.preferences
    }

    pub fn get_preference(&self, key: PreferenceKey) -> Option<&str> {
        self.preferences.get(&key).map(String::as_str)
    }

    /// Sets (or overwrites) one preference. Takes a typed `PreferenceKey`,
    /// not a `String` — an unknown key cannot be expressed here at all, so
    /// invariant 1 is a compile-time property for any Rust caller of this
    /// method.
    pub fn set_preference(&mut self, key: PreferenceKey, value: String) {
        self.preferences.insert(key, value);
    }
}

/// Errors a [`ConsultantPreferencesRepository`] can return.
///
/// Deliberately a small, `bff-core`-local error type rather than reusing
/// `sqlx::Error` directly — same trait-boundary discipline as `auth`'s
/// `SessionError` (PROMPT-10): `bff-core` stays a trait-interface-only
/// dependency, uncoupled from `persistence`'s storage error type. A real,
/// Postgres-backed implementation (`persistence`, ADR-010) is expected to
/// map `sqlx::Error` into this type at its own boundary.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// The underlying store could not be reached/used at all (e.g. a
    /// database connection failure).
    #[error("preferences store unavailable: {0}")]
    StoreUnavailable(String),
    /// The store was reachable, but the operation itself failed.
    #[error("preferences operation failed: {0}")]
    OperationFailed(String),
}

/// Repository port for [`ConsultantPreferences`]
/// (`consultant-experience-context.md` §1.4). Implemented against Postgres
/// in `persistence` (ADR-010); `bff-core` only defines the interface, per
/// ADR-004's trait-interface-only dependency direction.
///
/// `Send + Sync` so implementations can be shared behind an
/// `Arc<dyn ConsultantPreferencesRepository>` in Axum application state,
/// matching `auth::SessionProvider`'s convention.
#[async_trait::async_trait]
pub trait ConsultantPreferencesRepository: Send + Sync {
    /// Looks up a consultant's preferences. `Ok(None)` means no
    /// preferences have been saved yet (not an error — a freshly onboarded
    /// consultant has none).
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Option<ConsultantPreferences>, RepoError>;

    /// Persists the full aggregate. Upsert semantics on `consultant_id` —
    /// this is how invariant 2 ("exactly one aggregate per consultant") is
    /// satisfied at the storage boundary: saving twice for the same
    /// consultant replaces the row rather than creating a second one.
    async fn save(&self, prefs: &ConsultantPreferences) -> Result<(), RepoError>;

    /// Narrow, single-preference upsert — for a future BFF PATCH-style
    /// endpoint that updates one preference without round-tripping the
    /// whole aggregate. Same upsert-on-`consultant_id` semantics as
    /// [`Self::save`].
    async fn upsert_preference(
        &self,
        consultant_id: &str,
        key: PreferenceKey,
        value: String,
    ) -> Result<(), RepoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_constructs_an_empty_preference_set() {
        let prefs = ConsultantPreferences::new("consultant-1").unwrap();
        assert_eq!(prefs.consultant_id(), "consultant-1");
        assert!(prefs.preferences().is_empty());
    }

    #[test]
    fn new_rejects_empty_consultant_id() {
        let err = ConsultantPreferences::new("").unwrap_err();
        assert_eq!(err, ConsultantPreferencesError::EmptyConsultantId);
    }

    #[test]
    fn new_rejects_blank_consultant_id() {
        let err = ConsultantPreferences::new("   ").unwrap_err();
        assert_eq!(err, ConsultantPreferencesError::EmptyConsultantId);
    }

    #[test]
    fn set_preference_accepts_every_allow_listed_key() {
        let mut prefs = ConsultantPreferences::new("consultant-1").unwrap();
        prefs.set_preference(PreferenceKey::Theme, "dark".to_string());
        prefs.set_preference(PreferenceKey::DefaultView, "dashboard".to_string());
        prefs.set_preference(PreferenceKey::NotificationChannelOptIn, "email".to_string());

        assert_eq!(prefs.get_preference(PreferenceKey::Theme), Some("dark"));
        assert_eq!(prefs.get_preference(PreferenceKey::DefaultView), Some("dashboard"));
        assert_eq!(prefs.get_preference(PreferenceKey::NotificationChannelOptIn), Some("email"));
        assert_eq!(prefs.preferences().len(), 3);
    }

    #[test]
    fn set_preference_overwrites_an_existing_value() {
        let mut prefs = ConsultantPreferences::new("consultant-1").unwrap();
        prefs.set_preference(PreferenceKey::Theme, "dark".to_string());
        prefs.set_preference(PreferenceKey::Theme, "light".to_string());

        assert_eq!(prefs.get_preference(PreferenceKey::Theme), Some("light"));
        assert_eq!(prefs.preferences().len(), 1);
    }

    #[test]
    fn from_parts_reconstructs_and_revalidates() {
        let mut preferences = HashMap::new();
        preferences.insert(PreferenceKey::Theme, "dark".to_string());

        let prefs = ConsultantPreferences::from_parts("consultant-1".to_string(), preferences).unwrap();
        assert_eq!(prefs.get_preference(PreferenceKey::Theme), Some("dark"));

        let err = ConsultantPreferences::from_parts("".to_string(), HashMap::new()).unwrap_err();
        assert_eq!(err, ConsultantPreferencesError::EmptyConsultantId);
    }
}
