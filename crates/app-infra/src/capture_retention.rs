use std::path::{Path, PathBuf};

use chrono::{Local, LocalResult, NaiveDate, Offset, TimeZone};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, QueryBuilder, Row, Sqlite, SqlitePool};
use time::{format_description::well_known::Rfc3339, Date, Duration, OffsetDateTime, UtcOffset};

use crate::{processing::ProcessingJobStatus, Result};

const SQLITE_BIND_CHUNK_SIZE: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    Never,
    #[serde(rename = "days_7", alias = "days7")]
    Days7,
    #[serde(rename = "days_14", alias = "days14")]
    Days14,
    #[serde(rename = "days_30", alias = "days30")]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetentionCleanupMode {
    Manual,
    Automatic,
    Retry,
}

impl RetentionCleanupMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
            Self::Retry => "retry",
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCaptureSegmentWindow {
    pub id: i64,
    pub capture_session_id: String,
    pub source_session_id: String,
    pub segment_index: i64,
    pub media_file_path: String,
    pub sidecar_file_path: Option<String>,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionCleanupSummary {
    pub policy: String,
    pub cutoff_ended_before: Option<String>,
    pub eligible_capture_segments: i64,
    pub deleted_capture_segments: i64,
    #[serde(default)]
    pub deleted_capture_segment_media_paths: Vec<String>,
    pub deleted_frames: i64,
    #[serde(default)]
    pub deleted_frame_ids: Vec<i64>,
    pub deleted_audio_segments: i64,
    #[serde(default)]
    pub deleted_audio_segment_ids: Vec<i64>,
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
    capture_segment_id: Option<i64>,
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

    pub async fn upsert_capture_segment(
        &self,
        segment: &NewCaptureSegment,
    ) -> Result<CaptureSegment> {
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
                started_at = CASE
                    WHEN julianday(excluded.started_at) < julianday(capture_segments.started_at)
                    THEN excluded.started_at
                    ELSE capture_segments.started_at
                END,
                ended_at = CASE
                    WHEN julianday(excluded.ended_at) > julianday(capture_segments.ended_at)
                    THEN excluded.ended_at
                    ELSE capture_segments.ended_at
                END,
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

    pub async fn capture_segment_by_source(
        &self,
        source_kind: CaptureSourceKind,
        source_session_id: &str,
        segment_index: i64,
    ) -> Result<Option<CaptureSegment>> {
        let row = sqlx::query(
            "SELECT id, capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, workspace_dir_path, frame_dir_path, sidecar_file_path,
                    started_at, ended_at, status
             FROM capture_segments
             WHERE source_kind = ?1 AND source_session_id = ?2 AND segment_index = ?3",
        )
        .bind(source_kind.as_str())
        .bind(source_session_id)
        .bind(segment_index)
        .fetch_optional(&self.pool)
        .await?;

        row.map(map_capture_segment).transpose()
    }

    pub async fn list_finalized_screen_segments_overlapping_window(
        &self,
        start_at: &str,
        end_at: &str,
    ) -> Result<Vec<ScreenCaptureSegmentWindow>> {
        let rows = sqlx::query(
            "SELECT id, capture_session_id, source_session_id, segment_index, media_file_path,
                    sidecar_file_path, started_at, ended_at
             FROM capture_segments
             WHERE source_kind = 'screen'
               AND status != 'recording'
               AND media_file_path IS NOT NULL
               AND (
                 (julianday(ended_at) >= julianday(?1) AND julianday(started_at) <= julianday(?2))
                 OR id IN (
                   SELECT capture_segment_id
                   FROM frames
                   WHERE capture_segment_id IS NOT NULL
                     AND julianday(captured_at) >= julianday(?1)
                     AND julianday(captured_at) <= julianday(?2)
                 )
               )
             ORDER BY julianday(started_at) ASC, id ASC",
        )
        .bind(start_at)
        .bind(end_at)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(ScreenCaptureSegmentWindow {
                    id: row.get("id"),
                    capture_session_id: row.get("capture_session_id"),
                    source_session_id: row.get("source_session_id"),
                    segment_index: row.get("segment_index"),
                    media_file_path: row.get("media_file_path"),
                    sidecar_file_path: row.get("sidecar_file_path"),
                    started_at: row.get("started_at"),
                    ended_at: row.get("ended_at"),
                })
            })
            .collect()
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
            ended_at: captured_at.clone(),
            status: "completed".to_string(),
        })
        .await
        .map(Some)
    }

    pub async fn preview_cleanup(
        &self,
        policy: RetentionPolicy,
        local_now: OffsetDateTime,
        context: &RetentionCleanupContext,
    ) -> Result<RetentionCleanupSummary> {
        let Some(cutoff) = cutoff_ended_before(policy, local_now) else {
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
        local_now: OffsetDateTime,
        context: &RetentionCleanupContext,
    ) -> Result<RetentionCleanupSummary> {
        self.run_cleanup_with_mode(policy, local_now, context, RetentionCleanupMode::Manual)
            .await
    }

    pub async fn run_cleanup_with_mode(
        &self,
        policy: RetentionPolicy,
        local_now: OffsetDateTime,
        context: &RetentionCleanupContext,
        mode: RetentionCleanupMode,
    ) -> Result<RetentionCleanupSummary> {
        let Some(cutoff) = cutoff_ended_before(policy, local_now) else {
            return Ok(RetentionCleanupSummary {
                policy: policy.as_str().to_string(),
                ..Default::default()
            });
        };
        let mut summary = self.plan_cleanup(policy, cutoff.clone(), context).await?;
        if summary.eligible_capture_segments == 0
            && summary.deleted_frames == 0
            && summary.deleted_audio_segments == 0
        {
            return Ok(summary);
        }

        let segment_ids = eligible_segment_ids(&self.pool, &cutoff, context).await?;
        let orphan_frame_ids = orphan_frame_ids(&self.pool, &cutoff, context).await?;
        let orphan_audio_ids = orphan_audio_segment_ids(&self.pool, &cutoff, context).await?;
        if segment_ids.is_empty() && orphan_frame_ids.is_empty() && orphan_audio_ids.is_empty() {
            return Ok(summary);
        }

        let mut file_paths = file_paths_for_segments(&self.pool, &segment_ids).await?;
        file_paths.extend(file_paths_for_audio_segments(&self.pool, &segment_ids).await?);
        file_paths.extend(file_paths_for_orphan_frames(&self.pool, &orphan_frame_ids).await?);
        file_paths
            .extend(file_paths_for_orphan_audio_segments(&self.pool, &orphan_audio_ids).await?);
        file_paths.sort_by_key(|path| match path.path_kind.as_str() {
            "media_file" | "sidecar_file" => 0,
            "frame_dir" => 1,
            _ => 3,
        });
        file_paths.dedup_by(|a, b| a.path == b.path);
        let deleted_capture_segment_media_paths = file_paths
            .iter()
            .filter(|path| path.capture_segment_id.is_some() && path.path_kind == "media_file")
            .map(|path| path.path.clone())
            .collect::<Vec<_>>();
        let mut tx = self.pool.begin().await?;
        let mut frame_ids = ids_for_capture_segments(&mut tx, "frames", &segment_ids).await?;
        frame_ids.extend(orphan_frame_ids);
        let mut audio_ids =
            ids_for_capture_segments(&mut tx, "audio_segments", &segment_ids).await?;
        audio_ids.extend(orphan_audio_ids);
        let frame_batch_ids = frame_batch_ids_deletable_with_frames(&mut tx, &frame_ids).await?;
        let background_job_ids =
            background_job_ids_for_frame_batches(&mut tx, &frame_batch_ids).await?;
        let job_ids = processing_job_ids_for_subjects(&mut tx, &frame_ids, &audio_ids).await?;
        delete_search_documents_for_subjects(&mut tx, &frame_ids, &audio_ids).await?;
        summary.deleted_processing_results =
            delete_by_job_ids(&mut tx, "processing_results", &job_ids).await?;
        summary.deleted_processing_jobs = delete_processing_jobs(&mut tx, &job_ids).await?;
        delete_speaker_rows_for_audio_segments(&mut tx, &audio_ids).await?;
        let deleted_frame_ids = frame_ids.clone();
        let deleted_audio_segment_ids = audio_ids.clone();
        summary.deleted_frames = delete_by_ids(&mut tx, "frames", &frame_ids).await?;
        cleanup_unreferenced_frame_metadata_snapshots(&mut tx).await?;
        summary.deleted_frame_ids = deleted_frame_ids;
        summary.deleted_frame_batches =
            delete_by_ids(&mut tx, "frame_batches", &frame_batch_ids).await?;
        summary.deleted_background_jobs =
            delete_by_ids(&mut tx, "background_jobs", &background_job_ids).await?;
        summary.deleted_audio_segments =
            delete_by_ids(&mut tx, "audio_segments", &audio_ids).await?;
        summary.deleted_audio_segment_ids = deleted_audio_segment_ids;
        summary.deleted_capture_segments =
            delete_by_ids(&mut tx, "capture_segments", &segment_ids).await?;
        summary.deleted_capture_segment_media_paths = deleted_capture_segment_media_paths;
        let cleanup_run_id =
            insert_cleanup_run(&mut tx, &summary, mode.as_str(), "completed").await?;
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
             WHERE policy = ?1
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(policy.as_str())
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
            cutoff_ended_before: Some(cutoff.clone()),
            eligible_capture_segments: segment_ids.len() as i64,
            skipped_active_segments: count_active_skipped(&self.pool, context).await?,
            skipped_running_jobs: count_running_blocked_segments(&self.pool, &cutoff, context)
                .await?,
            pending_file_tombstones: self.pending_file_tombstone_count().await?,
            status: "skipped".to_string(),
            ..Default::default()
        };
        if !segment_ids.is_empty() {
            summary.deleted_frames =
                count_by_capture_segments(&self.pool, "frames", &segment_ids).await?;
            summary.deleted_audio_segments =
                count_by_capture_segments(&self.pool, "audio_segments", &segment_ids).await?;
        }
        summary.deleted_frames +=
            orphan_frame_ids(&self.pool, &cutoff, context).await?.len() as i64;
        summary.deleted_audio_segments += orphan_audio_segment_ids(&self.pool, &cutoff, context)
            .await?
            .len() as i64;
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

pub fn cutoff_ended_before(policy: RetentionPolicy, local_now: OffsetDateTime) -> Option<String> {
    cutoff_ended_before_with_midnight_offset(policy, local_now, local_midnight_offset)
}

fn cutoff_ended_before_with_midnight_offset(
    policy: RetentionPolicy,
    local_now: OffsetDateTime,
    resolve_midnight_offset: impl FnOnce(Date) -> Option<UtcOffset>,
) -> Option<String> {
    let days = policy.retention_days()?;
    let cutoff_date = local_now.date() - Duration::days(days - 1);
    let cutoff_offset = resolve_midnight_offset(cutoff_date).unwrap_or_else(|| local_now.offset());
    let cutoff = cutoff_date
        .midnight()
        .assume_offset(cutoff_offset)
        .to_offset(UtcOffset::UTC);
    Some(
        cutoff
            .format(&Rfc3339)
            .expect("RFC3339 formatting should succeed"),
    )
}

fn local_midnight_offset(date: Date) -> Option<UtcOffset> {
    let local_date = NaiveDate::from_ymd_opt(
        date.year(),
        u32::from(u8::from(date.month())),
        u32::from(date.day()),
    )?;
    let local_midnight = local_date.and_hms_opt(0, 0, 0)?;
    let offset_seconds = match Local.from_local_datetime(&local_midnight) {
        LocalResult::Single(datetime) => datetime.offset().fix().local_minus_utc(),
        LocalResult::Ambiguous(earliest, _) => earliest.offset().fix().local_minus_utc(),
        LocalResult::None => return None,
    };
    UtcOffset::from_whole_seconds(offset_seconds).ok()
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
                    capture_segment_id: Some(capture_segment_id),
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

async fn file_paths_for_audio_segments(
    pool: &SqlitePool,
    capture_segment_ids: &[i64],
) -> Result<Vec<SegmentFilePath>> {
    if capture_segment_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT capture_segment_id, file_path FROM audio_segments WHERE capture_segment_id IN (",
    );
    let mut separated = query.separated(", ");
    for id in capture_segment_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| SegmentFilePath {
            capture_segment_id: row.get("capture_segment_id"),
            path: row.get("file_path"),
            path_kind: "media_file".to_string(),
        })
        .collect())
}

