use super::*;
use crate::app_infra::test_helpers::*;

#[test]
fn delete_recent_capture_boundary_resumes_on_primary_error() {
    let mut resume_calls = 0;

    let result = complete_delete_recent_capture_boundary::<()>(
        true,
        Err("file tombstone failed".to_string()),
        || {
            resume_calls += 1;
            Ok(())
        },
    );

    assert_eq!(result, Err("file tombstone failed".to_string()));
    assert_eq!(resume_calls, 1);
}

#[test]
fn delete_recent_capture_boundary_reports_resume_failure_with_primary_error() {
    let result = complete_delete_recent_capture_boundary::<()>(
        true,
        Err("file tombstone failed".to_string()),
        || Err("resume failed".to_string()),
    );

    assert_eq!(
            result,
            Err("file tombstone failed; additionally failed to resume recording after delete recent capture boundary: resume failed".to_string())
        );
}

#[test]
fn delete_recent_capture_boundary_does_not_resume_when_not_required() {
    let result = complete_delete_recent_capture_boundary::<()>(
        false,
        Err("delete failed".to_string()),
        || panic!("resume should not be called"),
    );

    assert_eq!(result, Err("delete failed".to_string()));
}

#[test]
fn delete_recent_capture_boundary_does_not_resume_when_inactivity_paused() {
    let session = NativeCaptureSession {
        is_running: true,
        is_inactivity_paused: true,
        is_user_paused: false,
        is_low_disk_suspended: false,
        requested_sources: None,
        output_files: None,
        source_sessions: None,
        recording_started_at_unix_ms: None,
    };

    assert!(!should_resume_delete_recent_capture_boundary(&session));
}

#[test]
fn delete_recent_capture_boundary_does_not_resume_when_user_paused() {
    let session = NativeCaptureSession {
        is_running: true,
        is_inactivity_paused: false,
        is_user_paused: true,
        is_low_disk_suspended: false,
        requested_sources: None,
        output_files: None,
        source_sessions: None,
        recording_started_at_unix_ms: None,
    };

    assert!(!should_resume_delete_recent_capture_boundary(&session));
}

#[test]
fn delete_recent_capture_window_uses_boundary_end() {
    let ended = OffsetDateTime::parse(
        "2026-05-19T10:01:03Z",
        &time::format_description::well_known::Rfc3339,
    )
    .expect("boundary time should parse");

    let (started_at, ended_at) =
        format_delete_recent_capture_window(60, ended).expect("window should format");

    assert_eq!(started_at, "2026-05-19T10:00:03Z");
    assert_eq!(ended_at, "2026-05-19T10:01:03Z");
}

