//! Read/write projection over the single-row `licensing_state` table (migration
//! `0047`). The OS keychain is the source of truth for the signed license key +
//! trial record; this table is a fast-read cache for the startup gate and the
//! Settings UI, plus the anti-rollback high-water mark. The row (id = 1) is
//! seeded by the migration, so reads/updates never need to insert it.

use sqlx::{Row, SqlitePool};

use crate::Result;

/// A snapshot of the `licensing_state` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicensingStateRow {
    /// NULL until the first successful Capture starts the trial clock.
    pub trial_started_at_ms: Option<i64>,
    /// Anti-rollback high-water mark (max unix-ms timestamp ever observed).
    pub max_timestamp_ever_seen_ms: i64,
    pub license_id: Option<String>,
    pub tier: Option<String>,
    pub issued_at_ms: Option<i64>,
    pub update_through_ms: Option<i64>,
    pub email: Option<String>,
}

/// Read the (always-present) single licensing-state row.
pub async fn read_licensing_state(pool: &SqlitePool) -> Result<LicensingStateRow> {
    let row = sqlx::query(
        "SELECT trial_started_at_ms, max_timestamp_ever_seen_ms, license_id, tier, \
         issued_at_ms, update_through_ms, email \
         FROM licensing_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await?;

    Ok(LicensingStateRow {
        trial_started_at_ms: row.try_get("trial_started_at_ms")?,
        max_timestamp_ever_seen_ms: row.try_get("max_timestamp_ever_seen_ms")?,
        license_id: row.try_get("license_id")?,
        tier: row.try_get("tier")?,
        issued_at_ms: row.try_get("issued_at_ms")?,
        update_through_ms: row.try_get("update_through_ms")?,
        email: row.try_get("email")?,
    })
}

/// Start the trial clock, once. Only writes `trial_started_at_ms` when it is
/// still NULL, so repeated calls (e.g. every capture start) are no-ops.
pub async fn set_trial_started_once(pool: &SqlitePool, now_ms: i64) -> Result<()> {
    sqlx::query(
        "UPDATE licensing_state SET trial_started_at_ms = ? \
         WHERE id = 1 AND trial_started_at_ms IS NULL",
    )
    .bind(now_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear the stored trial start. Dev-only test knob (`MNEMA_TRIAL_RESET`);
/// nothing in the production flow ever un-starts a trial.
pub async fn clear_trial_started(pool: &SqlitePool) -> Result<()> {
    sqlx::query("UPDATE licensing_state SET trial_started_at_ms = NULL WHERE id = 1")
        .execute(pool)
        .await?;
    Ok(())
}

/// Raise the anti-rollback high-water mark to `max(current, now_ms)`.
pub async fn bump_max_timestamp_seen(pool: &SqlitePool, now_ms: i64) -> Result<()> {
    sqlx::query(
        "UPDATE licensing_state \
         SET max_timestamp_ever_seen_ms = MAX(max_timestamp_ever_seen_ms, ?) \
         WHERE id = 1",
    )
    .bind(now_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// Cache the verified license fields into the projection row.
pub async fn cache_license_fields(
    pool: &SqlitePool,
    license_id: &str,
    tier: &str,
    issued_at_ms: i64,
    update_through_ms: i64,
    email: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE licensing_state \
         SET license_id = ?, tier = ?, issued_at_ms = ?, update_through_ms = ?, email = ? \
         WHERE id = 1",
    )
    .bind(license_id)
    .bind(tier)
    .bind(issued_at_ms)
    .bind(update_through_ms)
    .bind(email)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clear the cached license fields back to NULL (delete / deactivate).
pub async fn clear_license_fields(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "UPDATE licensing_state \
         SET license_id = NULL, tier = NULL, issued_at_ms = NULL, \
         update_through_ms = NULL, email = NULL \
         WHERE id = 1",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    // The REAL migration chain (like db.rs), so migration 0047's seeded row and
    // `CHECK (id = 1)` are what these tests exercise — a hand-rolled schema
    // mirror would go green while the shipped migration drifted.
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seeded_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db");
        MIGRATOR.run(&pool).await.expect("migrations should apply");
        pool
    }

    #[test]
    fn migration_seeds_the_single_row_and_enforces_id_one() {
        block_on(async {
            let pool = seeded_pool().await;
            // The row exists without any test-side insert (0047 seeds it)…
            let state = read_licensing_state(&pool).await.unwrap();
            assert_eq!(state.max_timestamp_ever_seen_ms, 0);
            // …and the CHECK(id = 1) rejects any second row.
            let err = sqlx::query("INSERT INTO licensing_state (id) VALUES (2)")
                .execute(&pool)
                .await;
            assert!(err.is_err(), "CHECK(id = 1) must reject a second row");
        });
    }

    #[test]
    fn trial_start_writes_once() {
        block_on(async {
            let pool = seeded_pool().await;

            assert_eq!(
                read_licensing_state(&pool).await.unwrap().trial_started_at_ms,
                None
            );

            set_trial_started_once(&pool, 1_000).await.unwrap();
            assert_eq!(
                read_licensing_state(&pool).await.unwrap().trial_started_at_ms,
                Some(1_000)
            );

            // Second call is a no-op: the original start survives.
            set_trial_started_once(&pool, 9_999).await.unwrap();
            assert_eq!(
                read_licensing_state(&pool).await.unwrap().trial_started_at_ms,
                Some(1_000)
            );
        });
    }

    #[test]
    fn max_timestamp_only_rises() {
        block_on(async {
            let pool = seeded_pool().await;
            assert_eq!(
                read_licensing_state(&pool)
                    .await
                    .unwrap()
                    .max_timestamp_ever_seen_ms,
                0
            );

            bump_max_timestamp_seen(&pool, 500).await.unwrap();
            bump_max_timestamp_seen(&pool, 200).await.unwrap();
            assert_eq!(
                read_licensing_state(&pool)
                    .await
                    .unwrap()
                    .max_timestamp_ever_seen_ms,
                500
            );

            bump_max_timestamp_seen(&pool, 750).await.unwrap();
            assert_eq!(
                read_licensing_state(&pool)
                    .await
                    .unwrap()
                    .max_timestamp_ever_seen_ms,
                750
            );
        });
    }

    #[test]
    fn license_fields_round_trip_and_clear() {
        block_on(async {
            let pool = seeded_pool().await;

            cache_license_fields(&pool, "lic-1", "standard", 100, 200, "a@b.co")
                .await
                .unwrap();
            let state = read_licensing_state(&pool).await.unwrap();
            assert_eq!(state.license_id.as_deref(), Some("lic-1"));
            assert_eq!(state.tier.as_deref(), Some("standard"));
            assert_eq!(state.issued_at_ms, Some(100));
            assert_eq!(state.update_through_ms, Some(200));
            assert_eq!(state.email.as_deref(), Some("a@b.co"));

            clear_license_fields(&pool).await.unwrap();
            let cleared = read_licensing_state(&pool).await.unwrap();
            assert_eq!(cleared.license_id, None);
            assert_eq!(cleared.tier, None);
            assert_eq!(cleared.issued_at_ms, None);
            assert_eq!(cleared.update_through_ms, None);
            assert_eq!(cleared.email, None);
        });
    }
}
