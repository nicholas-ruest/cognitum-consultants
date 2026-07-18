//! Postgres-backed `DashboardConfigurationRepository` (PROMPT-21, ADR-010).
//!
//! Stores the aggregate across two normalized tables —
//! `dashboard_configurations` (one row per consultant) and `card_placements`
//! (one row per [`bff_core::CardPlacement`], `UNIQUE (consultant_id,
//! card_position)`) — see the `20260718003944_dashboard_configuration`
//! migration for why this differs from `consultant_preferences`'s single
//! JSONB column: invariant 2 (unique card positions) is explicit in
//! `consultant-experience-context.md` §1.2, and a real `UNIQUE` constraint
//! lets Postgres enforce it as defense-in-depth alongside
//! `DashboardConfiguration::add_card`'s own check.
//!
//! [`PgDashboardConfigurationRepository::save`] replaces a consultant's
//! entire card set in one transaction (delete-then-reinsert) rather than
//! diffing — the aggregate is small (a handful of cards at most) and this
//! keeps the write path simple and always consistent with the in-memory
//! aggregate, at the cost of reusing surrogate `id` values rather than
//! preserving them across saves (nothing in the aggregate models or reads
//! that `id` column, so this is invisible to callers).

use async_trait::async_trait;
use bff_core::{CardPlacement, DashboardConfiguration, DashboardConfigurationRepository, RepoError};
use sqlx::PgPool;

/// `DashboardConfigurationRepository` implemented against Postgres via
/// `sqlx`'s compile-time-checked `query!`/`query_as!` macros (ADR-010).
pub struct PgDashboardConfigurationRepository {
    pool: PgPool,
}

impl PgDashboardConfigurationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `card_placements` for a given consultant.
struct CardPlacementRow {
    module_id: String,
    card_position: i32,
}

#[async_trait]
impl DashboardConfigurationRepository for PgDashboardConfigurationRepository {
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Option<DashboardConfiguration>, RepoError> {
        let config_exists = sqlx::query!(
            r#"SELECT consultant_id FROM dashboard_configurations WHERE consultant_id = $1"#,
            consultant_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        let Some(_) = config_exists else {
            return Ok(None);
        };

        let rows = sqlx::query_as!(
            CardPlacementRow,
            r#"
            SELECT module_id, card_position
            FROM card_placements
            WHERE consultant_id = $1
            ORDER BY card_position
            "#,
            consultant_id,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        let cards = rows
            .into_iter()
            .map(|row| CardPlacement::new(row.module_id, row.card_position as u32))
            .collect();

        DashboardConfiguration::from_parts(consultant_id.to_owned(), cards)
            .map(Some)
            .map_err(|err| RepoError::OperationFailed(err.to_string()))
    }

    async fn save(&self, config: &DashboardConfiguration) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(|err| RepoError::StoreUnavailable(err.to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO dashboard_configurations (consultant_id, updated_at)
            VALUES ($1, now())
            ON CONFLICT (consultant_id)
            DO UPDATE SET updated_at = now()
            "#,
            config.consultant_id(),
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        sqlx::query!(r#"DELETE FROM card_placements WHERE consultant_id = $1"#, config.consultant_id())
            .execute(&mut *tx)
            .await
            .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        for card in config.cards() {
            sqlx::query!(
                r#"
                INSERT INTO card_placements (consultant_id, module_id, card_position)
                VALUES ($1, $2, $3)
                "#,
                config.consultant_id(),
                card.module_id(),
                card.position() as i32,
            )
            .execute(&mut *tx)
            .await
            .map_err(|err| RepoError::OperationFailed(err.to_string()))?;
        }

        tx.commit().await.map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }

    async fn delete_by_consultant_id(&self, consultant_id: &str) -> Result<(), RepoError> {
        // `card_placements` rows cascade-delete via the `ON DELETE CASCADE`
        // foreign key, so deleting the parent row is sufficient.
        sqlx::query!(r#"DELETE FROM dashboard_configurations WHERE consultant_id = $1"#, consultant_id,)
            .execute(&self.pool)
            .await
            .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bff_core::DashboardConfigurationError;
    use sqlx::Row;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;

    use super::*;

    fn all_permitted(_module_id: &str) -> bool {
        true
    }

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

    /// Round-trips a full `DashboardConfiguration` aggregate through
    /// Postgres: save, then read back, and confirm the data (including card
    /// order) matches.
    #[tokio::test]
    async fn save_and_find_round_trips_a_configuration() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool);

        let mut config = DashboardConfiguration::new("consultant-1", &all_permitted).unwrap();
        config.add_card(CardPlacement::new("legal", 3), &all_permitted).unwrap();

        repo.save(&config).await.expect("save failed");

        let found = repo
            .find_by_consultant_id("consultant-1")
            .await
            .expect("find failed")
            .expect("expected a saved row");

        assert_eq!(found, config);
    }

    #[tokio::test]
    async fn find_returns_none_for_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool);

        let found = repo.find_by_consultant_id("does-not-exist").await.expect("find failed");

        assert!(found.is_none());
    }

    /// Invariant 3 ("exactly one configuration per consultant"): saving
    /// twice for the same consultant must upsert, never create a second
    /// configuration row.
    #[tokio::test]
    async fn save_twice_for_same_consultant_upserts_not_duplicates() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool.clone());

        let mut config = DashboardConfiguration::new("consultant-1", &all_permitted).unwrap();
        repo.save(&config).await.expect("first save failed");

        config.add_card(CardPlacement::new("legal", 5), &all_permitted).unwrap();
        repo.save(&config).await.expect("second save failed");

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM dashboard_configurations")
            .fetch_one(&pool)
            .await
            .expect("count query failed")
            .get("count");
        assert_eq!(count, 1);

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap().unwrap();
        assert_eq!(found.cards().len(), config.cards().len());
    }