#[test]
fn delete_recent_capture_rows_prunes_unreferenced_metadata_snapshots() {
    run_async_test(async {
        let dir = TestDir::new("delete-recent-metadata-snapshots");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let pool = infra.pool();
        let deleted_media_path = dir
            .path()
            .join("recordings/2026/05/19/session-segment-0001.mov")
            .to_string_lossy()
            .to_string();

        sqlx::query(
                "INSERT INTO capture_sessions (
                    capture_session_id, started_at, stopped_at, status,
                    requested_screen, requested_microphone, requested_system_audio,
                    screen_source_session_id, segment_duration_seconds
                 ) VALUES ('cap-delete-recent', '2026-05-19T10:00:00Z', NULL, 'recording', 1, 0, 0, 'screen-delete-recent', 60)",
            )
            .execute(pool)
            .await
            .expect("capture session should insert");
        let segment_id: i64 = sqlx::query_scalar(
            "INSERT INTO capture_segments (
                    capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, started_at, ended_at, status
                 ) VALUES (
                    'cap-delete-recent', 'screen', 'screen-delete-recent', 1,
                    ?1, '2026-05-19T10:00:00Z', '2026-05-19T10:00:30Z', 'completed'
                 ) RETURNING id",
        )
        .bind(&deleted_media_path)
        .fetch_one(pool)
        .await
        .expect("capture segment should insert");
        let deleted_snapshot_id: i64 = sqlx::query_scalar(
            "INSERT INTO frame_metadata_snapshots (normalized_hash, snapshot_json)
                 VALUES ('deleted-hash', '{\"app\":\"deleted\"}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("deleted metadata snapshot should insert");
        let retained_snapshot_id: i64 = sqlx::query_scalar(
            "INSERT INTO frame_metadata_snapshots (normalized_hash, snapshot_json)
                 VALUES ('retained-hash', '{\"app\":\"retained\"}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("retained metadata snapshot should insert");
        sqlx::query(
                "INSERT INTO frames (
                    session_id, file_path, captured_at, capture_segment_id, metadata_snapshot_id
                 ) VALUES ('screen-delete-recent', '/tmp/deleted-frame.jpg', '2026-05-19T10:00:05Z', ?1, ?2)",
            )
            .bind(segment_id)
            .bind(deleted_snapshot_id)
            .execute(pool)
            .await
            .expect("deleted frame should insert");
        sqlx::query(
                "INSERT INTO frames (
                    session_id, file_path, captured_at, metadata_snapshot_id
                 ) VALUES ('screen-retained', '/tmp/retained-frame.jpg', '2026-05-19T09:00:00Z', ?1)",
            )
            .bind(retained_snapshot_id)
            .execute(pool)
            .await
            .expect("retained frame should insert");
        // Typed text is never named by Delete Recent Capture (conversations'
        // contract): a query run inside the deleted window survives it.
        let recent_query = ::app_infra::RecentQuery {
            text: "pricing page".to_string(),
            ask: false,
            at: 1_779_184_805_000, // 2026-05-19T10:00:05Z
        };
        ::app_infra::store_recent_queries(pool, vec![recent_query.clone()])
            .await
            .expect("recent query should store");

        let deletion =
            delete_recent_capture_rows(&infra, "2026-05-19T09:59:00Z", "2026-05-19T10:01:00Z")
                .await
                .expect("delete recent rows should complete");

        assert_eq!(deletion.deleted_frames, 1);
        assert_eq!(deletion.deleted_capture_segments, 1);
        assert_eq!(
            deletion.capture_segment_media_paths,
            vec![deleted_media_path]
        );
        assert_eq!(
            ::app_infra::load_recent_queries(pool).await.expect("load"),
            vec![recent_query]
        );
        let deleted_snapshot_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM frame_metadata_snapshots WHERE id = ?1")
                .bind(deleted_snapshot_id)
                .fetch_one(pool)
                .await
                .expect("deleted snapshot count should load");
        let retained_snapshot_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM frame_metadata_snapshots WHERE id = ?1")
                .bind(retained_snapshot_id)
                .fetch_one(pool)
                .await
                .expect("retained snapshot count should load");

        assert_eq!(deleted_snapshot_count, 0);
        assert_eq!(retained_snapshot_count, 1);
    });
}

