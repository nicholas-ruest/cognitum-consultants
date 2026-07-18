//! bff-core: domain-agnostic aggregation/composition logic and this repo's own
//! aggregates and DTOs (see ADR-004, ../ddd/consultant-experience-context.md).
//! Depends only on nexus-client's trait interfaces, never its concrete
//! implementations. `persistence` depends on `bff-core` (not the reverse) so
//! it can implement the repository traits defined here (e.g.
//! [`ConsultantPreferencesRepository`]) against the ADR-010 datastore.

mod consultant_preferences;
mod preference_key;

pub use consultant_preferences::{
    ConsultantPreferences, ConsultantPreferencesError, ConsultantPreferencesRepository, RepoError,
};
pub use preference_key::{ParsePreferenceKeyError, PreferenceKey};
