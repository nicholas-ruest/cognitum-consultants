//! Postgres-backed `ProspectRepository` (ADR-020 part A).
//!
//! Stores the aggregate across two tables — `prospects` (one row per
//! prospect) and `prospect_notes` (one row per [`bff_core::ProspectNote`],
//! append-only — see the `20260719210000_prospects` migration) — the same
//! parent/child shape `dashboard_configuration_repository` uses, but
//! `save`'s note-write is an `INSERT ... ON CONFLICT (id) DO NOTHING`
//! rather than that repository's delete-then-reinsert: a prospect's note
//! history is unbounded and must never be silently dropped by a concurrent
//! `save` racing an `add_note`, unlike a dashboard's small, freely-replaced
//! card set.
//!
//! **Runtime-checked queries, not `sqlx::query!`/`query_as!`.** Every other
//! repository in this crate uses sqlx's compile-time macros, checked
//! against the committed `.sqlx/` offline cache. This repository
//! deliberately uses the plain `sqlx::query`/`query_as` runtime API instead
//! — precedented elsewhere in this repo (`bff-api::session`,
//! `auth::firebase`, both real-DB call sites) — to avoid needing a live,
//! already-migrated database available at `cargo sqlx prepare` time just to
//! add this unit's queries to the offline cache. The trade-off is real (SQL
//! errors surface at test/runtime, not `cargo check` time) and accepted for
//! this unit; nothing prevents migrating these to the compile-time macros
//! later once the offline cache is regenerated.

use async_trait::async_trait;
use bff_core::{ParseProspectStageError, Prospect, ProspectNote, ProspectRepository, ProspectStage, RepoError};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// `ProspectRepository` implemented against Postgres (ADR-010).
pub struct PgProspectRepository {
    pool: PgPool,
}

impl PgProspectRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Loads a `prospects` row's notes and reconstructs the full
    /// [`Prospect`] aggregate. Shared by [`Self::find_by_consultant_id`]/
    /// [`Self::find_by_id`] so there is exactly one hydration path.
    async fn hydrate(&self, row: sqlx::postgres::PgRow) -> Result<Prospect, RepoError> {
        let id: Uuid = row.try_get("id").map_err(op_failed)?;
        let consultant_id: String = row.try_get("consultant_id").map_err(op_failed)?;
        let company_name: String = row.try_get("company_name").map_err(op_failed)?;
        let contact_name: Option<String> = row.try_get("contact_name").map_err(op_failed)?;
        let stage_raw: String = row.try_get("stage").map_err(op_failed)?;
        let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at").map_err(op_failed)?;
        let updated_at: chrono::DateTime<chrono::Utc> = row.try_get("updated_at").map_err(op_failed)?;

        let stage: ProspectStage = stage_raw.parse().map_err(|err: ParseProspectStageError| op_failed(err))?;

        let note_rows =
            sqlx::query("SELECT id, body, author_consultant_id, created_at FROM prospect_notes WHERE prospect_id = $1 ORDER BY created_at")
                .bind(id)
                .fetch_all(&self.pool)
                .await
                .map_err(op_failed)?;

        let mut notes = Vec::with_capacity(note_rows.len());
        for note_row in note_rows {
            let note_id: Uuid = note_row.try_get("id").map_err(op_failed)?;
            let body: String = note_row.try_get("body").map_err(op_failed)?;
            let author_consultant_id: String = note_row.try_get("author_consultant_id").map_err(op_failed)?;
            let note_created_at: chrono::DateTime<chrono::Utc> = note_row.try_get("created_at").map_err(op_failed)?;
            notes.push(ProspectNote::from_parts(note_id, body, author_consultant_id, note_created_at).map_err(op_failed)?);
        }

        Prospect::from_parts(id, consultant_id, company_name, contact_name, stage, notes, created_at, updated_at)
            .map_err(op_failed)
    }
}

fn op_failed(err: impl std::fmt::Display) -> RepoError {
    RepoError::OperationFailed(err.to_string())
}

