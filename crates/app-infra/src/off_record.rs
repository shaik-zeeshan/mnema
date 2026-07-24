//! Persistence for the timed off-the-record deadline.
//!
//! "Off the record" is the user pause of all capture families; the timed
//! variant carries a wall-clock deadline after which capture auto-resumes. The
//! deadline has to outlive the process — quitting mid-window must not turn a
//! 15-minute pause into a permanent one, and it must not turn it into an
//! immediate resume either. One row in the `app_settings` kv table (migration
//! `0001`, reused exactly as `system_audio.permission_evidence` does — no new
//! schema). An indefinite off-the-record deliberately does not persist: it is
//! the existing manual pause, which has never survived a restart.

use crate::db::CaptureDb;
use crate::Result;

const DEADLINE_KEY: &str = "capture.off_record_deadline_unix_ms";

#[derive(Clone)]
pub struct OffRecordStore {
    db: CaptureDb,
}

impl OffRecordStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// The persisted wall-clock deadline (unix ms), or `None` when no timed
    /// off-the-record window is armed. An unparseable stored value reads as
    /// `None` (never an error) so a corrupt row degrades to "not timed" rather
    /// than wedging startup.
    pub async fn deadline_unix_ms(&self) -> Result<Option<i64>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(DEADLINE_KEY)
                .fetch_optional(self.db.read())
                .await?;
        Ok(value.and_then(|value| value.trim().parse::<i64>().ok()))
    }

    /// Arms (or re-arms) the timed window. Last write wins.
    pub async fn set_deadline_unix_ms(&self, deadline_unix_ms: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(DEADLINE_KEY)
        .bind(deadline_unix_ms.to_string())
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// Disarms the window (back on the record, indefinite pause, stop, or an
    /// expired deadline handled at startup). Clearing an absent row is a no-op.
    pub async fn clear_deadline(&self) -> Result<()> {
        sqlx::query("DELETE FROM app_settings WHERE key = ?1")
            .bind(DEADLINE_KEY)
            .execute(self.db.write())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The crate's `tokio` dep has no `macros` feature, so there is no
    /// `#[tokio::test]` (mirrors `system_audio_evidence`'s test pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// An in-memory db with just the kv table from migration `0001`.
    async fn test_db() -> CaptureDb {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE app_settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&pool)
        .await
        .expect("app_settings table");
        CaptureDb::single(pool)
    }

    #[test]
    fn a_fresh_install_has_no_deadline() {
        block_on(async {
            let store = OffRecordStore::new(test_db().await);
            assert_eq!(store.deadline_unix_ms().await.expect("read"), None);
        });
    }

    // The restart path: the deadline written by one process is read back by a
    // fresh store over the same database — the state re-load a relaunch does.
    #[test]
    fn a_deadline_survives_a_store_reload() {
        block_on(async {
            let db = test_db().await;
            OffRecordStore::new(db.clone())
                .set_deadline_unix_ms(1_753_250_000_000)
                .await
                .expect("write");

            let reloaded = OffRecordStore::new(db);
            assert_eq!(
                reloaded.deadline_unix_ms().await.expect("read"),
                Some(1_753_250_000_000)
            );
        });
    }

    #[test]
    fn a_deadline_can_be_rearmed_and_cleared() {
        block_on(async {
            let store = OffRecordStore::new(test_db().await);

            store.set_deadline_unix_ms(1000).await.expect("write");
            store.set_deadline_unix_ms(2000).await.expect("re-arm");
            assert_eq!(store.deadline_unix_ms().await.expect("read"), Some(2000));

            store.clear_deadline().await.expect("clear");
            assert_eq!(store.deadline_unix_ms().await.expect("read"), None);

            store.clear_deadline().await.expect("clear absent is a no-op");
        });
    }

    #[test]
    fn a_corrupt_row_reads_as_no_deadline() {
        block_on(async {
            let db = test_db().await;
            sqlx::query("INSERT INTO app_settings (key, value) VALUES (?1, 'not-a-number')")
                .bind("capture.off_record_deadline_unix_ms")
                .execute(db.write())
                .await
                .expect("seed corrupt row");

            let store = OffRecordStore::new(db);
            assert_eq!(store.deadline_unix_ms().await.expect("read"), None);
        });
    }
}