async fn file_paths_for_orphan_frames(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<Vec<SegmentFilePath>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new("SELECT file_path FROM frames WHERE id IN (");
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| SegmentFilePath {
            capture_segment_id: None,
            path: row.get("file_path"),
            path_kind: "media_file".to_string(),
        })
        .collect())
}

async fn file_paths_for_orphan_audio_segments(
    pool: &SqlitePool,
    ids: &[i64],
) -> Result<Vec<SegmentFilePath>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT file_path FROM audio_segments WHERE id IN (");
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    Ok(query
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| SegmentFilePath {
            capture_segment_id: None,
            path: row.get("file_path"),
            path_kind: "media_file".to_string(),
        })
        .collect())
}

fn delete_path_if_safe(
    path: &str,
    context: &RetentionCleanupContext,
) -> std::result::Result<(), String> {
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
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT id FROM capture_segments WHERE ended_at < ");
    query.push_bind(cutoff);
    query.push(" AND status != 'recording'");
    push_exclusions(&mut query, context);
    query.push(" AND NOT EXISTS (
        SELECT 1 FROM processing_jobs
        WHERE status = 'running'
          AND ((subject_type = 'frame' AND subject_id IN (SELECT id FROM frames WHERE frames.capture_segment_id = capture_segments.id))
            OR (subject_type = 'audio_segment' AND subject_id IN (SELECT id FROM audio_segments WHERE audio_segments.capture_segment_id = capture_segments.id)))
    )");
    query.push(
        " AND NOT EXISTS (
        SELECT 1 FROM frames
        INNER JOIN frame_batches ON frame_batches.id = frames.frame_batch_id
        INNER JOIN background_jobs ON background_jobs.id = frame_batches.finalize_job_id
        WHERE frames.capture_segment_id = capture_segments.id
          AND background_jobs.status = 'running'
    )",
    );
    query.push(" ORDER BY ended_at ASC, id ASC");
    let rows = query.build().fetch_all(pool).await?;
    Ok(rows.into_iter().map(|row| row.get("id")).collect())
}

