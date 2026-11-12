//! Delete Recent Capture's storage half: the rows (and the derived User Context
//! cascade) deleted in one transaction with a `pending` tombstone per file, then
//! the files. Like a retention batch, a quit or crash between the commit and
//! the file delete leaves tombstones the retention healer finishes, never
//! files no row points at.
//! The recording boundary around it is the host's (it pauses live capture).

use super::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRecentCaptureRequestDto {
    pub window_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRecentCaptureSummaryDto {
    pub window_seconds: i64,
    pub started_at: String,
    pub ended_at: String,
    pub deleted_capture_segments: i64,
    pub deleted_frames: i64,
    pub deleted_audio_segments: i64,
    pub deleted_processing_jobs: i64,
    pub deleted_processing_results: i64,
    pub deleted_background_jobs: i64,
    pub deleted_frame_batches: i64,
    pub deleted_search_documents: i64,
    pub pending_file_tombstones: i64,
    pub file_delete_errors: i64,
}

#[derive(Debug)]
pub struct DeleteRecentCaptureDeletion {
    pub frame_ids: Vec<i64>,
    pub audio_segment_ids: Vec<i64>,
    /// `(tombstone id, path)` for every file the deleted rows named, written
    /// `pending` in the deleting transaction.
    pub(super) file_tombstones: Vec<(i64, String)>,
    pub capture_segment_media_paths: Vec<String>,
    pub deleted_capture_segments: i64,
    pub deleted_frames: i64,
    pub deleted_audio_segments: i64,
    pub deleted_processing_jobs: i64,
    pub deleted_processing_results: i64,
    pub deleted_background_jobs: i64,
    pub deleted_frame_batches: i64,
    pub deleted_search_documents: i64,
    // Derived **User Context** cascade (ADR 0029), purged in the SAME
    // transaction as the raw rows so a privacy-panic delete and its derived
    // cascade commit (or roll back) together — never raw-gone-but-dossier-left.
    pub deleted_user_context_activities: i64,
    pub deleted_user_context_conclusions: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeleteRecentCapturePath {
    capture_segment_id: Option<i64>,
    path: String,
    path_kind: String,
}

pub fn validate_delete_recent_window(seconds: i64) -> Result<i64, String> {
    match seconds {
        60 | 300 | 900 => Ok(seconds),
        _ => Err("Delete Recent Capture supports only 60, 300, or 900 second windows".to_string()),
    }
}

pub(super) async fn delete_recent_by_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Result<i64, sqlx::Error> {
    let mut deleted = 0;
    for chunk in ids.chunks(900) {
        if chunk.is_empty() {
            continue;
        }

        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "DELETE FROM {table} WHERE {column} IN ("
        ));
        let mut separated = query.separated(", ");
        for id in chunk {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        deleted += query.build().execute(&mut **tx).await?.rows_affected() as i64;
    }
    Ok(deleted)
}

pub(super) async fn cleanup_unreferenced_delete_recent_frame_metadata_snapshots(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, sqlx::Error> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'frame_metadata_snapshots'",
    )
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_none() {
        return Ok(0);
    }

    Ok(sqlx::query(
        "DELETE FROM frame_metadata_snapshots
         WHERE NOT EXISTS (
            SELECT 1 FROM frames WHERE frames.metadata_snapshot_id = frame_metadata_snapshots.id
         )",
    )
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64)
}

pub(super) async fn delete_recent_frame_batch_ids_deletable_with_frames(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    frame_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if frame_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT DISTINCT candidate.frame_batch_id AS id
         FROM frames candidate
         WHERE candidate.frame_batch_id IS NOT NULL
           AND candidate.id IN (",
    );
    let mut separated = query.separated(", ");
    for id in frame_ids {
        separated.push_bind(*id);
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
        separated.push_bind(*id);
    }
    separated.push_unseparated("))");

    query.build_query_scalar().fetch_all(&mut **tx).await
}

pub(super) async fn delete_recent_background_job_ids_for_frame_batches(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    frame_batch_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if frame_batch_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT DISTINCT finalize_job_id AS id
         FROM frame_batches
         WHERE finalize_job_id IS NOT NULL AND id IN (",
    );
    let mut separated = query.separated(", ");
    for id in frame_batch_ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(")");

    query.build_query_scalar().fetch_all(&mut **tx).await
}