#[test]
fn delete_recent_capture_rows_prunes_speaker_analysis_rows() {
    run_async_test(async {
        let dir = TestDir::new("delete-recent-speaker-rows");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let pool = infra.pool();

        let audio_segment_id: i64 = sqlx::query_scalar(
            "INSERT INTO audio_segments (
                    source_kind, source_session_id, segment_index, file_path, started_at, ended_at
                 ) VALUES (
                    'microphone', 'mic-delete-recent', 1, '/tmp/delete-recent-audio.m4a',
                    '2026-05-19T10:00:00Z', '2026-05-19T10:00:30Z'
                 ) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("audio segment should insert");
        let person_id: i64 = sqlx::query_scalar(
            "INSERT INTO person_profiles (display_name) VALUES ('Speaker') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("person profile should insert");
        let cluster_id: i64 = sqlx::query_scalar(
            "INSERT INTO recording_speaker_clusters (
                    session_id, provider, model_id, provider_cluster_id, stable_label
                 ) VALUES (
                    'mic-delete-recent', 'test-provider', 'test-model', 'cluster-1', 'Speaker 1'
                 ) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("speaker cluster should insert");
        sqlx::query(
                "INSERT INTO speaker_segment_clusters (
                    audio_segment_id, session_id, provider, model_id, provider_cluster_id,
                    stable_cluster_id, stable_label
                 ) VALUES (?1, 'mic-delete-recent', 'test-provider', 'test-model', 'segment-cluster-1', ?2, 'Speaker 1')",
            )
            .bind(audio_segment_id)
            .bind(cluster_id)
            .execute(pool)
            .await
            .expect("speaker segment cluster should insert");
        sqlx::query(
            "INSERT INTO speaker_turns (
                    audio_segment_id, session_id, cluster_id, start_ms, end_ms, transcript_text
                 ) VALUES (?1, 'mic-delete-recent', ?2, 0, 1000, 'hello')",
        )
        .bind(audio_segment_id)
        .bind(cluster_id)
        .execute(pool)
        .await
        .expect("speaker turn should insert");
        // Both production writers of this table set `is_deliberate = 1`
        // (`upsert_account_owner_voiceprint` and the audio-drawer link path), so a row
        // defaulted to 0 is a shape the field never contains. Retention's orphan sweep
        // now SPARES `is_deliberate = 1`; a privacy delete must not. Insert both shapes
        // so this test fails if that carve-out is ever copied into the delete path —
        // which is the promise migration 0052 makes in writing.
        sqlx::query(
                "INSERT INTO person_voice_embeddings (
                    person_id, provider, model_id, embedding, source_session_id, source_cluster_id,
                    is_deliberate
                 ) VALUES (?1, 'test-provider', 'test-model', X'010203', 'mic-delete-recent', ?2, 1)",
            )
            .bind(person_id)
            .bind(cluster_id)
            .execute(pool)
            .await
            .expect("deliberate voice embedding should insert");
        sqlx::query(
                "INSERT INTO person_voice_embeddings (
                    person_id, provider, model_id, embedding, source_session_id, source_cluster_id,
                    is_deliberate
                 ) VALUES (?1, 'test-provider', 'test-model', X'040506', 'mic-delete-recent', ?2, 0)",
            )
            .bind(person_id)
            .bind(cluster_id)
            .execute(pool)
            .await
            .expect("voice embedding should insert");
        sqlx::query(
            "INSERT INTO speaker_recognition_rejections (
                    person_id, provider, model_id, embedding, source_session_id, source_cluster_id
                 ) VALUES (?1, 'test-provider', 'test-model', X'040506', 'mic-delete-recent', ?2)",
        )
        .bind(person_id)
        .bind(cluster_id)
        .execute(pool)
        .await
        .expect("speaker rejection should insert");

        let deletion =
            delete_recent_capture_rows(&infra, "2026-05-19T09:59:00Z", "2026-05-19T10:01:00Z")
                .await
                .expect("delete recent rows should complete");

        assert_eq!(deletion.deleted_audio_segments, 1);
        for table in [
            "audio_segments",
            "speaker_turns",
            "speaker_segment_clusters",
            "recording_speaker_clusters",
            "person_voice_embeddings",
            "speaker_recognition_rejections",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            let count: i64 = sqlx::query_scalar(&sql)
                .fetch_one(pool)
                .await
                .expect("row count should load");
            assert_eq!(count, 0, "{table} should be pruned");
        }
        let person_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM person_profiles")
            .fetch_one(pool)
            .await
            .expect("person count should load");
        assert_eq!(person_count, 1);
    });
}

