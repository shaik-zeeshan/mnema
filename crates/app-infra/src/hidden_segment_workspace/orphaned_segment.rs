//! Orphaned hidden-segment workspaces: frame images that capture exported but
//! the persistence worker never recorded (app quit or crashed with frames still
//! in the channel). The DB has no frame rows and no `capture_segments` row for
//! them, nothing ever re-ingests frames from disk, and Retention / Delete Recent
//! Capture only act on DB rows — so without this they are rescanned and kept
//! forever.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{AppInfraError, Result};

/// How long an un-persisted frame image may sit before it counts as orphaned.
/// The live persistence channel drains in seconds; a day is far past any lag.
const ORPHANED_FRAME_ARTIFACT_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

/// True when the frames dir holds frame images and the newest is older than
/// [`ORPHANED_FRAME_ARTIFACT_GRACE`]. Any unreadable mtime reads as fresh.
pub(super) fn frame_artifacts_are_stale(frames_dir: &Path, now: SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(frames_dir) else {
        return false;
    };
    let mut newest = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !super::is_safe_frame_artifact_path(&path) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            return false;
        };
        newest = newest.max(Some(modified));
    }
    newest.is_some_and(|newest| {
        now.duration_since(newest)
            .is_ok_and(|age| age > ORPHANED_FRAME_ARTIFACT_GRACE)
    })
}

/// Delete an orphaned workspace's visible segment (`.mov` + `.frame-index.bin`
/// sidecar) when it was last written before `retention_cutoff` — the same
/// cutoff Retention Cleanup would have applied had the DB known about it.
/// `None` (retention Never) keeps it. Returns the bytes freed (0 when kept).
///
/// Only called for `OrphanedFrameArtifacts`, which already guarantees no
/// `capture_segments` row owns these paths.
pub(super) fn reclaim_orphaned_visible_segment(
    visible_segment_path: &str,
    retention_cutoff: Option<&str>,
) -> Result<u64> {
    let Some(cutoff) =
        retention_cutoff.and_then(|cutoff| OffsetDateTime::parse(cutoff, &Rfc3339).ok())
    else {
        return Ok(0);
    };
    let path = Path::new(visible_segment_path);
    let Ok(modified) = std::fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return Ok(0);
    };
    if modified >= SystemTime::from(cutoff) {
        return Ok(0);
    }

    // Sidecar first: if we stop in between, the `.mov` keeps the orphan
    // discoverable and the next pass finishes the job.
    let sidecar = capture_screen::screen_segment_frame_index_path(path);
    Ok(remove_file_counting(&sidecar)? + remove_file_counting(path)?)
}

