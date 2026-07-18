//! Postgres-backed `WorkflowSessionRepository` (PROMPT-22, ADR-010).
//!
//! Stores the aggregate as one row per session in
//! `cross_capability_workflow_sessions` (see the
//! `20260718004942_cross_capability_workflow_sessions` migration) — unlike
//! `dashboard_configurations`/`consultant_preferences`, `session_id` (not
//! `consultant_id`) is the primary key, since a consultant may accumulate
//! several sessions over time.
//!
//! [`PgWorkflowSessionRepository::find_active_by_consultant_id`] and
//! [`PgWorkflowSessionRepository::expire_older_than`] both filter on
//! `status`/`expires_at` server-side (not by loading rows and filtering in
//! Rust) so the `(consultant_id, status)` index the migration adds is
//! actually exercised, and so `expire_older_than`'s affected-row count
//! comes directly from Postgres rather than a separate count query.

use async_trait::async_trait;
use bff_core::{
    CrossCapabilityWorkflowSession, RepoError, WorkflowSessionError, WorkflowSessionRepository,
    WorkflowSessionStatus,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// `WorkflowSessionRepository` implemented against Postgres via `sqlx`'s
/// compile-time-checked `query!`/`query_as!` macros (ADR-010).
pub struct PgWorkflowSessionRepository {
    pool: PgPool,
}

impl PgWorkflowSessionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `cross_capability_workflow_sessions`.
struct WorkflowSessionRow {
    session_id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_reference: String,
    target_capability: String,
    target_reference: Option<String>,
    status: String,
    expires_at: DateTime<Utc>,
}

/// Converts a raw DB row into the aggregate, re-validating every field via
/// [`CrossCapabilityWorkflowSession::from_parts`] (including parsing
/// `status` back through [`WorkflowSessionStatus`]'s allow-list — an
/// unrecognized value can never reach the rest of the app).
fn row_to_aggregate(
    row: WorkflowSessionRow,
) -> Result<CrossCapabilityWorkflowSession, RepoError> {
    let status: WorkflowSessionStatus =
        row.status.parse().map_err(|err: bff_core::ParseWorkflowSessionStatusError| {
            RepoError::OperationFailed(err.to_string())
        })?;

    CrossCapabilityWorkflowSession::from_parts(
        row.session_id,
        row.consultant_id,
        row.origin_capability,
        row.origin_reference,
        row.target_capability,
        row.target_reference,
        status,
        row.expires_at,
    )
    .map_err(|err: WorkflowSessionError| RepoError::OperationFailed(err.to_string()))
}

#[async_trait]
impl WorkflowSessionRepository for PgWorkflowSessionRepository {
    async fn find_by_id(
        &self,
        session_id: Uuid,
    ) -> Result<Option<CrossCapabilityWorkflowSession>, RepoError> {
        let row = sqlx::query_as!(
            WorkflowSessionRow,
            r#"
            SELECT session_id, consultant_id, origin_capability, origin_reference,
                   target_capability, target_reference, status, expires_at
            FROM cross_capability_workflow_sessions
            WHERE session_id = $1
            "#,
            session_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        row.map(row_to_aggregate).transpose()
    }

    async fn find_active_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<CrossCapabilityWorkflowSession>, RepoError> {
        let rows = sqlx::query_as!(
            WorkflowSessionRow,
            r#"
            SELECT session_id, consultant_id, origin_capability, origin_reference,
                   target_capability, target_reference, status, expires_at
            FROM cross_capability_workflow_sessions
            WHERE consultant_id = $1
              AND status NOT IN ('completed', 'abandoned', 'expired')
              AND expires_at > now()
            ORDER BY expires_at
            "#,
            consultant_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        rows.into_iter().map(row_to_aggregate).collect()
    }

    async fn save(&self, session: &CrossCapabilityWorkflowSession) -> Result<(), RepoError> {
        sqlx::query!(
            r#"
            INSERT INTO cross_capability_workflow_sessions
                (session_id, consultant_id, origin_capability, origin_reference,
                 target_capability, target_reference, status, expires_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
            ON CONFLICT (session_id)
            DO UPDATE SET
                target_reference = EXCLUDED.target_reference,
                status = EXCLUDED.status,
                expires_at = EXCLUDED.expires_at,
                updated_at = now()
            "#,
            session.session_id(),
            session.consultant_id(),
            session.origin_capability(),
            session.origin_reference(),
            session.target_capability(),
            session.target_reference(),
            session.status().as_str(),
            session.expires_at(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }

    async fn expire_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, RepoError> {
        let result = sqlx::query!(
            r#"
            UPDATE cross_capability_workflow_sessions
            SET status = 'expired', updated_at = now()
            WHERE status NOT IN ('completed', 'abandoned', 'expired')
              AND expires_at < $1
            "#,
            cutoff,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    async fn migrated_pool() -> (
        PgPool,
        testcontainers_modules::testcontainers::ContainerAsync<Postgres>,
    ) {
        let container =
            Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = crate::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("./migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    /// Builds a session with an explicit `status`/`expires_at`, bypassing
    /// [`CrossCapabilityWorkflowSession::start`]'s TTL-from-now behavior —
    /// needed to seed rows in states/times that housekeeping tests need to
    /// exercise (e.g. "non-terminal but already past its `expires_at`").
    #[allow(clippy::too_many_arguments)]
    fn session_with(
        consultant_id: &str,
        status: WorkflowSessionStatus,
        expires_at: DateTime<Utc>,
    ) -> CrossCapabilityWorkflowSession {
        CrossCapabilityWorkflowSession::from_parts(
            Uuid::new_v4(),
            consultant_id.to_string(),
            "sales".to_string(),
            "lead-1".to_string(),
            "commit".to_string(),
            None,
            status,
            expires_at,
        )
        .expect("valid session parts")
    }

    /// Round-trips a full `CrossCapabilityWorkflowSession` through
    /// Postgres: save, then read back by id, and confirm the data matches.
    ///
    /// Uses a fixed, microsecond-precision `now` rather than `Utc::now()`:
    /// Postgres `TIMESTAMPTZ` only stores microsecond precision, so a
    /// nanosecond-precision Rust `Utc::now()` would round-trip as a
    /// slightly different value and fail this exact-equality check for a
    /// reason unrelated to what the test is verifying.
    #[tokio::test]
    async fn save_and_find_by_id_round_trips_a_session() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let session =
            CrossCapabilityWorkflowSession::start("consultant-1", "sales", "lead-42", "commit", now)
                .unwrap();

        repo.save(&session).await.expect("save failed");

        let found = repo
            .find_by_id(session.session_id())
            .await
            .expect("find failed")
            .expect("expected a saved row");

        assert_eq!(found, session);
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_unknown_session() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let found = repo.find_by_id(Uuid::new_v4()).await.expect("find failed");

        assert!(found.is_none());
    }

    /// Saving twice for the same `session_id` (e.g. after a
    /// `transition_to`) must update the existing row, not create a second
    /// one — this is how a caller persists a mutated aggregate.
    /// Same microsecond-precision-`now` rationale as
    /// `save_and_find_by_id_round_trips_a_session` above.
    #[tokio::test]
    async fn save_twice_for_same_session_id_upserts_not_duplicates() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool.clone());

        let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let mut session =
            CrossCapabilityWorkflowSession::start("consultant-1", "sales", "lead-42", "commit", now)
                .unwrap();
        repo.save(&session).await.expect("first save failed");

        session.transition_to(WorkflowSessionStatus::InProgress, now).unwrap();
        session.set_target_reference("proposal-9", now).unwrap();
        repo.save(&session).await.expect("second save failed");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cross_capability_workflow_sessions",
        )
        .fetch_one(&pool)
        .await
        .expect("count query failed");
        assert_eq!(count, 1);

        let found = repo.find_by_id(session.session_id()).await.unwrap().unwrap();
        assert_eq!(found, session);
        assert_eq!(found.status(), WorkflowSessionStatus::InProgress);
        assert_eq!(found.target_reference(), Some("proposal-9"));
    }

    /// `find_active_by_consultant_id` must return only sessions that are
    /// both non-terminal *and* not yet expired by time — a terminal
    /// session and a time-expired-but-not-yet-swept session must both be
    /// excluded, while a genuinely active one is returned.
    #[tokio::test]
    async fn find_active_by_consultant_id_excludes_terminal_and_time_expired_sessions() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let now = Utc::now();
        let active = session_with("consultant-1", WorkflowSessionStatus::InProgress, now + Duration::minutes(30));
        let terminal = session_with("consultant-1", WorkflowSessionStatus::Completed, now + Duration::minutes(30));
        let time_expired_not_yet_swept =
            session_with("consultant-1", WorkflowSessionStatus::InProgress, now - Duration::minutes(1));
        let other_consultant =
            session_with("consultant-2", WorkflowSessionStatus::Started, now + Duration::minutes(30));

        for session in [&active, &terminal, &time_expired_not_yet_swept, &other_consultant] {
            repo.save(session).await.expect("seed save failed");
        }

        let found = repo.find_active_by_consultant_id("consultant-1").await.expect("find failed");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id(), active.session_id());
    }

    #[tokio::test]
    async fn find_active_by_consultant_id_returns_empty_for_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let found = repo
            .find_active_by_consultant_id("does-not-exist")
            .await
            .expect("find failed");

        assert!(found.is_empty());
    }

    /// The core housekeeping-sweep contract: `expire_older_than` flips
    /// qualifying rows (non-terminal, `expires_at < cutoff`) to `Expired`
    /// and returns the exact count, while leaving already-terminal rows
    /// and rows not yet past `cutoff` completely untouched.
    #[tokio::test]
    async fn expire_older_than_flips_qualifying_rows_and_returns_the_count() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let now = Utc::now();
        let cutoff = now;

        let should_expire_started =
            session_with("consultant-1", WorkflowSessionStatus::Started, now - Duration::minutes(5));
        let should_expire_in_progress = session_with(
            "consultant-1",
            WorkflowSessionStatus::InProgress,
            now - Duration::minutes(1),
        );
        let not_yet_past_cutoff = session_with(
            "consultant-1",
            WorkflowSessionStatus::InProgress,
            now + Duration::minutes(10),
        );
        let already_completed =
            session_with("consultant-1", WorkflowSessionStatus::Completed, now - Duration::minutes(5));
        let already_abandoned =
            session_with("consultant-1", WorkflowSessionStatus::Abandoned, now - Duration::minutes(5));

        for session in
            [&should_expire_started, &should_expire_in_progress, &not_yet_past_cutoff, &already_completed, &already_abandoned]
        {
            repo.save(session).await.expect("seed save failed");
        }

        let affected = repo.expire_older_than(cutoff).await.expect("expire_older_than failed");
        assert_eq!(affected, 2);

        let expired_1 = repo.find_by_id(should_expire_started.session_id()).await.unwrap().unwrap();
        assert_eq!(expired_1.status(), WorkflowSessionStatus::Expired);
        let expired_2 = repo.find_by_id(should_expire_in_progress.session_id()).await.unwrap().unwrap();
        assert_eq!(expired_2.status(), WorkflowSessionStatus::Expired);

        let untouched_future = repo.find_by_id(not_yet_past_cutoff.session_id()).await.unwrap().unwrap();
        assert_eq!(untouched_future.status(), WorkflowSessionStatus::InProgress);

        let untouched_completed = repo.find_by_id(already_completed.session_id()).await.unwrap().unwrap();
        assert_eq!(untouched_completed.status(), WorkflowSessionStatus::Completed);

        let untouched_abandoned = repo.find_by_id(already_abandoned.session_id()).await.unwrap().unwrap();
        assert_eq!(untouched_abandoned.status(), WorkflowSessionStatus::Abandoned);
    }

    #[tokio::test]
    async fn expire_older_than_returns_zero_when_nothing_qualifies() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgWorkflowSessionRepository::new(pool);

        let now = Utc::now();
        let session =
            session_with("consultant-1", WorkflowSessionStatus::Started, now + Duration::minutes(30));
        repo.save(&session).await.expect("seed save failed");

        let affected = repo.expire_older_than(now).await.expect("expire_older_than failed");

        assert_eq!(affected, 0);
    }
}