/// ADR 0029 + atomicity: the derived **User Context** cascade must commit in
/// the SAME transaction as the raw delete, so a single
/// `delete_recent_capture_rows` call leaves neither the raw rows NOR the
/// dossier rows derived from them behind. Seeds a frame inside the window, an
/// Activity grounded in it, an overlapping Digest, and a Conclusion grounded
/// only by that Activity, then asserts all are gone and the counts surface.
#[test]
fn delete_recent_capture_rows_cascades_user_context_in_one_transaction() {
    run_async_test(async {
        let dir = TestDir::new("delete-recent-user-context-cascade");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let pool = infra.pool();

        // A frame inside the delete window — the cascade keys off its id.
        let frame_id: i64 = sqlx::query_scalar(
            "INSERT INTO frames (session_id, file_path, captured_at)
                 VALUES ('screen-uc', '/tmp/uc-frame.jpg', '2026-05-19T10:00:05Z')
                 RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("frame should insert");

        // An Activity grounded in that frame, plus an overlapping Digest.
        let activity_id: i64 = sqlx::query_scalar(
            "INSERT INTO user_context_activities (
                    title, summary, started_at_ms, ended_at_ms, created_at_ms
                 ) VALUES ('Worked on the spec', 'spec work', 1000, 2000, 1000)
                 RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("activity should insert");
        sqlx::query(
            "INSERT INTO user_context_activity_evidence (activity_id, subject_type, subject_id)
                 VALUES (?1, 'frame', ?2)",
        )
        .bind(activity_id)
        .bind(frame_id)
        .execute(pool)
        .await
        .expect("activity evidence should insert");
        sqlx::query(
            "INSERT INTO user_context_digests (
                    range_kind, range_start_ms, range_end_ms, narrative, input_fingerprint,
                    generated_at_ms
                 ) VALUES ('day', 0, 5000, 'a day of spec work', 'fp', 1000)",
        )
        .execute(pool)
        .await
        .expect("digest should insert");

        // A Conclusion grounded ONLY by that Activity -> zero surviving
        // support once the Activity is gone -> dropped by the formation bar.
        let conclusion_id: i64 = sqlx::query_scalar(
            "INSERT INTO user_context_conclusions (
                    subject, statement, confidence, formed_at_ms, last_supported_at_ms,
                    updated_at_ms, created_at_ms
                 ) VALUES ('Spec', 'Cares about the spec', 0.7, 1000, 1000, 1000, 1000)
                 RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("conclusion should insert");
        sqlx::query(
            "INSERT INTO user_context_conclusion_evidence (
                    conclusion_id, activity_id, stance, created_at_ms
                 ) VALUES (?1, ?2, 'support', 1000)",
        )
        .bind(conclusion_id)
        .bind(activity_id)
        .execute(pool)
        .await
        .expect("conclusion evidence should insert");

        let deletion =
            delete_recent_capture_rows(&infra, "2026-05-19T09:59:00Z", "2026-05-19T10:01:00Z")
                .await
                .expect("delete recent rows should complete");

        assert_eq!(deletion.deleted_frames, 1);
        assert_eq!(deletion.deleted_user_context_activities, 1);
        assert_eq!(deletion.deleted_user_context_conclusions, 1);

        // The raw frame AND every derived row committed away together.
        for table in [
            "frames",
            "user_context_activities",
            "user_context_activity_evidence",
            "user_context_conclusions",
            "user_context_conclusion_evidence",
            "user_context_digests",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            let count: i64 = sqlx::query_scalar(&sql)
                .fetch_one(pool)
                .await
                .expect("row count should load");
            assert_eq!(
                count, 0,
                "{table} should be purged atomically with the raw delete"
            );
        }
    });
}

