use std::path::{Path, PathBuf};
const RECORDINGS_DIR_NAME: &str = "recordings";

/// What onboarding's *Capture & Storage* screen measured about a candidate save
/// directory: the flow's only two hard gates read exactly these three fields.
///
/// `free_bytes` is `None` when the volume could not be read. An inability to
/// *measure* never blocks — same discipline as the capture pipeline's low-disk
/// preflight (ADR 0040); only a measured shortfall acts.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProbe {
    /// The path actually probed — the resolved default root when the caller
    /// passed a blank path, so the screen can display what it measured.
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub free_bytes: Option<u64>,
}

/// Probe a candidate save directory. Blank/absent resolves to
/// [`crate::native_capture::settings::default_save_directory`]
/// (`MNEMA_SAVE_DIRECTORY`, else `~/.mnema`). Infallible by design: every
/// failure mode is one of the three fields, not an error the screen must handle.
#[tauri::command]
pub fn probe_storage_path(path: Option<String>) -> StorageProbe {
    let requested = path.unwrap_or_default().trim().to_string();
    let resolved = if requested.is_empty() {
        crate::native_capture::settings::default_save_directory()
    } else {
        requested
    };
    let dir = PathBuf::from(&resolved);
    let exists = dir.is_dir();
    StorageProbe {
        path: resolved,
        exists,
        writable: exists && is_writable(&dir),
        free_bytes: crate::native_capture::disk_space::measure_free_space(
            &dir,
            crate::native_capture::disk_space::default_free_space_probe,
        ),
    }
}

/// Writability is PROVEN, not inferred: mode bits say nothing about ACLs or a
/// read-only mount, so write a probe file and remove it.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".mnema-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedStorageLayout {
    base_dir: PathBuf,
}

impl ManagedStorageLayout {
    pub(crate) fn from_save_directory(save_directory: &str) -> Self {
        Self {
            base_dir: PathBuf::from(save_directory),
        }
    }

    pub(crate) fn from_base_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub(crate) fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    pub(crate) fn recordings_root(&self) -> PathBuf {
        self.base_dir.join(RECORDINGS_DIR_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_storage_layout_uses_save_directory_as_base_dir() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");

        assert_eq!(layout.base_dir(), &PathBuf::from("/tmp/mnema-recordings"));
    }

    #[test]
    fn recordings_root_nests_under_save_directory() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");

        assert_eq!(
            layout.recordings_root(),
            PathBuf::from("/tmp/mnema-recordings").join("recordings")
        );
    }

    #[test]
    fn probe_reports_an_existing_writable_directory_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let probe = probe_storage_path(Some(dir.path().display().to_string()));

        assert_eq!(probe.path, dir.path().display().to_string());
        assert!(probe.exists);
        assert!(probe.writable);
        assert!(probe.free_bytes.is_some(), "a real volume must measure");
        assert!(!dir.path().join(".mnema-write-probe").exists());
    }

    #[test]
    fn probe_reports_a_missing_directory_as_neither_existing_nor_writable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("nope").join("deeper");
        let probe = probe_storage_path(Some(missing.display().to_string()));

        assert!(!probe.exists);
        assert!(!probe.writable);
        // Unmeasurable is not a shortfall: the ancestor walk still finds a volume.
        assert!(probe.free_bytes.is_some());
    }

    #[test]
    fn probe_resolves_a_blank_path_to_the_default_save_directory() {
        let probe = probe_storage_path(Some("  ".to_string()));

        assert_eq!(
            probe.path,
            crate::native_capture::settings::default_save_directory()
        );
        assert_eq!(probe_storage_path(None).path, probe.path);
    }

    #[test]
    fn recordings_root_is_child_of_base_dir() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");
        let base_dir = layout.base_dir().clone();
        let recordings_root = layout.recordings_root();

        assert_eq!(recordings_root.parent(), Some(base_dir.as_path()));
    }
}
