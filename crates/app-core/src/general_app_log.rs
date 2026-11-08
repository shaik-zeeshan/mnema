use capture_types::{CaptureErrorResponse, GeneralAppLogStatus};
use std::path::{Path, PathBuf};

use crate::AppCore;

fn general_app_log_path(core: &AppCore) -> Result<PathBuf, CaptureErrorResponse> {
    Ok(core
        .paths()
        .log
        .join(crate::debug_log::APP_LOG_FILE_NAME)
        .with_extension("log"))
}

fn status_for_path(path: &Path) -> GeneralAppLogStatus {
    // `metadata` on a missing file errors, so a missing log is `None`, not 0.
    let metadata = std::fs::metadata(path).ok().filter(|m| m.is_file());
    GeneralAppLogStatus {
        path: path.to_string_lossy().to_string(),
        exists: metadata.is_some(),
        size_bytes: metadata.map(|m| m.len()),
    }
}

fn open_target_path(path: &Path, exists: bool) -> Result<PathBuf, CaptureErrorResponse> {
    if exists {
        return Ok(path.to_path_buf());
    }

    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CaptureErrorResponse {
            code: "app_log_path_unavailable".to_string(),
            message: format!(
                "Failed to resolve containing directory for general application log path '{}'",
                path.display()
            ),
        })
}

fn delete_log_file_at_path(path: &Path) -> Result<(), CaptureErrorResponse> {
    let to_error = |error: std::io::Error| CaptureErrorResponse {
        code: "app_log_delete_failed".to_string(),
        message: format!("Failed to delete general application log: {error}"),
    };
    // Truncate, don't remove: the log plugin holds `rust.log` open in append
    // mode and only reopens it on rotation, so a removed file would swallow
    // every line until the next 5 MB. Append writes land at the new end.
    // ponytail: the plugin's size counter isn't reset, so its next rotation
    // comes early; harmless.
    match std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
    {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(to_error(error)),
        _ => {}
    }
    // The log plugin rotates into `rust_<date>.log` siblings (`.log.bak` on a
    // name collision); delete those too.
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem()) else {
        return Ok(());
    };
    let rotated_prefix = format!("{}_", stem.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&rotated_prefix)
            && (name.ends_with(".log") || name.ends_with(".log.bak"))
        {
            std::fs::remove_file(entry.path()).map_err(to_error)?;
        }
    }
    Ok(())
}

pub fn status(core: &AppCore) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(core)?;
    Ok(status_for_path(&path))
}

/// Opens the log file, or its folder while the file does not exist yet.
pub fn open(core: &AppCore) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(core)?;
    let status = status_for_path(&path);
    let target = open_target_path(&path, status.exists)?;
    let target_kind = if status.exists { "file" } else { "folder" };
    core.host()
        .open_url(&target.to_string_lossy())
        .map_err(|error| CaptureErrorResponse {
            code: "app_log_open_failed".to_string(),
            message: format!(
                "Failed to open general application log {target_kind} '{}': {error}",
                target.display()
            ),
        })?;
    Ok(status)
}

pub fn delete(core: &AppCore) -> Result<GeneralAppLogStatus, CaptureErrorResponse> {
    let path = general_app_log_path(core)?;
    delete_log_file_at_path(&path)?;
    Ok(status_for_path(&path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
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
            let path = std::env::temp_dir().join(format!("general-app-log-{label}-{unique}"));
            fs::create_dir_all(&path).expect("test directory should be created");
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

    #[test]
    fn status_for_path_reports_file_existence() {
        let dir = TestDir::new("status");
        let log_path = dir.path().join("rust.log");

        let missing = status_for_path(&log_path);
        assert_eq!(missing.path, log_path.to_string_lossy().to_string());
        assert!(!missing.exists);
        assert_eq!(missing.size_bytes, None);

        fs::write(&log_path, "hello").expect("log file should write");

        let present = status_for_path(&log_path);
        assert!(present.exists);
        assert_eq!(present.size_bytes, Some(5));
    }

    #[test]
    fn delete_log_file_at_path_empties_live_file_and_removes_rotations() {
        use std::io::Write;
        let dir = TestDir::new("delete");
        let log_path = dir.path().join("rust.log");
        fs::write(&log_path, "current").expect("log file should write");
        // The log plugin's handle: append mode, held across the delete.
        let mut plugin_handle = fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .expect("append handle should open");
        let rotated = log_path.with_file_name("rust_2026-10-13_14-20-01.log");
        fs::write(&rotated, "rotated").expect("rotated log file should write");
        let collided = log_path.with_file_name("rust_2026-10-13_14-20-01.log.bak");
        fs::write(&collided, "collided").expect("bak log file should write");
        let unrelated = log_path.with_file_name("rusty.log");
        fs::write(&unrelated, "other").expect("unrelated file should write");

        delete_log_file_at_path(&log_path).expect("log file should be deleted");
        assert_eq!(fs::read_to_string(&log_path).unwrap(), "");
        assert!(!rotated.exists());
        assert!(!collided.exists());
        assert!(unrelated.exists());

        // Lines the plugin writes after the delete stay readable.
        plugin_handle.write_all(b"after\n").unwrap();
        assert_eq!(fs::read_to_string(&log_path).unwrap(), "after\n");

        fs::remove_file(&log_path).unwrap();
        delete_log_file_at_path(&log_path).expect("missing log file should be ignored");
        assert!(!log_path.exists(), "a missing log is not recreated");
    }

    #[test]
    fn open_shows_the_log_folder_until_the_file_exists_then_the_file() {
        let dir = TestDir::new("open");
        let host = std::sync::Arc::new(crate::test_support::FakeHost::default());
        let core = crate::test_support::core_in(host.clone(), dir.path());
        let log_dir = dir.path().join("log");
        let log_path = log_dir.join("rust.log");

        let missing = open(&core).expect("open should succeed");
        assert!(!missing.exists);
        assert_eq!(missing.path, log_path.to_string_lossy());

        fs::create_dir_all(&log_dir).unwrap();
        fs::write(&log_path, "x").unwrap();
        assert_eq!(status(&core).unwrap().size_bytes, Some(1));
        assert!(open(&core).unwrap().exists);
        assert_eq!(
            *host.opened_urls.lock().unwrap(),
            vec![
                log_dir.to_string_lossy().to_string(),
                log_path.to_string_lossy().to_string()
            ]
        );

        assert_eq!(delete(&core).unwrap().size_bytes, Some(0));
    }
}
