//! Centralized, versioned preference-key allow-list
//! (`consultant-experience-context.md` §1.2, `ConsultantPreferences`
//! invariant 1: "every preference key must belong to a known, versioned
//! allow-list of preference types ... this context does not accept
//! arbitrary key/value pairs").
//!
//! [`PreferenceKey`] is the allow-list: using it (rather than a raw
//! `String`) as a `HashMap` key structurally prevents an unknown key from
//! ever existing in a constructed [`crate::ConsultantPreferences`] — a
//! `HashMap<PreferenceKey, String>` cannot contain a variant that isn't
//! defined below. [`PreferenceKey::from_str`] additionally enforces the
//! same allow-list at data-entry boundaries (JSON/DB deserialization) where
//! a raw string could otherwise smuggle an unknown key in before it becomes
//! a typed value.
//!
//! v1 of the allow-list. Add new variants here as new preferences are
//! introduced; never repurpose an existing variant's wire string (`as_str`)
//! for a different meaning — that would silently corrupt already-persisted
//! JSONB data (see `persistence`'s `consultant_preferences` migration).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A known, allow-listed consultant preference key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreferenceKey {
    /// UI color theme (e.g. light/dark).
    Theme,
    /// The consultant's default landing view/page.
    DefaultView,
    /// Whether the consultant has opted in to a given notification
    /// channel. (Multiple opt-in preferences may exist per channel in a
    /// future revision; v1 models this as a single flag-shaped value.)
    NotificationChannelOptIn,
}

impl PreferenceKey {
    /// Every allow-listed variant, in declaration order. Useful for tests
    /// and for any future "list known preference keys" API.
    pub const ALL: [PreferenceKey; 3] =
        [PreferenceKey::Theme, PreferenceKey::DefaultView, PreferenceKey::NotificationChannelOptIn];

    /// The wire/storage string for this key (DB JSONB key, JSON key).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::DefaultView => "default_view",
            Self::NotificationChannelOptIn => "notification_channel_opt_in",
        }
    }
}

impl fmt::Display for PreferenceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for PreferenceKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// A preference key string that is not on the allow-list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown preference key: {0:?}")]
pub struct ParsePreferenceKeyError(String);

impl FromStr for PreferenceKey {
    type Err = ParsePreferenceKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "theme" => Ok(Self::Theme),
            "default_view" => Ok(Self::DefaultView),
            "notification_channel_opt_in" => Ok(Self::NotificationChannelOptIn),
            other => Err(ParsePreferenceKeyError(other.to_string())),
        }
    }
}

/// Hand-rolled (not `#[derive]`) so the wire representation is exactly
/// [`PreferenceKey::as_str`] — a plain JSON string, matching how this type
/// is used as a `HashMap` key serialized to a JSONB object key by
/// `persistence`.
impl Serialize for PreferenceKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Enforces the allow-list at the deserialization boundary: any string not
/// recognized by [`PreferenceKey::from_str`] fails here, so invalid keys
/// can never reach a constructed [`crate::ConsultantPreferences`] via
/// JSON/DB deserialization.
impl<'de> Deserialize<'de> for PreferenceKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn from_str_accepts_every_allow_listed_key() {
        for key in PreferenceKey::ALL {
            assert_eq!(key.as_str().parse::<PreferenceKey>().unwrap(), key);
        }
    }

    #[test]
    fn from_str_rejects_unknown_key() {
        let err = "not_a_real_preference".parse::<PreferenceKey>().unwrap_err();
        assert_eq!(err.to_string(), "unknown preference key: \"not_a_real_preference\"");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(PreferenceKey::Theme.to_string(), "theme");
    }

    /// Proves the allow-list is enforced at the JSON deserialization
    /// boundary, not merely by Rust's type system: a raw JSON string that
    /// isn't on the allow-list must fail to become a `PreferenceKey`, so
    /// invalid data arriving from a JSON/DB source can never reach a
    /// `ConsultantPreferences` aggregate.
    #[test]
    fn unknown_key_rejected_when_deserialized_from_json() {
        let result: Result<PreferenceKey, _> = serde_json::from_str("\"totally_unknown\"");
        assert!(result.is_err());
    }

    /// Same boundary proof, but for the shape `persistence` actually
    /// stores: a `HashMap<PreferenceKey, String>` decoded from a JSON
    /// object. An unknown key anywhere in the object must fail the whole
    /// deserialization rather than silently dropping/ignoring it.
    #[test]
    fn unknown_key_rejected_when_deserializing_preferences_map() {
        let json = r#"{"theme": "dark", "not_a_real_preference": "x"}"#;
        let result: Result<HashMap<PreferenceKey, String>, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn known_keys_map_deserializes_successfully() {
        let json = r#"{"theme": "dark", "default_view": "dashboard"}"#;
        let map: HashMap<PreferenceKey, String> = serde_json::from_str(json).unwrap();
        assert_eq!(map.get(&PreferenceKey::Theme), Some(&"dark".to_string()));
        assert_eq!(map.get(&PreferenceKey::DefaultView), Some(&"dashboard".to_string()));
    }
}