    /// `save`'s delete-then-reinsert must not leak stale card rows: after a
    /// second save with fewer cards, only the current cards remain.
    #[tokio::test]
    async fn save_replaces_the_full_card_set() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool.clone());

        let mut config = DashboardConfiguration::new("consultant-1", &all_permitted).unwrap();
        config.add_card(CardPlacement::new("legal", 10), &all_permitted).unwrap();
        repo.save(&config).await.expect("first save failed");

        config.remove_card(10);
        repo.save(&config).await.expect("second save failed");

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap().unwrap();
        assert!(found.cards().iter().all(|card| card.position() != 10));
    }

    #[tokio::test]
    async fn delete_by_consultant_id_removes_the_configuration_and_its_cards() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool.clone());

        let config = DashboardConfiguration::new("consultant-1", &all_permitted).unwrap();
        repo.save(&config).await.expect("save failed");

        repo.delete_by_consultant_id("consultant-1").await.expect("delete failed");

        assert!(repo.find_by_consultant_id("consultant-1").await.unwrap().is_none());

        let card_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM card_placements")
            .fetch_one(&pool)
            .await
            .expect("count query failed")
            .get("count");
        assert_eq!(card_count, 0, "cards must cascade-delete with the parent configuration");
    }

    #[tokio::test]
    async fn delete_by_consultant_id_is_not_an_error_for_an_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgDashboardConfigurationRepository::new(pool);

        repo.delete_by_consultant_id("does-not-exist").await.expect("delete should be a no-op, not an error");
    }

    /// Invariant 2's DB-level defense-in-depth: a direct duplicate-position
    /// insert (bypassing the aggregate entirely) must be rejected by the
    /// `UNIQUE (consultant_id, card_position)` constraint itself, proving
    /// the DB layer enforces this invariant too, not just
    /// `DashboardConfiguration::add_card`.
    #[tokio::test]
    async fn duplicate_position_insert_is_rejected_at_the_database_layer() {
        let (pool, _container) = migrated_pool().await;

        sqlx::query!(
            r#"INSERT INTO dashboard_configurations (consultant_id) VALUES ($1)"#,
            "consultant-1",
        )
        .execute(&pool)
        .await
        .expect("seed configuration insert failed");

        sqlx::query!(
            r#"INSERT INTO card_placements (consultant_id, module_id, card_position) VALUES ($1, $2, $3)"#,
            "consultant-1",
            "sales",
            0,
        )
        .execute(&pool)
        .await
        .expect("first card insert failed");

        let duplicate_result = sqlx::query!(
            r#"INSERT INTO card_placements (consultant_id, module_id, card_position) VALUES ($1, $2, $3)"#,
            "consultant-1",
            "commit",
            0,
        )
        .execute(&pool)
        .await;

        let err = duplicate_result.expect_err("duplicate (consultant_id, card_position) must be rejected");
        assert!(
            err.as_database_error().is_some_and(|db_err| db_err.is_unique_violation()),
            "expected a unique constraint violation, got: {err}",
        );
    }

    /// `from_parts`'s own defense-in-depth (invariant 2 re-checked on
    /// reconstruction) — confirms the error type surfaces correctly if it
    /// were ever hit via this path, using the aggregate directly rather
    /// than the DB.
    #[test]
    fn from_parts_error_variant_is_position_already_occupied() {
        let cards = vec![CardPlacement::new("sales", 0), CardPlacement::new("commit", 0)];
        let err = DashboardConfiguration::from_parts("consultant-1".to_string(), cards).unwrap_err();
        assert_eq!(err, DashboardConfigurationError::PositionAlreadyOccupied(0));
    }
}
