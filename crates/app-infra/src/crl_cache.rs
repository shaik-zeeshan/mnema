//! Verbatim cache of the fetched CRL wire string in `app_settings` (key
//! `licensing.crl`, migration `0001`). The stored value is the signed document
//! exactly as received; it is re-verified with [`crate::parse_and_verify_crl`]
//! on every read, so the cache needs no trust and no schema migration. Mirrors
//! the `set_local_offset_minutes`/`local_offset_minutes` upsert/read pattern.

use sqlx::SqlitePool;

use crate::Result;

const CRL_CACHE_KEY: &str = "licensing.crl";

/// The cached signed CRL wire string, or `None` if never fetched. Verbatim —
/// callers re-verify before trusting it.
pub async fn load_cached_crl(pool: &SqlitePool) -> Result<Option<String>> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
            .bind(CRL_CACHE_KEY)
            .fetch_optional(pool)
            .await?;
    Ok(value)
}

/// Store the signed CRL wire string verbatim, overwriting any previous value.
/// Freshness (monotonic `issued_at`) is decided by the caller before storing.
pub async fn store_cached_crl(pool: &SqlitePool, wire: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(CRL_CACHE_KEY)
    .bind(wire)
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

    // The REAL migration chain (like db.rs), so the `app_settings` shape these
    // upserts hit is the shipped one, not a hand-rolled mirror that could drift.
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    async fn app_settings_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db");
        MIGRATOR.run(&pool).await.expect("migrations should apply");
        pool
    }

    #[test]
    fn load_returns_none_when_never_stored() {
        block_on(async {
            let pool = app_settings_pool().await;
            assert_eq!(load_cached_crl(&pool).await.unwrap(), None);
        });
    }

    #[test]
    fn store_then_load_returns_the_exact_wire_verbatim() {
        block_on(async {
            let pool = app_settings_pool().await;
            // A signature-bearing wire string with `.`, `+`, `/`, `=` — must survive
            // byte-for-byte (no trimming/normalization) so the re-verify on read works.
            let wire = "eyJzY2hlbWEiOjF9.AbC+/dEf==";
            store_cached_crl(&pool, wire).await.unwrap();
            assert_eq!(load_cached_crl(&pool).await.unwrap().as_deref(), Some(wire));
        });
    }

    #[test]
    fn store_overwrites_previous_value_not_appends() {
        block_on(async {
            let pool = app_settings_pool().await;
            store_cached_crl(&pool, "first.sig").await.unwrap();
            store_cached_crl(&pool, "second.sig").await.unwrap();
            // The ON CONFLICT branch: the newer (fresher) CRL replaces the old one.
            assert_eq!(
                load_cached_crl(&pool).await.unwrap().as_deref(),
                Some("second.sig")
            );
            // Exactly one row for the key (no duplicate/append).
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM app_settings WHERE key = ?1")
                    .bind(CRL_CACHE_KEY)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(count, 1);
        });
    }
}
