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
