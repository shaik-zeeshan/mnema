//! Quick Recall's recent queries: one JSON value in the encrypted `app_settings`
//! table (key `quick_recall.recent_queries`, migration `0001`), never
//! localStorage. They take the conversations' contract (migration `0028`
//! header): Retention prunes entries past its cutoff, Wipe User Context deletes
//! the key, and Delete Recent Capture leaves them alone. Mirrors the
//! `local_offset_minutes` upsert/read pattern.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::Result;

const RECENT_QUERIES_KEY: &str = "quick_recall.recent_queries";

/// How many queries are kept — searches and asks in one list.
pub const MAX_RECENT_QUERIES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentQuery {
    pub text: String,
    /// `true` when it was asked (Ask mode), `false` for a search.
    pub ask: bool,
    /// When it was last run, unix ms.
    pub at: i64,
}

/// The stored list, newest first. Never stored or unreadable → empty.
pub async fn load_recent_queries(pool: &SqlitePool) -> Result<Vec<RecentQuery>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
        .bind(RECENT_QUERIES_KEY)
        .fetch_optional(pool)
        .await?;
    Ok(value
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

/// Replace the list (the frontend owns ordering, de-duplication and ⌫-removal).
/// Stored newest first and capped at [`MAX_RECENT_QUERIES`]; returns what was kept.
pub async fn store_recent_queries(
    pool: &SqlitePool,
    mut queries: Vec<RecentQuery>,
) -> Result<Vec<RecentQuery>> {
    queries.retain(|query| !query.text.trim().is_empty());
    queries.sort_by(|a, b| b.at.cmp(&a.at));
    queries.truncate(MAX_RECENT_QUERIES);
    write(pool, &queries).await?;
    Ok(queries)
}

/// Retention: drop entries run before `cutoff_ms`. Returns how many went.
pub async fn prune_recent_queries_older_than(pool: &SqlitePool, cutoff_ms: i64) -> Result<u64> {
    let mut queries = load_recent_queries(pool).await?;
    let before = queries.len();
    queries.retain(|query| query.at >= cutoff_ms);
    let pruned = before - queries.len();
    if pruned > 0 {
        write(pool, &queries).await?;
    }
    Ok(pruned as u64)
}

/// Wipe User Context: forget every recent query.
pub async fn delete_recent_queries(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM app_settings WHERE key = ?1")
        .bind(RECENT_QUERIES_KEY)
        .execute(pool)
        .await?;
    Ok(())
}

async fn write(pool: &SqlitePool, queries: &[RecentQuery]) -> Result<()> {
    let json = serde_json::to_string(queries).expect("recent queries serialize");
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(RECENT_QUERIES_KEY)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppInfra;

    fn query(text: &str, at: i64) -> RecentQuery {
        RecentQuery {
            text: text.to_string(),
            ask: false,
            at,
        }
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    #[test]
    fn store_keeps_the_newest_eight_and_prune_and_delete_forget() {
        block_on(async {
            let dir = std::env::temp_dir().join(format!(
                "mnema-recent-queries-{}-{}",
                std::process::id(),
                now_nanos()
            ));
            let infra = AppInfra::initialize(&dir).await.expect("infra");
            let pool = infra.write_pool();
            assert!(load_recent_queries(pool).await.unwrap().is_empty());

            let mut many: Vec<RecentQuery> =
                (0..10).map(|at| query(&format!("q{at}"), at)).collect();
            many.push(query("  ", 99));
            let kept = store_recent_queries(pool, many).await.unwrap();
            assert_eq!(kept.len(), MAX_RECENT_QUERIES);
            assert_eq!(kept[0], query("q9", 9), "newest first, blank dropped");
            assert_eq!(load_recent_queries(pool).await.unwrap(), kept);

            assert_eq!(prune_recent_queries_older_than(pool, 7).await.unwrap(), 5);
            let left = load_recent_queries(pool).await.unwrap();
            assert_eq!(left.iter().map(|q| q.at).collect::<Vec<_>>(), vec![9, 8, 7]);

            delete_recent_queries(pool).await.unwrap();
            assert!(load_recent_queries(pool).await.unwrap().is_empty());
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
