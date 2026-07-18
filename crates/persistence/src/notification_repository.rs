//! Postgres-backed `NotificationRepository` (PROMPT-29, ADR-010).
//!
//! Stores the aggregate as one row per notification in `notification_items`
//! (see the `20260718022630_notification_items` migration) — `id` (not
//! `consultant_id`) is the primary key, since a consultant accumulates many
//! notifications over time, same shape as
//! `cross_capability_workflow_sessions`.
//!
//! ## `save`'s idempotent-insert semantics: `DO NOTHING`, not `DO UPDATE`
//!
//! [`PgNotificationRepository::save`] uses `INSERT ... ON CONFLICT
//! (origin_capability, origin_event_id) DO NOTHING`. `DO UPDATE` was
//! considered and rejected: `save` is the *ingestion* path (invariant 1,
//! `consultant-experience-context.md` §2.2) — by the time a redelivered
//! event reaches it, the consultant may already have progressed the
//! existing row's `read_state` to `read` (invariant 3, one-way). A `DO
//! UPDATE SET ... = EXCLUDED...` would overwrite that row with the
//! redelivered event's fields, which — if the update touched `read_state`
//! at all — would silently regress it back to `unread`, directly violating
//! invariant 3. Even restricting `DO UPDATE` to non-state columns
//! (title/body/deep_link) buys little: those are meant to be a stable,
//! display-safe summary of *one* upstream event, not a value that mutates
//! post-ingestion. `DO NOTHING` keeps the first-ingested row authoritative
//! and makes redelivery a true no-op, which is what "idempotent ingestion"
//! means here.
//!
//! [`PgNotificationRepository::mark_read`] is a separate, guarded
//! `UPDATE ... WHERE read_state = 'unread'` — it is the only path that
//! mutates `read_state` after ingestion, and it is deliberately lenient
//! (no-op if already read or unknown id) per its trait doc comment.

use async_trait::async_trait;
use bff_core::{NotificationItem, NotificationRepository, ReadState, RepoError, SaveOutcome};
use sqlx::PgPool;
use uuid::Uuid;

/// `NotificationRepository` implemented against Postgres via `sqlx`'s
/// compile-time-checked `query!`/`query_as!` macros (ADR-010).
pub struct PgNotificationRepository {
    pool: PgPool,
}

