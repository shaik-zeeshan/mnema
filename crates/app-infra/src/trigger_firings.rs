//! The Trigger firing ledger (issue #176, ADR 0058).
//!
//! Every Firing decision writes exactly one row here: `completed` (with the
//! run's conversation link), `skipped` (nothing to work with — honest reason),
//! or `failed` (the AI run did not complete after retries). The ledger is what
//! makes firings accountable under good-news-only delivery: notifications fire
//! only on `completed`, so skips and failures surface ONLY as last-run status
//! read from these rows. The per-trigger Cooldown is also enforced from the
//! newest row, so it survives an app restart.
//!
//! Trigger definitions live in `triggers.json` (config, not DB — ADR 0058):
//! `trigger_id` is an id string across the file/DB boundary with deliberately
//! no FK; [`TriggerFiringsStore::delete_firings`] is the delete-by-id half of
//! that contract, for when a trigger is deleted.

use crate::db::CaptureDb;
use crate::Result;

/// How a Firing ended. Stored as its lowercase string in `trigger_firings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerFiringOutcome {
    /// The run finished with an answer; the only outcome that notifies.
    Completed,
    /// No run happened because there was nothing to work with.
    Skipped,
    /// The run did not complete after retries.
    Failed,
}

impl TriggerFiringOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            TriggerFiringOutcome::Completed => "completed",
            TriggerFiringOutcome::Skipped => "skipped",
            TriggerFiringOutcome::Failed => "failed",
        }
    }

    /// Parse a stored outcome. The CHECK constraint makes unknown values
    /// unreachable in practice; an unrecognized string reads as `Failed` (the
    /// honest degradation — never silently "completed").
    fn parse(value: &str) -> Self {
        match value {
            "completed" => TriggerFiringOutcome::Completed,
            "skipped" => TriggerFiringOutcome::Skipped,
            _ => TriggerFiringOutcome::Failed,
        }
    }
}

/// One ledger row.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerFiring {
    pub trigger_id: String,
    pub fired_at_ms: i64,
    pub outcome: TriggerFiringOutcome,
    pub reason: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Clone)]
pub struct TriggerFiringsStore {
    db: CaptureDb,
}

