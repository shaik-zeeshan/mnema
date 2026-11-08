use std::path::{Path, PathBuf};

use crate::AppCore;

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
/// [`crate::recording_settings::default_save_directory`]
/// (`MNEMA_SAVE_DIRECTORY`, else `~/.mnema`). Infallible by design: every
/// failure mode is one of the three fields, not an error the screen must handle.
/// Never call it on the main thread: every call here is `open`/`unlink`/`statfs`
/// against a directory the *user* picked — an external disk that has to spin up,
/// or a stale SMB/NFS mount under `/Volumes` whose syscalls block for the mount's
/// timeout. That is a whole-app freeze for a readout.
pub fn probe_storage_path(path: Option<String>) -> StorageProbe {
    let requested = path.unwrap_or_default().trim().to_string();
    let resolved = if requested.is_empty() {
        crate::recording_settings::default_save_directory()
    } else {
        requested
    };
    let dir = PathBuf::from(&resolved);
    let exists = dir.is_dir();
    StorageProbe {
        path: resolved,
        exists,
        writable: exists && is_writable(&dir),
        free_bytes: crate::disk_space::measure_free_space(
            &dir,
            crate::disk_space::default_free_space_probe,
        ),
    }
}

/// Settings › Storage meter + status bar size: the cached disk walk plus a
/// fresh free-space read of the volume.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageDto {
    #[serde(flatten)]
    pub usage: ::app_infra::StorageUsage,
    pub free_bytes: Option<u64>,
}

// ponytail: one process-wide cache; the lock is held across the walk so
// concurrent callers wait for one walk instead of starting their own.
static STORAGE_USAGE_CACHE: std::sync::Mutex<
    Option<(std::time::Instant, ::app_infra::StorageUsage)>,
> = std::sync::Mutex::new(None);
const STORAGE_USAGE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// `refresh` forces a new walk (Settings › Storage opening, after a cleanup);
/// otherwise a walk younger than five minutes is reused. Measures the storage
/// root IN EFFECT (the running infra's), not a just-picked pending one.
/// Walks the disk: keep it off the main thread (see `probe_storage_path`).
pub fn get_storage_usage(core: &AppCore, refresh: Option<bool>) -> StorageUsageDto {
    let base_dir = core
        .try_state::<crate::AppInfraState>()
        .map(|infra| infra.base_dir().to_path_buf())
        .unwrap_or_else(|| {
            PathBuf::from(
                crate::recording_settings::load_recording_settings_or_default(core).save_directory,
            )
        });
    let mut cache = STORAGE_USAGE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let usage = match *cache {
        Some((at, usage)) if !refresh.unwrap_or(false) && at.elapsed() < STORAGE_USAGE_TTL => usage,
        _ => {
            let data = &core.paths().config;
            let model_dirs = vec![
                audio_transcription::audio_transcription_models_dir(data),
                speaker_analysis::speaker_analysis_models_dir(data),
                semantic_search::semantic_search_models_dir(data),
                ocr::ocr_models_dir(data),
            ];
            let usage = ::app_infra::measure_storage_usage(&base_dir, &model_dirs);
            *cache = Some((std::time::Instant::now(), usage));
            usage
        }
    };
    drop(cache);
    StorageUsageDto {
        usage,
        free_bytes: crate::disk_space::measure_free_space(
            &base_dir,
            crate::disk_space::default_free_space_probe,
        ),
    }
}

/// Writability is PROVEN, not inferred: mode bits say nothing about ACLs or a
/// read-only mount, so write a probe file and remove it.
/// The name is per-call and the open is `create_new` (`O_CREAT|O_EXCL`) so the
/// probe can only ever create its own file: `File::create` reuses whatever sits
/// at a fixed path, and opening a FIFO or a device node there blocks the caller
/// forever. It also means a probe file left behind by a crash can never make a
/// perfectly writable folder read as unwritable, which would hold the gate shut
/// with no way out.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(
        ".mnema-write-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedStorageLayout {
    base_dir: PathBuf,
}

impl ManagedStorageLayout {
    pub fn from_save_directory(save_directory: &str) -> Self {
        Self {
            base_dir: PathBuf::from(save_directory),
        }
    }

    pub fn from_base_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    pub fn recordings_root(&self) -> PathBuf {
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
            crate::recording_settings::default_save_directory()
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

    #[test]
    fn storage_usage_walks_the_saved_directory_and_model_stores_before_infra_opens() {
        let dir = tempfile::tempdir().expect("temp dir");
        let core = crate::test_support::core_in(Default::default(), dir.path());
        let save = dir.path().join("save");
        std::fs::create_dir_all(save.join("db")).unwrap();
        std::fs::write(save.join("db").join("app.sqlite3"), [0u8; 100]).unwrap();
        let models = ocr::ocr_models_dir(&core.paths().config);
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("model.bin"), [0u8; 40]).unwrap();
        let mut settings = crate::recording_settings::default_recording_settings();
        settings.save_directory = save.display().to_string();
        crate::recording_settings::persist_recording_settings(&core, &settings).unwrap();

        let usage = get_storage_usage(&core, Some(true));

        assert_eq!(usage.usage.index_bytes, 100);
        assert_eq!(usage.usage.models_bytes, 40);
        assert!(usage.free_bytes.is_some());
    }
}
