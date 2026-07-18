//! Postgres-backed `ActionQueueRepository` (PROMPT-29, ADR-010).
//!
//! Stores the aggregate as one row per entry in `action_queue_entries` (see
//! the `20260718022631_action_queue_entries` migration) — same shape as
//! `notification_repository.rs`: `id` is the primary key,
//! `(origin_capability, origin_event_id)` is the DB-native idempotency
//! constraint.
//!
//! [`PgActionQueueRepository::save`] uses the same `ON CONFLICT ... DO
//! NOTHING` semantics as `PgNotificationRepository::save`, for the same
//! reason: `save` is the ingestion path, and a redelivered event must not
//! be able to regress a row's `action_state` (invariant 2, no regression)
//! back to `pending` after the consultant has already progressed it. See
//! `notification_repository.rs`'s module docs for the full argument against
//! `DO UPDATE`.
//!
//! [`PgActionQueueRepository::mark_completed`] re-validates
//! `confirmation_event_id` is non-empty even though
//! `bff_core::ActionQueueEntry::complete` already does — this repository
//! method is a direct-SQL shortcut that bypasses loading the aggregate, so
//! invariant 3's guard (`consultant-experience-context.md` §2.2) has to be
//! re-asserted here rather than inherited "for free" from the aggregate.

use async_trait::async_trait;
use bff_core::{ActionQueueEntry, ActionQueueEntryError, ActionQueueRepository, ActionState, RepoError, SaveOutcome};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// `ActionQueueRepository` implemented against Postgres via `sqlx`'s
/// compile-time-checked `query!`/`query_as!` macros (ADR-010).
pub struct PgActionQueueRepository {
    pool: PgPool,
}