pub(super) async fn delete_recent_speaker_rows_for_audio_segments(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    audio_segment_ids: &[i64],
) -> Result<(), sqlx::Error> {
    if audio_segment_ids.is_empty() {
        return Ok(());
    }

    delete_recent_by_ids(tx, "speaker_turns", "audio_segment_id", audio_segment_ids).await?;
    delete_recent_by_ids(
        tx,
        "speaker_segment_clusters",
        "audio_segment_id",
        audio_segment_ids,
    )
    .await?;

    let orphan_cluster_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM recording_speaker_clusters
         WHERE NOT EXISTS (
            SELECT 1 FROM speaker_turns WHERE speaker_turns.cluster_id = recording_speaker_clusters.id
         )
           AND NOT EXISTS (
            SELECT 1 FROM speaker_segment_clusters
            WHERE speaker_segment_clusters.stable_cluster_id = recording_speaker_clusters.id
         )",
    )
    .fetch_all(&mut **tx)
    .await?;
    if orphan_cluster_ids.is_empty() {
        return Ok(());
    }

    delete_recent_by_ids(
        tx,
        "person_voice_embeddings",
        "source_cluster_id",
        &orphan_cluster_ids,
    )
    .await?;
    delete_recent_by_ids(
        tx,
        "speaker_recognition_rejections",
        "source_cluster_id",
        &orphan_cluster_ids,
    )
    .await?;
    delete_recent_by_ids(tx, "recording_speaker_clusters", "id", &orphan_cluster_ids).await?;
    Ok(())
}

pub(super) fn delete_recent_path_sort_key(path: &DeleteRecentCapturePath) -> (u8, &str) {
    let rank = match path.path_kind.as_str() {
        "media_file" | "sidecar_file" => 0,
        "frame_dir" => 1,
        _ => 2,
    };
    (rank, path.path.as_str())
}

pub(super) fn dedupe_delete_recent_paths(paths: &mut Vec<DeleteRecentCapturePath>) {
    paths.sort_by(|left, right| {
        delete_recent_path_sort_key(left).cmp(&delete_recent_path_sort_key(right))
    });
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.path.clone()));
}

/// **Delete Recent Capture** derived-data cascade (ADR 0029), run INSIDE the
/// raw-delete transaction so the privacy panic button's raw delete and its
/// derived purge are atomic. This is a thin adapter: it delegates to the
/// **canonical** cascade
/// [`::app_infra::user_context::cascade_derived_for_deleted_subjects_in`]
/// (`crates/app-infra/src/user_context/store.rs`), passing the SAME `tx` that
/// deleted the raw frames/audio so raw-delete + derived-purge commit or roll
/// back as one — something the pooled
/// `UserContextStore::delete_derived_for_capture_subjects` wrapper cannot do
/// because it owns its own transaction. There is now ONE copy of the cascade SQL
/// (and one chunk size), so the two paths can no longer drift. Returns the
/// dropped (activities, conclusions) counts; digests are purged too but their
/// count isn't surfaced to callers (matches the store summary's use).
pub(super) async fn cascade_user_context_for_deleted_subjects(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    frame_ids: &[i64],
    audio_segment_ids: &[i64],
) -> Result<(i64, i64), ::app_infra::AppInfraError> {
    if frame_ids.is_empty() && audio_segment_ids.is_empty() {
        return Ok((0, 0));
    }

    let summary = ::app_infra::user_context::cascade_derived_for_deleted_subjects_in(
        &mut *tx,
        frame_ids,
        audio_segment_ids,
    )
    .await?;

    Ok((summary.deleted_activities, summary.deleted_conclusions))
}