impl PgNotificationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `notification_items`.
struct NotificationItemRow {
    id: Uuid,
    consultant_id: String,
    origin_capability: String,
    origin_event_id: String,
    title: String,
    body: String,
    deep_link: Option<String>,
    read_state: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Converts a raw DB row into the aggregate, re-validating every field via
/// [`NotificationItem::from_parts`] (including parsing `read_state` back
/// through [`ReadState`]'s allow-list — an unrecognized value can never
/// reach the rest of the app).
fn row_to_aggregate(row: NotificationItemRow) -> Result<NotificationItem, RepoError> {
    let read_state: ReadState = row
        .read_state
        .parse()
        .map_err(|err: bff_core::ParseReadStateError| RepoError::OperationFailed(err.to_string()))?;

    NotificationItem::from_parts(
        row.id,
        row.consultant_id,
        row.origin_capability,
        row.origin_event_id,
        row.title,
        row.body,
        row.deep_link,
        read_state,
        row.created_at,
    )
    .map_err(|err: bff_core::NotificationItemError| RepoError::OperationFailed(err.to_string()))
}

#[async_trait]
impl NotificationRepository for PgNotificationRepository {
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Vec<NotificationItem>, RepoError> {
        let rows = sqlx::query_as!(
            NotificationItemRow,
            r#"
            SELECT id, consultant_id, origin_capability, origin_event_id,
                   title, body, deep_link, read_state, created_at
            FROM notification_items
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

    async fn find_by_id(&self, id: Uuid) -> Result<Option<NotificationItem>, RepoError> {
        let row = sqlx::query_as!(
            NotificationItemRow,
            r#"
            SELECT id, consultant_id, origin_capability, origin_event_id,
                   title, body, deep_link, read_state, created_at
            FROM notification_items
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        row.map(row_to_aggregate).transpose()
    }

    async fn save(&self, item: &NotificationItem) -> Result<SaveOutcome, RepoError> {
        let result = sqlx::query!(
            r#"
            INSERT INTO notification_items
                (id, consultant_id, origin_capability, origin_event_id,
                 title, body, deep_link, read_state, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (origin_capability, origin_event_id) DO NOTHING
            "#,
            item.id(),
            item.consultant_id(),
            item.origin_capability(),
            item.origin_event_id(),
            item.title(),
            item.body(),
            item.deep_link(),
            item.read_state().as_str(),
            item.created_at(),
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(if result.rows_affected() == 1 { SaveOutcome::Inserted } else { SaveOutcome::AlreadyExists })
    }

    async fn mark_read(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query!(
            r#"
            UPDATE notification_items
            SET read_state = 'read'
            WHERE id = $1 AND read_state = 'unread'
            "#,
            id,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
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

    fn t0() -> chrono::DateTime<chrono::Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    fn item(consultant_id: &str, origin_event_id: &str) -> NotificationItem {
        NotificationItem::new(
            consultant_id,
            "sales",
            origin_event_id,
            "Referral submitted",
            "A new referral was submitted for review.",
            Some("https://app.example.com/sales/referrals/1".to_string()),
            t0(),
        )
        .expect("valid notification item")
    }

    #[tokio::test]
    async fn save_and_find_by_consultant_id_round_trips_an_item() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let item = item("consultant-1", "event-1");
        let outcome = repo.save(&item).await.expect("save failed");
        assert_eq!(outcome, SaveOutcome::Inserted);

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0], item);
    }

    /// `find_by_id` (PROMPT-32's NOTIFY/LISTEN bridge reconstruction path):
    /// a saved item is found by its own id, and an unknown id is `Ok(None)`,
    /// not an error.
    #[tokio::test]
    async fn find_by_id_finds_a_saved_item_and_returns_none_for_an_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let item = item("consultant-1", "event-1");
        repo.save(&item).await.expect("save failed");

        let found = repo.find_by_id(item.id()).await.expect("find_by_id failed");
        assert_eq!(found, Some(item));

        let missing = repo.find_by_id(Uuid::new_v4()).await.expect("find_by_id failed");
        assert_eq!(missing, None);
    }

    #[tokio::test]
    async fn find_by_consultant_id_returns_empty_for_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

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
        let repo = PgNotificationRepository::new(pool.clone());

        let first = item("consultant-1", "event-1");
        let redelivered = item("consultant-1", "event-1");

        let first_outcome = repo.save(&first).await.expect("first save failed");
        let second_outcome = repo.save(&redelivered).await.expect("second save failed");

        assert_eq!(first_outcome, SaveOutcome::Inserted);
        assert_eq!(second_outcome, SaveOutcome::AlreadyExists);

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_items WHERE origin_capability = 'sales' AND origin_event_id = 'event-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("count query failed");
        assert_eq!(count, 1);
    }

    /// Redelivery must not clobber locally-progressed `read_state` — the
    /// concrete failure mode `DO NOTHING` (not `DO UPDATE`) protects
    /// against.
    #[tokio::test]
    async fn duplicate_delivery_after_mark_read_does_not_regress_read_state() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let first = item("consultant-1", "event-1");
        repo.save(&first).await.expect("first save failed");
        repo.mark_read(first.id()).await.expect("mark_read failed");

        let redelivered = item("consultant-1", "event-1");
        repo.save(&redelivered).await.expect("redelivery save failed");

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].read_state(), ReadState::Read);
    }

    #[tokio::test]
    async fn mark_read_transitions_an_unread_item() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let item = item("consultant-1", "event-1");
        repo.save(&item).await.expect("save failed");

        repo.mark_read(item.id()).await.expect("mark_read failed");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].read_state(), ReadState::Read);
    }

    /// Repository-layer `mark_read` is lenient: calling it twice, or on an
    /// unknown id, is a no-op rather than an error (see the trait doc
    /// comment for why this differs from the aggregate's strict behavior).
    #[tokio::test]
    async fn mark_read_twice_is_a_no_op_not_an_error() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let item = item("consultant-1", "event-1");
        repo.save(&item).await.expect("save failed");

        repo.mark_read(item.id()).await.expect("first mark_read failed");
        repo.mark_read(item.id()).await.expect("second mark_read failed");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap();
        assert_eq!(found[0].read_state(), ReadState::Read);
    }

    #[tokio::test]
    async fn mark_read_on_unknown_id_is_a_no_op_not_an_error() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgNotificationRepository::new(pool);

        repo.mark_read(Uuid::new_v4()).await.expect("mark_read should be a no-op, not an error");
    }
}