impl PgActionQueueRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `action_queue_entries`.
struct ActionQueueEntryRow {
    id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_event_id: String,
    title: String,
    body: String,
    deep_link: Option<String>,
    action_state: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

/// Converts a raw DB row into the aggregate, re-validating every field via
/// [`ActionQueueEntry::from_parts`] (including parsing `action_state` back
/// through [`ActionState`]'s allow-list).
fn row_to_aggregate(row: ActionQueueEntryRow) -> Result<ActionQueueEntry, RepoError> {
    let action_state: ActionState = row
        .action_state
        .parse()
        .map_err(|err: bff_core::ParseActionStateError| RepoError::OperationFailed(err.to_string()))?;

    ActionQueueEntry::from_parts(
        row.id,
        row.consultant_id,
        row.origin_capability,
        row.origin_event_id,
        row.title,
        row.body,
        row.deep_link,
        action_state,
        row.expires_at,
        row.created_at,
    )
    .map_err(|err: ActionQueueEntryError| RepoError::OperationFailed(err.to_string()))
}

#[async_trait]
impl ActionQueueRepository for PgActionQueueRepository {
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<ActionQueueEntry>, RepoError> {
        let rows = sqlx::query_as!(
            ActionQueueEntryRow,
            r#"
            SELECT id, consultant_id, origin_capability, origin_event_id,
                   title, body, deep_link, action_state, expires_at, created_at
            FROM action_queue_entries
            WHERE consultant_id = $1
            ORDER BY created_at DESC
            "#,
            consultant_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        rows.into_iter().map(row_to_aggregate).collect()
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ActionQueueEntry>, RepoError> {
        let row = sqlx::query_as!(
            ActionQueueEntryRow,
            r#"
            SELECT id, consultant_id, origin_capability, origin_event_id,
                   title, body, deep_link, action_state, expires_at, created_at
            FROM action_queue_entries
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        row.map(row_to_aggregate).transpose()
    }

    async fn find_by_origin_event(
        &self,
        origin_capability: &str,
        origin_event_id: &str,
    ) -> Result<Option<ActionQueueEntry>, RepoError> {
        let row = sqlx::query_as!(
            ActionQueueEntryRow,
            r#"
            SELECT id, consultant_id, origin_capability, origin_event_id,
                   title, body, deep_link, action_state, expires_at, created_at
            FROM action_queue_entries
            WHERE origin_capability = $1 AND origin_event_id = $2
            "#,
            origin_capability,
            origin_event_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        row.map(row_to_aggregate).transpose()
    }

    async fn save(&self, entry: &ActionQueueEntry) -> Result<SaveOutcome, RepoError> {
        let result = sqlx::query!(
            r#"
            INSERT INTO action_queue_entries
                (id, consultant_id, origin_capability, origin_event_id,
                 title, body, deep_link, action_state, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (origin_capability, origin_event_id) DO NOTHING
            "#,
            entry.id(),
            entry.consultant_id(),
            entry.origin_capability(),
            entry.origin_event_id(),
            entry.title(),
            entry.body(),
            entry.deep_link(),
            entry.action_state().as_str(),
            entry.expires_at(),
            entry.created_at(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(if result.rows_affected() == 1 { SaveOutcome::Inserted } else { SaveOutcome::AlreadyExists })
    }

    async fn mark_started(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query!(
            r#"
            UPDATE action_queue_entries
            SET action_state = 'in_progress'
            WHERE id = $1 AND action_state = 'pending'
            "#,
            id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }

    async fn mark_completed(
        &self,
        id: Uuid,
        confirmation_event_id: &str,
    ) -> Result<(), RepoError> {
        if confirmation_event_id.trim().is_empty() {
            return Err(RepoError::OperationFailed(
                ActionQueueEntryError::EmptyConfirmationEventId.to_string(),
            ));
        }

        sqlx::query!(
            r#"
            UPDATE action_queue_entries
            SET action_state = 'completed'
            WHERE id = $1 AND action_state = 'in_progress'
            "#,
            id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }

    async fn expire_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, RepoError> {
        let result = sqlx::query!(
            r#"
            UPDATE action_queue_entries
            SET action_state = 'expired'
            WHERE action_state NOT IN ('completed', 'expired')
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

    fn entry(consultant_id: &str, origin_event_id: &str, expires_at: DateTime<Utc>) -> ActionQueueEntry {
        ActionQueueEntry::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Collaboration request",
            "A collaboration request needs your response.",
            Some("https://app.example.com/sales/collab/1".to_string()),
            expires_at,
            Utc::now(),
        )
        .expect("valid action queue entry")
    }

    #[allow(clippy::too_many_arguments)]
    fn entry_with_state(
        consultant_id: &str,
        origin_event_id: &str,
        action_state: ActionState,
        expires_at: DateTime<Utc>,
    ) -> ActionQueueEntry {
        ActionQueueEntry::from_parts(
            Uuid::new_v4(),
            consultant_id.to_string(),
            "sales".to_string(),
            origin_event_id.to_string(),
            "Collaboration request".to_string(),
            "A collaboration request needs your response.".to_string(),
            None,
            action_state,
            expires_at,
            Utc::now(),
        )
        .expect("valid action queue entry parts")
    }

    /// Uses a fixed, microsecond-precision timestamp rather than
    /// `Utc::now()`: Postgres `TIMESTAMPTZ` only stores microsecond
    /// precision, so a nanosecond-precision Rust `Utc::now()` would
    /// round-trip as a slightly different value and fail this exact
    /// equality check for a reason unrelated to what the test verifies
    /// (same rationale as `workflow_session_repository`'s equivalent
    /// test).
    #[tokio::test]
    async fn save_and_find_by_consultant_id_round_trips_an_entry() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let entry = ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "event-1",
            "Collaboration request",
            "A collaboration request needs your response.",
            Some("https://app.example.com/sales/collab/1".to_string()),
            now + Duration::hours(24),
            now,
        )
        .expect("valid action queue entry");
        let outcome = repo.save(&entry).await.expect("save failed");
        assert_eq!(outcome, SaveOutcome::Inserted);

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0], entry);
    }

    /// `find_by_id` (PROMPT-32's NOTIFY/LISTEN bridge reconstruction path):
    /// a saved entry is found by its own id, and an unknown id is `Ok(None)`,
    /// not an error.
    ///
    /// Builds the entry with a fixed, microsecond-precision timestamp
    /// (rather than the `entry()`/`entry_with_state()` helpers above, which
    /// bake in `Utc::now()` for `created_at`) — see
    /// `save_and_find_by_consultant_id_round_trips_an_entry`'s doc comment
    /// for why a nanosecond-precision Rust timestamp would fail this exact
    /// equality check for a reason unrelated to `find_by_id`.
    #[tokio::test]
    async fn find_by_id_finds_a_saved_entry_and_returns_none_for_an_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let entry = ActionQueueEntry::new(
            "consultant-1",
            "sales",
            "event-1",
            "Collaboration request",
            "A collaboration request needs your response.",
            Some("https://app.example.com/sales/collab/1".to_string()),
            now + Duration::hours(24),
            now,
        )
        .expect("valid action queue entry");
        repo.save(&entry).await.expect("save failed");

        let found = repo.find_by_id(entry.id()).await.expect("find_by_id failed");
        assert_eq!(found, Some(entry));

        let missing = repo.find_by_id(Uuid::new_v4()).await.expect("find_by_id failed");
        assert_eq!(missing, None);
    }

    /// `find_by_origin_event` (PROMPT-38): resolves the entry a
    /// `(origin_capability, origin_event_id)` pair created — the lookup a
    /// later confirmation event uses (see `bff_core::event_ingestion`'s
    /// `ingest_confirmation`) since it has no entry `id` to call
    /// `find_by_id` with, only a reference to the origin event.
    #[tokio::test]
    async fn find_by_origin_event_finds_a_saved_entry_and_returns_none_when_unmatched() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let entry = ActionQueueEntry::new(
            "consultant-1",
            "execution",
            "ta-1",
            "Task Assigned",
            "You have been assigned a new delivery task.",
            None,
            now + Duration::hours(24),
            now,
        )
        .expect("valid action queue entry");
        repo.save(&entry).await.expect("save failed");

        let found =
            repo.find_by_origin_event("execution", "ta-1").await.expect("find_by_origin_event failed");
        assert_eq!(found, Some(entry));

        let wrong_capability =
            repo.find_by_origin_event("sales", "ta-1").await.expect("find_by_origin_event failed");
        assert_eq!(wrong_capability, None);

        let wrong_event_id =
            repo.find_by_origin_event("execution", "does-not-exist").await.expect("find_by_origin_event failed");
        assert_eq!(wrong_event_id, None);
    }

    #[tokio::test]
    async fn find_by_consultant_id_returns_empty_for_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let found = repo.find_by_consultant_id("does-not-exist").await.expect("find failed");

        assert!(found.is_empty());
    }

    /// The core idempotency contract (invariant 1): delivering the same
    /// `(origin_capability, origin_event_id)` twice must leave exactly one
    /// row in the database — queried directly, not inferred from the
    /// repository's return value.
    #[tokio::test]
    async fn duplicate_event_delivery_produces_exactly_one_row() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool.clone());

        let expires_at = Utc::now() + Duration::hours(24);
        let first = entry("consultant-1", "event-1", expires_at);
        let redelivered = entry("consultant-1", "event-1", expires_at);

        let first_outcome = repo.save(&first).await.expect("first save failed");
        let second_outcome = repo.save(&redelivered).await.expect("second save failed");

        assert_eq!(first_outcome, SaveOutcome::Inserted);
        assert_eq!(second_outcome, SaveOutcome::AlreadyExists);

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM action_queue_entries WHERE origin_capability = 'sales' AND origin_event_id = 'event-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("count query failed");
        assert_eq!(count, 1);
    }

    /// Redelivery must not regress an already-progressed `action_state`
    /// back to `pending` — the concrete failure mode `DO NOTHING` (not `DO
    /// UPDATE`) protects against.
    #[tokio::test]
    async fn duplicate_delivery_after_start_does_not_regress_action_state() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let expires_at = Utc::now() + Duration::hours(24);
        let first = entry("consultant-1", "event-1", expires_at);
        repo.save(&first).await.expect("first save failed");
        repo.mark_started(first.id()).await.expect("mark_started failed");

        let redelivered = entry("consultant-1", "event-1", expires_at);
        repo.save(&redelivered).await.expect("redelivery save failed");

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action_state(), ActionState::InProgress);
    }

