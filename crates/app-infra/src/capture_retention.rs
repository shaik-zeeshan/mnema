use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, QueryBuilder, Row, Sqlite, SqlitePool};
use time::{format_description::well_known::Rfc3339, Date, Duration};

use crate::{processing::ProcessingJobStatus, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    Never,
    Days7,
    Days14,
    Days30,
}

impl RetentionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Days7 => "days_7",
            Self::Days14 => "days_14",
            Self::Days30 => "days_30",
        }
    }

    pub fn retention_days(self) -> Option<i64> {
        match self {
            Self::Never => None,
            Self::Days7 => Some(7),
            Self::Days14 => Some(14),
            Self::Days30 => Some(30),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSourceKind {
    Screen,
    Microphone,
    SystemAudio,
}

impl CaptureSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Microphone => "microphone",
            Self::SystemAudio => "system_audio",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewCaptureSession {
    pub capture_session_id: String,
    pub started_at: String,
    pub requested_screen: bool,
    pub requested_microphone: bool,
    pub requested_system_audio: bool,
    pub screen_source_session_id: Option<String>,
    pub microphone_source_session_id: Option<String>,
    pub system_audio_source_session_id: Option<String>,
    pub segment_duration_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewCaptureSegment {
    pub capture_session_id: String,
    pub source_kind: CaptureSourceKind,
    pub source_session_id: String,
    pub segment_index: i64,
    pub media_file_path: Option<String>,
    pub workspace_dir_path: Option<String>,
    pub frame_dir_path: Option<String>,
    pub sidecar_file_path: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSegment {
    pub id: i64,
    pub capture_session_id: String,
    pub source_kind: CaptureSourceKind,
    pub source_session_id: String,
    pub segment_index: i64,
    pub media_file_path: Option<String>,
    pub workspace_dir_path: Option<String>,
    pub frame_dir_path: Option<String>,
    pub sidecar_file_path: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionCleanupSummary {
    pub policy: String,
    pub cutoff_ended_before: Option<String>,
    pub eligible_capture_segments: i64,
    pub deleted_capture_segments: i64,
    pub deleted_frames: i64,
    pub deleted_audio_segments: i64,
    pub deleted_processing_jobs: i64,
    pub deleted_processing_results: i64,
    pub deleted_background_jobs: i64,
    pub deleted_frame_batches: i64,
    pub skipped_running_jobs: i64,
    pub skipped_active_segments: i64,
    pub pending_file_tombstones: i64,
    pub file_delete_errors: i64,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RetentionCleanupContext {
    pub active_capture_segment_ids: Vec<i64>,
    pub active_source_session_ids: Vec<String>,
    pub save_directory: Option<String>,
}

#[derive(Debug, Clone)]
struct SegmentFilePath {
    capture_segment_id: i64,
    path: String,
    path_kind: String,
}

#[derive(Clone)]
pub struct CaptureRetentionStore {
    pool: SqlitePool,
}

impl CaptureRetentionStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_capture_session(&self, session: &NewCaptureSession) -> Result<()> {
        sqlx::query(
            "INSERT INTO capture_sessions (
                capture_session_id, started_at, status, requested_screen, requested_microphone,
                requested_system_audio, screen_source_session_id, microphone_source_session_id,
                system_audio_source_session_id, segment_duration_seconds
            ) VALUES (?1, ?2, 'recording', ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(capture_session_id) DO UPDATE SET
                stopped_at = NULL,
                status = 'recording',
                requested_screen = excluded.requested_screen,
                requested_microphone = excluded.requested_microphone,
                requested_system_audio = excluded.requested_system_audio,
                screen_source_session_id = excluded.screen_source_session_id,
                microphone_source_session_id = excluded.microphone_source_session_id,
                system_audio_source_session_id = excluded.system_audio_source_session_id,
                segment_duration_seconds = excluded.segment_duration_seconds,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&session.capture_session_id)
        .bind(&session.started_at)
        .bind(session.requested_screen)
        .bind(session.requested_microphone)
        .bind(session.requested_system_audio)
        .bind(session.screen_source_session_id.as_deref())
        .bind(session.microphone_source_session_id.as_deref())
        .bind(session.system_audio_source_session_id.as_deref())
        .bind(session.segment_duration_seconds)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_capture_session(
        &self,
        capture_session_id: &str,
        stopped_at: &str,
        status: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE capture_sessions
             SET stopped_at = ?2, status = ?3, updated_at = CURRENT_TIMESTAMP
             WHERE capture_session_id = ?1",
        )
        .bind(capture_session_id)
        .bind(stopped_at)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_capture_sessions_for_source_session_ids(
        &self,
        source_session_ids: &[String],
        stopped_at: &str,
        status: &str,
    ) -> Result<u64> {
        if source_session_ids.is_empty() {
            return Ok(0);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE capture_sessions
             SET stopped_at = ",
        );
        query.push_bind(stopped_at);
        query.push(", status = ");
        query.push_bind(status);
        query.push(", updated_at = CURRENT_TIMESTAMP WHERE ");
        for (index, source_session_id) in source_session_ids.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query.push("screen_source_session_id = ");
            query.push_bind(source_session_id);
            query.push(" OR microphone_source_session_id = ");
            query.push_bind(source_session_id);
            query.push(" OR system_audio_source_session_id = ");
            query.push_bind(source_session_id);
        }
        Ok(query.build().execute(&self.pool).await?.rows_affected())
    }

    pub async fn upsert_capture_segment(&self, segment: &NewCaptureSegment) -> Result<CaptureSegment> {
        sqlx::query(
            "INSERT INTO capture_segments (
                capture_session_id, source_kind, source_session_id, segment_index, media_file_path,
                workspace_dir_path, frame_dir_path, sidecar_file_path, started_at, ended_at, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(source_kind, source_session_id, segment_index) DO UPDATE SET
                capture_session_id = excluded.capture_session_id,
                media_file_path = COALESCE(excluded.media_file_path, capture_segments.media_file_path),
                workspace_dir_path = COALESCE(excluded.workspace_dir_path, capture_segments.workspace_dir_path),
                frame_dir_path = COALESCE(excluded.frame_dir_path, capture_segments.frame_dir_path),
                sidecar_file_path = COALESCE(excluded.sidecar_file_path, capture_segments.sidecar_file_path),
                started_at = excluded.started_at,
                ended_at = excluded.ended_at,
                status = excluded.status,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&segment.capture_session_id)
        .bind(segment.source_kind.as_str())
        .bind(&segment.source_session_id)
        .bind(segment.segment_index)
        .bind(segment.media_file_path.as_deref())
        .bind(segment.workspace_dir_path.as_deref())
        .bind(segment.frame_dir_path.as_deref())
        .bind(segment.sidecar_file_path.as_deref())
        .bind(&segment.started_at)
        .bind(&segment.ended_at)
        .bind(&segment.status)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "SELECT id, capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, workspace_dir_path, frame_dir_path, sidecar_file_path,
                    started_at, ended_at, status
             FROM capture_segments
             WHERE source_kind = ?1 AND source_session_id = ?2 AND segment_index = ?3",
        )
        .bind(segment.source_kind.as_str())
        .bind(&segment.source_session_id)
        .bind(segment.segment_index)
        .fetch_one(&self.pool)
        .await?;
        map_capture_segment(row)
    }

    pub async fn upsert_screen_segment_for_source_session(
        &self,
        source_session_id: &str,
        segment_index: i64,
        media_file_path: String,
        workspace_dir_path: String,
        frame_dir_path: String,
        sidecar_file_path: String,
        captured_at: String,
    ) -> Result<Option<CaptureSegment>> {
        let Some(row) = sqlx::query(
            "SELECT capture_session_id FROM capture_sessions
             WHERE screen_source_session_id = ?1
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(source_session_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let capture_session_id: String = row.get("capture_session_id");
        self.upsert_capture_segment(&NewCaptureSegment {
            capture_session_id,
            source_kind: CaptureSourceKind::Screen,
            source_session_id: source_session_id.to_string(),
            segment_index,
            media_file_path: Some(media_file_path),
            workspace_dir_path: Some(workspace_dir_path),
            frame_dir_path: Some(frame_dir_path),
            sidecar_file_path: Some(sidecar_file_path),
            started_at: captured_at.clone(),
            ended_at: captured_at,
            status: "completed".to_string(),
        })
        .await
        .map(Some)
    }

    pub async fn preview_cleanup(
        &self,
        policy: RetentionPolicy,
        local_today: Date,
        context: &RetentionCleanupContext,
    ) -> Result<RetentionCleanupSummary> {
        let Some(cutoff) = cutoff_ended_before(policy, local_today) else {
            return Ok(RetentionCleanupSummary {
                policy: policy.as_str().to_string(),
                ..Default::default()
            });
        };
        self.plan_cleanup(policy, cutoff, context).await
    }

    pub async fn run_cleanup(
        &self,
        policy: RetentionPolicy,
        local_today: Date,
        context: &RetentionCleanupContext,
    ) -> Result<RetentionCleanupSummary> {
        let Some(cutoff) = cutoff_ended_before(policy, local_today) else {
            return Ok(RetentionCleanupSummary {
                policy: policy.as_str().to_string(),
                ..Default::default()
            });
        };
        let mut summary = self.plan_cleanup(policy, cutoff.clone(), context).await?;
        if summary.eligible_capture_segments == 0 {
            return Ok(summary);
        }

        let segment_ids = eligible_segment_ids(&self.pool, &cutoff, context).await?;
        if segment_ids.is_empty() {
            return Ok(summary);
        }

        let file_paths = file_paths_for_segments(&self.pool, &segment_ids).await?;
        let mut tx = self.pool.begin().await?;
        let frame_ids = ids_for_capture_segments(&mut tx, "frames", &segment_ids).await?;
        let audio_ids = ids_for_capture_segments(&mut tx, "audio_segments", &segment_ids).await?;
        let frame_batch_ids = frame_batch_ids_for_frames(&mut tx, &frame_ids).await?;
        let background_job_ids = background_job_ids_for_frame_batches(&mut tx, &frame_batch_ids).await?;
        let job_ids = processing_job_ids_for_subjects(&mut tx, &frame_ids, &audio_ids).await?;
        summary.deleted_processing_results = delete_by_job_ids(&mut tx, "processing_results", &job_ids).await?;
        summary.deleted_processing_jobs = delete_processing_jobs(&mut tx, &job_ids).await?;
        delete_speaker_rows_for_audio_segments(&mut tx, &audio_ids).await?;
        summary.deleted_frames = delete_by_ids(&mut tx, "frames", &frame_ids).await?;
        summary.deleted_frame_batches = delete_by_ids(&mut tx, "frame_batches", &frame_batch_ids).await?;
        summary.deleted_background_jobs = delete_by_ids(&mut tx, "background_jobs", &background_job_ids).await?;
        summary.deleted_audio_segments = delete_by_ids(&mut tx, "audio_segments", &audio_ids).await?;
        summary.deleted_capture_segments = delete_by_ids(&mut tx, "capture_segments", &segment_ids).await?;
        let cleanup_run_id = insert_cleanup_run(&mut tx, &summary, "manual", "completed").await?;
        tx.commit().await?;
        let file_delete_errors = self
            .delete_segment_files_and_tombstone(cleanup_run_id, &file_paths, context)
            .await?;
        if file_delete_errors > 0 {
            summary.file_delete_errors = file_delete_errors;
            summary.pending_file_tombstones = self.pending_file_tombstone_count().await?;
            summary.status = "completed_with_file_errors".to_string();
            sqlx::query(
                "UPDATE retention_cleanup_runs
                 SET status = 'completed_with_file_errors', pending_file_tombstones = ?2
                 WHERE id = ?1",
            )
            .bind(cleanup_run_id)
            .bind(summary.pending_file_tombstones)
            .execute(&self.pool)
            .await?;
        } else {
            summary.status = "completed".to_string();
        }
        Ok(summary)
    }

    pub async fn latest_status(&self, policy: RetentionPolicy) -> Result<RetentionCleanupSummary> {
        let row = sqlx::query(
            "SELECT policy, cutoff_ended_before, status, deleted_capture_segments, deleted_frames,
                    deleted_audio_segments, deleted_processing_jobs, deleted_processing_results,
                    deleted_background_jobs, deleted_frame_batches,
                    skipped_running_jobs, skipped_active_segments, pending_file_tombstones,
                    error_message, created_at
             FROM retention_cleanup_runs
             ORDER BY id DESC
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let pending = self.pending_file_tombstone_count().await?;
        let Some(row) = row else {
            return Ok(RetentionCleanupSummary {
                policy: policy.as_str().to_string(),
                pending_file_tombstones: pending,
                status: "skipped".to_string(),
                ..Default::default()
            });
        };
        Ok(RetentionCleanupSummary {
            policy: row.get("policy"),
            cutoff_ended_before: row.get("cutoff_ended_before"),
            deleted_capture_segments: row.get("deleted_capture_segments"),
            deleted_frames: row.get("deleted_frames"),
            deleted_audio_segments: row.get("deleted_audio_segments"),
            deleted_processing_jobs: row.get("deleted_processing_jobs"),
            deleted_processing_results: row.get("deleted_processing_results"),
            deleted_background_jobs: row.get("deleted_background_jobs"),
            deleted_frame_batches: row.get("deleted_frame_batches"),
            skipped_running_jobs: row.get("skipped_running_jobs"),
            skipped_active_segments: row.get("skipped_active_segments"),
            pending_file_tombstones: pending,
            status: row.get("status"),
            error_message: row.get("error_message"),
            created_at: row.get("created_at"),
            ..Default::default()
        })
    }

    pub async fn retry_pending_file_tombstones(
        &self,
        context: &RetentionCleanupContext,
    ) -> Result<i64> {
        let rows = sqlx::query(
            "SELECT id, path, path_kind FROM retention_file_tombstones
             WHERE status IN ('pending', 'failed')
             ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut resolved = 0_i64;
        for row in rows {
            let id: i64 = row.get("id");
            let path: String = row.get("path");
            match delete_path_if_safe(&path, context) {
                Ok(()) => {
                    resolved += 1;
                    sqlx::query(
                        "UPDATE retention_file_tombstones
                         SET status = 'resolved', resolved_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                    )
                    .bind(id)
                    .execute(&self.pool)
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
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        Ok(resolved)
    }

    async fn plan_cleanup(
        &self,
        policy: RetentionPolicy,
        cutoff: String,
        context: &RetentionCleanupContext,
    ) -> Result<RetentionCleanupSummary> {
        let segment_ids = eligible_segment_ids(&self.pool, &cutoff, context).await?;
        let mut summary = RetentionCleanupSummary {
            policy: policy.as_str().to_string(),
            cutoff_ended_before: Some(cutoff),
            eligible_capture_segments: segment_ids.len() as i64,
            skipped_active_segments: count_active_skipped(&self.pool, context).await?,
            skipped_running_jobs: count_running_blocked_segments(&self.pool).await?,
            pending_file_tombstones: self.pending_file_tombstone_count().await?,
            status: "skipped".to_string(),
            ..Default::default()
        };
        if !segment_ids.is_empty() {
            summary.deleted_frames = count_by_capture_segments(&self.pool, "frames", &segment_ids).await?;
            summary.deleted_audio_segments =
                count_by_capture_segments(&self.pool, "audio_segments", &segment_ids).await?;
        }
        Ok(summary)
    }

    async fn pending_file_tombstone_count(&self) -> Result<i64> {
        Ok(sqlx::query(
            "SELECT COUNT(*) AS count FROM retention_file_tombstones WHERE status IN ('pending', 'failed')",
        )
        .fetch_one(&self.pool)
        .await?
        .get("count"))
    }

    async fn delete_segment_files_and_tombstone(
        &self,
        cleanup_run_id: i64,
        paths: &[SegmentFilePath],
        context: &RetentionCleanupContext,
    ) -> Result<i64> {
        let mut failures = 0_i64;
        for path in paths {
            if let Err(error) = delete_path_if_safe(&path.path, context) {
                failures += 1;
                sqlx::query(
                    "INSERT INTO retention_file_tombstones
                        (cleanup_run_id, capture_segment_id, path, path_kind, status, last_error, attempt_count)
                     VALUES (?1, ?2, ?3, ?4, 'failed', ?5, 1)",
                )
                .bind(cleanup_run_id)
                .bind(path.capture_segment_id)
                .bind(&path.path)
                .bind(&path.path_kind)
                .bind(error)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(failures)
    }
}

pub fn cutoff_ended_before(policy: RetentionPolicy, local_today: Date) -> Option<String> {
    let days = policy.retention_days()?;
    let cutoff_date = local_today - Duration::days(days - 1);
    let cutoff = cutoff_date.midnight().assume_utc();
    Some(cutoff.format(&Rfc3339).expect("RFC3339 formatting should succeed"))
}

async fn file_paths_for_segments(pool: &SqlitePool, ids: &[i64]) -> Result<Vec<SegmentFilePath>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, media_file_path, workspace_dir_path, frame_dir_path, sidecar_file_path
         FROM capture_segments WHERE id IN (",
    );
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let rows = query.build().fetch_all(pool).await?;
    let mut paths = Vec::new();
    for row in rows {
        let capture_segment_id = row.get("id");
        for (column, kind) in [
            ("media_file_path", "media_file"),
            ("sidecar_file_path", "sidecar_file"),
            ("frame_dir_path", "frame_dir"),
            ("workspace_dir_path", "workspace_dir"),
        ] {
            if let Some(path) = row.get::<Option<String>, _>(column) {
                paths.push(SegmentFilePath {
                    capture_segment_id,
                    path,
                    path_kind: kind.to_string(),
                });
            }
        }
    }
    paths.sort_by_key(|path| match path.path_kind.as_str() {
        "media_file" | "sidecar_file" => 0,
        "frame_dir" => 1,
        _ => 2,
    });
    paths.dedup_by(|a, b| a.path == b.path);
    Ok(paths)
}

fn delete_path_if_safe(path: &str, context: &RetentionCleanupContext) -> std::result::Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("refusing to delete a relative retention path".to_string());
    }
    if let Some(save_directory) = context.save_directory.as_deref() {
        let save_directory = Path::new(save_directory);
        if !path.starts_with(save_directory) {
            return Err(format!(
                "refusing to delete retention path outside saveDirectory: {}",
                path.display()
            ));
        }
    }
    if path.is_dir() {
        std::fs::remove_dir_all(&path)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| error.to_string())
    } else {
        std::fs::remove_file(&path)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| error.to_string())
    }
}

async fn eligible_segment_ids(
    pool: &SqlitePool,
    cutoff: &str,
    context: &RetentionCleanupContext,
) -> Result<Vec<i64>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id FROM capture_segments WHERE ended_at < ",
    );
    query.push_bind(cutoff);
    query.push(" AND status != 'recording'");
    push_exclusions(&mut query, context);
    query.push(" AND NOT EXISTS (
        SELECT 1 FROM processing_jobs
        WHERE status = 'running'
          AND ((subject_type = 'frame' AND subject_id IN (SELECT id FROM frames WHERE frames.capture_segment_id = capture_segments.id))
            OR (subject_type = 'audio_segment' AND subject_id IN (SELECT id FROM audio_segments WHERE audio_segments.capture_segment_id = capture_segments.id)))
    )");
    query.push(" AND NOT EXISTS (
        SELECT 1 FROM frames
        INNER JOIN frame_batches ON frame_batches.id = frames.frame_batch_id
        INNER JOIN background_jobs ON background_jobs.id = frame_batches.finalize_job_id
        WHERE frames.capture_segment_id = capture_segments.id
          AND background_jobs.status = 'running'
    )");
    query.push(" ORDER BY ended_at ASC, id ASC");
    let rows = query.build().fetch_all(pool).await?;
    Ok(rows.into_iter().map(|row| row.get("id")).collect())
}

fn push_exclusions<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    context: &'a RetentionCleanupContext,
) {
    if !context.active_capture_segment_ids.is_empty() {
        query.push(" AND id NOT IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_capture_segment_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    if !context.active_source_session_ids.is_empty() {
        query.push(" AND source_session_id NOT IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_source_session_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
}

async fn count_by_capture_segments(pool: &SqlitePool, table: &str, ids: &[i64]) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT COUNT(*) AS count FROM {table} WHERE capture_segment_id IN ("
    ));
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query.build().fetch_one(pool).await?.get("count"))
}

async fn ids_for_capture_segments(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    ids: &[i64],
) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT id FROM {table} WHERE capture_segment_id IN ("
    ));
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

async fn processing_job_ids_for_subjects(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    frame_ids: &[i64],
    audio_ids: &[i64],
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    for (subject_type, subject_ids) in [("frame", frame_ids), ("audio_segment", audio_ids)] {
        if subject_ids.is_empty() {
            continue;
        }
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT id FROM processing_jobs WHERE subject_type = ");
        query.push_bind(subject_type);
        query.push(" AND subject_id IN (");
        let mut separated = query.separated(", ");
        for id in subject_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        ids.extend(
            query
                .build()
                .fetch_all(&mut **tx)
                .await?
                .into_iter()
                .map(|row| row.get::<i64, _>("id")),
        );
    }
    Ok(ids)
}

async fn frame_batch_ids_for_frames(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    frame_ids: &[i64],
) -> Result<Vec<i64>> {
    if frame_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT frame_batch_id AS id FROM frames WHERE frame_batch_id IS NOT NULL AND id IN (",
    );
    let mut separated = query.separated(", ");
    for id in frame_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

async fn background_job_ids_for_frame_batches(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    batch_ids: &[i64],
) -> Result<Vec<i64>> {
    if batch_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT finalize_job_id AS id FROM frame_batches WHERE finalize_job_id IS NOT NULL AND id IN (",
    );
    let mut separated = query.separated(", ");
    for id in batch_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

async fn delete_by_job_ids(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    ids: &[i64],
) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE job_id IN ("));
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query.build().execute(&mut **tx).await?.rows_affected() as i64)
}

async fn delete_processing_jobs(tx: &mut sqlx::Transaction<'_, Sqlite>, ids: &[i64]) -> Result<i64> {
    delete_by_ids(tx, "processing_jobs", ids).await
}

async fn delete_by_ids(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    ids: &[i64],
) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query.build().execute(&mut **tx).await?.rows_affected() as i64)
}

async fn delete_speaker_rows_for_audio_segments(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    audio_ids: &[i64],
) -> Result<()> {
    delete_by_subject_ids(tx, "speaker_turns", "audio_segment_id", audio_ids).await?;
    delete_by_subject_ids(tx, "speaker_segment_clusters", "audio_segment_id", audio_ids).await?;
    sqlx::query(
        "DELETE FROM recording_speaker_clusters
         WHERE NOT EXISTS (SELECT 1 FROM speaker_turns WHERE speaker_turns.cluster_id = recording_speaker_clusters.id)
           AND NOT EXISTS (SELECT 1 FROM speaker_segment_clusters WHERE speaker_segment_clusters.stable_cluster_id = recording_speaker_clusters.id)",
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM person_voice_embeddings
         WHERE source_cluster_id IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM recording_speaker_clusters WHERE recording_speaker_clusters.id = person_voice_embeddings.source_cluster_id)",
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM speaker_recognition_rejections
         WHERE source_cluster_id IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM recording_speaker_clusters WHERE recording_speaker_clusters.id = speaker_recognition_rejections.source_cluster_id)",
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn delete_by_subject_ids(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE {column} IN ("));
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query.build().execute(&mut **tx).await?.rows_affected() as i64)
}

async fn count_active_skipped(
    pool: &SqlitePool,
    context: &RetentionCleanupContext,
) -> Result<i64> {
    if context.active_capture_segment_ids.is_empty() && context.active_source_session_ids.is_empty() {
        return Ok(0);
    }
    let mut query = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS count FROM capture_segments WHERE 1=1");
    if !context.active_capture_segment_ids.is_empty() {
        query.push(" AND id IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_capture_segment_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    if !context.active_source_session_ids.is_empty() {
        query.push(" OR source_session_id IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_source_session_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    Ok(query.build().fetch_one(pool).await?.get("count"))
}

async fn count_running_blocked_segments(pool: &SqlitePool) -> Result<i64> {
    let count = sqlx::query(
        "SELECT COUNT(DISTINCT capture_segments.id) AS count
         FROM capture_segments
         LEFT JOIN frames ON frames.capture_segment_id = capture_segments.id
         LEFT JOIN audio_segments ON audio_segments.capture_segment_id = capture_segments.id
         INNER JOIN processing_jobs ON processing_jobs.status = ?1
            AND ((processing_jobs.subject_type = 'frame' AND processing_jobs.subject_id = frames.id)
              OR (processing_jobs.subject_type = 'audio_segment' AND processing_jobs.subject_id = audio_segments.id))",
    )
    .bind(ProcessingJobStatus::Running.as_str())
    .fetch_one(pool)
    .await?
    .get("count");
    Ok(count)
}

async fn insert_cleanup_run(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    summary: &RetentionCleanupSummary,
    mode: &str,
    status: &str,
) -> Result<i64> {
    let result = sqlx::query(
        "INSERT INTO retention_cleanup_runs (
            policy, mode, cutoff_ended_before, status, deleted_capture_segments, deleted_frames,
            deleted_audio_segments, deleted_processing_jobs, deleted_processing_results,
            deleted_background_jobs, deleted_frame_batches,
            skipped_running_jobs, skipped_active_segments, pending_file_tombstones
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )
    .bind(&summary.policy)
    .bind(mode)
    .bind(summary.cutoff_ended_before.as_deref())
    .bind(status)
    .bind(summary.deleted_capture_segments)
    .bind(summary.deleted_frames)
    .bind(summary.deleted_audio_segments)
    .bind(summary.deleted_processing_jobs)
    .bind(summary.deleted_processing_results)
    .bind(summary.deleted_background_jobs)
    .bind(summary.deleted_frame_batches)
    .bind(summary.skipped_running_jobs)
    .bind(summary.skipped_active_segments)
    .bind(summary.pending_file_tombstones)
    .execute(&mut **tx)
    .await?;
    Ok(result.last_insert_rowid())
}

fn map_capture_segment(row: SqliteRow) -> Result<CaptureSegment> {
    let source_kind = match row.get::<String, _>("source_kind").as_str() {
        "screen" => CaptureSourceKind::Screen,
        "system_audio" => CaptureSourceKind::SystemAudio,
        _ => CaptureSourceKind::Microphone,
    };
    Ok(CaptureSegment {
        id: row.get("id"),
        capture_session_id: row.get("capture_session_id"),
        source_kind,
        source_session_id: row.get("source_session_id"),
        segment_index: row.get("segment_index"),
        media_file_path: row.get("media_file_path"),
        workspace_dir_path: row.get("workspace_dir_path"),
        frame_dir_path: row.get("frame_dir_path"),
        sidecar_file_path: row.get("sidecar_file_path"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        status: row.get("status"),
    })
}
