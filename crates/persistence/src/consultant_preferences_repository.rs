//! Postgres-backed `ConsultantPreferencesRepository` (PROMPT-20, ADR-010).
//!
//! Stores the aggregate as a single row per consultant in the
//! `consultant_preferences` table (see the
//! `20260718002751_consultant_preferences` migration), with `preferences`
//! as a JSONB object keyed by [`PreferenceKey`]'s wire strings.
//! `bff_core::PreferenceKey`'s allow-list (enum + `FromStr`/`Deserialize`)
//! is what guarantees a row read back out of this table can never surface
//! an unknown key to the rest of the app — an unknown key anywhere in the
//! JSONB object would fail to deserialize into
//! `HashMap<PreferenceKey, String>` before it ever reaches
//! `ConsultantPreferences::from_parts`.
//!
//! `consultant_id TEXT PRIMARY KEY` plus `INSERT ... ON CONFLICT (consultant_id)
//! DO UPDATE` in [`PgConsultantPreferencesRepository::save`]/
//! [`PgConsultantPreferencesRepository::upsert_preference`] is what
//! satisfies invariant 2 ("exactly one `ConsultantPreferences` aggregate
//! per consultant") at the storage boundary — Postgres's primary key
//! constraint makes a second row for the same consultant impossible.

use std::collections::HashMap;

use async_trait::async_trait;
use bff_core::{ConsultantPreferences, ConsultantPreferencesRepository, PreferenceKey, RepoError};
use sqlx::PgPool;
use sqlx::types::Json;

/// `ConsultantPreferencesRepository` implemented against Postgres via
/// `sqlx`'s compile-time-checked `query!`/`query_as!` macros (ADR-010).
pub struct PgConsultantPreferencesRepository {
    pool: PgPool,
}

impl PgConsultantPreferencesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Row shape selected out of `consultant_preferences` — only the columns
/// this repository's read path needs (not `created_at`/`updated_at`, which
/// the [`ConsultantPreferences`] aggregate itself doesn't model).
struct PreferencesRow {
    consultant_id: String,
    preferences: Json<HashMap<PreferenceKey, String>>,
}

#[async_trait]
impl ConsultantPreferencesRepository for PgConsultantPreferencesRepository {
    async fn find_by_consultant_id(
        &self,
        consultant_id: &str,
    ) -> Result<Option<ConsultantPreferences>, RepoError> {
        let row = sqlx::query_as!(
            PreferencesRow,
            r#"
            SELECT consultant_id, preferences AS "preferences: Json<HashMap<PreferenceKey, String>>"
            FROM consultant_preferences
            WHERE consultant_id = $1
            "#,
            consultant_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        row.map(|row| ConsultantPreferences::from_parts(row.consultant_id, row.preferences.0))
            .transpose()
            .map_err(|err| RepoError::OperationFailed(err.to_string()))
    }

    async fn save(&self, prefs: &ConsultantPreferences) -> Result<(), RepoError> {
        let preferences = Json(prefs.preferences().clone());
        sqlx::query!(
            r#"
            INSERT INTO consultant_preferences (consultant_id, preferences, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (consultant_id)
            DO UPDATE SET preferences = EXCLUDED.preferences, updated_at = now()
            "#,
            prefs.consultant_id(),
            preferences as _,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }

    async fn upsert_preference(
        &self,
        consultant_id: &str,
        key: PreferenceKey,
        value: String,
    ) -> Result<(), RepoError> {
        let key = key.as_str();
        sqlx::query!(
            r#"
            INSERT INTO consultant_preferences (consultant_id, preferences, updated_at)
            VALUES ($1, jsonb_build_object($2::text, $3::text), now())
            ON CONFLICT (consultant_id)
            DO UPDATE SET
                preferences = consultant_preferences.preferences || jsonb_build_object($2::text, $3::text),
                updated_at = now()
            "#,
            consultant_id,
            key,
            value,
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepoError::OperationFailed(err.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bff_core::ConsultantPreferences;
    use sqlx::Row;
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

    /// Round-trips a full `ConsultantPreferences` aggregate through
    /// Postgres: save, then read back, and confirm the data matches.
    #[tokio::test]
    async fn save_and_find_round_trips_preferences() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantPreferencesRepository::new(pool);

        let mut prefs = ConsultantPreferences::new("consultant-1").unwrap();
        prefs.set_preference(PreferenceKey::Theme, "dark".to_string());
        prefs.set_preference(PreferenceKey::DefaultView, "dashboard".to_string());

        repo.save(&prefs).await.expect("save failed");

        let found = repo
            .find_by_consultant_id("consultant-1")
            .await
            .expect("find failed")
            .expect("expected a saved row");

        assert_eq!(found, prefs);
    }

    #[tokio::test]
    async fn find_returns_none_for_unknown_consultant() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantPreferencesRepository::new(pool);

        let found = repo.find_by_consultant_id("does-not-exist").await.expect("find failed");

        assert!(found.is_none());
    }

    /// Invariant 2 ("exactly one aggregate per consultant"): saving twice
    /// for the same consultant must upsert, never create a second row.
    #[tokio::test]
    async fn save_twice_for_same_consultant_upserts_not_duplicates() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantPreferencesRepository::new(pool.clone());

        let mut prefs = ConsultantPreferences::new("consultant-1").unwrap();
        prefs.set_preference(PreferenceKey::Theme, "dark".to_string());
        repo.save(&prefs).await.expect("first save failed");

        prefs.set_preference(PreferenceKey::Theme, "light".to_string());
        repo.save(&prefs).await.expect("second save failed");

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM consultant_preferences")
            .fetch_one(&pool)
            .await
            .expect("count query failed")
            .get("count");
        assert_eq!(count, 1);

        let found = repo.find_by_consultant_id("consultant-1").await.unwrap().unwrap();
        assert_eq!(found.get_preference(PreferenceKey::Theme), Some("light"));
    }

    /// Same invariant, exercised via the narrower `upsert_preference` path.
    #[tokio::test]
    async fn upsert_preference_creates_then_updates_without_duplicating_rows() {
        let (pool, _container) = migrated_pool().await;
        let repo = PgConsultantPreferencesRepository::new(pool.clone());

        repo.upsert_preference("consultant-2", PreferenceKey::Theme, "dark".to_string())
            .await
            .expect("first upsert failed");
        repo.upsert_preference(
            "consultant-2",
            PreferenceKey::NotificationChannelOptIn,
            "email".to_string(),
        )
        .await
        .expect("second upsert failed");

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM consultant_preferences")
            .fetch_one(&pool)
            .await
            .expect("count query failed")
            .get("count");
        assert_eq!(count, 1);

        let found = repo.find_by_consultant_id("consultant-2").await.unwrap().unwrap();
        assert_eq!(found.get_preference(PreferenceKey::Theme), Some("dark"));
        assert_eq!(
            found.get_preference(PreferenceKey::NotificationChannelOptIn),
            Some("email")
        );
    }
}
