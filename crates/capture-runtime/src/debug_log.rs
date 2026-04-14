use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_LOG_FILE_BYTES: u64 = 1_048_576;
const DEFAULT_MAX_LOG_BACKUPS: usize = 5;

#[derive(Debug, Clone, Copy)]
struct RotationPolicy {
    max_file_bytes: u64,
    max_backups: usize,
}

const DEFAULT_ROTATION_POLICY: RotationPolicy = RotationPolicy {
    max_file_bytes: DEFAULT_MAX_LOG_FILE_BYTES,
    max_backups: DEFAULT_MAX_LOG_BACKUPS,
};

#[derive(Debug, Clone, Default)]
struct DebugLogRuntime {
    enabled: bool,
    path: Option<PathBuf>,
}

fn runtime() -> &'static Mutex<DebugLogRuntime> {
    static RUNTIME: OnceLock<Mutex<DebugLogRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(DebugLogRuntime::default()))
}

fn runtime_lock() -> MutexGuard<'static, DebugLogRuntime> {
    match runtime().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn runtime_lock_for_write() -> Option<MutexGuard<'static, DebugLogRuntime>> {
    if std::thread::panicking() {
        match runtime().try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    } else {
        Some(runtime_lock())
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn format_log_line(message: &str) -> String {
    format!("[{}] {}", now_unix_ms(), message)
}

fn backup_path(path: &Path, index: usize) -> io::Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("debug log path '{}' has no file name", path.display()),
        )
    })?;

    Ok(path.with_file_name(format!("{}.{}", file_name.to_string_lossy(), index)))
}

fn backup_suffix<'a>(base_file_name: &str, candidate_file_name: &'a str) -> Option<&'a str> {
    candidate_file_name
        .strip_prefix(base_file_name)?
        .strip_prefix('.')
}

