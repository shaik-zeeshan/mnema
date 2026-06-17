use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    frame_batch_store::{FrameBatchStatus, FrameBatchStore, SegmentWorkspaceFrameBatchReferences},
    processing::{ProcessingStore, SegmentWorkspaceOcrReference},
    AppInfraError, Result,
};

fn visible_segment_appears_openable(path: &Path) -> bool {
    const SEARCH_WINDOW_BYTES: u64 = 256 * 1024;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let file_len = metadata.len();
    if file_len < 8 {
        return false;
    }

    let prefix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    let mut prefix = vec![0_u8; prefix_len];
    if std::io::Read::read_exact(&mut file, &mut prefix).is_err() {
        return false;
    }
    if prefix.windows(4).any(|window| window == b"moov") {
        return true;
    }

    if file_len <= SEARCH_WINDOW_BYTES {
        return false;
    }

    let suffix_len = file_len.min(SEARCH_WINDOW_BYTES) as usize;
    if std::io::Seek::seek(&mut file, std::io::SeekFrom::End(-(suffix_len as i64))).is_err() {
        return false;
    }
    let mut suffix = vec![0_u8; suffix_len];
    if std::io::Read::read_exact(&mut file, &mut suffix).is_err() {
        return false;
    }

    suffix.windows(4).any(|window| window == b"moov")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HiddenSegmentWorkspacePaths {
    pub workspace_dir: String,
    pub frames_dir: String,
    pub visible_segment_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentWorkspaceCleanupDisposition {
    ReferencedByIncompleteBatch,
    ReferencedByNonterminalOcr,
    MissingVisibleSegmentSibling,
    /// The visible recording is permanently dead (missing or never finalized —
    /// no `moov` atom, e.g. a segment abandoned when a display went unavailable
    /// mid-capture, see ADR 0021) AND the frame artifacts have already been
    /// consumed from disk. Nothing remains to preserve, so the husk workspace is
    /// safe to reclaim instead of being skipped forever.
    DeadSegmentWithoutArtifacts,
    PendingFrameArtifacts,
    CompletedOnly,
    NoReferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SegmentWorkspaceCleanupDebugInfo {
    pub paths: HiddenSegmentWorkspacePaths,
    pub disposition: SegmentWorkspaceCleanupDisposition,
    pub safe_to_remove: bool,
    pub visible_segment_exists: bool,
    pub frame_count: i64,
    pub batch_references: Vec<crate::SegmentWorkspaceBatchReference>,
    pub nonterminal_ocr_references: Vec<SegmentWorkspaceOcrReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HiddenSegmentWorkspaceRepairResult {
    pub scanned_workspace_count: u64,
    pub removed_workspace_count: u64,
    pub skipped_workspace_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenSegmentWorkspaceRepairContext {
    pub active_workspace_dirs: BTreeSet<String>,
}

#[derive(Clone)]
pub struct HiddenSegmentWorkspaceRepair {
    frame_batches: FrameBatchStore,
    processing: ProcessingStore,
}

impl HiddenSegmentWorkspacePaths {
    pub fn from_workspace_dir(workspace_dir: &Path) -> Option<Self> {
        let workspace_name = workspace_dir.file_name()?.to_str()?;
        if !workspace_name.starts_with('.') || !workspace_name.contains("-segment-") {
            return None;
        }

        let visible_segment_name = workspace_name.strip_prefix('.')?;
        let frames_dir = workspace_dir.join("frames");
        let visible_segment_path = workspace_dir
            .parent()?
            .join(format!("{visible_segment_name}.mov"));

        Some(Self {
            workspace_dir: workspace_dir.to_string_lossy().to_string(),
            frames_dir: frames_dir.to_string_lossy().to_string(),
            visible_segment_path: visible_segment_path.to_string_lossy().to_string(),
        })
    }

    pub fn from_frame_artifact_path(path: &Path) -> Option<Self> {
        let frames_dir = path.parent()?;
        if frames_dir.file_name()?.to_str()? != "frames" {
            return None;
        }

        let workspace_dir = frames_dir.parent()?;
        Self::from_workspace_dir(workspace_dir)
    }
}

impl HiddenSegmentWorkspaceRepair {
    pub(crate) fn new(frame_batches: FrameBatchStore, processing: ProcessingStore) -> Self {
        Self {
            frame_batches,
            processing,
        }
    }

    pub async fn classify_hidden_segment_workspace(
        &self,
        workspace_dir: &Path,
    ) -> Result<Option<SegmentWorkspaceCleanupDebugInfo>> {
        let Some(paths) = HiddenSegmentWorkspacePaths::from_workspace_dir(workspace_dir) else {
            return Ok(None);
        };

        let workspace_prefix = format!("{}/", paths.workspace_dir);
        let SegmentWorkspaceFrameBatchReferences {
            frame_count,
            batch_references,
        } = self
            .frame_batches
            .list_frame_batch_references_for_workspace(&workspace_prefix)
            .await?;
        let nonterminal_ocr_references = self
            .processing
            .list_nonterminal_ocr_references_for_workspace(&workspace_prefix)
            .await?;
        let visible_segment_exists = Path::new(&paths.visible_segment_path).exists();
        let visible_segment_usable = visible_segment_exists
            && visible_segment_appears_openable(Path::new(&paths.visible_segment_path));

        let disposition = if frame_count == 0 {
            let has_frame_artifacts =
                hidden_workspace_has_frame_artifacts(Path::new(&paths.frames_dir));
            if has_frame_artifacts {
                if visible_segment_exists {
                    SegmentWorkspaceCleanupDisposition::PendingFrameArtifacts
                } else {
                    SegmentWorkspaceCleanupDisposition::MissingVisibleSegmentSibling
                }
            } else {
                SegmentWorkspaceCleanupDisposition::NoReferences
            }
        } else if !nonterminal_ocr_references.is_empty() {
            SegmentWorkspaceCleanupDisposition::ReferencedByNonterminalOcr
        } else if batch_references.iter().any(|reference| {
            !matches!(
                reference.status,
                FrameBatchStatus::Completed | FrameBatchStatus::Failed
            )
        }) {
            SegmentWorkspaceCleanupDisposition::ReferencedByIncompleteBatch
        } else if !visible_segment_usable {
            // Dead visible recording (missing or never finalized). Keep the
            // workspace only while its frame artifacts still exist on disk as the
            // sole surviving record; once they've been consumed (all batches and
            // OCR here are already terminal) there is nothing left to protect, so
            // reclaim the husk instead of skipping it forever.
            if hidden_workspace_has_frame_artifacts(Path::new(&paths.frames_dir)) {
                SegmentWorkspaceCleanupDisposition::MissingVisibleSegmentSibling
            } else {
                SegmentWorkspaceCleanupDisposition::DeadSegmentWithoutArtifacts
            }
        } else if !batch_references.is_empty() {
            SegmentWorkspaceCleanupDisposition::CompletedOnly
        } else {
            SegmentWorkspaceCleanupDisposition::NoReferences
        };

        Ok(Some(SegmentWorkspaceCleanupDebugInfo {
            paths,
            safe_to_remove: matches!(
                disposition,
                SegmentWorkspaceCleanupDisposition::CompletedOnly
                    | SegmentWorkspaceCleanupDisposition::NoReferences
                    | SegmentWorkspaceCleanupDisposition::DeadSegmentWithoutArtifacts
            ),
            disposition,
            visible_segment_exists,
            frame_count,
            batch_references,
            nonterminal_ocr_references,
        }))
    }

    pub async fn repair_hidden_segment_workspaces_with_context(
        &self,
        recordings_root: &Path,
        context: &HiddenSegmentWorkspaceRepairContext,
    ) -> Result<HiddenSegmentWorkspaceRepairResult> {
        let workspace_dirs = collect_hidden_segment_workspace_dirs(recordings_root)?;

        let mut result = HiddenSegmentWorkspaceRepairResult {
            scanned_workspace_count: workspace_dirs.len() as u64,
            ..HiddenSegmentWorkspaceRepairResult::default()
        };

        for workspace_dir in workspace_dirs {
            let Some(paths) = HiddenSegmentWorkspacePaths::from_workspace_dir(&workspace_dir)
            else {
                continue;
            };

            if matches_active_workspace(&paths, &context.active_workspace_dirs) {
                capture_runtime::debug_log!(
                    "[app-infra][hidden-segment-workspaces] skipped active workspace {}",
                    paths.workspace_dir
                );
                result.skipped_workspace_count += 1;
                continue;
            }

            let Some(info) = self
                .classify_hidden_segment_workspace(&workspace_dir)
                .await?
            else {
                continue;
            };

            if info.safe_to_remove {
                // A reclaimed dead-segment husk leaves its truncated, unplayable
                // `.mov` sibling behind. Delete it too, otherwise removing the
                // workspace dir below makes the husk undetectable on the next pass
                // and the dead `.mov` leaks forever. Gated strictly to
                // DeadSegmentWithoutArtifacts: the only safe-to-remove disposition
                // whose visible segment was classified as permanently dead. Other
                // safe dispositions (CompletedOnly / NoReferences) may have a real,
                // openable recording that must never be touched.
                if matches!(
                    info.disposition,
                    SegmentWorkspaceCleanupDisposition::DeadSegmentWithoutArtifacts
                ) {
                    reclaim_dead_visible_segment(&info.paths.visible_segment_path)?;
                }
                match std::fs::remove_dir_all(&workspace_dir) {
                    Ok(()) => {
                        capture_runtime::debug_log!(
                            "[app-infra][hidden-segment-workspaces] removed workspace {} (disposition={:?}, frame_count={}, visible_segment_exists={})",
                            info.paths.workspace_dir,
                            info.disposition,
                            info.frame_count,
                            info.visible_segment_exists
                        );
                        result.removed_workspace_count += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        capture_runtime::debug_log!(
                            "[app-infra][hidden-segment-workspaces] treated missing workspace as removed {} (disposition={:?}, frame_count={}, visible_segment_exists={})",
                            info.paths.workspace_dir,
                            info.disposition,
                            info.frame_count,
                            info.visible_segment_exists
                        );
                        result.removed_workspace_count += 1;
                    }
                    Err(error) => return Err(AppInfraError::Io(error)),
                }
            } else {
                capture_runtime::debug_log!(
                    "[app-infra][hidden-segment-workspaces] skipped workspace {} (disposition={:?}, frame_count={}, visible_segment_exists={})",
                    info.paths.workspace_dir,
                    info.disposition,
                    info.frame_count,
                    info.visible_segment_exists
                );
                result.skipped_workspace_count += 1;
            }
        }

        Ok(result)
    }
}

/// Delete the dead, never-finalized `.mov` sibling of a reclaimable husk.
///
/// Only ever called for [`SegmentWorkspaceCleanupDisposition::DeadSegmentWithoutArtifacts`],
/// where the visible recording was already classified as permanently dead
/// (missing or never finalized — no `moov` atom) and the frame artifacts are
/// gone, so there is nothing left to preserve. The file is re-confirmed
/// un-openable immediately before unlinking — defense in depth, so that a
/// recording somehow finalized between classification and now is left intact
/// rather than destroyed. A missing file reads as not-openable and the unlink
/// resolves to a no-op `NotFound`.
///
/// Removing the `.mov` *before* the workspace dir keeps reclamation crash-safe:
/// if we stop in between, the dir survives and the husk is re-detected and
/// reclaimed on the next pass.
fn reclaim_dead_visible_segment(visible_segment_path: &str) -> Result<()> {
    let path = Path::new(visible_segment_path);

    if visible_segment_appears_openable(path) {
        capture_runtime::debug_log!(
            "[app-infra][hidden-segment-workspaces] preserved now-openable visible segment {} (skipped husk .mov deletion)",
            visible_segment_path
        );
        return Ok(());
    }

    match std::fs::remove_file(path) {
        Ok(()) => {
            capture_runtime::debug_log!(
                "[app-infra][hidden-segment-workspaces] removed dead visible segment {}",
                visible_segment_path
            );
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppInfraError::Io(error)),
    }
}

fn collect_hidden_segment_workspace_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut workspace_dirs = Vec::new();
    collect_hidden_segment_workspace_dirs_inner(root, &mut workspace_dirs)?;
    Ok(workspace_dirs)
}

fn collect_hidden_segment_workspace_dirs_inner(
    root: &Path,
    workspace_dirs: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if HiddenSegmentWorkspacePaths::from_workspace_dir(&path).is_some() {
            workspace_dirs.push(path);
            continue;
        }

        collect_hidden_segment_workspace_dirs_inner(&path, workspace_dirs)?;
    }

    Ok(())
}

fn matches_active_workspace(
    paths: &HiddenSegmentWorkspacePaths,
    active_workspace_dirs: &BTreeSet<String>,
) -> bool {
    active_workspace_dirs.contains(&paths.workspace_dir)
}

fn hidden_workspace_has_frame_artifacts(frames_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(frames_dir) else {
        return false;
    };

    entries.flatten().any(|entry| {
        let path = entry.path();
        is_safe_frame_artifact_path(&path)
    })
}

fn is_safe_frame_artifact_path(path: &Path) -> bool {
    if !path.is_file() || !path.is_absolute() {
        return false;
    }
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return false;
    }
    let frames_dir = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    let parent_is_frames = frames_dir.file_name().is_some_and(|name| name == "frames");
    if !parent_is_frames {
        return false;
    }
    let segment_dir_name = frames_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if !segment_dir_name.contains("-segment-") {
        return false;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    file_name.starts_with("frame-")
        && [".png", ".jpg", ".jpeg"]
            .into_iter()
            .any(|ext| file_name.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Database, processing::NewFrame, ProcessingJobDraft, ProcessingJobStatus};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("hidden-segment-workspace-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should exist");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_async_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(test);
    }

    fn write_openable_visible_segment(path: &Path) {
        fs::write(path, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10moovtrak")
            .expect("visible segment should exist");
    }

    #[test]
    fn hidden_segment_workspace_paths_resolve_visible_segment_path() {
        let frame_path = PathBuf::from(
            "/tmp/2026/04/12/.session-abc-segment-0004/frames/frame-1744459200123-7.png",
        );

        let paths = HiddenSegmentWorkspacePaths::from_frame_artifact_path(&frame_path)
            .expect("hidden workspace paths should resolve");

        assert_eq!(
            paths.workspace_dir,
            "/tmp/2026/04/12/.session-abc-segment-0004"
        );
        assert_eq!(
            paths.frames_dir,
            "/tmp/2026/04/12/.session-abc-segment-0004/frames"
        );
        assert_eq!(
            paths.visible_segment_path,
            "/tmp/2026/04/12/session-abc-segment-0004.mov"
        );
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_missing_visible_segment() {
        run_async_test(async {
            let dir = TestDir::new("classify-missing-visible");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store, processing.clone());

            let workspace_dir = dir.path().join("2026/04/12/.session-preview-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            let frame_path = frames_dir.join("frame-1.png");
            // The frame artifact still on disk is the only surviving record of the
            // dead segment, so the workspace must be preserved.
            fs::write(&frame_path, b"png").expect("frame artifact should exist");

            processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::MissingVisibleSegmentSibling
            );
            assert!(!info.safe_to_remove);
            assert!(!info.visible_segment_exists);
            assert_eq!(info.frame_count, 1);
            assert!(info.batch_references.is_empty());
            assert!(info.nonterminal_ocr_references.is_empty());
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reclaims_dead_segment_when_artifacts_are_consumed() {
        run_async_test(async {
            let dir = TestDir::new("classify-dead-segment-husk");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store, processing.clone());

            // Dead segment (no visible .mov) whose frame artifacts have already
            // been consumed: the frames/ dir exists but holds no frame images, and
            // a DB frame row lingers pointing at the deleted artifact. There is
            // nothing left to protect, so the husk should be reclaimable.
            let workspace_dir = dir.path().join("2026/04/12/.session-preview-segment-0009");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            let frame_path = frames_dir.join("frame-1.png");
            processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::DeadSegmentWithoutArtifacts
            );
            assert!(info.safe_to_remove);
            assert!(!info.visible_segment_exists);
            assert_eq!(info.frame_count, 1);
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_empty_missing_visible_as_no_references() {
        run_async_test(async {
            let dir = TestDir::new("classify-empty-missing-visible");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                crate::ProcessingStore::new(pool),
            );

            let workspace_dir = dir.path().join("2026/04/12/.session-empty-segment-0001");
            fs::create_dir_all(workspace_dir.join("frames")).expect("frames dir should exist");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::NoReferences
            );
            assert!(info.safe_to_remove);
            assert!(!info.visible_segment_exists);
            assert_eq!(info.frame_count, 0);
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_visible_segment_with_live_frame_artifacts_as_pending(
    ) {
        run_async_test(async {
            let dir = TestDir::new("classify-pending-frame-artifacts");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                crate::ProcessingStore::new(pool),
            );

            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-live-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            write_openable_visible_segment(&segment_dir.join("session-live-segment-0001.mov"));
            fs::write(frames_dir.join("frame-1.jpg"), b"jpg").expect("frame artifact should exist");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::PendingFrameArtifacts
            );
            assert!(!info.safe_to_remove);
            assert!(info.visible_segment_exists);
            assert_eq!(info.frame_count, 0);
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_incomplete_batch_before_missing_video() {
        run_async_test(async {
            let dir = TestDir::new("classify-incomplete-batch");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store.clone(), processing.clone());

            let workspace_dir = dir.path().join("2026/04/12/.session-preview-segment-0002");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            let frame_path = frames_dir.join("frame-1.png");

            let batch = store
                .upsert_open_batch_for_frame("session-preview", "2026-04-12T10:00:00Z")
                .await
                .expect("batch should persist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::ReferencedByIncompleteBatch
            );
            assert!(!info.safe_to_remove);
            assert_eq!(info.batch_references.len(), 1);
            assert_eq!(info.batch_references[0].batch_id, batch.id);
            assert_eq!(info.batch_references[0].status, FrameBatchStatus::Open);
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_nonterminal_ocr_before_completed_only() {
        run_async_test(async {
            let dir = TestDir::new("classify-nonterminal-ocr");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store.clone(), processing.clone());

            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0003");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            write_openable_visible_segment(&segment_dir.join("session-preview-segment-0003.mov"));
            let frame_path = frames_dir.join("frame-1.png");

            let batch = store
                .upsert_open_batch_for_frame("session-preview", "2026-04-12T10:00:00Z")
                .await
                .expect("batch should persist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            store
                .close_completed_batches_for_session("session-preview", None)
                .await
                .expect("batch should close");
            processing
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should queue");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::ReferencedByNonterminalOcr
            );
            assert!(!info.safe_to_remove);
            assert_eq!(info.batch_references[0].status, FrameBatchStatus::Closed);
            assert_eq!(info.nonterminal_ocr_references.len(), 1);
            assert_eq!(
                info.nonterminal_ocr_references[0].status,
                ProcessingJobStatus::Queued
            );
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_reports_completed_only_as_safe_to_remove() {
        run_async_test(async {
            let dir = TestDir::new("classify-completed-only");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store.clone(), processing.clone());

            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0004");
            let frames_dir = workspace_dir.join("frames");
            fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            write_openable_visible_segment(&segment_dir.join("session-preview-segment-0004.mov"));
            let frame_path = frames_dir.join("frame-1.png");

            let batch = store
                .upsert_open_batch_for_frame("session-preview", "2026-04-12T10:00:00Z")
                .await
                .expect("batch should persist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            let job = processing
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should queue");
            let claimed = processing
                .claim_queued_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(job.id, Some("terminal for cleanup"))
                .await
                .expect("job should be terminal");
            store
                .close_completed_batches_for_session("session-preview", None)
                .await
                .expect("batch should close");
            store
                .mark_batch_processing(batch.id)
                .await
                .expect("batch should mark processing");
            store
                .mark_batch_completed(batch.id, None)
                .await
                .expect("batch should complete");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::CompletedOnly
            );
            assert!(info.safe_to_remove);
            assert!(info.visible_segment_exists);
            assert!(info.nonterminal_ocr_references.is_empty());
            assert_eq!(info.batch_references[0].status, FrameBatchStatus::Completed);
        });
    }

    #[test]
    fn classify_hidden_segment_workspace_preserves_completed_workspace_when_visible_segment_is_invalid(
    ) {
        run_async_test(async {
            let dir = TestDir::new("classify-invalid-visible-segment");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store.clone(), processing.clone());

            let segment_dir = dir.path().join("2026/04/12");
            let workspace_dir = segment_dir.join(".session-preview-segment-0005");
            let frames_dir = workspace_dir.join("frames");
            std::fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            std::fs::write(
                segment_dir.join("session-preview-segment-0005.mov"),
                b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk",
            )
            .expect("invalid visible segment should exist");
            let frame_path = frames_dir.join("frame-1.png");
            // Frame artifact still on disk: the dead-video workspace is preserved.
            std::fs::write(&frame_path, b"png").expect("frame artifact should exist");

            let batch = store
                .upsert_open_batch_for_frame("session-preview", "2026-04-12T10:00:00Z")
                .await
                .expect("batch should persist");
            let frame = processing
                .insert_frame(&NewFrame::new(
                    "session-preview",
                    frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");
            store
                .attach_frame_to_batch(frame.id, batch.id, &frame.captured_at)
                .await
                .expect("frame should attach");
            let job = processing
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("ocr job should queue");
            let claimed = processing
                .claim_queued_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            assert_eq!(claimed.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(job.id, Some("terminal for cleanup"))
                .await
                .expect("job should be terminal");
            store
                .close_completed_batches_for_session("session-preview", None)
                .await
                .expect("batch should close");
            store
                .mark_batch_processing(batch.id)
                .await
                .expect("batch should mark processing");
            store
                .mark_batch_completed(batch.id, None)
                .await
                .expect("batch should complete");

            let info = repair
                .classify_hidden_segment_workspace(&workspace_dir)
                .await
                .expect("classification should succeed")
                .expect("classification should exist");

            assert_eq!(
                info.disposition,
                SegmentWorkspaceCleanupDisposition::MissingVisibleSegmentSibling
            );
            assert!(!info.safe_to_remove);
            assert!(info.visible_segment_exists);
            assert!(info.nonterminal_ocr_references.is_empty());
            assert_eq!(info.batch_references[0].status, FrameBatchStatus::Completed);
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_removes_only_safe_workspaces() {
        run_async_test(async {
            let dir = TestDir::new("repair-safe-workspaces");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let store = crate::FrameBatchStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(store.clone(), processing.clone());
            let recordings_root = dir.path().join("recordings");
            let day_dir = recordings_root.join("2026/04/12");

            let safe_workspace_dir = day_dir.join(".session-safe-segment-0001");
            let safe_frames_dir = safe_workspace_dir.join("frames");
            std::fs::create_dir_all(&safe_frames_dir).expect("safe frames dir should exist");
            write_openable_visible_segment(&day_dir.join("session-safe-segment-0001.mov"));
            let safe_frame_path = safe_frames_dir.join("frame-1.jpg");
            std::fs::write(&safe_frame_path, b"jpg").expect("safe frame should exist");

            let safe_batch = store
                .upsert_open_batch_for_frame("session-safe", "2026-04-12T10:00:00Z")
                .await
                .expect("safe batch should persist");
            let safe_frame = processing
                .insert_frame(&NewFrame::new(
                    "session-safe",
                    safe_frame_path.to_string_lossy().to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("safe frame should persist");
            store
                .attach_frame_to_batch(safe_frame.id, safe_batch.id, &safe_frame.captured_at)
                .await
                .expect("safe frame should attach");
            let safe_job = processing
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(safe_frame.id))
                .await
                .expect("safe ocr job should queue");
            let claimed_safe_job = processing
                .claim_queued_job(safe_job.id)
                .await
                .expect("safe job should claim")
                .expect("safe job should exist");
            assert_eq!(claimed_safe_job.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(safe_job.id, Some("terminal for repair"))
                .await
                .expect("safe job should be terminal");
            store
                .close_completed_batches_for_session("session-safe", None)
                .await
                .expect("safe batch should close");
            store
                .mark_batch_processing(safe_batch.id)
                .await
                .expect("safe batch should mark processing");
            store
                .mark_batch_completed(safe_batch.id, None)
                .await
                .expect("safe batch should complete");

            let skipped_workspace_dir = day_dir.join(".session-skip-segment-0001");
            let skipped_frames_dir = skipped_workspace_dir.join("frames");
            std::fs::create_dir_all(&skipped_frames_dir).expect("skipped frames dir should exist");
            let skipped_frame_path = skipped_frames_dir.join("frame-1.jpg");
            std::fs::write(&skipped_frame_path, b"jpg").expect("skipped frame should exist");

            let skipped_batch = store
                .upsert_open_batch_for_frame("session-skip", "2026-04-12T11:00:00Z")
                .await
                .expect("skipped batch should persist");
            let skipped_frame = processing
                .insert_frame(&NewFrame::new(
                    "session-skip",
                    skipped_frame_path.to_string_lossy().to_string(),
                    "2026-04-12T11:00:00Z",
                ))
                .await
                .expect("skipped frame should persist");
            store
                .attach_frame_to_batch(
                    skipped_frame.id,
                    skipped_batch.id,
                    &skipped_frame.captured_at,
                )
                .await
                .expect("skipped frame should attach");
            let skipped_job = processing
                .enqueue_job(&ProcessingJobDraft::for_frame_ocr(skipped_frame.id))
                .await
                .expect("skipped ocr job should queue");
            let claimed_skipped_job = processing
                .claim_queued_job(skipped_job.id)
                .await
                .expect("skipped job should claim")
                .expect("skipped job should exist");
            assert_eq!(claimed_skipped_job.status, ProcessingJobStatus::Running);
            processing
                .mark_job_failed(skipped_job.id, Some("terminal for repair"))
                .await
                .expect("skipped job should be terminal");
            store
                .close_completed_batches_for_session("session-skip", None)
                .await
                .expect("skipped batch should close");
            store
                .mark_batch_processing(skipped_batch.id)
                .await
                .expect("skipped batch should mark processing");
            store
                .mark_batch_completed(skipped_batch.id, None)
                .await
                .expect("skipped batch should complete");

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext::default(),
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 2);
            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.skipped_workspace_count, 1);
            assert!(
                !safe_workspace_dir.exists(),
                "safe workspace should be removed"
            );
            assert!(
                skipped_workspace_dir.exists(),
                "workspace without visible segment should be preserved"
            );
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_removes_empty_missing_visible_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-empty-missing-visible");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                crate::ProcessingStore::new(pool),
            );
            let recordings_root = dir.path().join("recordings");
            let workspace_dir = recordings_root.join("2026/04/12/.session-empty-segment-0001");

            std::fs::create_dir_all(workspace_dir.join("frames"))
                .expect("empty frames dir should exist");

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext::default(),
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.skipped_workspace_count, 0);
            assert!(
                !workspace_dir.exists(),
                "empty workspace without visible segment should be removed"
            );
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_reclaims_dead_segment_husk() {
        run_async_test(async {
            let dir = TestDir::new("repair-dead-segment-husk");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                processing.clone(),
            );
            let recordings_root = dir.path().join("recordings");

            // Dead segment: no visible .mov, a lingering DB frame row, and an empty
            // frames/ dir (artifacts already consumed). Nothing left to protect, so
            // the periodic repair should reclaim the husk rather than skip it.
            let workspace_dir = recordings_root.join("2026/04/12/.session-dead-segment-0001");
            let frames_dir = workspace_dir.join("frames");
            std::fs::create_dir_all(&frames_dir).expect("frames dir should exist");
            processing
                .insert_frame(&NewFrame::new(
                    "session-dead",
                    frames_dir
                        .join("frame-1.png")
                        .to_string_lossy()
                        .to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext::default(),
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.skipped_workspace_count, 0);
            assert!(
                !workspace_dir.exists(),
                "dead-segment husk with consumed artifacts should be reclaimed"
            );
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_deletes_dead_visible_segment_sibling() {
        run_async_test(async {
            let dir = TestDir::new("repair-dead-segment-mov");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let processing = crate::ProcessingStore::new(pool.clone());
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                processing.clone(),
            );
            let recordings_root = dir.path().join("recordings");
            let day_dir = recordings_root.join("2026/04/12");

            // Dead segment: a truncated, never-finalized .mov (no moov atom), an
            // empty frames/ dir (artifacts already consumed), and a lingering DB
            // frame row. Reclaiming the husk must take the dead .mov sibling with
            // it, otherwise the file leaks once the workspace dir disappears.
            let workspace_dir = day_dir.join(".session-dead-segment-0001");
            std::fs::create_dir_all(workspace_dir.join("frames"))
                .expect("frames dir should exist");
            let dead_mov = day_dir.join("session-dead-segment-0001.mov");
            std::fs::write(&dead_mov, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk")
                .expect("dead visible segment should exist");
            processing
                .insert_frame(&NewFrame::new(
                    "session-dead",
                    workspace_dir
                        .join("frames/frame-1.png")
                        .to_string_lossy()
                        .to_string(),
                    "2026-04-12T10:00:00Z",
                ))
                .await
                .expect("frame should persist");

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext::default(),
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.removed_workspace_count, 1);
            assert!(!workspace_dir.exists(), "husk dir should be reclaimed");
            assert!(
                !dead_mov.exists(),
                "dead .mov sibling should be deleted alongside the husk"
            );
        });
    }

    #[test]
    fn reclaim_dead_visible_segment_removes_unopenable_file() {
        let dir = TestDir::new("reclaim-dead-mov");
        let mov = dir.path().join("session-dead-segment-0001.mov");
        std::fs::write(&mov, b"\0\0\0\x14ftypqt  \0\0\0\0qt  \0\0\0\x10mdatjunk")
            .expect("dead visible segment should exist");

        reclaim_dead_visible_segment(&mov.to_string_lossy())
            .expect("reclaiming a dead segment should succeed");

        assert!(!mov.exists(), "an unopenable .mov should be removed");
    }

    #[test]
    fn reclaim_dead_visible_segment_preserves_openable_file() {
        let dir = TestDir::new("reclaim-openable-mov");
        let mov = dir.path().join("session-live-segment-0001.mov");
        write_openable_visible_segment(&mov);

        reclaim_dead_visible_segment(&mov.to_string_lossy())
            .expect("reclaim should succeed without touching an openable recording");

        assert!(
            mov.exists(),
            "a now-openable recording must never be deleted (defense in depth)"
        );
    }

    #[test]
    fn reclaim_dead_visible_segment_is_noop_when_missing() {
        let dir = TestDir::new("reclaim-missing-mov");
        let mov = dir.path().join("session-gone-segment-0001.mov");

        reclaim_dead_visible_segment(&mov.to_string_lossy())
            .expect("reclaiming a missing segment should be a no-op");

        assert!(!mov.exists());
    }

    #[test]
    fn repair_hidden_segment_workspaces_with_context_skips_active_screen_session_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-active-session-skip");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                crate::ProcessingStore::new(pool),
            );
            let recordings_root = dir.path().join("recordings");
            let day_dir = recordings_root.join("2026/04/12");
            let workspace_dir = day_dir.join(".active-screen-session-segment-0001");

            std::fs::create_dir_all(workspace_dir.join("frames"))
                .expect("active frames dir should exist");
            write_openable_visible_segment(&day_dir.join("active-screen-session-segment-0001.mov"));

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext {
                        active_workspace_dirs: BTreeSet::from([workspace_dir
                            .to_string_lossy()
                            .to_string()]),
                    },
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 1);
            assert_eq!(result.removed_workspace_count, 0);
            assert_eq!(result.skipped_workspace_count, 1);
            assert!(
                workspace_dir.exists(),
                "active workspace should be preserved"
            );
        });
    }

    #[test]
    fn repair_hidden_segment_workspaces_with_context_skips_only_current_active_segment_workspace() {
        run_async_test(async {
            let dir = TestDir::new("repair-active-session-current-only");
            let database = Database::initialize(dir.path())
                .await
                .expect("database should initialize");
            let pool = database.pool().clone();
            let repair = HiddenSegmentWorkspaceRepair::new(
                crate::FrameBatchStore::new(pool.clone()),
                crate::ProcessingStore::new(pool),
            );
            let recordings_root = dir.path().join("recordings");
            let day_dir = recordings_root.join("2026/04/12");
            let old_workspace_dir = day_dir.join(".active-screen-session-segment-0001");
            let current_workspace_dir = day_dir.join(".active-screen-session-segment-0002");

            std::fs::create_dir_all(old_workspace_dir.join("frames"))
                .expect("old frames dir should exist");
            std::fs::create_dir_all(current_workspace_dir.join("frames"))
                .expect("current frames dir should exist");
            write_openable_visible_segment(&day_dir.join("active-screen-session-segment-0001.mov"));
            write_openable_visible_segment(&day_dir.join("active-screen-session-segment-0002.mov"));

            let result = repair
                .repair_hidden_segment_workspaces_with_context(
                    &recordings_root,
                    &HiddenSegmentWorkspaceRepairContext {
                        active_workspace_dirs: BTreeSet::from([current_workspace_dir
                            .to_string_lossy()
                            .to_string()]),
                    },
                )
                .await
                .expect("repair should succeed");

            assert_eq!(result.scanned_workspace_count, 2);
            assert_eq!(result.removed_workspace_count, 1);
            assert_eq!(result.skipped_workspace_count, 1);
            assert!(
                !old_workspace_dir.exists(),
                "old safe workspace from the same screen session should be removed"
            );
            assert!(
                current_workspace_dir.exists(),
                "current active workspace should be preserved"
            );
        });
    }
}
