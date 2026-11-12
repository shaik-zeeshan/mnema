//! One derivation window's writes as ONE transaction: every Activity the window
//! inserts or extends, plus the `derivation_run` row that marks the window
//! done. Quit can drop an in-flight derivation tick at any await; written
//! piecemeal, a cancel between the Activities and the run row left Activities
//! saved with the window still un-done, so the next launch re-derived it
//! (re-billed) and nested duplicates. Dropped before `commit_with_run`, the
//! transaction rolls back and the window is simply derived again.

use sqlx::{Executor, Sqlite, SqliteConnection, Transaction};

use super::store::{category_to_str, focus_to_str, now_ms, NewActivity, NewDerivationRun};
use super::UserContextStore;
use crate::Result;

/// An open write transaction for one derivation window. Not committed until
/// [`ActivityWindowWrite::commit_with_run`].
pub struct ActivityWindowWrite {
    tx: Transaction<'static, Sqlite>,
}

impl UserContextStore {
    /// Start one window's atomic write. Holds the writer lock until committed
    /// or dropped, so only the write loop belongs inside it (no AI call).
    pub async fn begin_activity_window_write(&self) -> Result<ActivityWindowWrite> {
        Ok(ActivityWindowWrite {
            tx: self.db.begin_write().await?,
        })
    }
}

impl ActivityWindowWrite {
    /// [`UserContextStore::insert_activity_with_evidence`] inside this window's
    /// transaction.
    pub async fn insert_activity(&mut self, draft: &NewActivity) -> Result<i64> {
        insert_activity_tx(&mut self.tx, draft).await
    }

    /// [`UserContextStore::extend_activity_with_evidence`] inside this window's
    /// transaction; `false` when the row is gone.
    pub async fn extend_activity(&mut self, activity_id: i64, draft: &NewActivity) -> Result<bool> {
        extend_activity_tx(&mut self.tx, activity_id, draft).await
    }

    /// Write the run row that marks the window done and commit everything.
    pub async fn commit_with_run(mut self, run: NewDerivationRun) -> Result<i64> {
        let id = insert_derivation_run_with(&mut *self.tx, &run).await?;
        self.tx.commit().await?;
        Ok(id)
    }
}

