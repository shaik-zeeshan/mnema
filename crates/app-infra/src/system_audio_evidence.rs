//! Persistence for the system-audio denial heuristic (ADR 0052).
//!
//! A Core Audio process tap cannot be asked whether it was granted, so the only
//! evidence is what taps have delivered — and that has to outlive the session
//! that observed it, or a denied user would be told nothing until they happened
//! to be recording. One row in the `app_settings` kv table (migration `0001`,
//! reused exactly as `user_context.local_offset_minutes` does — no new schema).
//!
//! Deliberately a `String` in and a `String` out: the states and the rule that
//! folds them live in `capture-system-audio`, which this crate does not depend
//! on, and a bare value keeps it that way.

use crate::db::CaptureDb;
use crate::Result;

const EVIDENCE_KEY: &str = "system_audio.permission_evidence";

#[derive(Clone)]
pub struct SystemAudioEvidenceStore {
    db: CaptureDb,
}

impl SystemAudioEvidenceStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// The stored evidence, or `None` when no tap has ever been judged. A
    /// missing row is the fresh-install case and is never an error.
    pub async fn evidence(&self) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(EVIDENCE_KEY)
                .fetch_optional(self.db.read())
                .await?,
        )
    }

    /// Overwrites the evidence. Callers only ever strengthen it, so there is no
    /// compare-and-set here: last write wins, and every writer agrees.
    pub async fn set_evidence(&self, evidence: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(EVIDENCE_KEY)
        .bind(evidence)
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
    /// `#[tokio::test]` (mirrors `user_context::store`'s test pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// An in-memory store with just the kv table from migration `0001`.
    async fn test_store() -> SystemAudioEvidenceStore {
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

        SystemAudioEvidenceStore::new(CaptureDb::single(pool))
    }

    #[test]
    fn a_fresh_install_has_no_evidence() {
        block_on(async {
            assert_eq!(test_store().await.evidence().await.expect("read"), None);
        });
    }

    #[test]
    fn evidence_round_trips_and_overwrites() {
        block_on(async {
            let store = test_store().await;

            store.set_evidence("silent_session").await.expect("write");
            assert_eq!(
                store.evidence().await.expect("read"),
                Some("silent_session".to_string())
            );

            store.set_evidence("sound_heard").await.expect("rewrite");
            assert_eq!(
                store.evidence().await.expect("read"),
                Some("sound_heard".to_string())
            );
        });
    }
}
