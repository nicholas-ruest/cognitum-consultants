//! Postgres-backed `ConsultantActionItemRepository` (ADR-020 part B).
//!
//! One row per item in `consultant_action_items` (see the
//! `20260719210500_consultant_action_items` migration) — same
//! one-row-per-aggregate, `id`-keyed shape as `notification_items`.
//!
//! Uses the plain `sqlx::query`/`query_as` runtime API rather than the
//! compile-time `query!`/`query_as!` macros — see
//! `prospect_repository`'s module docs for why (same rationale, same
//! trade-off, applies identically here).

use async_trait::async_trait;
use bff_core::{ConsultantActionItem, ConsultantActionItemRepository, RepoError};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// `ConsultantActionItemRepository` implemented against Postgres (ADR-010).
pub struct PgConsultantActionItemRepository {
    pool: PgPool,
}

impl PgConsultantActionItemRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `consultant_action_items`.
#[derive(FromRow)]
struct ConsultantActionItemRow {
    id: Uuid,
    consultant_id: String,
    title: String,
    notes: Option<String>,
    done: bool,
    linked_prospect_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn op_failed(err: impl std::fmt::Display) -> RepoError {
    RepoError::OperationFailed(err.to_string())
}

fn row_to_aggregate(row: ConsultantActionItemRow) -> Result<ConsultantActionItem, RepoError> {
    ConsultantActionItem::from_parts(
        row.id,
        row.consultant_id,
        row.title,
        row.notes,
        row.done,
        row.linked_prospect_id,
        row.created_at,
        row.updated_at,
    )
    .map_err(op_failed)
}

#[async_trait]
impl ConsultantActionItemRepository for PgConsultantActionItemRepository {
    async fn find_by_consultant_id(&self, consultant_id: &str) -> Result<Vec<ConsultantActionItem>, RepoError> {
        let rows = sqlx::query_as::<_, ConsultantActionItemRow>(
            "SELECT id, consultant_id, title, notes, done, linked_prospect_id, created_at, updated_at
             FROM consultant_action_items WHERE consultant_id = $1 ORDER BY created_at DESC",
        )
        .bind(consultant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(op_failed)?;

        rows.into_iter().map(row_to_aggregate).collect()
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ConsultantActionItem>, RepoError> {
        let row = sqlx::query_as::<_, ConsultantActionItemRow>(
            "SELECT id, consultant_id, title, notes, done, linked_prospect_id, created_at, updated_at
             FROM consultant_action_items WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(op_failed)?;

        row.map(row_to_aggregate).transpose()
    }

    async fn save(&self, item: &ConsultantActionItem) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO consultant_action_items
                (id, consultant_id, title, notes, done, linked_prospect_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET
               title = EXCLUDED.title,
               notes = EXCLUDED.notes,
               done = EXCLUDED.done,
               linked_prospect_id = EXCLUDED.linked_prospect_id,
               updated_at = EXCLUDED.updated_at",
        )
        .bind(item.id())
        .bind(item.consultant_id())
        .bind(item.title())
        .bind(item.notes())
        .bind(item.done())
        .bind(item.linked_prospect_id())
        .bind(item.created_at())
        .bind(item.updated_at())
        .execute(&self.pool)
        .await
        .map_err(op_failed)?;

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM consultant_action_items WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(op_failed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bff_core::ProspectRepository;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    async fn migrated_pool() -> (PgPool, testcontainers_modules::testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.expect("failed to start postgres container");
        let host = container.get_host().await.expect("failed to resolve container host");
        let port = container.get_host_port_ipv4(5432).await.expect("failed to resolve container port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = crate::create_pool(&database_url).await.expect("create_pool failed to connect");
        sqlx::migrate!("./migrations").run(&pool).await.expect("migration failed to run");

        (pool, container)
    }

    fn t0() -> chrono::DateTime<chrono::Utc> {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn save_and_find_by_id_round_trips_an_item() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool);

        let item = ConsultantActionItem::new("consultant-1", "Call Acme back", None, None, t0()).unwrap();
        repo.save(&item).await.expect("save failed");

        let found = repo.find_by_id(item.id()).await.expect("find failed").expect("expected a saved row");

        assert_eq!(found, item);
    }

    #[tokio::test]
    async fn save_persists_an_optional_linked_prospect_id() {
        let (pool, _container) = migrated_pool().await;

        // The FK requires a real prospects row to reference.
        let prospect_repo = crate::PgProspectRepository::new(pool.clone());
        let prospect = bff_core::Prospect::new("consultant-1", "Acme", None, t0()).unwrap();
        prospect_repo.save(&prospect).await.unwrap();

        let repo = PgConsultantActionItemRepository::new(pool);
        let item =
            ConsultantActionItem::new("consultant-1", "Follow up", None, Some(prospect.id()), t0()).unwrap();
        repo.save(&item).await.expect("save failed");

        let found = repo.find_by_id(item.id()).await.unwrap().unwrap();
        assert_eq!(found.linked_prospect_id(), Some(prospect.id()));
    }

    #[tokio::test]
    async fn find_by_consultant_id_returns_only_that_consultants_items_newest_first() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool);

        let a = ConsultantActionItem::new("consultant-1", "First", None, None, t0()).unwrap();
        let b = ConsultantActionItem::new("consultant-1", "Second", None, None, t0() + chrono::Duration::hours(1)).unwrap();
        let other = ConsultantActionItem::new("consultant-2", "Not mine", None, None, t0()).unwrap();
        repo.save(&a).await.unwrap();
        repo.save(&b).await.unwrap();
        repo.save(&other).await.unwrap();

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title(), "Second", "newest first");
        assert_eq!(found[1].title(), "First");
    }

    #[tokio::test]
    async fn save_twice_upserts_not_duplicates() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool.clone());

        let mut item = ConsultantActionItem::new("consultant-1", "Call Acme", None, None, t0()).unwrap();
        repo.save(&item).await.expect("first save failed");

        item.set_done(true, t0() + chrono::Duration::hours(1));
        repo.save(&item).await.expect("second save failed");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM consultant_action_items")
            .fetch_one(&pool)
            .await
            .expect("count query failed");
        assert_eq!(count, 1);

        let found = repo.find_by_id(item.id()).await.unwrap().unwrap();
        assert!(found.done());
    }

    #[tokio::test]
    async fn delete_removes_the_item() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool);

        let item = ConsultantActionItem::new("consultant-1", "Call Acme", None, None, t0()).unwrap();
        repo.save(&item).await.unwrap();

        repo.delete(item.id()).await.expect("delete failed");

        assert!(repo.find_by_id(item.id()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_is_not_an_error_for_an_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool);

        repo.delete(Uuid::new_v4()).await.expect("delete of an unknown id must not error");
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantActionItemRepository::new(pool);

        assert!(repo.find_by_id(Uuid::new_v4()).await.unwrap().is_none());
    }
}