/// Insert an Activity and its evidence rows (duplicate evidence ignored).
pub(super) async fn insert_activity_tx(
    conn: &mut SqliteConnection,
    draft: &NewActivity,
) -> Result<i64> {
    let activity_id = sqlx::query(
        "INSERT INTO user_context_activities \
            (title, summary, category, focus, started_at_ms, ended_at_ms, derivation_run_id, created_at_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&draft.title)
    .bind(&draft.summary)
    .bind(draft.category.map(category_to_str))
    .bind(draft.focus.map(focus_to_str))
    .bind(draft.started_at_ms)
    .bind(draft.ended_at_ms)
    .bind(draft.derivation_run_id)
    .bind(now_ms())
    .execute(&mut *conn)
    .await?
    .last_insert_rowid();

    for evidence in &draft.evidence {
        sqlx::query(
            "INSERT OR IGNORE INTO user_context_activity_evidence \
                (activity_id, subject_type, subject_id, captured_at_ms, is_headline) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(activity_id)
        .bind(&evidence.subject_type)
        .bind(evidence.subject_id)
        .bind(evidence.captured_at_ms)
        .bind(evidence.is_headline as i64)
        .execute(&mut *conn)
        .await?;
    }
    Ok(activity_id)
}

/// Grow an Activity to cover `draft` (see
/// [`UserContextStore::extend_activity_with_evidence`] for the rules).
pub(super) async fn extend_activity_tx(
    conn: &mut SqliteConnection,
    activity_id: i64,
    draft: &NewActivity,
) -> Result<bool> {
    let updated = sqlx::query(
        "UPDATE user_context_activities SET \
            title = COALESCE(NULLIF(?2, ''), title), summary = COALESCE(NULLIF(?3, ''), summary), \
            category = COALESCE(?4, category), focus = COALESCE(?5, focus), \
            started_at_ms = MIN(started_at_ms, ?6), ended_at_ms = MAX(ended_at_ms, ?7), \
            updated_at_ms = ?8 \
         WHERE id = ?1",
    )
    .bind(activity_id)
    .bind(&draft.title)
    .bind(&draft.summary)
    .bind(draft.category.map(category_to_str))
    .bind(draft.focus.map(focus_to_str))
    .bind(draft.started_at_ms)
    .bind(draft.ended_at_ms)
    .bind(now_ms())
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        return Ok(false);
    }

    for evidence in &draft.evidence {
        sqlx::query(
            "INSERT OR IGNORE INTO user_context_activity_evidence \
                (activity_id, subject_type, subject_id, captured_at_ms, is_headline) \
             VALUES (?1, ?2, ?3, ?4, 0)",
        )
        .bind(activity_id)
        .bind(&evidence.subject_type)
        .bind(evidence.subject_id)
        .bind(evidence.captured_at_ms)
        .execute(&mut *conn)
        .await?;
    }
    Ok(true)
}

/// Insert a derivation-run ledger row on a pool or inside a transaction.
pub(super) async fn insert_derivation_run_with<'c, E>(
    executor: E,
    run: &NewDerivationRun,
) -> Result<i64>
where
    E: Executor<'c, Database = Sqlite>,
{
    Ok(sqlx::query(
        "INSERT INTO user_context_derivation_runs \
            (kind, window_start_ms, window_end_ms, status, activities_derived, \
             conclusions_derived, input_tokens, output_tokens, provider, model, error, \
             ungrounded, guardrail_suppressed, below_formation_bar, resurface_blocked, \
             superseded, supersede_degraded, supersede_blocked, \
             created_at_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                 ?16, ?17, ?18, ?19)",
    )
    .bind(&run.kind)
    .bind(run.window_start_ms)
    .bind(run.window_end_ms)
    .bind(&run.status)
    .bind(run.activities_derived)
    .bind(run.conclusions_derived)
    .bind(run.input_tokens)
    .bind(run.output_tokens)
    .bind(run.provider.as_deref())
    .bind(run.model.as_deref())
    .bind(run.error.as_deref())
    .bind(run.gate_drops.ungrounded)
    .bind(run.gate_drops.guardrail_suppressed)
    .bind(run.gate_drops.below_formation_bar)
    .bind(run.gate_drops.resurface_blocked)
    .bind(run.gate_drops.superseded)
    .bind(run.gate_drops.supersede_degraded)
    .bind(run.gate_drops.supersede_blocked)
    .bind(now_ms())
    .execute(executor)
    .await?
    .last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_context::{DistillationGateDrops, NewActivityEvidence};
    use crate::AppInfra;

    fn activity(frame: i64) -> NewActivity {
        NewActivity {
            title: "Edited the config".to_string(),
            summary: String::new(),
            category: None,
            focus: None,
            started_at_ms: frame,
            ended_at_ms: frame,
            derivation_run_id: None,
            evidence: vec![NewActivityEvidence {
                subject_type: "frame".to_string(),
                subject_id: frame,
                captured_at_ms: Some(frame),
                is_headline: true,
            }],
        }
    }

    fn completed_run(activities_derived: i64) -> NewDerivationRun {
        NewDerivationRun {
            kind: "activity".to_string(),
            window_start_ms: Some(0),
            window_end_ms: Some(100),
            status: "completed".to_string(),
            activities_derived,
            conclusions_derived: 0,
            input_tokens: 0,
            output_tokens: 0,
            provider: None,
            model: None,
            error: None,
            gate_drops: DistillationGateDrops::default(),
        }
    }

    /// Activities and the run that marks their window done are both saved or
    /// both absent: a write dropped before commit (quit mid-tick) leaves
    /// neither, so the window is simply derived again.
    #[test]
    fn activities_and_their_run_row_commit_together_or_not_at_all() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let unique = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos();
                let dir = std::env::temp_dir().join(format!("activity-window-write-{unique}"));
                std::fs::create_dir_all(&dir).expect("test dir");
                let infra = AppInfra::initialize(&dir).await.expect("app infra");
                let store = infra.user_context();
                let saved = || async {
                    (
                        store.count_activities().await.expect("count"),
                        store.list_derivation_runs(10).await.expect("runs").len(),
                    )
                };

                let mut dropped = store.begin_activity_window_write().await.expect("begin");
                dropped
                    .insert_activity(&activity(10))
                    .await
                    .expect("insert");
                drop(dropped);
                assert_eq!(saved().await, (0, 0), "a dropped window saves nothing");

                let mut write = store.begin_activity_window_write().await.expect("begin");
                let id = write.insert_activity(&activity(10)).await.expect("insert");
                assert!(write
                    .extend_activity(id, &activity(20))
                    .await
                    .expect("extend"));
                write
                    .commit_with_run(completed_run(1))
                    .await
                    .expect("commit");
                assert_eq!(
                    saved().await,
                    (1, 1),
                    "the Activity and its run land together"
                );
                let run = &store.list_derivation_runs(1).await.expect("runs")[0];
                assert_eq!(
                    (run.status.as_str(), run.activities_derived),
                    ("completed", 1)
                );
                let _ = std::fs::remove_dir_all(&dir);
            });
    }
}
