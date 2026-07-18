//! persistence: repository trait implementations over the ADR-010 datastore.
//!
//! U09 stood up the Postgres connection pool and the `sqlx-cli`
//! migration-tooling convention. U20 (PROMPT-20) adds the first concrete
//! repository implementation, [`PgConsultantPreferencesRepository`], for
//! `bff_core::ConsultantPreferencesRepository`
//! (`.plans/ddd/consultant-experience-context.md` §1.4). See `README.md`
//! for the local-dev / CI migration and offline-query-check workflow.

mod action_queue_entry_repository;
mod consultant_preferences_repository;
mod dashboard_configuration_repository;
mod notification_repository;
mod workflow_session_repository;

pub use action_queue_entry_repository::PgActionQueueRepository;
pub use consultant_preferences_repository::PgConsultantPreferencesRepository;
pub use dashboard_configuration_repository::PgDashboardConfigurationRepository;
pub use notification_repository::PgNotificationRepository;
pub use workflow_session_repository::PgWorkflowSessionRepository;

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Re-exported so downstream crates (e.g. `bff-api`'s `AppState`, U11) can
/// name the pool type returned by [`create_pool`] without taking their own
/// direct dependency on `sqlx` just for this one type.
pub use sqlx::postgres::PgPool as Pool;

/// Default maximum number of pooled connections.
///
/// ADR-010 does not pin a specific pool size; this is a conservative
/// default picked ahead of any real load testing (there are zero deployed
/// callers yet). Revisit once U11+ introduces real query traffic and this
/// can be tuned from actual usage/connection-saturation data rather than a
/// guess.
const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// How long `create_pool` waits for its initial connection before giving
/// up. `sqlx`'s own default (30s) is tuned for steady-state connection
/// *acquisition* under load, not a one-shot startup reachability check —
/// left at the default, an unreachable Postgres at startup would hang
/// `bff-api`'s fail-fast panic for 30s instead of failing promptly.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Builds a Postgres connection pool for `database_url`.
///
/// This resolves as soon as the pool has established at least one live
/// connection, so a caller that `.await`s it and fails fast on `Err`
/// (as `bff-api`'s `main.rs` does) gets 12-factor-style fail-fast startup
/// behavior: the process refuses to come up if Postgres is unreachable,
/// rather than silently serving traffic it can't actually fulfill once
/// persistence-backed routes exist (ADR-010: this repo's own aggregates —
/// `DashboardConfiguration`, `ConsultantPreferences`, notification/action
/// queue state — require Postgres to be the multi-instance source of
/// truth, not an optional side dependency).
pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(DEFAULT_MAX_CONNECTIONS)
        .acquire_timeout(CONNECT_TIMEOUT)
        .connect(database_url)
        .await
}

/// Cheap connectivity probe for readiness checks (ADR-014): runs a trivial
/// `SELECT 1` against `pool`, acquiring a connection from the existing pool
/// rather than opening a new one. Returns `Err` if Postgres is unreachable
/// or the query otherwise fails. Callers that need a bounded worst-case
/// latency (e.g. `bff-api`'s `GET /readyz`) should wrap this in their own
/// `tokio::time::timeout` — this function does not impose one itself, so it
/// stays usable from contexts that already have their own timeout policy.
pub async fn check_connectivity(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").fetch_one(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::Row;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    /// End-to-end proof that `create_pool` produces a working pool against
    /// a real Postgres instance: launches a throwaway container (Docker,
    /// via `testcontainers-modules`, per ADR-013 layer 3 / ADR-010),
    /// connects, and runs a trivial query.
    #[tokio::test]
    async fn create_pool_connects_and_runs_a_query() {
        let container =
            Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = create_pool(&database_url).await.expect("create_pool failed to connect");

        let row = sqlx::query("SELECT 1 AS one")
            .fetch_one(&pool)
            .await
            .expect("SELECT 1 failed to execute");
        let value: i32 = row.get("one");

        assert_eq!(value, 1);
    }

    /// `check_connectivity` (ADR-014 readiness probe) succeeds against a
    /// live pool, mirroring `create_pool_connects_and_runs_a_query` above.
    #[tokio::test]
    async fn check_connectivity_succeeds_against_a_live_pool() {
        let container =
            Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = create_pool(&database_url).await.expect("create_pool failed to connect");

        check_connectivity(&pool).await.expect("check_connectivity failed against a live pool");
    }

    /// `check_connectivity` reports `Err` (not a panic, not a silent `Ok`)
    /// when Postgres is unreachable — the failure mode `GET /readyz` needs
    /// to be able to detect and turn into a `503`. Uses `connect_lazy`
    /// against a loopback port nothing listens on (immediate
    /// `ECONNREFUSED`, no container teardown timing to race) so this stays
    /// fast and deterministic rather than depending on how quickly a
    /// stopped testcontainer actually stops accepting connections.
    #[tokio::test]
    async fn check_connectivity_fails_when_postgres_is_unreachable() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/postgres")
            .expect("connect_lazy should not eagerly connect");

        let result = check_connectivity(&pool).await;

        assert!(result.is_err(), "expected check_connectivity to fail against an unreachable pool");
    }
}