async fn orphan_frame_ids(
    pool: &SqlitePool,
    cutoff: &str,
    context: &RetentionCleanupContext,
) -> Result<Vec<i64>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id FROM frames WHERE capture_segment_id IS NULL AND captured_at < ",
    );
    query.push_bind(cutoff);
    if !context.active_source_session_ids.is_empty() {
        query.push(" AND session_id NOT IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_source_session_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    query.push(
        " AND NOT EXISTS (
        SELECT 1 FROM processing_jobs
        WHERE subject_type = 'frame'
          AND subject_id = frames.id
          AND status = 'running'
    )",
    );
    query.push(
        " AND NOT EXISTS (
        SELECT 1 FROM frame_batches
        INNER JOIN background_jobs ON background_jobs.id = frame_batches.finalize_job_id
        WHERE frame_batches.id = frames.frame_batch_id
          AND background_jobs.status = 'running'
    )",
    );
    query.push(" ORDER BY captured_at ASC, id ASC");
    Ok(query
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

async fn orphan_audio_segment_ids(
    pool: &SqlitePool,
    cutoff: &str,
    context: &RetentionCleanupContext,
) -> Result<Vec<i64>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id FROM audio_segments WHERE capture_segment_id IS NULL AND ended_at < ",
    );
    query.push_bind(cutoff);
    if !context.active_source_session_ids.is_empty() {
        query.push(" AND source_session_id NOT IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_source_session_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    query.push(
        " AND NOT EXISTS (
        SELECT 1 FROM processing_jobs
        WHERE subject_type = 'audio_segment'
          AND subject_id = audio_segments.id
          AND status = 'running'
    )",
    );
    query.push(" ORDER BY ended_at ASC, id ASC");
    Ok(query
        .build()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.get("id"))
        .collect())
}

