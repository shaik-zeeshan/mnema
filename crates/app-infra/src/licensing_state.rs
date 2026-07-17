//! Read/write projection over the single-row `licensing_state` table (migration
//! `0048`). Since the licensegate migration the OS keychain is the source of
//! truth for every licensing artifact (key, receipt, first-seen stamp); this
//! table holds ONLY the anti-rollback high-water mark — the one value that must
//! survive a keychain-less reinstall of the same DB. The row (id = 1) is seeded
//! by the migration, so reads/updates never need to insert it.

use sqlx::SqlitePool;

use crate::Result;

/// The anti-rollback high-water mark (max unix-ms timestamp ever observed).
pub async fn read_max_timestamp_seen(pool: &SqlitePool) -> Result<i64> {
    let value: i64 =
        sqlx::query_scalar("SELECT max_timestamp_ever_seen_ms FROM licensing_state WHERE id = 1")
            .fetch_one(pool)
            .await?;
    Ok(value)
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

    // The REAL migration chain (like db.rs), so migration 0048's reshaped row
    // and `CHECK (id = 1)` are what these tests exercise — a hand-rolled schema
    // mirror would go green while the shipped migration drifted.
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn seeded_pool() -> SqlitePool {
        // Migration 0039 creates a `vec0` virtual table; the auto-extension is
        // process-global, so register it here too or a filtered test run fails.
        crate::db::register_vec0_auto_extension();
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
            // The row exists without any test-side insert (0048 seeds it)…
            assert_eq!(read_max_timestamp_seen(&pool).await.unwrap(), 0);
            // …and the CHECK(id = 1) rejects any second row.
            let err = sqlx::query("INSERT INTO licensing_state (id) VALUES (2)")
                .execute(&pool)
                .await;
            assert!(err.is_err(), "CHECK(id = 1) must reject a second row");
        });
    }

    #[test]
    fn migration_0048_drops_old_columns_but_keeps_the_high_water_mark() {
        block_on(async {
            // Simulate a field DB: the 0047 shape with data in every column,
            // then apply exactly the 0048 SQL. Only the high-water mark may
            // survive — everything else is rebuilt from the keychain.
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db");
            sqlx::raw_sql(include_str!("../migrations/0047_licensing_state.sql"))
                .execute(&pool)
                .await
                .expect("0047 shape should apply");
            sqlx::raw_sql(
                "UPDATE licensing_state SET trial_started_at_ms = 111, \
                 max_timestamp_ever_seen_ms = 12345, license_id = 'lic-old', \
                 tier = 'license', issued_at_ms = 1, update_through_ms = 2, \
                 email = 'a@b.co' WHERE id = 1",
            )
            .execute(&pool)
            .await
            .expect("old-shape row should populate");

            sqlx::raw_sql(include_str!(
                "../migrations/0048_licensing_state_licensegate.sql"
            ))
            .execute(&pool)
            .await
            .expect("0048 should apply on a populated 0047 DB");

            assert_eq!(read_max_timestamp_seen(&pool).await.unwrap(), 12345);
            // The old columns are gone (old artifacts are abandoned, not read).
            let err = sqlx::query("SELECT trial_started_at_ms FROM licensing_state")
                .fetch_one(&pool)
                .await;
            assert!(err.is_err(), "old columns must not survive 0048");
        });
    }

    #[test]
    fn max_timestamp_only_rises() {
        block_on(async {
            let pool = seeded_pool().await;
            assert_eq!(read_max_timestamp_seen(&pool).await.unwrap(), 0);

            bump_max_timestamp_seen(&pool, 500).await.unwrap();
            bump_max_timestamp_seen(&pool, 200).await.unwrap();
            assert_eq!(read_max_timestamp_seen(&pool).await.unwrap(), 500);

            bump_max_timestamp_seen(&pool, 750).await.unwrap();
            assert_eq!(read_max_timestamp_seen(&pool).await.unwrap(), 750);
        });
    }
}
