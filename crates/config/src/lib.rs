//! config: typed application configuration loaded from environment
//! variables, with sensible dev defaults (ADR-004's `config` crate
//! responsibility; ADR-014's 12-factor env-var configuration convention).
//!
//! Deliberately dependency-free: `std::env::var` is sufficient for this
//! small, well-understood set of variables. Note crates.io publishes a
//! package also named `config`, which would collide (by Cargo dependency
//! key) with this workspace's own `config` crate if depended on directly —
//! flagged as a risk during PROMPT-01. That crate is intentionally not used
//! here; `std::env::var` covers this scope with no extra dependency.
//!
//! Kept simple and decoupled on purpose (per this crate's own mandate): just
//! env-var parsing and dev-default fallback, no domain logic.

use std::env;

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const PORT_ENV: &str = "PORT";
const LOG_LEVEL_ENV: &str = "RUST_LOG";
const NEXUS_ENDPOINT_URL_ENV: &str = "NEXUS_ENDPOINT_URL";

const DEFAULT_DATABASE_URL: &str = "postgres://localhost:5432/cognitum_consultants";
const DEFAULT_PORT: u16 = 3000;
const DEFAULT_LOG_LEVEL: &str = "info";
// Real Nexus endpoint is not yet known (ADR-014 leaves the orchestrator/
// environment generic); this is a dev-only placeholder, TBD once a target
// environment is confirmed.
const DEFAULT_NEXUS_ENDPOINT_URL: &str = "http://localhost:8080";

/// Application configuration, loaded once at startup from environment
/// variables (falling back to dev defaults when a variable is unset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Postgres connection string (ADR-010). May contain credentials in a
    /// real deployment — do not log this field verbatim; use
    /// [`Config::redacted_database_url`] instead.
    pub database_url: String,
    /// TCP port `bff-api` binds to.
    pub port: u16,
    /// Log verbosity, in `tracing-subscriber` `EnvFilter` syntax (e.g.
    /// `info`, `debug`, `bff_api=debug,info`). Sourced from `RUST_LOG` — the
    /// same variable `bff-api`'s `telemetry` module already reads directly
    /// via `EnvFilter::try_from_default_env()` (ADR-012) — rather than a
    /// second, redundant `LOG_LEVEL` variable. This field exists so the
    /// resolved value can be surfaced alongside the rest of startup config
    /// (e.g. logged), not to replace `telemetry`'s own `EnvFilter` parsing:
    /// `EnvFilter` directive syntax is `tracing`-specific behavior, and
    /// duplicating it here would be exactly the kind of domain logic this
    /// crate is meant to stay free of.
    ///
    /// `LOG_FORMAT` (pretty vs. JSON output rendering) deliberately stays
    /// out of this struct: it is consumed in exactly one place
    /// (`bff-api`'s `telemetry::init`), has no cross-crate relevance the
    /// way `port`/`database_url`/`nexus_endpoint_url` do, and is a
    /// rendering detail of that one call site rather than an application
    /// setting — pulling it in would add indirection with no benefit.
    pub log_level: String,
    /// Base URL of the Nexus routing layer that `nexus-client` (ADR-007)
    /// will call. Placeholder until a real Nexus environment is confirmed.
    pub nexus_endpoint_url: String,
}

impl Config {
    /// Loads configuration from environment variables, falling back to dev
    /// defaults for anything unset.
    ///
    /// # Panics
    /// Panics if `PORT` is set but is not a valid `u16` — an invalid port is
    /// a fatal startup misconfiguration (CLAUDE.md: validate input at
    /// system boundaries), not something to silently default past.
    pub fn load() -> Config {
        Self::from_env(|key| env::var(key).ok())
    }

    /// Core loader, parameterized over a variable lookup function so it can
    /// be unit-tested without touching real process environment state
    /// (`std::env::set_var` is `unsafe` as of the 2024 edition and racy
    /// under parallel test execution regardless).
    fn from_env(get: impl Fn(&str) -> Option<String>) -> Config {
        let database_url = get(DATABASE_URL_ENV).unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned());

        let port = match get(PORT_ENV) {
            Some(raw) => raw
                .parse::<u16>()
                .unwrap_or_else(|err| panic!("{PORT_ENV} must be a valid u16, got {raw:?}: {err}")),
            None => DEFAULT_PORT,
        };

        let log_level = get(LOG_LEVEL_ENV).unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());

        let nexus_endpoint_url =
            get(NEXUS_ENDPOINT_URL_ENV).unwrap_or_else(|| DEFAULT_NEXUS_ENDPOINT_URL.to_owned());

        Config { database_url, port, log_level, nexus_endpoint_url }
    }

    /// A form of `database_url` safe to log: masks any `user:password@`
    /// userinfo portion, leaving scheme/host/path visible for debugging.
    pub fn redacted_database_url(&self) -> String {
        redact_credentials(&self.database_url)
    }
}

fn redact_credentials(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host_and_path)) => format!("{scheme}://***@{host_and_path}"),
            None => format!("{scheme}://{rest}"),
        },
        None => "***".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |key| vars.get(key).map(|value| (*value).to_owned())
    }

    #[test]
    fn defaults_apply_when_env_is_empty() {
        let config = Config::from_env(lookup(HashMap::new()));

        assert_eq!(config.database_url, DEFAULT_DATABASE_URL);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.log_level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.nexus_endpoint_url, DEFAULT_NEXUS_ENDPOINT_URL);
    }

    #[test]
    fn env_vars_override_defaults() {
        let vars = HashMap::from([
            (DATABASE_URL_ENV, "postgres://user:pass@db.internal:5432/prod"),
            (PORT_ENV, "4000"),
            (LOG_LEVEL_ENV, "debug"),
            (NEXUS_ENDPOINT_URL_ENV, "https://nexus.example.com"),
        ]);

        let config = Config::from_env(lookup(vars));

        assert_eq!(config.database_url, "postgres://user:pass@db.internal:5432/prod");
        assert_eq!(config.port, 4000);
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.nexus_endpoint_url, "https://nexus.example.com");
    }

    #[test]
    #[should_panic(expected = "PORT must be a valid u16")]
    fn invalid_port_panics() {
        let vars = HashMap::from([(PORT_ENV, "not-a-number")]);
        Config::from_env(lookup(vars));
    }

    #[test]
    fn redacts_credentials_in_database_url() {
        let config = Config {
            database_url: "postgres://user:sekret@db.internal:5432/prod".to_owned(),
            port: DEFAULT_PORT,
            log_level: DEFAULT_LOG_LEVEL.to_owned(),
            nexus_endpoint_url: DEFAULT_NEXUS_ENDPOINT_URL.to_owned(),
        };

        assert_eq!(config.redacted_database_url(), "postgres://***@db.internal:5432/prod");
    }

    #[test]
    fn redacts_database_url_without_credentials() {
        let config = Config {
            database_url: DEFAULT_DATABASE_URL.to_owned(),
            port: DEFAULT_PORT,
            log_level: DEFAULT_LOG_LEVEL.to_owned(),
            nexus_endpoint_url: DEFAULT_NEXUS_ENDPOINT_URL.to_owned(),
        };

        assert_eq!(config.redacted_database_url(), DEFAULT_DATABASE_URL);
    }
}
