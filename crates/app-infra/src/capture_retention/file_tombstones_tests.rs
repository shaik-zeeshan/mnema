//! A retention sweep stopped between batches (quit, crash, a failing batch)
//! must never strand capture files whose rows are already gone.

use std::fs;
use std::path::PathBuf;

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

use super::tests::{create_retention_cleanup_tables, create_test_dir, rfc3339};
use super::*;

/// Six segments (more than one batch), each with a real media file; segment 1
/// also owns a frame folder. Returns the pool, the save dir, the media paths
/// and that frame folder.
async fn seed_six_segments(name: &str) -> (SqlitePool, PathBuf, Vec<PathBuf>, PathBuf) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory db should open");
    create_retention_cleanup_tables(&pool).await;
    let dir = create_test_dir(name);
    let frame_dir = dir.join("segment-1-frames");
    fs::create_dir_all(&frame_dir).expect("frame dir should be created");
    fs::write(frame_dir.join("0001.png"), b"png").expect("frame should be written");
    let mut media = Vec::new();
    for id in 1..=6_i64 {
        let path = dir.join(format!("segment-{id}.mov"));
        fs::write(&path, b"media").expect("media should be written");
        sqlx::query(
            "INSERT INTO capture_segments (
                id, capture_session_id, source_kind, source_session_id, segment_index,
                media_file_path, frame_dir_path, started_at, ended_at, status
             ) VALUES (?1, 'capture-1', 'screen', 'screen-source-1', ?1, ?2, ?3,
                '2026-05-10T15:00:00Z', '2026-05-10T15:05:00Z', 'completed')",
        )
        .bind(id)
        .bind(path.to_string_lossy().as_ref())
        .bind((id == 1).then(|| frame_dir.to_string_lossy().to_string()))
        .execute(&pool)
        .await
        .expect("capture segment should insert");
        media.push(path);
    }
    (pool, dir, media, frame_dir)
}

async fn tombstone_paths(pool: &SqlitePool, status: &str) -> Vec<String> {
    let mut paths: Vec<String> =
        sqlx::query_scalar("SELECT path FROM retention_file_tombstones WHERE status = ?1")
            .bind(status)
            .fetch_all(pool)
            .await
            .expect("tombstones should query");
    paths.sort();
    paths
}

async fn segment_ids(pool: &SqlitePool) -> Vec<i64> {
    sqlx::query_scalar("SELECT id FROM capture_segments ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("segments should query")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
}

#[test]
fn sweep_killed_after_a_batch_commit_leaves_pending_tombstones_the_healer_finishes() {
    runtime().block_on(async {
        let (pool, dir, media, frame_dir) = seed_six_segments("killed-after-commit").await;
        let store = CaptureRetentionStore::new(CaptureDb::single(pool.clone()));
        let context = RetentionCleanupContext {
            save_directory: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        let first_batch = [1_i64, 2, 3, 4];
        let paths = file_paths_for_segments(&pool, &first_batch)
            .await
            .expect("paths should load");
        let refs = paths.iter().collect::<Vec<_>>();

        // The kill point: batch 1's rows are committed gone, its files not yet
        // touched (the post-commit delete never runs).
        let mut summary = RetentionCleanupSummary::default();
        store
            .delete_eligible_batch(&first_batch, &[], &[], &refs, &mut summary)
            .await
            .expect("batch should commit");

        assert_eq!(segment_ids(&pool).await, vec![5, 6]);
        let mut expected = media[..4]
            .iter()
            .chain([&frame_dir])
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(tombstone_paths(&pool, "pending").await, expected);
        assert!(
            media.iter().all(|path| path.exists()),
            "no file deleted yet"
        );

        let resolved = store
            .retry_pending_file_tombstones(&context)
            .await
            .expect("healer should run");

        assert_eq!(resolved, 5);
        assert!(media[..4].iter().all(|path| !path.exists()));
        assert!(!frame_dir.exists(), "frame folder should be removed");
        assert!(
            media[4..].iter().all(|path| path.exists()),
            "kept segments keep files"
        );
        assert_eq!(store.pending_file_tombstone_count().await.unwrap(), 0);
        fs::remove_dir_all(&dir).ok();
    });
}

#[test]
fn sweep_failing_in_a_later_batch_keeps_earlier_deletes_whole_and_strands_nothing() {
    runtime().block_on(async {
        let (pool, dir, media, frame_dir) = seed_six_segments("fails-in-batch-two").await;
        // Batch 2 (segments 5–6) aborts inside its transaction.
        sqlx::query(
            "CREATE TRIGGER fail_batch_two BEFORE DELETE ON capture_segments
             WHEN OLD.id = 5 BEGIN SELECT RAISE(ABORT, 'injected'); END",
        )
        .execute(&pool)
        .await
        .expect("trigger should create");
        let store = CaptureRetentionStore::new(CaptureDb::single(pool.clone()));
        let context = RetentionCleanupContext {
            save_directory: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        let result = store
            .run_cleanup(
                RetentionPolicy::Days7,
                rfc3339("2026-05-17T15:10:00Z"),
                &context,
            )
            .await;

        assert!(result.is_err(), "the injected abort should surface");
        assert_eq!(segment_ids(&pool).await, vec![5, 6]);
        assert!(
            media[..4].iter().all(|path| !path.exists()),
            "batch 1 files removed"
        );
        assert!(!frame_dir.exists());
        assert!(
            media[4..].iter().all(|path| path.exists()),
            "batch 2 rolled back"
        );
        let tombstones: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM retention_file_tombstones")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            tombstones, 0,
            "done files leave no row; batch 2's rolled back"
        );
        fs::remove_dir_all(&dir).ok();
    });
}
