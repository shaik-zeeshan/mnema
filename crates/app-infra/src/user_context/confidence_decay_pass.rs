//! The confidence-decay beat's write side: one transaction per pass.
//!
//! Writing each decayed Conclusion as its own pair of auto-committed statements
//! cost ~17k tiny commits (and dozens of WAL checkpoints) per beat on a real
//! dossier. Batching the pass makes it one commit, and a failed pass leaves
//! nothing half-applied.

use capture_types::ConclusionStatus;

use super::store::{now_ms, status_to_str, UserContextStore};
use crate::Result;

impl UserContextStore {
    /// Persist one decay pass atomically: for each `(conclusion_id, confidence,
    /// status)` the same UPDATE as [`UserContextStore::update_conclusion_confidence`]
    /// (stamping `last_decayed_at_ms = now`) and the same Confidence History INSERT
    /// as [`UserContextStore::insert_confidence_snapshot`], then the history prune
    /// to `max_snapshots_per_conclusion`. An empty pass writes nothing, prune
    /// included.
    pub async fn apply_confidence_decay_pass(
        &self,
        decayed: &[(i64, f64, ConclusionStatus)],
        now: i64,
        max_snapshots_per_conclusion: i64,
    ) -> Result<()> {
        if decayed.is_empty() {
            return Ok(());
        }
        let updated_at_ms = now_ms();
        let mut transaction = self.db.begin_write().await?;
        for &(id, confidence, status) in decayed {
            sqlx::query(
                "UPDATE user_context_conclusions \
                 SET confidence = ?2, status = ?3, last_decayed_at_ms = ?4, updated_at_ms = ?5 \
                 WHERE id = ?1",
            )
            .bind(id)
            .bind(confidence)
            .bind(status_to_str(status))
            .bind(now)
            .bind(updated_at_ms)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO user_context_confidence_history \
                    (conclusion_id, confidence, snapshot_at_ms) \
                 VALUES (?1, ?2, ?3)",
            )
            .bind(id)
            .bind(confidence)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "DELETE FROM user_context_confidence_history \
             WHERE id IN (\
                 SELECT id FROM (\
                     SELECT id, \
                            ROW_NUMBER() OVER (\
                                PARTITION BY conclusion_id \
                                ORDER BY snapshot_at_ms DESC, id DESC\
                            ) AS rn \
                     FROM user_context_confidence_history\
                 ) ranked \
                 WHERE ranked.rn > ?1\
             )",
        )
        .bind(max_snapshots_per_conclusion.max(0))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "confidence_decay_pass_tests.rs"]
mod tests;
