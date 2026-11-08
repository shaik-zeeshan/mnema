//! The app log: `<log dir>/rust.log`, rotated at 5 MB keeping 4 archives (5 files),
//! plus stderr. Info and above globally; Debug for our own targets
//! (`app_core::debug_log::APP_LOG_TARGET_PREFIXES` + this crate) in debug builds.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const FILE_NAME: &str = "rust.log";
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const KEPT_ARCHIVES: usize = 4;

/// This crate's own target prefix, on top of the shared list.
const GPUI_TARGET: &str = "mnema_gpui";

struct FileLog {
    dir: PathBuf,
    file: Mutex<Option<(File, u64)>>,
}

fn is_app_target(target: &str) -> bool {
    std::iter::once(&GPUI_TARGET)
        .chain(app_core::debug_log::APP_LOG_TARGET_PREFIXES)
        .any(|prefix| {
            target == *prefix
                || target
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with("::"))
        })
}

fn allowed(metadata: &Metadata) -> bool {
    let app_max = if cfg!(debug_assertions) {
        Level::Debug
    } else {
        Level::Info
    };
    let max = if is_app_target(metadata.target()) {
        app_max
    } else {
        Level::Info
    };
    metadata.level() <= max
}

impl FileLog {
    fn open(dir: &Path) -> Option<(File, u64)> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(FILE_NAME))
            .ok()?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Some((file, len))
    }

    /// rust.log -> rust.1.log -> … -> rust.4.log (the oldest is dropped).
    fn rotate(&self) {
        let archive = |n: usize| self.dir.join(format!("rust.{n}.log"));
        let _ = std::fs::remove_file(archive(KEPT_ARCHIVES));
        for n in (1..KEPT_ARCHIVES).rev() {
            let _ = std::fs::rename(archive(n), archive(n + 1));
        }
        let _ = std::fs::rename(self.dir.join(FILE_NAME), archive(1));
    }
}

impl Log for FileLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        allowed(metadata)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let stamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let line = format!(
            "[{stamp}][{}][{}] {}\n",
            record.level(),
            record.target(),
            record.args()
        );
        eprint!("{line}");
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|(_, len)| *len >= MAX_FILE_BYTES)
        {
            *guard = None;
            self.rotate();
        }
        if guard.is_none() {
            *guard = Self::open(&self.dir);
        }
        if let Some((file, len)) = guard.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                *len += line.len() as u64;
            }
        }
    }

    fn flush(&self) {
        if let Some((file, _)) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = file.flush();
        }
    }
}

/// Install once, before anything logs. A log dir we can't create leaves stderr only.
pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let logger = FileLog {
        dir: dir.to_path_buf(),
        file: Mutex::new(FileLog::open(dir)),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Debug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_targets_match_whole_path_segments() {
        assert!(is_app_target("mnema_gpui"));
        assert!(is_app_target("mnema_gpui::feel"));
        assert!(is_app_target("app_infra::jobs"));
        assert!(is_app_target("app_core"));
        assert!(!is_app_target("mnema_gpuix"));
        assert!(!is_app_target("sqlx::query"));
    }

    #[test]
    fn rotation_keeps_four_archives() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileLog {
            dir: dir.path().into(),
            file: Mutex::new(None),
        };
        for round in 0..6 {
            std::fs::write(dir.path().join(FILE_NAME), format!("{round}")).unwrap();
            log.rotate();
        }
        let names: Vec<_> = (1..=5)
            .map(|n| dir.path().join(format!("rust.{n}.log")).exists())
            .collect();
        assert_eq!(names, [true, true, true, true, false]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("rust.1.log")).unwrap(),
            "5"
        );
    }
}