fn push_exclusions<'a>(query: &mut QueryBuilder<'a, Sqlite>, context: &'a RetentionCleanupContext) {
    if !context.active_capture_segment_ids.is_empty() {
        query.push(" AND capture_segments.id NOT IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_capture_segment_ids {
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

async fn frame_batch_ids_deletable_with_frames(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    frame_ids: &[i64],
) -> Result<Vec<i64>> {
    if frame_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT candidate.frame_batch_id AS id
         FROM frames candidate
         WHERE candidate.frame_batch_id IS NOT NULL
           AND candidate.id IN (",
    );
    let mut separated = query.separated(", ");
    for id in frame_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(
        ")
           AND NOT EXISTS (
               SELECT 1 FROM frames retained
               WHERE retained.frame_batch_id = candidate.frame_batch_id
                 AND retained.id NOT IN (",
    );
    let mut separated = query.separated(", ");
    for id in frame_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated("))");
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

async fn delete_search_documents_for_subjects(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    frame_ids: &[i64],
    audio_ids: &[i64],
) -> Result<()> {
    let search_exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'search_documents'",
    )
    .fetch_optional(&mut **tx)
    .await?;
    if search_exists.is_none() {
        return Ok(());
    }

    let mut document_ids: Vec<i64> = Vec::new();
    for frame_chunk in frame_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT id FROM search_documents WHERE frame_id IN (");
        let mut separated = query.separated(", ");
        for id in frame_chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        document_ids.extend(
            query
                .build()
                .fetch_all(&mut **tx)
                .await?
                .into_iter()
                .map(|row| row.get::<i64, _>("id")),
        );
    }
    for audio_chunk in audio_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id FROM search_documents WHERE audio_segment_id IN (",
        );
        let mut separated = query.separated(", ");
        for id in audio_chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        document_ids.extend(
            query
                .build()
                .fetch_all(&mut **tx)
                .await?
                .into_iter()
                .map(|row| row.get::<i64, _>("id")),
        );
    }
    if document_ids.is_empty() {
        return Ok(());
    }

    for document_chunk in document_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
        let mut doc_query =
            QueryBuilder::<Sqlite>::new("DELETE FROM search_documents WHERE id IN (");
        let mut doc_separated = doc_query.separated(", ");
        for id in document_chunk {
            doc_separated.push_bind(*id);
        }
        doc_separated.push_unseparated(")");
        doc_query.build().execute(&mut **tx).await?;
    }

    Ok(())
}

async fn delete_processing_jobs(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    ids: &[i64],
) -> Result<i64> {
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
    let mut deleted = 0;
    for chunk in ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
        let mut separated = query.separated(", ");
        for id in chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        deleted += query.build().execute(&mut **tx).await?.rows_affected() as i64;
    }
    Ok(deleted)
}

async fn cleanup_unreferenced_frame_metadata_snapshots(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<i64> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'frame_metadata_snapshots'",
    )
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_none() {
        return Ok(0);
    }

    Ok(sqlx::query(
        "DELETE FROM frame_metadata_snapshots \
         WHERE NOT EXISTS (SELECT 1 FROM frames WHERE frames.metadata_snapshot_id = frame_metadata_snapshots.id)",
    )
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64)
}

