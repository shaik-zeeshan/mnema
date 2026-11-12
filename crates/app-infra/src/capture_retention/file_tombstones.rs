//! File tombstones: the durable record that a capture file must be removed.
//!
//! A retention sweep writes a `pending` tombstone for every file of a batch in
//! the SAME transaction that deletes the batch's rows, so a quit or crash
//! between the row delete and the file delete can never strand a file with no
//! row pointing at it. `retry_pending_file_tombstones` (run at the top of every
//! retention worker pass) is the healer that finishes whatever was left.

use sqlx::{Row, Sqlite};

use super::{delete_path_if_safe, CaptureRetentionStore, RetentionCleanupContext, SegmentFilePath};
use crate::Result;

impl CaptureRetentionStore {
    pub async fn retry_pending_file_tombstones(
        &self,
        context: &RetentionCleanupContext,
    ) -> Result<i64> {
        let rows = sqlx::query(
            "SELECT id, path, path_kind FROM retention_file_tombstones
             WHERE status IN ('pending', 'failed')
             ORDER BY id ASC",
        )
        .fetch_all(self.db.read())
        .await?;
        let mut resolved = 0_i64;
        for row in rows {
            let id: i64 = row.get("id");
            let path: String = row.get("path");
            match delete_path_off_thread(path, context).await {
                Ok(()) => {
                    resolved += 1;
                    sqlx::query(
                        "UPDATE retention_file_tombstones
                         SET status = 'resolved', resolved_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                    )
                    .bind(id)
                    .execute(self.db.write())
                    .await?;
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE retention_file_tombstones
                         SET status = 'failed', last_error = ?2, attempt_count = attempt_count + 1,
                             updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                    )
                    .bind(id)
                    .bind(error)
                    .execute(self.db.write())
                    .await?;
                }
            }
        }
        Ok(resolved)
    }

    pub async fn pending_file_tombstone_count(&self) -> Result<i64> {
        Ok(sqlx::query(
            "SELECT COUNT(*) AS count FROM retention_file_tombstones WHERE status IN ('pending', 'failed')",
        )
        .fetch_one(self.db.read())
        .await?
        .get("count"))
    }

    pub async fn insert_file_tombstone(
        &self,
        cleanup_run_id: Option<i64>,
        capture_segment_id: Option<i64>,
        path: &str,
        path_kind: &str,
        last_error: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO retention_file_tombstones
                (cleanup_run_id, capture_segment_id, path, path_kind, status, last_error, attempt_count)
             VALUES (?1, ?2, ?3, ?4, 'failed', ?5, 1)",
        )
        .bind(cleanup_run_id)
        .bind(capture_segment_id)
        .bind(path)
        .bind(path_kind)
        .bind(last_error)
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// Delete the files a committed batch tombstoned, off the async worker
    /// (`remove_dir_all` on a frame folder can take seconds). A removed file's
    /// tombstone row is dropped (a clean delete never left a row); a failure is
    /// marked `failed` for the healer. Returns the failure count. Dropped
    /// mid-way (quit), the remaining tombstones stay `pending` and the next
    /// worker pass finishes them.
    pub(super) async fn remove_tombstoned_files(
        &self,
        tombstones: Vec<(i64, String)>,
        context: &RetentionCleanupContext,
    ) -> Result<i64> {
        let mut failures = 0_i64;
        for (id, path) in tombstones {
            match delete_path_off_thread(path, context).await {
                Ok(()) => {
                    sqlx::query("DELETE FROM retention_file_tombstones WHERE id = ?1")
                        .bind(id)
                        .execute(self.db.write())
                        .await?;
                }
                Err(error) => {
                    failures += 1;
                    sqlx::query(
                        "UPDATE retention_file_tombstones
                         SET status = 'failed', last_error = ?2, attempt_count = 1,
                             updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                    )
                    .bind(id)
                    .bind(error)
                    .execute(self.db.write())
                    .await?;
                }
            }
        }
        Ok(failures)
    }
}

/// Record a `pending` tombstone for each path inside the caller's batch
/// transaction, returning `(tombstone id, path)` for the post-commit delete.
/// `cleanup_run_id` stays NULL: the run row is written after the sweep (it
/// carries the final counts) and nothing reads the link.
pub(super) async fn insert_pending_tombstones(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    paths: &[&SegmentFilePath],
) -> Result<Vec<(i64, String)>> {
    let mut tombstones = Vec::with_capacity(paths.len());
    for path in paths {
        let id = sqlx::query(
            "INSERT INTO retention_file_tombstones
                (capture_segment_id, path, path_kind, status)
             VALUES (?1, ?2, ?3, 'pending')",
        )
        .bind(path.capture_segment_id)
        .bind(&path.path)
        .bind(&path.path_kind)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();
        tombstones.push((id, path.path.clone()));
    }
    Ok(tombstones)
}

async fn delete_path_off_thread(
    path: String,
    context: &RetentionCleanupContext,
) -> std::result::Result<(), String> {
    let context = context.clone();
    tokio::task::spawn_blocking(move || delete_path_if_safe(&path, &context))
        .await
        .unwrap_or_else(|error| Err(format!("file delete task failed: {error}")))
}