    #[tokio::test]
    async fn mark_started_transitions_a_pending_entry() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let entry = entry("consultant-1", "event-1", Utc::now() + Duration::hours(24));
        repo.save(&entry).await.expect("save failed");

        repo.mark_started(entry.id()).await.expect("mark_started failed");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].action_state(), ActionState::InProgress);
    }

    #[tokio::test]
    async fn mark_completed_requires_a_non_empty_confirmation_event_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let entry = entry("consultant-1", "event-1", Utc::now() + Duration::hours(24));
        repo.save(&entry).await.expect("save failed");
        repo.mark_started(entry.id()).await.expect("mark_started failed");

        let err = repo.mark_completed(entry.id(), "").await.unwrap_err();
        assert!(matches!(err, RepoError::OperationFailed(_)));

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].action_state(), ActionState::InProgress);
    }

    #[tokio::test]
    async fn mark_completed_transitions_an_in_progress_entry_with_confirmation() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let entry = entry("consultant-1", "event-1", Utc::now() + Duration::hours(24));
        repo.save(&entry).await.expect("save failed");
        repo.mark_started(entry.id()).await.expect("mark_started failed");

        repo.mark_completed(entry.id(), "nexus-confirmation-42")
            .await
            .expect("mark_completed failed");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].action_state(), ActionState::Completed);
    }

    /// `mark_completed` must not fire directly from `pending`, even with a
    /// valid confirmation id (the WHERE guard restricts to `in_progress`).
    #[tokio::test]
    async fn mark_completed_does_not_transition_a_pending_entry() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let entry = entry("consultant-1", "event-1", Utc::now() + Duration::hours(24));
        repo.save(&entry).await.expect("save failed");

        repo.mark_completed(entry.id(), "nexus-confirmation-42")
            .await
            .expect("mark_completed should be a lenient no-op, not an error");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].action_state(), ActionState::Pending);
    }

    /// The housekeeping-sweep contract: `expire_older_than` flips
    /// qualifying rows (non-terminal, `expires_at < cutoff`) to `Expired`
    /// and returns the exact count, while leaving already-terminal rows
    /// and rows not yet past `cutoff` untouched.
    #[tokio::test]
    async fn expire_older_than_flips_qualifying_rows_and_returns_the_count() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let now = Utc::now();
        let cutoff = now;

        let should_expire_pending =
            entry_with_state("consultant-1", "event-1", ActionState::Pending, now - Duration::minutes(5));
        let should_expire_in_progress = entry_with_state(
            "consultant-1",
            "event-2",
            ActionState::InProgress,
            now - Duration::minutes(1),
        );
        let not_yet_past_cutoff = entry_with_state(
            "consultant-1",
            "event-3",
            ActionState::InProgress,
            now + Duration::minutes(10),
        );
        let already_completed = entry_with_state(
            "consultant-1",
            "event-4",
            ActionState::Completed,
            now - Duration::minutes(5),
        );
        let already_expired = entry_with_state(
            "consultant-1",
            "event-5",
            ActionState::Expired,
            now - Duration::minutes(5),
        );

        for entry in [
            &should_expire_pending,
            &should_expire_in_progress,
            &not_yet_past_cutoff,
            &already_completed,
            &already_expired,
        ] {
            repo.save(entry).await.expect("seed save failed");
        }

        let affected = repo.expire_older_than(cutoff).await.expect("expire_older_than failed");
        assert_eq!(affected, 2);

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        let state_of = |id: Uuid| found.iter().find(|e| e.id() == id).unwrap().action_state();

        assert_eq!(state_of(should_expire_pending.id()), ActionState::Expired);
        assert_eq!(state_of(should_expire_in_progress.id()), ActionState::Expired);
        assert_eq!(state_of(not_yet_past_cutoff.id()), ActionState::InProgress);
        assert_eq!(state_of(already_completed.id()), ActionState::Completed);
        assert_eq!(state_of(already_expired.id()), ActionState::Expired);
    }

    #[tokio::test]
    async fn expire_older_than_returns_zero_when_nothing_qualifies() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgActionQueueRepository::new(pool);

        let entry = entry_with_state(
            "consultant-1",
            "event-1",
            ActionState::Pending,
            Utc::now() + Duration::minutes(30),
        );
        repo.save(&entry).await.expect("seed save failed");

        let affected = repo.expire_older_than(Utc::now()).await.expect("expire_older_than failed");

        assert_eq!(affected, 0);
    }
}