pub async fn delete_recent_capture_rows(
    infra: &::app_infra::AppInfra,
    started_at: &str,
    ended_at: &str,
) -> Result<DeleteRecentCaptureDeletion, String> {
    // Read-modify-write on the multi-connection Writer Pool: begin with
    // `BEGIN IMMEDIATE` (via `begin_write`) so the read→write upgrade can't
    // deadlock two writers, the pattern ADR 0041 standardized everywhere else.
    let mut tx = infra
        .begin_write()
        .await
        .map_err(|error| format!("failed to begin delete transaction: {error}"))?;

    let frame_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM frames
         WHERE (captured_at >= ?1 AND captured_at <= ?2)
            OR capture_segment_id IN (
                SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1
            )",
    )
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("failed to list frames for deletion: {error}"))?;

    let audio_segment_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM audio_segments
         WHERE (started_at <= ?2 AND ended_at >= ?1)
            OR capture_segment_id IN (
                SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1
            )",
    )
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("failed to list audio segments for deletion: {error}"))?;

    let capture_segment_media_paths: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT media_file_path FROM capture_segments
         WHERE started_at <= ?2 AND ended_at >= ?1 AND media_file_path IS NOT NULL",
    )
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("failed to list deleted capture segment media paths: {error}"))?;

    let mut paths: Vec<DeleteRecentCapturePath> = sqlx::query_as::<_, (Option<i64>, String, String)>(
        "SELECT capture_segment_id, file_path AS path, 'media_file' AS path_kind FROM frames
         WHERE (captured_at >= ?1 AND captured_at <= ?2)
            OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
         UNION
         SELECT capture_segment_id, file_path AS path, 'media_file' AS path_kind FROM audio_segments
         WHERE (started_at <= ?2 AND ended_at >= ?1)
            OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
         UNION
         SELECT id AS capture_segment_id, media_file_path AS path, 'media_file' AS path_kind
         FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1 AND media_file_path IS NOT NULL
         UNION
         SELECT id AS capture_segment_id, sidecar_file_path AS path, 'sidecar_file' AS path_kind
         FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1 AND sidecar_file_path IS NOT NULL
         UNION
         SELECT id AS capture_segment_id, workspace_dir_path AS path, 'workspace_dir' AS path_kind
         FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1 AND workspace_dir_path IS NOT NULL
         UNION
         SELECT id AS capture_segment_id, frame_dir_path AS path, 'frame_dir' AS path_kind
         FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1 AND frame_dir_path IS NOT NULL",
    )
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("failed to list paths for deletion: {error}"))?
    .into_iter()
    .map(
        |(capture_segment_id, path, path_kind)| DeleteRecentCapturePath {
            capture_segment_id,
            path,
            path_kind,
        },
    )
    .collect();
    dedupe_delete_recent_paths(&mut paths);

    let deleted_processing_jobs = sqlx::query(
        "DELETE FROM processing_jobs
         WHERE (subject_type = 'frame' AND subject_id IN (
                SELECT id FROM frames
                WHERE (captured_at >= ?1 AND captured_at <= ?2)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))
            OR (subject_type = 'audio_segment' AND subject_id IN (
                SELECT id FROM audio_segments
                WHERE (started_at <= ?2 AND ended_at >= ?1)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("failed to delete processing jobs: {error}"))?
    .rows_affected() as i64;

    let deleted_processing_results = sqlx::query(
        "DELETE FROM processing_results
         WHERE (subject_type = 'frame' AND subject_id IN (
                SELECT id FROM frames
                WHERE (captured_at >= ?1 AND captured_at <= ?2)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))
            OR (subject_type = 'audio_segment' AND subject_id IN (
                SELECT id FROM audio_segments
                WHERE (started_at <= ?2 AND ended_at >= ?1)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("failed to delete processing results: {error}"))?
    .rows_affected() as i64;

    let deleted_search_documents = sqlx::query(
        "DELETE FROM search_documents
         WHERE (frame_id IN (
                SELECT id FROM frames
                WHERE (captured_at >= ?1 AND captured_at <= ?2)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))
            OR (audio_segment_id IN (
                SELECT id FROM audio_segments
                WHERE (started_at <= ?2 AND ended_at >= ?1)
                   OR capture_segment_id IN (SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1)
            ))",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("failed to delete search documents: {error}"))?
    .rows_affected() as i64;

    delete_recent_speaker_rows_for_audio_segments(&mut tx, &audio_segment_ids)
        .await
        .map_err(|error| format!("failed to delete speaker analysis rows: {error}"))?;

    let frame_batch_ids = delete_recent_frame_batch_ids_deletable_with_frames(&mut tx, &frame_ids)
        .await
        .map_err(|error| format!("failed to list frame batches for deletion: {error}"))?;
    let background_job_ids =
        delete_recent_background_job_ids_for_frame_batches(&mut tx, &frame_batch_ids)
            .await
            .map_err(|error| format!("failed to list frame batch jobs for deletion: {error}"))?;

    let deleted_frames = sqlx::query(
        "DELETE FROM frames
         WHERE (captured_at >= ?1 AND captured_at <= ?2)
            OR capture_segment_id IN (
                SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1
            )",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("failed to delete frames: {error}"))?
    .rows_affected() as i64;

    cleanup_unreferenced_delete_recent_frame_metadata_snapshots(&mut tx)
        .await
        .map_err(|error| {
            format!("failed to delete unreferenced frame metadata snapshots: {error}")
        })?;

    let deleted_frame_batches =
        delete_recent_by_ids(&mut tx, "frame_batches", "id", &frame_batch_ids)
            .await
            .map_err(|error| format!("failed to delete frame batches: {error}"))?;
    let deleted_background_jobs =
        delete_recent_by_ids(&mut tx, "background_jobs", "id", &background_job_ids)
            .await
            .map_err(|error| format!("failed to delete frame batch jobs: {error}"))?;

    let deleted_audio_segments = sqlx::query(
        "DELETE FROM audio_segments
             WHERE (started_at <= ?2 AND ended_at >= ?1)
                OR capture_segment_id IN (
                    SELECT id FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1
                )",
    )
    .bind(started_at)
    .bind(ended_at)
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("failed to delete audio segments: {error}"))?
    .rows_affected() as i64;

    let deleted_capture_segments =
        sqlx::query("DELETE FROM capture_segments WHERE started_at <= ?2 AND ended_at >= ?1")
            .bind(started_at)
            .bind(ended_at)
            .execute(&mut *tx)
            .await
            .map_err(|error| format!("failed to delete capture segments: {error}"))?
            .rows_affected() as i64;

    // ADR 0029: the privacy panic button cascades into derived **User Context**,
    // and that cascade MUST be atomic with the raw delete — a crash or error
    // between committing the raw delete and running the cascade would leave the
    // dossier describing content that no longer exists ("spent 5 minutes on [the
    // sensitive thing]"). Folding it into THIS transaction makes raw-delete +
    // derived-purge commit or roll back as one. (Time-based Retention Policy
    // deliberately does NOT take this path — it never touches user_context_*.)
    let (deleted_user_context_activities, deleted_user_context_conclusions) =
        cascade_user_context_for_deleted_subjects(&mut tx, &frame_ids, &audio_segment_ids)
            .await
            .map_err(|error| format!("failed to cascade user-context deletion: {error}"))?;

    let tombstone_paths: Vec<_> = paths
        .iter()
        .filter(|path| !path.path.trim().is_empty())
        .map(|path| {
            (
                path.capture_segment_id,
                path.path.as_str(),
                path.path_kind.as_str(),
            )
        })
        .collect();
    let file_tombstones = ::app_infra::CaptureRetentionStore::insert_pending_file_tombstones(
        &mut tx,
        &tombstone_paths,
    )
    .await
    .map_err(|error| format!("failed to record delete recent file tombstones: {error}"))?;

    tx.commit()
        .await
        .map_err(|error| format!("failed to commit delete transaction: {error}"))?;

    Ok(DeleteRecentCaptureDeletion {
        frame_ids,
        audio_segment_ids,
        file_tombstones,
        capture_segment_media_paths,
        deleted_capture_segments,
        deleted_frames,
        deleted_audio_segments,
        deleted_processing_jobs,
        deleted_processing_results,
        deleted_background_jobs,
        deleted_frame_batches,
        deleted_search_documents,
        deleted_user_context_activities,
        deleted_user_context_conclusions,
    })
}

/// Deletes the files the committed delete tombstoned (off the async worker);
/// a removed file's tombstone is dropped, a failure stays `failed` for the
/// retention healer. Returns the failure count.
pub async fn delete_recent_capture_files_and_tombstone(
    infra: &::app_infra::AppInfra,
    deletion: &DeleteRecentCaptureDeletion,
    context: &::app_infra::RetentionCleanupContext,
) -> Result<i64, String> {
    infra
        .capture_retention()
        .remove_tombstoned_files(deletion.file_tombstones.clone(), context)
        .await
        .map_err(|error| format!("failed to delete recent capture files: {error}"))
}

pub fn complete_delete_recent_capture_boundary<T>(
    should_resume_after_boundary: bool,
    primary_result: Result<T, String>,
    resume: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    if !should_resume_after_boundary {
        return primary_result;
    }

    match (primary_result, resume()) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(resume_error)) => Err(format!(
            "deleted recent capture, but failed to resume recording: {resume_error}"
        )),
        (Err(primary_error), Ok(())) => Err(primary_error),
        (Err(primary_error), Err(resume_error)) => Err(format!(
            "{primary_error}; additionally failed to resume recording after delete recent capture boundary: {resume_error}"
        )),
    }
}

pub fn should_resume_delete_recent_capture_boundary(session: &NativeCaptureSession) -> bool {
    session.is_running && !session.is_user_paused && !session.is_inactivity_paused
}

pub fn format_delete_recent_capture_window(
    window_seconds: i64,
    ended: OffsetDateTime,
) -> Result<(String, String), String> {
    let started = ended - time::Duration::seconds(window_seconds);
    let started_at = started
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("failed to format delete window start: {error}"))?;
    let ended_at = ended
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| format!("failed to format delete window end: {error}"))?;

    Ok((started_at, ended_at))
}