async fn delete_speaker_rows_for_audio_segments(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    audio_ids: &[i64],
) -> Result<()> {
    delete_by_subject_ids(tx, "speaker_turns", "audio_segment_id", audio_ids).await?;
    delete_by_subject_ids(
        tx,
        "speaker_segment_clusters",
        "audio_segment_id",
        audio_ids,
    )
    .await?;
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

async fn count_active_skipped(pool: &SqlitePool, context: &RetentionCleanupContext) -> Result<i64> {
    if context.active_capture_segment_ids.is_empty() && context.active_source_session_ids.is_empty()
    {
        return Ok(0);
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS count FROM capture_segments WHERE ");
    if !context.active_capture_segment_ids.is_empty() {
        query.push("id IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_capture_segment_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    if !context.active_source_session_ids.is_empty() {
        if !context.active_capture_segment_ids.is_empty() {
            query.push(" OR ");
        }
        query.push("source_session_id IN (");
        let mut separated = query.separated(", ");
        for id in &context.active_source_session_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    Ok(query.build().fetch_one(pool).await?.get("count"))
}

async fn count_running_blocked_segments(
    pool: &SqlitePool,
    cutoff: &str,
    context: &RetentionCleanupContext,
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(DISTINCT capture_segments.id) AS count
         FROM capture_segments
         WHERE capture_segments.ended_at < ",
    );
    query.push_bind(cutoff);
    query.push(" AND capture_segments.status != 'recording'");
    push_exclusions(&mut query, context);
    query.push(
        " AND (
            EXISTS (
                SELECT 1 FROM processing_jobs
                WHERE status = ",
    );
    query.push_bind(ProcessingJobStatus::Running.as_str());
    query.push(
        "         AND ((subject_type = 'frame' AND subject_id IN (SELECT id FROM frames WHERE frames.capture_segment_id = capture_segments.id))
                  OR (subject_type = 'audio_segment' AND subject_id IN (SELECT id FROM audio_segments WHERE audio_segments.capture_segment_id = capture_segments.id)))
            )
            OR EXISTS (
                SELECT 1 FROM frames
                INNER JOIN frame_batches ON frame_batches.id = frames.frame_batch_id
                INNER JOIN background_jobs ON background_jobs.id = frame_batches.finalize_job_id
                WHERE frames.capture_segment_id = capture_segments.id
                  AND background_jobs.status = 'running'
            )
        )",
    );
    let count = query.build().fetch_one(pool).await?.get("count");
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn rfc3339(value: &str) -> OffsetDateTime {
        OffsetDateTime::parse(value, &Rfc3339).expect("test timestamp should parse")
    }

    #[test]
    fn day_policy_keeps_local_calendar_cutoff() {
        let cutoff = cutoff_ended_before_with_midnight_offset(
            RetentionPolicy::Days7,
            rfc3339("2026-05-10T12:34:56Z"),
            |_| Some(UtcOffset::UTC),
        );

        assert_eq!(cutoff.as_deref(), Some("2026-05-04T00:00:00Z"));
    }

    #[test]
    fn day_policy_converts_local_calendar_cutoff_to_utc() {
        let cutoff = cutoff_ended_before_with_midnight_offset(
            RetentionPolicy::Days7,
            rfc3339("2026-05-10T12:34:56-07:00"),
            |_| Some(UtcOffset::from_hms(-7, 0, 0).expect("test offset should be valid")),
        );

        assert_eq!(cutoff.as_deref(), Some("2026-05-04T07:00:00Z"));
    }

    #[test]
    fn day_policy_uses_cutoff_dates_local_offset() {
        let cutoff = cutoff_ended_before_with_midnight_offset(
            RetentionPolicy::Days14,
            rfc3339("2026-03-15T12:34:56-07:00"),
            |_| Some(UtcOffset::from_hms(-8, 0, 0).expect("test offset should be valid")),
        );

        assert_eq!(cutoff.as_deref(), Some("2026-03-02T08:00:00Z"));
    }

    async fn create_retention_cleanup_tables(pool: &SqlitePool) {
        for statement in [
            "CREATE TABLE capture_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                capture_session_id TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                segment_index INTEGER NOT NULL,
                media_file_path TEXT,
                workspace_dir_path TEXT,
                frame_dir_path TEXT,
                sidecar_file_path TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(source_kind, source_session_id, segment_index)
            )",
            "CREATE TABLE frames (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                file_path TEXT NOT NULL,
                captured_at TEXT NOT NULL,
                capture_segment_id INTEGER,
                frame_batch_id INTEGER
            )",
            "CREATE TABLE audio_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_kind TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                segment_index INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                capture_segment_id INTEGER
            )",
            "CREATE TABLE processing_jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject_type TEXT,
                subject_id INTEGER,
                processor TEXT,
                status TEXT
            )",
            "CREATE TABLE processing_results (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id INTEGER,
                output_json TEXT
            )",
            "CREATE TABLE frame_batches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                finalize_job_id INTEGER
            )",
            "CREATE TABLE background_jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                status TEXT
            )",
            "CREATE TABLE speaker_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                audio_segment_id INTEGER,
                cluster_id INTEGER
            )",
            "CREATE TABLE speaker_segment_clusters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                audio_segment_id INTEGER,
                stable_cluster_id INTEGER
            )",
            "CREATE TABLE recording_speaker_clusters (
                id INTEGER PRIMARY KEY AUTOINCREMENT
            )",
            "CREATE TABLE person_voice_embeddings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_cluster_id INTEGER
            )",
            "CREATE TABLE speaker_recognition_rejections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_cluster_id INTEGER
            )",
            "CREATE TABLE retention_cleanup_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                policy TEXT NOT NULL,
                mode TEXT NOT NULL,
                cutoff_ended_before TEXT,
                status TEXT NOT NULL,
                deleted_capture_segments INTEGER NOT NULL DEFAULT 0,
                deleted_frames INTEGER NOT NULL DEFAULT 0,
                deleted_audio_segments INTEGER NOT NULL DEFAULT 0,
                deleted_processing_jobs INTEGER NOT NULL DEFAULT 0,
                deleted_processing_results INTEGER NOT NULL DEFAULT 0,
                deleted_background_jobs INTEGER NOT NULL DEFAULT 0,
                deleted_frame_batches INTEGER NOT NULL DEFAULT 0,
                skipped_running_jobs INTEGER NOT NULL DEFAULT 0,
                skipped_active_segments INTEGER NOT NULL DEFAULT 0,
                pending_file_tombstones INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            "CREATE TABLE retention_file_tombstones (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cleanup_run_id INTEGER,
                capture_segment_id INTEGER,
                path TEXT NOT NULL,
                path_kind TEXT NOT NULL,
                status TEXT NOT NULL,
                last_error TEXT,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                resolved_at TEXT
            )",
        ] {
            sqlx::query(statement)
                .execute(pool)
                .await
                .expect("retention cleanup test table should be created");
        }
    }

    fn create_test_dir(name: &str) -> PathBuf {
        let id = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "mnema-retention-{name}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test directory should be created");
        dir
    }

    #[test]
    fn active_segment_exclusion_keeps_older_segments_from_same_source_eligible() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            sqlx::query(
                "CREATE TABLE capture_segments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    capture_session_id TEXT NOT NULL,
                    source_kind TEXT NOT NULL,
                    source_session_id TEXT NOT NULL,
                    segment_index INTEGER NOT NULL,
                    media_file_path TEXT,
                    workspace_dir_path TEXT,
                    frame_dir_path TEXT,
                    sidecar_file_path TEXT,
                    started_at TEXT NOT NULL,
                    ended_at TEXT NOT NULL,
                    status TEXT NOT NULL,
                    UNIQUE(source_kind, source_session_id, segment_index)
                )",
            )
            .execute(&pool)
            .await
            .expect("capture_segments table should be created");
            for statement in [
                "CREATE TABLE frames (id INTEGER PRIMARY KEY AUTOINCREMENT, capture_segment_id INTEGER, frame_batch_id INTEGER)",
                "CREATE TABLE audio_segments (id INTEGER PRIMARY KEY AUTOINCREMENT, capture_segment_id INTEGER)",
                "CREATE TABLE processing_jobs (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_type TEXT, subject_id INTEGER, status TEXT)",
                "CREATE TABLE frame_batches (id INTEGER PRIMARY KEY AUTOINCREMENT, finalize_job_id INTEGER)",
                "CREATE TABLE background_jobs (id INTEGER PRIMARY KEY AUTOINCREMENT, status TEXT)",
            ] {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .expect("supporting table should be created");
            }
            sqlx::query(
                "INSERT INTO capture_segments (
                    capture_session_id, source_kind, source_session_id, segment_index, started_at, ended_at, status
                 ) VALUES
                    ('capture-1', 'screen', 'screen-source-1', 1, '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed'),
                    ('capture-1', 'screen', 'screen-source-1', 2, '2026-05-01T00:05:00Z', '2026-05-01T00:10:00Z', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("segments should insert");

            let ids = eligible_segment_ids(
                &pool,
                "2026-05-02T00:00:00Z",
                &RetentionCleanupContext {
                    active_capture_segment_ids: vec![2],
                    active_source_session_ids: vec!["screen-source-1".to_string()],
                    save_directory: None,
                },
            )
            .await
            .expect("eligible ids should query");

            assert_eq!(ids, vec![1]);
        });
    }

    #[test]
    fn screen_segment_upsert_tracks_first_and_last_frame_times() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "CREATE TABLE capture_sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    capture_session_id TEXT NOT NULL,
                    screen_source_session_id TEXT,
                    microphone_source_session_id TEXT,
                    system_audio_source_session_id TEXT
                )",
            )
            .execute(&pool)
            .await
            .expect("capture_sessions table should be created");
            sqlx::query(
                "INSERT INTO capture_sessions (capture_session_id, screen_source_session_id)
                 VALUES ('capture-1', 'screen-source-1')",
            )
            .execute(&pool)
            .await
            .expect("capture session should insert");

            let store = CaptureRetentionStore::new(pool.clone());
            store
                .upsert_screen_segment_for_source_session(
                    "screen-source-1",
                    1,
                    "/tmp/segment.mov".to_string(),
                    "/tmp/.segment".to_string(),
                    "/tmp/.segment/frames".to_string(),
                    "/tmp/segment.frame-index.bin".to_string(),
                    "2026-05-16T07:45:17.086Z".to_string(),
                )
                .await
                .expect("first frame should upsert segment");
            let segment = store
                .upsert_screen_segment_for_source_session(
                    "screen-source-1",
                    1,
                    "/tmp/segment.mov".to_string(),
                    "/tmp/.segment".to_string(),
                    "/tmp/.segment/frames".to_string(),
                    "/tmp/segment.frame-index.bin".to_string(),
                    "2026-05-16T07:45:32.105Z".to_string(),
                )
                .await
                .expect("later frame should upsert segment")
                .expect("segment should exist");

            assert_eq!(segment.started_at, "2026-05-16T07:45:17.086Z");
            assert_eq!(segment.ended_at, "2026-05-16T07:45:32.105Z");
        });
    }

    #[test]
    fn screen_segment_upsert_orders_mixed_precision_timestamps_by_time() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            let store = CaptureRetentionStore::new(pool);

            store
                .upsert_capture_segment(&NewCaptureSegment {
                    capture_session_id: "capture-1".to_string(),
                    source_kind: CaptureSourceKind::Screen,
                    source_session_id: "screen-source-1".to_string(),
                    segment_index: 1,
                    media_file_path: Some("/tmp/segment.mov".to_string()),
                    workspace_dir_path: None,
                    frame_dir_path: None,
                    sidecar_file_path: None,
                    started_at: "2026-05-16T07:45:30.100Z".to_string(),
                    ended_at: "2026-05-16T07:45:30Z".to_string(),
                    status: "completed".to_string(),
                })
                .await
                .expect("initial segment should upsert");
            let segment = store
                .upsert_capture_segment(&NewCaptureSegment {
                    capture_session_id: "capture-1".to_string(),
                    source_kind: CaptureSourceKind::Screen,
                    source_session_id: "screen-source-1".to_string(),
                    segment_index: 1,
                    media_file_path: Some("/tmp/segment.mov".to_string()),
                    workspace_dir_path: None,
                    frame_dir_path: None,
                    sidecar_file_path: None,
                    started_at: "2026-05-16T07:45:30Z".to_string(),
                    ended_at: "2026-05-16T07:45:30.100Z".to_string(),
                    status: "completed".to_string(),
                })
                .await
                .expect("later segment should upsert");

            assert_eq!(segment.started_at, "2026-05-16T07:45:30Z");
            assert_eq!(segment.ended_at, "2026-05-16T07:45:30.100Z");
        });
    }

    #[test]
    fn screen_segment_window_query_finds_legacy_zero_duration_segment_by_frame_time() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, started_at, ended_at, status
                 ) VALUES (
                    42, 'capture-1', 'screen', 'screen-source-1', 1,
                    '/tmp/segment.mov', '2026-05-16T07:45:17.086Z',
                    '2026-05-16T07:45:17.086Z', 'completed'
                 )",
            )
            .execute(&pool)
            .await
            .expect("legacy zero-duration segment should insert");
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at, capture_segment_id)
                 VALUES ('screen-source-1', '/tmp/frame.jpg', '2026-05-16T07:45:30.100Z', 42)",
            )
            .execute(&pool)
            .await
            .expect("linked frame should insert");

            let store = CaptureRetentionStore::new(pool);
            let segments = store
                .list_finalized_screen_segments_overlapping_window(
                    "2026-05-16T07:45:30Z",
                    "2026-05-16T07:45:31Z",
                )
                .await
                .expect("segments should query");

            assert_eq!(segments.len(), 1);
            assert_eq!(segments[0].id, 42);
        });
    }

    #[test]
    fn screen_segment_window_query_orders_mixed_precision_timestamps_by_time() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, started_at, ended_at, status
                 ) VALUES
                    (1, 'capture-1', 'screen', 'screen-source-1', 1,
                     '/tmp/segment-1.mov', '2026-05-16T07:45:30Z',
                     '2026-05-16T07:45:30.050Z', 'completed'),
                    (2, 'capture-1', 'screen', 'screen-source-1', 2,
                     '/tmp/segment-2.mov', '2026-05-16T07:45:30.100Z',
                     '2026-05-16T07:45:31Z', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("segments should insert");

            let store = CaptureRetentionStore::new(pool);
            let segments = store
                .list_finalized_screen_segments_overlapping_window(
                    "2026-05-16T07:45:30Z",
                    "2026-05-16T07:45:31Z",
                )
                .await
                .expect("segments should query");

            let ids = segments
                .into_iter()
                .map(|segment| segment.id)
                .collect::<Vec<_>>();
            assert_eq!(ids, vec![1, 2]);
        });
    }

    #[test]
    fn cleanup_deletes_expired_orphan_frame_rows() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at, capture_segment_id, frame_batch_id)
                 VALUES ('screen-source-1', '/tmp/mnema-expired-orphan-frame.jpg', '2026-05-10T15:01:50Z', NULL, NULL)",
            )
            .execute(&pool)
            .await
            .expect("orphan frame should insert");
            sqlx::query(
                "INSERT INTO processing_jobs (subject_type, subject_id, processor, status)
                 VALUES ('frame', 1, 'ocr', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("processing job should insert");
            sqlx::query("INSERT INTO processing_results (job_id, output_json) VALUES (1, '{}')")
                .execute(&pool)
                .await
                .expect("processing result should insert");

            let summary = CaptureRetentionStore::new(pool.clone())
                .run_cleanup(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:10:00Z"),
                    &RetentionCleanupContext::default(),
                )
                .await
                .expect("cleanup should succeed");

            assert_eq!(summary.eligible_capture_segments, 0);
            assert_eq!(summary.deleted_frames, 1);
            assert_eq!(summary.deleted_processing_jobs, 1);
            assert_eq!(summary.deleted_processing_results, 1);
            assert_eq!(summary.status, "completed");
            for table in ["frames", "processing_jobs", "processing_results"] {
                let count: i64 = sqlx::query(&format!("SELECT COUNT(*) AS count FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .expect("count should query")
                    .get("count");
                assert_eq!(count, 0, "{table} should be empty after cleanup");
            }
        });
    }

    #[test]
    fn cleanup_deletes_all_audio_files_for_expired_capture_segments() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            let dir = create_test_dir("attached-audio-files");
            let screen_path = dir.join("capture-1-segment-0001.mov");
            let first_audio_path = dir.join("audio-1.wav");
            let second_audio_path = dir.join("audio-2.wav");
            for path in [&screen_path, &first_audio_path, &second_audio_path] {
                fs::write(path, b"media").expect("test media file should be written");
            }

            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, started_at, ended_at, status
                 ) VALUES (
                    1, 'capture-1', 'screen', 'screen-source-1', 1,
                    ?1, '2026-05-10T15:00:00Z', '2026-05-10T15:05:00Z', 'completed'
                 )",
            )
            .bind(screen_path.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .expect("capture segment should insert");
            sqlx::query(
                "INSERT INTO audio_segments (
                    source_kind, source_session_id, segment_index, file_path, started_at, ended_at,
                    capture_segment_id
                 ) VALUES
                    ('microphone', 'mic-source-1', 1, ?1, '2026-05-10T15:00:00Z', '2026-05-10T15:01:00Z', 1),
                    ('microphone', 'mic-source-1', 2, ?2, '2026-05-10T15:01:00Z', '2026-05-10T15:05:00Z', 1)",
            )
            .bind(first_audio_path.to_string_lossy().as_ref())
            .bind(second_audio_path.to_string_lossy().as_ref())
            .execute(&pool)
            .await
            .expect("audio segments should insert");

            let summary = CaptureRetentionStore::new(pool.clone())
                .run_cleanup(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:10:00Z"),
                    &RetentionCleanupContext {
                        save_directory: Some(dir.to_string_lossy().to_string()),
                        ..Default::default()
                    },
                )
                .await
                .expect("cleanup should succeed");

            assert_eq!(summary.deleted_capture_segments, 1);
            assert_eq!(summary.deleted_audio_segments, 2);
            assert!(!screen_path.exists(), "screen media file should be deleted");
            assert!(!first_audio_path.exists(), "first audio file should be deleted");
            assert!(!second_audio_path.exists(), "second audio file should be deleted");

            fs::remove_dir_all(&dir).ok();
        });
    }

    #[test]
    fn cleanup_persists_requested_run_mode() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at, capture_segment_id, frame_batch_id)
                 VALUES ('screen-source-1', '/tmp/mnema-expired-orphan-frame.jpg', '2026-05-10T15:01:50Z', NULL, NULL)",
            )
            .execute(&pool)
            .await
            .expect("orphan frame should insert");

            CaptureRetentionStore::new(pool.clone())
                .run_cleanup_with_mode(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:10:00Z"),
                    &RetentionCleanupContext::default(),
                    RetentionCleanupMode::Automatic,
                )
                .await
                .expect("cleanup should succeed");

            let mode: String = sqlx::query_scalar("SELECT mode FROM retention_cleanup_runs")
                .fetch_one(&pool)
                .await
                .expect("cleanup run mode should query");
            assert_eq!(mode, "automatic");
        });
    }

    #[test]
    fn latest_status_returns_latest_run_for_requested_policy() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO retention_cleanup_runs (
                    policy, mode, cutoff_ended_before, status, deleted_capture_segments,
                    deleted_frames, deleted_audio_segments, deleted_processing_jobs,
                    deleted_processing_results, deleted_background_jobs, deleted_frame_batches,
                    skipped_running_jobs, skipped_active_segments, pending_file_tombstones
                 ) VALUES
                    ('days_7', 'manual', '2026-05-01T00:00:00Z', 'completed', 7, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                    ('days_14', 'manual', '2026-05-02T00:00:00Z', 'completed', 14, 0, 0, 0, 0, 0, 0, 0, 0, 0)",
            )
            .execute(&pool)
            .await
            .expect("cleanup runs should insert");

            let status = CaptureRetentionStore::new(pool)
                .latest_status(RetentionPolicy::Days7)
                .await
                .expect("latest status should query");

            assert_eq!(status.policy, "days_7");
            assert_eq!(status.deleted_capture_segments, 7);
        });
    }

    #[test]
    fn count_active_skipped_filters_by_source_session_when_segment_ids_empty() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index, started_at, ended_at, status
                 ) VALUES
                    (1, 'capture-1', 'screen', 'active-source', 1, '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed'),
                    (2, 'capture-2', 'screen', 'inactive-source', 1, '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("segments should insert");

            let skipped = count_active_skipped(
                &pool,
                &RetentionCleanupContext {
                    active_capture_segment_ids: Vec::new(),
                    active_source_session_ids: vec!["active-source".to_string()],
                    save_directory: None,
                },
            )
            .await
            .expect("active skipped count should query");

            assert_eq!(skipped, 1);
        });
    }

    #[test]
    fn cleanup_counts_running_jobs_only_for_expired_inactive_segments() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index, started_at, ended_at, status
                 ) VALUES
                    (1, 'capture-1', 'screen', 'expired-source', 1, '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed'),
                    (2, 'capture-2', 'screen', 'recent-source', 1, '2026-05-16T00:00:00Z', '2026-05-16T00:05:00Z', 'completed'),
                    (3, 'capture-3', 'screen', 'active-source', 1, '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("segments should insert");
            sqlx::query(
                "INSERT INTO frames (id, session_id, file_path, captured_at, capture_segment_id, frame_batch_id)
                 VALUES
                    (1, 'expired-source', '/tmp/expired.jpg', '2026-05-01T00:01:00Z', 1, NULL),
                    (2, 'recent-source', '/tmp/recent.jpg', '2026-05-16T00:01:00Z', 2, NULL),
                    (3, 'active-source', '/tmp/active.jpg', '2026-05-01T00:01:00Z', 3, NULL)",
            )
            .execute(&pool)
            .await
            .expect("frames should insert");
            sqlx::query(
                "INSERT INTO processing_jobs (subject_type, subject_id, processor, status)
                 VALUES
                    ('frame', 1, 'ocr', 'running'),
                    ('frame', 2, 'ocr', 'running'),
                    ('frame', 3, 'ocr', 'running')",
            )
            .execute(&pool)
            .await
            .expect("processing jobs should insert");

            let summary = CaptureRetentionStore::new(pool)
                .preview_cleanup(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:10:00Z"),
                    &RetentionCleanupContext {
                        active_capture_segment_ids: vec![3],
                        active_source_session_ids: Vec::new(),
                        save_directory: None,
                    },
                )
                .await
                .expect("cleanup preview should succeed");

            assert_eq!(summary.eligible_capture_segments, 0);
            assert_eq!(summary.skipped_running_jobs, 1);
        });
    }

    #[test]
    fn cleanup_counts_running_finalize_jobs_as_running_blockers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index, started_at, ended_at, status
                 ) VALUES (
                    1, 'capture-1', 'screen', 'expired-source', 1,
                    '2026-05-01T00:00:00Z', '2026-05-01T00:05:00Z', 'completed'
                 )",
            )
            .execute(&pool)
            .await
            .expect("segment should insert");
            sqlx::query("INSERT INTO background_jobs (id, status) VALUES (10, 'running')")
                .execute(&pool)
                .await
                .expect("background job should insert");
            sqlx::query("INSERT INTO frame_batches (id, finalize_job_id) VALUES (20, 10)")
                .execute(&pool)
                .await
                .expect("frame batch should insert");
            sqlx::query(
                "INSERT INTO frames (id, session_id, file_path, captured_at, capture_segment_id, frame_batch_id)
                 VALUES (30, 'expired-source', '/tmp/expired.jpg', '2026-05-01T00:01:00Z', 1, 20)",
            )
            .execute(&pool)
            .await
            .expect("frame should insert");

            let summary = CaptureRetentionStore::new(pool)
                .preview_cleanup(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:10:00Z"),
                    &RetentionCleanupContext::default(),
                )
                .await
                .expect("cleanup preview should succeed");

            assert_eq!(summary.eligible_capture_segments, 0);
            assert_eq!(summary.skipped_running_jobs, 1);
        });
    }

    #[test]
    fn cleanup_preserves_frame_batch_with_retained_frames() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db should open");
            create_retention_cleanup_tables(&pool).await;
            sqlx::query(
                "INSERT INTO capture_segments (
                    id, capture_session_id, source_kind, source_session_id, segment_index, started_at, ended_at, status
                 ) VALUES
                    (1, 'capture-1', 'screen', 'screen-source-1', 1, '2026-05-10T15:00:00Z', '2026-05-10T15:00:59Z', 'completed'),
                    (2, 'capture-1', 'screen', 'screen-source-1', 2, '2026-05-11T15:01:00Z', '2026-05-11T15:01:59Z', 'completed')",
            )
            .execute(&pool)
            .await
            .expect("segments should insert");
            sqlx::query("INSERT INTO frame_batches (id, finalize_job_id) VALUES (10, NULL)")
                .execute(&pool)
                .await
                .expect("frame batch should insert");
            sqlx::query(
                "INSERT INTO frames (id, session_id, file_path, captured_at, capture_segment_id, frame_batch_id)
                 VALUES
                    (1, 'screen-source-1', '/tmp/deleted-frame.jpg', '2026-05-10T15:00:30Z', 1, 10),
                    (2, 'screen-source-1', '/tmp/retained-frame.jpg', '2026-05-11T15:01:30Z', 2, 10)",
            )
            .execute(&pool)
            .await
            .expect("frames should insert");

            let summary = CaptureRetentionStore::new(pool.clone())
                .run_cleanup(
                    RetentionPolicy::Days7,
                    rfc3339("2026-05-17T15:06:30Z"),
                    &RetentionCleanupContext::default(),
                )
                .await
                .expect("cleanup should succeed");

            assert_eq!(summary.deleted_capture_segments, 1);
            assert_eq!(summary.deleted_frames, 1);
            assert_eq!(summary.deleted_frame_batches, 0);

            let retained_frame_batch_id: Option<i64> =
                sqlx::query_scalar("SELECT frame_batch_id FROM frames WHERE id = 2")
                    .fetch_one(&pool)
                    .await
                    .expect("retained frame should query");
            assert_eq!(retained_frame_batch_id, Some(10));

            let batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frame_batches")
                .fetch_one(&pool)
                .await
                .expect("frame batch count should query");
            assert_eq!(batch_count, 1);
        });
    }
}