fn is_backup_file_name(base_file_name: &str, candidate_file_name: &str) -> bool {
    backup_suffix(base_file_name, candidate_file_name)
        .map(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

fn related_log_paths(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    if path.exists() {
        paths.push(path.to_path_buf());
    }

    let Some(parent) = path.parent() else {
        paths.sort();
        paths.dedup();
        return Ok(paths);
    };

    let Some(file_name) = path.file_name() else {
        paths.sort();
        paths.dedup();
        return Ok(paths);
    };

    let base_file_name = file_name.to_string_lossy().to_string();
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            paths.sort();
            paths.dedup();
            return Ok(paths);
        }
        Err(error) => return Err(error),
    };

    for entry in entries {
        let entry = entry?;
        let entry_file_name = entry.file_name();
        let entry_file_name = entry_file_name.to_string_lossy();
        if is_backup_file_name(&base_file_name, &entry_file_name) {
            paths.push(entry.path());
        }
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn debug_log_files_exist_inner(path: &Path) -> io::Result<bool> {
    Ok(!related_log_paths(path)?.is_empty())
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rotate_log_file(path: &Path, policy: RotationPolicy) -> io::Result<()> {
    if policy.max_backups == 0 {
        return remove_file_if_exists(path);
    }

    remove_file_if_exists(&backup_path(path, policy.max_backups)?)?;

    for index in (1..policy.max_backups).rev() {
        rename_if_exists(&backup_path(path, index)?, &backup_path(path, index + 1)?)?;
    }

    rename_if_exists(path, &backup_path(path, 1)?)
}

fn rotate_log_file_if_needed(
    path: &Path,
    incoming_line_bytes: u64,
    policy: RotationPolicy,
) -> io::Result<()> {
    let current_size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    if current_size.saturating_add(incoming_line_bytes) <= policy.max_file_bytes {
        return Ok(());
    }

    rotate_log_file(path, policy)
}

fn append_log_line_to_path(path: &Path, line: &str, policy: RotationPolicy) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    rotate_log_file_if_needed(path, line.len() as u64 + 1, policy)?;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

fn append_log_line_to_configured_path(line: &str) {
    let Some(runtime) = runtime_lock_for_write() else {
        return;
    };

    if !runtime.enabled {
        return;
    }

    let append_result = runtime.path.as_deref().map(|path| {
        append_log_line_to_path(path, line, DEFAULT_ROTATION_POLICY)
            .map_err(|error| (path.to_path_buf(), error))
    });

    if let Some(Err((path, error))) = append_result {
        eprintln!(
            "[{}] failed to append debug log to {}: {}",
            now_unix_ms(),
            path.display(),
            error
        );
    }
}

pub fn configure_debug_log(enabled: bool, path: Option<PathBuf>) {
    let mut runtime = runtime_lock();
    runtime.enabled = enabled;
    runtime.path = path;
}

pub fn write_debug_log(message: impl AsRef<str>) {
    let message = message.as_ref();
    log::debug!("{message}");
    write_debug_log_to_file(message);
}

pub fn write_debug_log_to_file(message: impl AsRef<str>) {
    let line = format_log_line(message.as_ref());
    append_log_line_to_configured_path(&line);
}

pub fn write_debug_log_fmt(args: fmt::Arguments<'_>) {
    write_debug_log(args.to_string());
}

pub fn debug_log_files_exist(path: &Path) -> bool {
    let _runtime = runtime_lock();
    debug_log_files_exist_inner(path).unwrap_or_else(|_| path.exists())
}

pub fn delete_debug_log_files(path: &Path) -> io::Result<()> {
    let _runtime = runtime_lock();

    for candidate in related_log_paths(path)? {
        remove_file_if_exists(&candidate)?;
    }

    Ok(())
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
            let path =
                std::env::temp_dir().join(format!("capture-runtime-debug-log-{label}-{unique}"));
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
    fn append_log_line_to_path_creates_and_appends_log_file() {
        let dir = TestDir::new("append");
        let log_path = dir.path().join("debug.log");

        append_log_line_to_path(&log_path, "[1] first line", DEFAULT_ROTATION_POLICY)
            .expect("first write should succeed");
        append_log_line_to_path(&log_path, "[2] second line", DEFAULT_ROTATION_POLICY)
            .expect("second write should succeed");

        let contents = fs::read_to_string(&log_path).expect("log file should exist");
        assert!(contents.contains("first line"));
        assert!(contents.contains("second line"));
    }

    #[test]
    fn append_log_line_to_path_rotates_existing_file_when_size_limit_is_reached() {
        let dir = TestDir::new("rotate");
        let log_path = dir.path().join("debug.log");
        let policy = RotationPolicy {
            max_file_bytes: 15,
            max_backups: 2,
        };

        append_log_line_to_path(&log_path, "1234567890", policy)
            .expect("first write should succeed");
        append_log_line_to_path(&log_path, "abcdefghij", policy)
            .expect("second write should rotate");
        append_log_line_to_path(&log_path, "klmnopqrst", policy)
            .expect("third write should rotate again");

        let current = fs::read_to_string(&log_path).expect("current log file should exist");
        let backup_1 =
            fs::read_to_string(backup_path(&log_path, 1).expect("backup path should resolve"))
                .expect("first backup should exist");
        let backup_2 =
            fs::read_to_string(backup_path(&log_path, 2).expect("backup path should resolve"))
                .expect("second backup should exist");

        assert!(current.contains("klmnopqrst"));
        assert!(backup_1.contains("abcdefghij"));
        assert!(backup_2.contains("1234567890"));
    }

    #[test]
    fn debug_log_files_exist_detects_base_and_backup_files() {
        let dir = TestDir::new("exists");
        let log_path = dir.path().join("debug.log");

        assert!(!debug_log_files_exist_inner(&log_path).expect("existence check should succeed"));

        fs::write(
            backup_path(&log_path, 1).expect("backup path should resolve"),
            "rotated",
        )
        .expect("backup log should write");

        assert!(debug_log_files_exist_inner(&log_path).expect("existence check should succeed"));
    }

    #[test]
    fn delete_debug_log_files_removes_base_and_backups() {
        let dir = TestDir::new("delete");
        let log_path = dir.path().join("debug.log");
        let backup = backup_path(&log_path, 1).expect("backup path should resolve");

        fs::write(&log_path, "current").expect("current log should write");
        fs::write(&backup, "backup").expect("backup log should write");

        delete_debug_log_files(&log_path).expect("delete should succeed");

        assert!(!log_path.exists());
        assert!(!backup.exists());
    }
}