impl TriggerFiringsStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    /// Append one Firing outcome to the ledger.
    pub async fn record_firing(
        &self,
        trigger_id: &str,
        fired_at_ms: i64,
        outcome: TriggerFiringOutcome,
        reason: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO trigger_firings (trigger_id, fired_at_ms, outcome, reason, conversation_id) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(trigger_id)
        .bind(fired_at_ms)
        .bind(outcome.as_str())
        .bind(reason)
        .bind(conversation_id)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// The trigger's newest ledger row — the Cooldown anchor (ANY outcome
    /// counts: a skip or failure still holds the cooldown, per
    /// docs/triggers/CONTEXT.md "never fires again within 10 min of its last
    /// firing") and the Triggers page's last-run status.
    pub async fn last_firing(&self, trigger_id: &str) -> Result<Option<TriggerFiring>> {
        let row: Option<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT fired_at_ms, outcome, reason, conversation_id FROM trigger_firings \
             WHERE trigger_id = ?1 ORDER BY fired_at_ms DESC, rowid DESC LIMIT 1",
        )
        .bind(trigger_id)
        .fetch_optional(self.db.read())
        .await?;
        Ok(row.map(
            |(fired_at_ms, outcome, reason, conversation_id)| TriggerFiring {
                trigger_id: trigger_id.to_string(),
                fired_at_ms,
                outcome: TriggerFiringOutcome::parse(&outcome),
                reason,
                conversation_id,
            },
        ))
    }

    /// The trigger's newest `completed` firings that carry a conversation link,
    /// newest first, capped at `limit`. Feeds the Context Assembly (issue #183):
    /// previous run reports the next run of the same trigger compounds on.
    pub async fn recent_completed_firings(
        &self,
        trigger_id: &str,
        limit: u32,
    ) -> Result<Vec<TriggerFiring>> {
        let rows: Vec<(i64, Option<String>, String)> = sqlx::query_as(
            "SELECT fired_at_ms, reason, conversation_id FROM trigger_firings \
             WHERE trigger_id = ?1 AND outcome = 'completed' AND conversation_id IS NOT NULL \
             ORDER BY fired_at_ms DESC, rowid DESC LIMIT ?2",
        )
        .bind(trigger_id)
        .bind(limit)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(fired_at_ms, reason, conversation_id)| TriggerFiring {
                trigger_id: trigger_id.to_string(),
                fired_at_ms,
                outcome: TriggerFiringOutcome::Completed,
                reason,
                conversation_id: Some(conversation_id),
            })
            .collect())
    }

    /// The trigger's newest ledger rows, ANY outcome, newest first, capped at
    /// `limit`. Feeds the per-trigger runs ledger screen (issue #182): every
    /// firing — completed, skipped, failed — with its honest reason.
    pub async fn recent_firings(&self, trigger_id: &str, limit: u32) -> Result<Vec<TriggerFiring>> {
        let rows: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT fired_at_ms, outcome, reason, conversation_id FROM trigger_firings \
             WHERE trigger_id = ?1 ORDER BY fired_at_ms DESC, rowid DESC LIMIT ?2",
        )
        .bind(trigger_id)
        .bind(limit)
        .fetch_all(self.db.read())
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(fired_at_ms, outcome, reason, conversation_id)| TriggerFiring {
                    trigger_id: trigger_id.to_string(),
                    fired_at_ms,
                    outcome: TriggerFiringOutcome::parse(&outcome),
                    reason,
                    conversation_id,
                },
            )
            .collect())
    }

    /// Drop every ledger row for a deleted trigger (the no-FK contract's
    /// delete-by-id half; the management UI arrives with issue #182).
    pub async fn delete_firings(&self, trigger_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM trigger_firings WHERE trigger_id = ?1")
            .bind(trigger_id)
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
    /// `#[tokio::test]` (mirrors `trigger_state`'s test pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// An in-memory pool with just the `trigger_firings` table from migration
    /// `0051`.
    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE trigger_firings (
                trigger_id TEXT NOT NULL,
                fired_at_ms INTEGER NOT NULL,
                outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'skipped', 'failed')),
                reason TEXT,
                conversation_id TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("trigger_firings table");
        pool
    }

    #[test]
    fn ledger_records_all_three_outcomes_with_reasons_and_reads_the_latest() {
        block_on(async {
            let pool = test_pool().await;
            let store = TriggerFiringsStore::new(CaptureDb::single(pool.clone()));

            store
                .record_firing(
                    "evening",
                    1_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-1"),
                )
                .await
                .expect("completed row");
            store
                .record_firing(
                    "evening",
                    2_000,
                    TriggerFiringOutcome::Skipped,
                    Some("not recording during window"),
                    None,
                )
                .await
                .expect("skipped row");
            store
                .record_firing(
                    "evening",
                    3_000,
                    TriggerFiringOutcome::Failed,
                    Some("AI run did not complete after 3 attempts"),
                    None,
                )
                .await
                .expect("failed row");

            // Every firing decision left exactly one row.
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trigger_firings")
                .fetch_one(&pool)
                .await
                .expect("count");
            assert_eq!(count, 3);

            // The newest row wins, fields intact.
            assert_eq!(
                store.last_firing("evening").await.expect("read"),
                Some(TriggerFiring {
                    trigger_id: "evening".to_string(),
                    fired_at_ms: 3_000,
                    outcome: TriggerFiringOutcome::Failed,
                    reason: Some("AI run did not complete after 3 attempts".to_string()),
                    conversation_id: None,
                })
            );

            // A trigger with no rows reads None.
            assert_eq!(store.last_firing("other").await.expect("read"), None);
        });
    }

    #[test]
    fn cooldown_anchor_survives_a_restart() {
        block_on(async {
            let pool = test_pool().await;
            // "First app run": record a completed firing.
            let before = TriggerFiringsStore::new(CaptureDb::single(pool.clone()));
            before
                .record_firing(
                    "evening",
                    50_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-1"),
                )
                .await
                .expect("write");
            drop(before);

            // "After restart": a FRESH store over the same DB still sees the
            // firing, so the cooldown window holds across the restart.
            let after = TriggerFiringsStore::new(CaptureDb::single(pool));
            let last = after
                .last_firing("evening")
                .await
                .expect("read")
                .expect("row survives");
            assert_eq!(last.fired_at_ms, 50_000);
            assert_eq!(last.outcome, TriggerFiringOutcome::Completed);
        });
    }

    #[test]
    fn recent_completed_firings_returns_only_linked_completed_rows_newest_first() {
        block_on(async {
            let pool = test_pool().await;
            let store = TriggerFiringsStore::new(CaptureDb::single(pool));

            // A completed-with-conversation, a skip, a failure, a completed
            // WITHOUT a conversation link, and a newer completed-with-conversation.
            store
                .record_firing(
                    "evening",
                    1_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-old"),
                )
                .await
                .expect("row");
            store
                .record_firing("evening", 2_000, TriggerFiringOutcome::Skipped, Some("r"), None)
                .await
                .expect("row");
            store
                .record_firing("evening", 3_000, TriggerFiringOutcome::Failed, Some("r"), None)
                .await
                .expect("row");
            store
                .record_firing("evening", 4_000, TriggerFiringOutcome::Completed, None, None)
                .await
                .expect("row");
            store
                .record_firing(
                    "evening",
                    5_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-new"),
                )
                .await
                .expect("row");
            // Another trigger's rows never bleed in.
            store
                .record_firing(
                    "weekly",
                    6_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-other"),
                )
                .await
                .expect("row");

            let runs = store
                .recent_completed_firings("evening", 10)
                .await
                .expect("read");
            let ids: Vec<&str> = runs
                .iter()
                .filter_map(|run| run.conversation_id.as_deref())
                .collect();
            // Only completed rows WITH a conversation link, newest first.
            assert_eq!(ids, vec!["conv-new", "conv-old"]);

            // The limit caps the window.
            let capped = store
                .recent_completed_firings("evening", 1)
                .await
                .expect("read");
            assert_eq!(capped.len(), 1);
            assert_eq!(capped[0].conversation_id.as_deref(), Some("conv-new"));
        });
    }

    #[test]
    fn recent_firings_returns_every_outcome_newest_first_with_a_cap() {
        block_on(async {
            let pool = test_pool().await;
            let store = TriggerFiringsStore::new(CaptureDb::single(pool));
            store
                .record_firing(
                    "evening",
                    1_000,
                    TriggerFiringOutcome::Completed,
                    None,
                    Some("conv-1"),
                )
                .await
                .expect("row");
            store
                .record_firing(
                    "evening",
                    2_000,
                    TriggerFiringOutcome::Skipped,
                    Some("not recording"),
                    None,
                )
                .await
                .expect("row");
            store
                .record_firing(
                    "evening",
                    3_000,
                    TriggerFiringOutcome::Failed,
                    Some("AI run did not complete"),
                    Some("conv-1"),
                )
                .await
                .expect("row");
            // Another trigger's rows never bleed in.
            store
                .record_firing("weekly", 4_000, TriggerFiringOutcome::Completed, None, None)
                .await
                .expect("row");

            let runs = store.recent_firings("evening", 50).await.expect("read");
            assert_eq!(
                runs.iter()
                    .map(|run| (run.fired_at_ms, run.outcome))
                    .collect::<Vec<_>>(),
                vec![
                    (3_000, TriggerFiringOutcome::Failed),
                    (2_000, TriggerFiringOutcome::Skipped),
                    (1_000, TriggerFiringOutcome::Completed),
                ]
            );
            assert_eq!(runs[0].reason.as_deref(), Some("AI run did not complete"));
            assert_eq!(runs[0].conversation_id.as_deref(), Some("conv-1"));

            // The cap holds.
            let capped = store.recent_firings("evening", 2).await.expect("read");
            assert_eq!(capped.len(), 2);
            assert_eq!(capped[0].fired_at_ms, 3_000);
        });
    }

    #[test]
    fn delete_firings_removes_only_that_triggers_rows() {
        block_on(async {
            let pool = test_pool().await;
            let store = TriggerFiringsStore::new(CaptureDb::single(pool));
            store
                .record_firing("evening", 1_000, TriggerFiringOutcome::Completed, None, None)
                .await
                .expect("write");
            store
                .record_firing("weekly", 2_000, TriggerFiringOutcome::Skipped, Some("r"), None)
                .await
                .expect("write");

            store.delete_firings("evening").await.expect("delete");

            assert_eq!(store.last_firing("evening").await.expect("read"), None);
            assert!(store.last_firing("weekly").await.expect("read").is_some());
        });
    }
}