#[test]
fn delete_recent_capture_rows_prunes_empty_frame_batches() {
    run_async_test(async {
        let dir = TestDir::new("delete-recent-frame-batches");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let pool = infra.pool();

        sqlx::query(
                "INSERT INTO capture_sessions (
                    capture_session_id, started_at, stopped_at, status,
                    requested_screen, requested_microphone, requested_system_audio,
                    screen_source_session_id, segment_duration_seconds
                 ) VALUES ('cap-delete-batch', '2026-05-19T10:00:00Z', NULL, 'recording', 1, 0, 0, 'screen-delete-batch', 60)",
            )
            .execute(pool)
            .await
            .expect("capture session should insert");
        let segment_id: i64 = sqlx::query_scalar(
                "INSERT INTO capture_segments (
                    capture_session_id, source_kind, source_session_id, segment_index,
                    media_file_path, started_at, ended_at, status
                 ) VALUES (
                    'cap-delete-batch', 'screen', 'screen-delete-batch', 1,
                    '/tmp/delete-batch-segment.mov', '2026-05-19T10:00:00Z', '2026-05-19T10:00:30Z', 'completed'
                 ) RETURNING id",
            )
            .fetch_one(pool)
            .await
            .expect("capture segment should insert");
        let job_id: i64 = sqlx::query_scalar(
            "INSERT INTO background_jobs (kind, status, payload_json)
                 VALUES ('frame_batch_finalize', 'queued', '{}') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("background job should insert");
        let frame_batch_id: i64 = sqlx::query_scalar(
            "INSERT INTO frame_batches (
                    session_id, batch_key, batch_started_at, batch_ended_at, status,
                    frame_count, finalize_job_id
                 ) VALUES (
                    'screen-delete-batch', 'batch-1',
                    '2026-05-19T10:00:00Z', '2026-05-19T10:00:30Z', 'closed',
                    1, ?1
                 ) RETURNING id",
        )
        .bind(job_id)
        .fetch_one(pool)
        .await
        .expect("frame batch should insert");
        sqlx::query(
            "INSERT INTO frames (
                    session_id, file_path, captured_at, capture_segment_id, frame_batch_id
                 ) VALUES (
                    'screen-delete-batch', '/tmp/delete-batch-frame.jpg',
                    '2026-05-19T10:00:05Z', ?1, ?2
                 )",
        )
        .bind(segment_id)
        .bind(frame_batch_id)
        .execute(pool)
        .await
        .expect("frame should insert");

        let deletion =
            delete_recent_capture_rows(&infra, "2026-05-19T09:59:00Z", "2026-05-19T10:01:00Z")
                .await
                .expect("delete recent rows should complete");

        assert_eq!(deletion.deleted_frames, 1);
        assert_eq!(deletion.deleted_frame_batches, 1);
        assert_eq!(deletion.deleted_background_jobs, 1);
        let frame_batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frame_batches")
            .fetch_one(pool)
            .await
            .expect("frame batch count should load");
        let background_job_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM background_jobs")
            .fetch_one(pool)
            .await
            .expect("background job count should load");
        assert_eq!(frame_batch_count, 0);
        assert_eq!(background_job_count, 0);
    });
}

#[test]
fn delete_recent_capture_files_guard_save_directory_and_tombstone_failures() {
    run_async_test(async {
        let save_dir = TestDir::new("delete-recent-safe-files");
        let outside_dir = TestDir::new("delete-recent-outside-files");
        let infra = ::app_infra::AppInfra::initialize(save_dir.path())
            .await
            .expect("app infra should initialize");
        let inside_file = save_dir.path().join("recordings/inside-frame.jpg");
        fs::create_dir_all(
            inside_file
                .parent()
                .expect("inside file should have parent"),
        )
        .expect("inside parent should be created");
        fs::write(&inside_file, b"inside").expect("inside file should be written");
        let outside_file = outside_dir.path().join("outside-frame.jpg");
        fs::write(&outside_file, b"outside").expect("outside file should be written");

        let (inside, outside) = (
            inside_file.to_string_lossy().to_string(),
            outside_file.to_string_lossy().to_string(),
        );
        let mut tx = infra.begin_write().await.expect("tx should begin");
        let file_tombstones = ::app_infra::CaptureRetentionStore::insert_pending_file_tombstones(
            &mut tx,
            &[
                (Some(1), inside.as_str(), "media_file"),
                (Some(2), outside.as_str(), "media_file"),
            ],
        )
        .await
        .expect("tombstones should insert");
        tx.commit().await.expect("tombstones should commit");

        let deletion = DeleteRecentCaptureDeletion {
            frame_ids: Vec::new(),
            audio_segment_ids: Vec::new(),
            file_tombstones,
            capture_segment_media_paths: Vec::new(),
            deleted_capture_segments: 0,
            deleted_frames: 0,
            deleted_audio_segments: 0,
            deleted_processing_jobs: 0,
            deleted_processing_results: 0,
            deleted_background_jobs: 0,
            deleted_frame_batches: 0,
            deleted_search_documents: 0,
            deleted_user_context_activities: 0,
            deleted_user_context_conclusions: 0,
        };
        let context = ::app_infra::RetentionCleanupContext {
            save_directory: Some(save_dir.path().display().to_string()),
            ..Default::default()
        };

        let failures = delete_recent_capture_files_and_tombstone(&infra, &deletion, &context)
            .await
            .expect("file deletion should record tombstones");

        assert_eq!(failures, 1);
        assert!(
            !inside_file.exists(),
            "inside saveDirectory file should be deleted"
        );
        assert!(
            outside_file.exists(),
            "outside saveDirectory file should not be deleted"
        );
        let tombstone_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM retention_file_tombstones")
                .fetch_one(infra.pool())
                .await
                .expect("tombstone count should load");
        assert_eq!(tombstone_count, 1);
    });
}

