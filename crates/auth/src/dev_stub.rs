//! Dev-only session provider stub (ADR-008 "Interim dev-stub"). Gated
//! behind the `dev-auth` Cargo feature *and* a runtime environment check,
//! so it refuses to activate outside a `dev` environment even if the
//! feature is accidentally compiled into a deployed build.
//!
//! No real credential check, no Armor/OIDC integration: every session this
//! stub issues is for the same fixed dev identity
//! ([`DEV_CONSULTANT_ID`]), per ADR-008's "a fixed set of dev consultant
//! identities, no real credential check".

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{Duration, Utc};
use config::Config;
use uuid::Uuid;

use crate::{Session, SessionError, SessionProvider};

/// The fixed dev identity every dev-stub session is issued for.
pub const DEV_CONSULTANT_ID: &str = "dev-consultant-001";

/// How long a dev-stub session is valid for before expiring. Generous,
/// since this only exists to unblock local dev loops, not to model real
/// session lifetimes.
const DEV_SESSION_TTL_MINUTES: i64 = 60 * 24;

/// In-memory, single-process dev-stub [`SessionProvider`].
///
/// **Storage choice: in-memory (`Mutex<HashMap<Uuid, Session>>`), not
/// routed through `persistence`'s Postgres pool.** ADR-008 assigns
/// *persisted* session storage (surviving a BFF instance restart, working
/// under ADR-014's horizontal scaling) to the real, Armor-backed provider
/// landing in U11. The dev-stub is explicitly a throwaway, single-instance,
/// local-dev-only convenience that can never run outside `dev` (enforced
/// below); it has no restart-survival or multi-instance requirement to
/// justify wiring it to Postgres, and keeping it in-memory avoids adding a
/// database dependency to the inner dev loop this stub exists to unblock.
/// U11 can revisit this if local dev workflows end up wanting dev sessions
/// that survive a `cargo run` restart.
pub struct DevStubSessionProvider {
    sessions: Mutex<HashMap<Uuid, Session>>,
}

impl DevStubSessionProvider {
    /// Constructs a new dev-stub provider.
    ///
    /// # Panics
    /// Panics if `config.environment` is not `"dev"`
    /// ([`config::DEV_ENVIRONMENT`]). This is a deliberate, ADR-008-
    /// mandated safety valve: even with the `dev-auth` feature compiled
    /// in, the stub must refuse to activate at runtime anywhere other than
    /// a dev environment, since it performs no real credential check.
    pub fn new(config: &Config) -> Self {
        assert!(
            config.is_dev(),
            "DevStubSessionProvider must not be constructed outside a `dev` environment \
             (ADR-008); got environment={:?}. This stub issues sessions with no real \
             credential check and must never run in staging/prod.",
            config.environment
        );

        Self { sessions: Mutex::new(HashMap::new()) }
    }

    /// Issues (and stores) a new dev session for the fixed dev consultant
    /// identity ([`DEV_CONSULTANT_ID`]).
    ///
    /// Not part of [`SessionProvider`]: a real Armor-backed login will be
    /// structurally different (an OIDC authorization-code exchange, per
    /// ADR-008), so this stub-specific constructor doesn't try to
    /// anticipate that shape.
    pub async fn create_dev_session(&self) -> Result<Session, SessionError> {
        let session = Session {
            session_id: Uuid::new_v4(),
            consultant_id: DEV_CONSULTANT_ID.to_owned(),
            expires_at: Utc::now() + Duration::minutes(DEV_SESSION_TTL_MINUTES),
        };

        let mut sessions = lock_sessions(&self.sessions)?;
        sessions.insert(session.session_id, session.clone());

        Ok(session)
    }
}

#[async_trait::async_trait]
impl SessionProvider for DevStubSessionProvider {
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>, SessionError> {
        let sessions = lock_sessions(&self.sessions)?;

        Ok(sessions.get(&session_id).filter(|session| session.expires_at > Utc::now()).cloned())
    }

    async fn delete_session(&self, session_id: Uuid) -> Result<(), SessionError> {
        let mut sessions = lock_sessions(&self.sessions)?;
        sessions.remove(&session_id);
        Ok(())
    }
}

fn lock_sessions(
    sessions: &Mutex<HashMap<Uuid, Session>>,
) -> Result<std::sync::MutexGuard<'_, HashMap<Uuid, Session>>, SessionError> {
    sessions
        .lock()
        .map_err(|_| SessionError::StoreUnavailable("dev session store lock poisoned".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_config() -> Config {
        Config { environment: config::DEV_ENVIRONMENT.to_owned(), ..test_config_defaults() }
    }

    fn prod_config() -> Config {
        Config { environment: "prod".to_owned(), ..test_config_defaults() }
    }

    fn test_config_defaults() -> Config {
        Config {
            database_url: "postgres://localhost:5432/test".to_owned(),
            port: 3000,
            log_level: "info".to_owned(),
            nexus_endpoint_url: "http://localhost:8080".to_owned(),
            environment: config::DEV_ENVIRONMENT.to_owned(),
            static_dir: None,
            firebase_project_id: None,
                nexus_caller_service_account_email: None,
        }
    }

    #[test]
    #[should_panic(expected = "must not be constructed outside a `dev` environment")]
    fn refuses_to_construct_outside_dev_environment() {
        let _ = DevStubSessionProvider::new(&prod_config());
    }

    #[tokio::test]
    async fn constructs_in_dev_and_creates_a_session_for_the_fixed_consultant() {
        let provider = DevStubSessionProvider::new(&dev_config());

        let session = provider.create_dev_session().await.expect("create_dev_session failed");

        assert_eq!(session.consultant_id, DEV_CONSULTANT_ID);
        assert!(session.expires_at > Utc::now());
    }

    #[tokio::test]
    async fn get_session_returns_a_previously_created_session() {
        let provider = DevStubSessionProvider::new(&dev_config());
        let created = provider.create_dev_session().await.expect("create_dev_session failed");

        let found = provider.get_session(created.session_id).await.expect("get_session failed");

        assert_eq!(found, Some(created));
    }

    #[tokio::test]
    async fn get_session_returns_none_for_an_unknown_id() {
        let provider = DevStubSessionProvider::new(&dev_config());

        let found = provider.get_session(Uuid::new_v4()).await.expect("get_session failed");

        assert_eq!(found, None);
    }
}