/// Total size in bytes of a file, or of everything under a directory.
pub(super) fn path_size_bytes(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| path_size_bytes(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

fn remove_file_counting(path: &Path) -> Result<u64> {
    let size = path_size_bytes(path);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(size),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(AppInfraError::Io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        HiddenSegmentWorkspaceRepair, HiddenSegmentWorkspaceRepairContext,
        SegmentWorkspaceCleanupDisposition,
    };
    use crate::{
        db::{CaptureDb, Database},
        CaptureRetentionStore, NewCaptureSession,
    };
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    struct Fixture {
        root: PathBuf,
        repair: HiddenSegmentWorkspaceRepair,
        retention: CaptureRetentionStore,
        _database: Database,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct Orphan {
        workspace_dir: PathBuf,
        mov: PathBuf,
        sidecar: PathBuf,
    }

    async fn fixture(label: &str) -> Fixture {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("orphaned-segment-{label}-{unique}"));
        fs::create_dir_all(&root).expect("test directory should exist");
        let database = Database::initialize(&root)
            .await
            .expect("database should initialize");
        let db = CaptureDb::single(database.pool().clone());
        Fixture {
            repair: HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(db.clone()),
                crate::ProcessingStore::new(db.clone()),
            ),
            retention: CaptureRetentionStore::new(db),
            root,
            _database: database,
        }
    }

    fn set_age(path: &Path, age: Duration) {
        fs::File::options()
            .write(true)
            .open(path)
            .expect("file should open")
            .set_modified(SystemTime::now() - age)
            .expect("mtime should be settable");
    }

    /// A finalized `.mov` + sidecar and a workspace of frame images the DB
    /// never recorded; images and `.mov` backdated by the given ages.
    fn write_orphan(fixture: &Fixture, frame_age: Duration, mov_age: Duration) -> Orphan {
        let day_dir = fixture.root.join("recordings/2026/07/27");
        let workspace_dir = day_dir.join(".screen_orphan-segment-0001");
        let frames_dir = workspace_dir.join("frames");
        fs::create_dir_all(&frames_dir).expect("frames dir should exist");
        let mov = day_dir.join("screen_orphan-segment-0001.mov");
        let sidecar = day_dir.join("screen_orphan-segment-0001.frame-index.bin");
        fs::write(&mov, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak")
            .expect("visible segment should exist");
        fs::write(&sidecar, b"index").expect("sidecar should exist");
        set_age(&mov, mov_age);
        for name in ["frame-1-000000.jpg", "frame-2-000001.jpg"] {
            let frame = frames_dir.join(name);
            fs::write(&frame, b"jpg").expect("frame artifact should exist");
            set_age(&frame, frame_age);
        }
        Orphan {
            workspace_dir,
            mov,
            sidecar,
        }
    }

    fn cutoff_days_ago(days: u32) -> Option<String> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::days(i64::from(days));
        Some(cutoff.format(&Rfc3339).expect("cutoff should format"))
    }

    async fn repair(
        fixture: &Fixture,
        retention_cutoff: Option<String>,
    ) -> crate::HiddenSegmentWorkspaceRepairResult {
        fixture
            .repair
            .repair_hidden_segment_workspaces_with_context(
                &fixture.root.join("recordings"),
                &HiddenSegmentWorkspaceRepairContext {
                    retention_cutoff,
                    ..Default::default()
                },
            )
            .await
            .expect("repair should succeed")
    }

    async fn disposition(fixture: &Fixture, orphan: &Orphan) -> SegmentWorkspaceCleanupDisposition {
        fixture
            .repair
            .classify_hidden_segment_workspace(&orphan.workspace_dir)
            .await
            .expect("classification should succeed")
            .expect("classification should exist")
            .disposition
    }

    fn run(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    #[test]
    fn stale_orphan_is_reclaimed_and_mov_kept_when_retention_is_never() {
        run(async {
            let fixture = fixture("never").await;
            let orphan = write_orphan(&fixture, 2 * DAY, 90 * DAY);
            assert_eq!(
                disposition(&fixture, &orphan).await,
                SegmentWorkspaceCleanupDisposition::OrphanedFrameArtifacts
            );

            let result = repair(&fixture, None).await;

            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.removed_orphaned_workspace_count, 1);
            assert_eq!(result.removed_orphaned_segment_count, 0);
            assert!(result.reclaimed_bytes > 0);
            assert!(!orphan.workspace_dir.exists(), "orphan images should go");
            assert!(orphan.mov.exists(), "retention Never keeps the .mov");
            assert!(orphan.sidecar.exists());
        });
    }

    #[test]
    fn fresh_unpersisted_frames_stay_pending() {
        run(async {
            let fixture = fixture("fresh").await;
            let orphan = write_orphan(&fixture, Duration::from_secs(60), 90 * DAY);
            assert_eq!(
                disposition(&fixture, &orphan).await,
                SegmentWorkspaceCleanupDisposition::PendingFrameArtifacts
            );

            let result = repair(&fixture, cutoff_days_ago(7)).await;

            assert_eq!(result.removed_workspace_count, 0);
            assert!(orphan.workspace_dir.exists());
            assert!(orphan.mov.exists());
        });
    }

    #[test]
    fn orphan_mov_is_deleted_only_once_past_the_retention_cutoff() {
        run(async {
            let fixture = fixture("cutoff").await;
            let young = write_orphan(&fixture, 2 * DAY, 3 * DAY);
            repair(&fixture, cutoff_days_ago(7)).await;
            assert!(!young.workspace_dir.exists());
            assert!(young.mov.exists(), "younger than the window: kept");
            assert!(young.sidecar.exists());

            let old = write_orphan(&fixture, 2 * DAY, 30 * DAY);
            let result = repair(&fixture, cutoff_days_ago(7)).await;
            assert_eq!(result.removed_orphaned_segment_count, 1);
            assert!(!old.workspace_dir.exists());
            assert!(!old.mov.exists(), "past the window: deleted");
            assert!(!old.sidecar.exists(), "sidecar goes with it");
        });
    }

    #[test]
    fn workspace_with_a_capture_segment_row_is_never_reclaimed() {
        run(async {
            let fixture = fixture("segment-row").await;
            let orphan = write_orphan(&fixture, 2 * DAY, 90 * DAY);
            fixture
                .retention
                .create_capture_session(&NewCaptureSession {
                    capture_session_id: "capture-orphan".to_string(),
                    started_at: "2026-07-27T06:00:00Z".to_string(),
                    requested_screen: true,
                    requested_microphone: false,
                    requested_system_audio: false,
                    screen_source_session_id: Some("screen_orphan".to_string()),
                    microphone_source_session_id: None,
                    system_audio_source_session_id: None,
                    segment_duration_seconds: 300,
                })
                .await
                .expect("capture session should persist");
            fixture
                .retention
                .upsert_screen_segment_for_source_session(
                    "screen_orphan",
                    1,
                    orphan.mov.to_string_lossy().to_string(),
                    orphan.workspace_dir.to_string_lossy().to_string(),
                    orphan
                        .workspace_dir
                        .join("frames")
                        .to_string_lossy()
                        .to_string(),
                    orphan.sidecar.to_string_lossy().to_string(),
                    "2026-07-27T06:00:00Z".to_string(),
                )
                .await
                .expect("segment should persist")
                .expect("segment row should exist");

            assert_eq!(
                disposition(&fixture, &orphan).await,
                SegmentWorkspaceCleanupDisposition::PendingFrameArtifacts
            );
            let result = repair(&fixture, cutoff_days_ago(7)).await;

            assert_eq!(result.removed_workspace_count, 0);
            assert!(orphan.workspace_dir.exists());
            assert!(orphan.mov.exists(), "a .mov with a DB row is never deleted");
            assert!(orphan.sidecar.exists());
        });
    }
}
