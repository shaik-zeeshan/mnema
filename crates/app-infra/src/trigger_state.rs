//! Per-trigger last-fired persistence for the Triggers evaluator (issue #175).
//!
//! Trigger *definitions* are config (`triggers.json`, ADR 0058); the evaluator
//! only needs one durable fact per trigger — when its Schedule condition last
//! fired — so a missed-time wake within the same natural period still fires
//! (catch-up survives an app restart) and an already-fired occurrence never
//! double-fires. One row per trigger in the `app_settings` kv table (migration
//! `0001`, reused exactly as `system_audio_evidence` does — no new schema).
//!
//! The full firing ledger (outcome, reason, conversation link, cooldown) lives
//! in [`crate::trigger_firings`]; this kv row is only the evaluator's cursor.

use crate::db::CaptureDb;
use crate::Result;

const LAST_FIRED_KEY_PREFIX: &str = "triggers.last_fired.";

/// The GLOBAL Meeting release grace (docs/triggers/CONTEXT.md: not per-trigger
/// — it belongs to the one detector). Written by the #182 Settings knob; read
/// here. Absent = the caller's default (2 minutes).
const MEETING_RELEASE_GRACE_KEY: &str = "triggers.meeting_release_grace_minutes";

fn last_fired_key(trigger_id: &str) -> String {
    format!("{LAST_FIRED_KEY_PREFIX}{trigger_id}")
}

#[derive(Clone)]
pub struct TriggerStateStore {
    db: CaptureDb,
}

impl TriggerStateStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// When this trigger last fired (unix ms), or `None` when it never has. An
    /// unparseable stored value reads as `None` (never an error) so a corrupt
    /// row degrades to "never fired" rather than wedging the evaluator.
    pub async fn last_fired_ms(&self, trigger_id: &str) -> Result<Option<i64>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(last_fired_key(trigger_id))
                .fetch_optional(self.db.read())
                .await?;
        Ok(value.and_then(|value| value.trim().parse::<i64>().ok()))
    }

    /// The global Meeting release grace in minutes, or `None` when never set
    /// (caller applies the 2-minute default). An unparseable value degrades to
    /// `None`, never an error.
    pub async fn meeting_release_grace_minutes(&self) -> Result<Option<i64>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(MEETING_RELEASE_GRACE_KEY)
                .fetch_optional(self.db.read())
                .await?;
        Ok(value.and_then(|value| value.trim().parse::<i64>().ok()))
    }

    /// Record that this trigger fired at `fired_at_ms`. Last write wins.
    pub async fn set_last_fired_ms(&self, trigger_id: &str, fired_at_ms: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(last_fired_key(trigger_id))
        .bind(fired_at_ms.to_string())
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

    /// An in-memory store with just the kv table from migration `0001`.
    async fn test_store() -> TriggerStateStore {
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

        TriggerStateStore::new(CaptureDb::single(pool))
    }

    #[test]
    fn never_fired_reads_none() {
        block_on(async {
            let store = test_store().await;
            assert_eq!(store.last_fired_ms("evening").await.expect("read"), None);
        });
    }

    #[test]
    fn last_fired_round_trips_per_trigger_and_overwrites() {
        block_on(async {
            let store = test_store().await;

            store.set_last_fired_ms("evening", 1_000).await.expect("write");
            store.set_last_fired_ms("weekly", 2_000).await.expect("write");

            // Per-trigger isolation.
            assert_eq!(
                store.last_fired_ms("evening").await.expect("read"),
                Some(1_000)
            );
            assert_eq!(
                store.last_fired_ms("weekly").await.expect("read"),
                Some(2_000)
            );

            // Last write wins.
            store.set_last_fired_ms("evening", 5_000).await.expect("rewrite");
            assert_eq!(
                store.last_fired_ms("evening").await.expect("read"),
                Some(5_000)
            );
        });
    }

    #[test]
    fn meeting_release_grace_reads_the_kv_or_none() {
        block_on(async {
            let store = test_store().await;
            // Never set → None (the caller applies the 2-minute default).
            assert_eq!(
                store.meeting_release_grace_minutes().await.expect("read"),
                None
            );
            // The #182 Settings knob writes the same kv row.
            sqlx::query("INSERT INTO app_settings (key, value) VALUES (?1, '5')")
                .bind(super::MEETING_RELEASE_GRACE_KEY)
                .execute(store.db.write())
                .await
                .expect("insert grace row");
            assert_eq!(
                store.meeting_release_grace_minutes().await.expect("read"),
                Some(5)
            );
        });
    }

    #[test]
    fn corrupt_value_degrades_to_never_fired() {
        block_on(async {
            let store = test_store().await;
            sqlx::query("INSERT INTO app_settings (key, value) VALUES (?1, 'not-a-number')")
                .bind(super::last_fired_key("evening"))
                .execute(store.db.write())
                .await
                .expect("insert corrupt row");
            assert_eq!(store.last_fired_ms("evening").await.expect("read"), None);
        });
    }
}