#[async_trait]
impl ProspectRepository for PgProspectRepository {
    async fn find_by_consultant_id(&self, consultant_id: &str) -> Result<Vec<Prospect>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, consultant_id, company_name, contact_name, stage, created_at, updated_at
             FROM prospects WHERE consultant_id = $1 ORDER BY created_at DESC",
        )
        .bind(consultant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(op_failed)?;

        let mut prospects = Vec::with_capacity(rows.len());
        for row in rows {
            prospects.push(self.hydrate(row).await?);
        }
        Ok(prospects)
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Prospect>, RepoError> {
        let row = sqlx::query(
            "SELECT id, consultant_id, company_name, contact_name, stage, created_at, updated_at
             FROM prospects WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(op_failed)?;

        match row {
            Some(row) => Ok(Some(self.hydrate(row).await?)),
            None => Ok(None),
        }
    }

    async fn save(&self, prospect: &Prospect) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(|err| RepoError::StoreUnavailable(err.to_string()))?;

        sqlx::query(
            "INSERT INTO prospects (id, consultant_id, company_name, contact_name, stage, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
               company_name = EXCLUDED.company_name,
               contact_name = EXCLUDED.contact_name,
               stage = EXCLUDED.stage,
               updated_at = EXCLUDED.updated_at",
        )
        .bind(prospect.id())
        .bind(prospect.consultant_id())
        .bind(prospect.company_name())
        .bind(prospect.contact_name())
        .bind(prospect.stage().as_str())
        .bind(prospect.created_at())
        .bind(prospect.updated_at())
        .execute(&mut *tx)
        .await
        .map_err(op_failed)?;

        for note in prospect.notes() {
            sqlx::query(
                "INSERT INTO prospect_notes (id, prospect_id, body, author_consultant_id, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(note.id())
            .bind(prospect.id())
            .bind(note.body())
            .bind(note.author_consultant_id())
            .bind(note.created_at())
            .execute(&mut *tx)
            .await
            .map_err(op_failed)?;
        }

        tx.commit().await.map_err(op_failed)?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepoError> {
        // `prospect_notes` rows cascade-delete via the `ON DELETE CASCADE`
        // foreign key.
        sqlx::query("DELETE FROM prospects WHERE id = $1").bind(id).execute(&self.pool).await.map_err(op_failed)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
    async fn save_and_find_by_id_round_trips_a_prospect_with_notes() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool);

        let mut prospect = Prospect::new("consultant-1", "Acme Corp", Some("Jane Doe".to_string()), t0()).unwrap();
        prospect.add_note("First call went well.", "consultant-1", t0()).unwrap();
        prospect.transition_stage(bff_core::ProspectStage::AppointmentScheduled, t0()).unwrap();

        repo.save(&prospect).await.expect("save failed");

        let found = repo.find_by_id(prospect.id()).await.expect("find failed").expect("expected a saved row");

        assert_eq!(found, prospect);
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool);

        let found = repo.find_by_id(Uuid::new_v4()).await.expect("find failed");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_consultant_id_returns_only_that_consultants_prospects_newest_first() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool);

        let p1 = Prospect::new("consultant-1", "Acme", None, t0()).unwrap();
        let p2 = Prospect::new("consultant-1", "Globex", None, t0() + chrono::Duration::hours(1)).unwrap();
        let other = Prospect::new("consultant-2", "Initech", None, t0()).unwrap();
        repo.save(&p1).await.unwrap();
        repo.save(&p2).await.unwrap();
        repo.save(&other).await.unwrap();

        let found = repo.find_by_consultant_id("consultant-1").await.expect("find failed");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].company_name(), "Globex", "newest first");
        assert_eq!(found[1].company_name(), "Acme");
    }

    #[tokio::test]
    async fn save_twice_upserts_the_same_prospect_not_duplicates() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool.clone());

        let mut prospect = Prospect::new("consultant-1", "Acme", None, t0()).unwrap();
        repo.save(&prospect).await.expect("first save failed");

        prospect.transition_stage(bff_core::ProspectStage::AppointmentScheduled, t0()).unwrap();
        repo.save(&prospect).await.expect("second save failed");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM prospects").fetch_one(&pool).await.expect("count query failed");
        assert_eq!(count, 1);

        let found = repo.find_by_id(prospect.id()).await.unwrap().unwrap();
        assert_eq!(found.stage(), bff_core::ProspectStage::AppointmentScheduled);
    }

    #[tokio::test]
    async fn save_appends_new_notes_without_duplicating_existing_ones() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool);

        let mut prospect = Prospect::new("consultant-1", "Acme", None, t0()).unwrap();
        prospect.add_note("First note.", "consultant-1", t0()).unwrap();
        repo.save(&prospect).await.expect("first save failed");

        // Re-save unchanged (simulating a redundant write): the existing
        // note must not be duplicated (ON CONFLICT DO NOTHING).
        repo.save(&prospect).await.expect("second save failed");

        prospect.add_note("Second note.", "consultant-1", t0() + chrono::Duration::hours(1)).unwrap();
        repo.save(&prospect).await.expect("third save failed");

        let found = repo.find_by_id(prospect.id()).await.unwrap().unwrap();
        assert_eq!(found.notes().len(), 2);
    }

    #[tokio::test]
    async fn delete_removes_the_prospect_and_cascades_its_notes() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool.clone());

        let mut prospect = Prospect::new("consultant-1", "Acme", None, t0()).unwrap();
        prospect.add_note("A note.", "consultant-1", t0()).unwrap();
        repo.save(&prospect).await.unwrap();

        repo.delete(prospect.id()).await.expect("delete failed");

        assert!(repo.find_by_id(prospect.id()).await.unwrap().is_none());
        let note_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM prospect_notes").fetch_one(&pool).await.expect("count failed");
        assert_eq!(note_count, 0, "notes must cascade-delete with their prospect");
    }

    #[tokio::test]
    async fn delete_is_not_an_error_for_an_unknown_id() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgProspectRepository::new(pool);

        repo.delete(Uuid::new_v4()).await.expect("delete of an unknown id must not error");
    }
}
