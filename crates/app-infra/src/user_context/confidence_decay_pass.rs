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
    /// status)` update the row (stamping `last_decayed_at_ms = now`) and append a
    /// Confidence History snapshot, then prune history to the newest
    /// `max_snapshots_per_conclusion` per Conclusion (table-wide). An empty pass
    /// writes nothing, prune included.
    ///
    /// A row deleted, dismissed or pinned since it was listed is skipped (no
    /// update, no snapshot) rather than failing the pass. Returns the number of
    /// rows actually decayed.
    pub async fn apply_confidence_decay_pass(
        &self,
        decayed: &[(i64, f64, ConclusionStatus)],
        now: i64,
        max_snapshots_per_conclusion: i64,
    ) -> Result<u64> {
        if decayed.is_empty() {
            return Ok(0);
        }
        let mut applied = 0u64;
        let updated_at_ms = now_ms();
        let mut transaction = self.db.begin_write().await?;
        for &(id, confidence, status) in decayed {
            let updated = sqlx::query(
                "UPDATE user_context_conclusions \
                 SET confidence = ?2, status = ?3, last_decayed_at_ms = ?4, updated_at_ms = ?5 \
                 WHERE id = ?1 AND status IN ('visible', 'faded') AND COALESCE(pinned, 0) = 0",
            )
            .bind(id)
            .bind(confidence)
            .bind(status_to_str(status))
            .bind(now)
            .bind(updated_at_ms)
            .execute(&mut *transaction)
            .await?;
            // Gone or no longer decayable: its history INSERT would fail the FK
            // (deleted) or record a decay that did not happen.
            if updated.rows_affected() == 0 {
                continue;
            }
            applied += 1;
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
        Ok(applied)
    }
}

#[cfg(test)]
#[path = "confidence_decay_pass_tests.rs"]
mod tests;