/// The kill point: Delete Recent's rows are committed gone but its files were
/// never touched (quit/crash before the file delete). The files must stay
/// findable as `pending` tombstones the retention healer then removes.
#[test]
fn delete_recent_capture_rows_tombstones_files_in_the_deleting_transaction() {
    run_async_test(async {
        let dir = TestDir::new("delete-recent-tombstones-in-tx");
        let infra = ::app_infra::AppInfra::initialize(dir.path())
            .await
            .expect("app infra should initialize");
        let pool = infra.pool();
        let media = dir.path().join("recordings/segment-0001.mov");
        fs::create_dir_all(media.parent().expect("media parent")).expect("media dir");
        fs::write(&media, b"media").expect("media should be written");
        let media_path = media.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO capture_sessions (
                capture_session_id, started_at, stopped_at, status,
                requested_screen, requested_microphone, requested_system_audio,
                screen_source_session_id, segment_duration_seconds
             ) VALUES ('cap-tomb', '2026-05-19T10:00:00Z', NULL, 'recording', 1, 0, 0, 'screen-tomb', 60)",
        )
        .execute(pool)
        .await
        .expect("capture session should insert");
        sqlx::query(
            "INSERT INTO capture_segments (
                capture_session_id, source_kind, source_session_id, segment_index,
                media_file_path, started_at, ended_at, status
             ) VALUES ('cap-tomb', 'screen', 'screen-tomb', 1, ?1,
                '2026-05-19T10:00:00Z', '2026-05-19T10:00:30Z', 'completed')",
        )
        .bind(&media_path)
        .execute(pool)
        .await
        .expect("capture segment should insert");

        let deletion =
            delete_recent_capture_rows(&infra, "2026-05-19T09:59:00Z", "2026-05-19T10:01:00Z")
                .await
                .expect("delete recent rows should complete");

        assert_eq!(deletion.deleted_capture_segments, 1);
        assert!(media.exists(), "the row delete alone never touches files");
        let pending: Vec<String> = sqlx::query_scalar(
            "SELECT path FROM retention_file_tombstones WHERE status = 'pending'",
        )
        .fetch_all(pool)
        .await
        .expect("tombstones should load");
        assert_eq!(pending, vec![media_path]);

        let context = ::app_infra::RetentionCleanupContext {
            save_directory: Some(dir.path().display().to_string()),
            ..Default::default()
        };
        let resolved = infra
            .capture_retention()
            .retry_pending_file_tombstones(&context)
            .await
            .expect("healer should run");
        assert_eq!(resolved, 1);
        assert!(!media.exists(), "the healer removes the stranded file");
    });
}
