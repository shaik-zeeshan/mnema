//! Storage usage by source: the Settings › Storage meter and the status bar
//! size. Walks the disk (no size is stored anywhere), so callers cache it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    /// Screen segments, their frame-index sidecars and hidden frame workspaces.
    pub screen_bytes: u64,
    pub microphone_bytes: u64,
    pub system_audio_bytes: u64,
    /// The database directory (`<base>/db`: SQLite + WAL/SHM + key sidecar).
    pub index_bytes: u64,
    pub models_bytes: u64,
    /// Distinct `recordings/YYYY/MM/DD` days holding capture — the
    /// denominator for a per-day rate.
    pub capture_days: u32,
}

/// Sum everything under `<base>/recordings` by source, plus `<base>/db` and
/// each model store directory. Unreadable entries are skipped, never an error:
/// this is a readout.
///
/// Source = the capture id prefix the planner gives every audio file
/// (`mic_…`, `sysaudio_…`, under the day's `audio/`); everything else in the
/// recordings tree is screen.
pub fn measure_storage_usage(base_dir: &Path, model_dirs: &[PathBuf]) -> StorageUsage {
    let mut usage = StorageUsage::default();
    let recordings = base_dir.join("recordings");
    let mut days = HashSet::new();
    walk(&recordings, &mut |path, len| {
        let rel = path.strip_prefix(&recordings).unwrap_or(path);
        let has_prefix = |prefix: &str| {
            rel.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(|s| s.trim_start_matches('.').starts_with(prefix))
            })
        };
        let bucket = if has_prefix("mic_") {
            &mut usage.microphone_bytes
        } else if has_prefix("sysaudio_") {
            &mut usage.system_audio_bytes
        } else {
            &mut usage.screen_bytes
        };
        *bucket += len;
        if rel.components().count() > 3 {
            days.insert(rel.components().take(3).collect::<PathBuf>());
        }
    });
    usage.capture_days = days.len() as u32;
    usage.index_bytes = dir_size(&base_dir.join("db"));
    usage.models_bytes = model_dirs.iter().map(|dir| dir_size(dir)).sum();
    usage
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    walk(dir, &mut |_, len| total += len);
    total
}

/// Depth-first over regular files. `DirEntry::file_type` doesn't follow
/// symlinks, so a link can't loop the walk or count a file twice.
fn walk(dir: &Path, visit: &mut impl FnMut(&Path, u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, visit);
        } else if kind.is_file() {
            if let Ok(meta) = entry.metadata() {
                visit(&path, meta.len());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; len]).unwrap();
    }

    #[test]
    fn sums_each_source_index_and_models() {
        let base = std::env::temp_dir().join(format!(
            "mnema-storage-usage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let day = base.join("recordings/2026/10/11");
        write(&day.join("screen_a-segment-0001.mov"), 100);
        write(&day.join("screen_a-segment-0001.frame-index.bin"), 5);
        write(&day.join(".screen_a-segment-0002/frames/frame-1.jpg"), 20);
        write(&day.join("audio/mic_b-segment-0001.m4a"), 30);
        write(&day.join("audio/sysaudio_c-segment-0001-17.m4a"), 7);
        write(
            &base.join("recordings/2026/10/10/audio/mic_b-segment-0002.m4a"),
            3,
        );
        write(&base.join("db/app.sqlite3"), 40);
        write(&base.join("db/app.sqlite3-wal"), 2);
        let models = base.join("models");
        write(&models.join("whisper/base.bin"), 50);

        let usage = measure_storage_usage(&base, &[models, base.join("missing-models")]);
        std::fs::remove_dir_all(&base).unwrap();

        assert_eq!(
            usage,
            StorageUsage {
                screen_bytes: 125,
                microphone_bytes: 33,
                system_audio_bytes: 7,
                index_bytes: 42,
                models_bytes: 50,
                capture_days: 2,
            }
        );
    }

    #[test]
    fn missing_base_dir_reads_as_empty() {
        let usage = measure_storage_usage(Path::new("/nonexistent/mnema-storage"), &[]);
        assert_eq!(usage, StorageUsage::default());
    }
}
