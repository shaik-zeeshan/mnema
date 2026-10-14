//! Read-side status commands for the `/debug` surface (developer options).
//!
//! These are all cheap, poll-friendly reads over data that already exists in
//! SQLite or on disk. They live here rather than in `app_infra.rs` so the debug
//! page's growing read surface does not pile onto that (already very large)
//! module.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::app_infra::AppInfraState;

/// Default ceiling for the frame-batch and derivation-run listings. Both are
/// polled from the debug page, so they are always bounded.
const DEFAULT_LIST_LIMIT: i64 = 50;

/// What the **Semantic Index Backfill** worker itself is doing, as of its last
/// sweep pass — the half of [`SemanticIndexStatusDto`] that no DB query can
/// answer.
///
/// The worker's own copy of this lives in task-local `SweepState`
/// (`semantic_search_worker.rs`), which nothing outside the sweep loop can see;
/// the worker publishes a snapshot here at the end of every pass
/// (`SweepState::health_snapshot`) so the debug page can read it. It is a plain
/// observation mirror, never an input: the sweep never reads it back, so a stale
/// or defaulted snapshot cannot affect embedding.
///
/// ponytail: lives here (with the DTO it feeds) rather than in the worker, which
/// is already far past the repo's 800-line ceiling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticWorkerHealth {
    /// Whether the worker is holding a loaded **Semantic Search Model**. `false`
    /// is normal, not a fault: the worker drops the embedder after an idle grace
    /// period and reloads it on the next anchor.
    pub model_loaded: bool,
    /// Consecutive embedder LOAD failures. At the worker's cap the model is
    /// treated as corrupt and the load-retry loop stops.
    pub consecutive_load_failures: u32,
    /// Anchors quarantined **since app start** (in-memory, never persisted — a
    /// restart clears them, which is the intended liveness).
    pub quarantined_count: usize,
    /// Why the last load attempt failed, if it did. `None` once a load succeeds.
    pub last_load_error: Option<String>,
}

/// [`SemanticWorkerHealth`] shared between the worker (one writer) and the debug
/// read side (pollers). A `std::sync::Mutex` per the repo invariant — every
/// critical section here is a short, await-free snapshot copy.
pub type SemanticWorkerHealthState = Arc<Mutex<SemanticWorkerHealth>>;

/// Health of the **Semantic Index**: how much is indexed, how much is waiting,
/// and what the embedding worker itself is doing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIndexStatusDto {
    /// Stored **Semantic Search Vector**s — the live index size.
    pub vector_count: i64,
    /// `direct` anchors still lacking a vector (the backfill backlog).
    pub backlog_count: i64,
    /// Live `vec0` column width. `None` when the table is absent, which is also
    /// the "index unusable, search degrades to keyword-only" signal.
    pub live_dimension: Option<usize>,

    // ---- the worker half, from the shared [`SemanticWorkerHealth`] snapshot ----
    // The `Option`s are the wire shape the frontend already mirrors. They are now
    // always `Some` (an unpublished snapshot defaults to "nothing loaded, nothing
    // failed, nothing quarantined", which is exactly true before the first pass),
    // except `last_load_error`, whose `None` is meaningful.
    /// Whether the worker holds a loaded model. `Some(false)` is normal while it
    /// is caught up — it drops the embedder after an idle grace period.
    pub model_loaded: Option<bool>,
    /// Consecutive embedder LOAD failures; at the worker's cap the model is
    /// treated as corrupt and the retry loop stops.
    pub consecutive_load_failures: Option<u32>,
    /// Anchors quarantined since app start (in-memory; a restart clears them).
    pub quarantined_count: Option<usize>,
    /// Why the last model load failed. `None` when the last attempt succeeded.
    pub last_load_error: Option<String>,
}

#[tauri::command]
pub async fn get_semantic_index_status(
    infra: tauri::State<'_, AppInfraState>,
    worker_health: tauri::State<'_, SemanticWorkerHealthState>,
) -> Result<SemanticIndexStatusDto, String> {
    // Copy the snapshot out and drop the guard before any `.await` below: a
    // `std::sync::Mutex` guard must never be held across an await point.
    let health = worker_health
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();

    let store = infra.semantic_search();
    let vector_count = store
        .count_vectors()
        .await
        .map_err(|error| format!("failed to count semantic vectors: {error}"))?;
    let backlog_count = store
        .count_anchors_missing_vector()
        .await
        .map_err(|error| format!("failed to count semantic index backlog: {error}"))?;
    let live_dimension = store
        .live_vector_dimension()
        .await
        .map_err(|error| format!("failed to read semantic vector dimension: {error}"))?;

    Ok(SemanticIndexStatusDto {
        vector_count,
        backlog_count,
        live_dimension,
        model_loaded: Some(health.model_loaded),
        consecutive_load_failures: Some(health.consecutive_load_failures),
        quarantined_count: Some(health.quarantined_count),
        last_load_error: health.last_load_error,
    })
}

/// Frame batches that have not completed (including `failed` ones, which carry
/// `last_error`), newest first.
#[tauri::command]
pub async fn list_frame_batches(
    infra: tauri::State<'_, AppInfraState>,
    limit: Option<i64>,
) -> Result<Vec<::app_infra::FrameBatch>, String> {
    infra
        .list_unfinished_frame_batches(limit.unwrap_or(DEFAULT_LIST_LIMIT))
        .await
        .map_err(|error| format!("failed to list frame batches: {error}"))
}

/// The tail of the User Context derivation-run ledger, newest first.
#[tauri::command]
pub async fn list_user_context_derivation_runs(
    infra: tauri::State<'_, AppInfraState>,
    limit: Option<i64>,
) -> Result<Vec<::app_infra::DerivationRun>, String> {
    infra
        .user_context()
        .list_derivation_runs(limit.unwrap_or(DEFAULT_LIST_LIMIT))
        .await
        .map_err(|error| format!("failed to list derivation runs: {error}"))
}

/// Which log file to tail. A closed enum, never a caller-supplied path: the
/// debug page can only reach the two logs the app itself writes, so there is no
/// path-traversal surface (`../../../etc/passwd` does not deserialize).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppLogFile {
    /// The `tauri-plugin-log` application log (`rust.log`).
    Rust,
    /// The native-capture debug log (`native-capture-debug.log`).
    NativeCapture,
}

/// The last `lines` lines of one app log.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogTailDto {
    /// Resolved on-disk path, so the debug page can show where it read from.
    pub path: String,
    pub exists: bool,
    /// Up to `lines` lines, oldest first. Empty when the file is missing.
    pub lines: Vec<String>,
}

/// Hard ceiling on a single tail request — a debug readout, not a log viewer.
const MAX_TAIL_LINES: usize = 2_000;

/// Bytes read per backward step when tailing a log.
const TAIL_BLOCK_BYTES: u64 = 64 * 1024;

/// Read the last `lines` lines of `path`, oldest first.
///
/// Reads backward from the end in `TAIL_BLOCK_BYTES` blocks until it holds
/// more than `lines` newlines or reaches the start, so a poll costs O(tail),
/// not O(file) — rust.log runs to 5 MB and the Logs tab polls every 2 s.
/// ponytail: one line longer than the whole file still reads to the start;
/// fine for a log a human triggers.
fn tail_lines(path: &std::path::Path, lines: usize) -> std::io::Result<Vec<String>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        // Missing file is an empty tail, not an error: the log simply has not
        // been written yet (or the user just deleted it from this same page).
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut pos = file.seek(SeekFrom::End(0))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut newlines = 0;
    // `> lines` newlines guarantees the first kept line is complete, even with
    // a trailing newline.
    while pos > 0 && newlines <= lines {
        let step = pos.min(TAIL_BLOCK_BYTES);
        pos -= step;
        file.seek(SeekFrom::Start(pos))?;
        let mut block = vec![0; step as usize];
        file.read_exact(&mut block)?;
        newlines += block.iter().filter(|&&byte| byte == b'\n').count();
        block.extend_from_slice(&buf);
        buf = block;
    }
    if buf.is_empty() {
        return Ok(Vec::new());
    }

    // Not at the start: everything up to the first newline is a partial line.
    let mut text = buf.as_slice();
    if pos > 0 {
        let first = text
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(0, |at| at + 1);
        text = &text[first..];
    }
    // A trailing newline must not produce a phantom empty last line.
    let text = text.strip_suffix(b"\n").unwrap_or(text);
    let all: Vec<&[u8]> = text.split(|&byte| byte == b'\n').collect();
    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| {
            // Lossless UTF-8 is not guaranteed for a log a panic hook wrote
            // into; lossy-convert a line rather than fail the whole tail.
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            String::from_utf8_lossy(line).into_owned()
        })
        .collect())
}

fn app_log_path(app_handle: &tauri::AppHandle, file: AppLogFile) -> Result<PathBuf, String> {
    match file {
        AppLogFile::Rust => app_handle
            .path()
            .app_log_dir()
            .map(|dir| dir.join(crate::APP_LOG_FILE_NAME).with_extension("log"))
            .map_err(|error| format!("failed to resolve application log path: {error}")),
        AppLogFile::NativeCapture => Ok(
            crate::native_capture::debug_log::native_capture_debug_log_path(app_handle),
        ),
    }
}

// Async so the file read runs off the main thread.
#[tauri::command]
pub async fn tail_app_log(
    app_handle: tauri::AppHandle,
    file: AppLogFile,
    lines: Option<usize>,
) -> Result<AppLogTailDto, String> {
    let path = app_log_path(&app_handle, file)?;
    let lines = lines.unwrap_or(200).clamp(1, MAX_TAIL_LINES);
    let tail = tail_lines(&path, lines)
        .map_err(|error| format!("failed to read log '{}': {error}", path.display()))?;

    Ok(AppLogTailDto {
        exists: path.is_file(),
        path: path.to_string_lossy().to_string(),
        lines: tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("debug-status-{label}-{unique}"));
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
    fn tail_lines_returns_the_last_n_lines_oldest_first() {
        let dir = TestDir::new("last-n");
        let log = dir.path().join("rust.log");
        fs::write(&log, "one\ntwo\nthree\nfour\nfive\n").expect("log should write");

        let tail = tail_lines(&log, 2).expect("tail should read");
        assert_eq!(tail, vec!["four".to_string(), "five".to_string()]);
    }

    #[test]
    fn tail_lines_returns_the_whole_file_when_it_is_shorter_than_n() {
        let dir = TestDir::new("short");
        let log = dir.path().join("rust.log");
        fs::write(&log, "one\ntwo\n").expect("log should write");

        let tail = tail_lines(&log, 100).expect("tail should read");
        assert_eq!(tail, vec!["one".to_string(), "two".to_string()]);

        // A trailing newline must not produce a phantom empty last line.
        assert_eq!(tail.len(), 2);
    }

    #[test]
    fn tail_lines_treats_a_missing_file_as_an_empty_tail() {
        let dir = TestDir::new("missing");
        let log = dir.path().join("rust.log");

        let tail = tail_lines(&log, 10).expect("missing log should not error");
        assert!(tail.is_empty());
    }

    #[test]
    fn tail_lines_keeps_only_n_lines_across_a_long_file() {
        let dir = TestDir::new("long");
        let log = dir.path().join("rust.log");
        let body: String = (0..5_000).map(|index| format!("line {index}\n")).collect();
        fs::write(&log, body).expect("log should write");

        let tail = tail_lines(&log, 3).expect("tail should read");
        assert_eq!(
            tail,
            vec![
                "line 4997".to_string(),
                "line 4998".to_string(),
                "line 4999".to_string(),
            ]
        );
    }

    #[test]
    fn tail_lines_reads_backward_across_block_boundaries() {
        let dir = TestDir::new("blocks");
        let log = dir.path().join("rust.log");
        // ~3 blocks of 37-byte lines, so lines straddle every block boundary,
        // plus one invalid-UTF-8 line that must not fail the tail.
        let count = (3 * TAIL_BLOCK_BYTES as usize) / 37;
        let mut body: Vec<u8> = (0..count)
            .flat_map(|index| format!("line {index:08} padding padding pad\n").into_bytes())
            .collect();
        body.extend_from_slice(b"bad \xff byte\n");
        assert!(body.len() as u64 > 2 * TAIL_BLOCK_BYTES);
        fs::write(&log, body).expect("log should write");

        let tail = tail_lines(&log, 2_000).expect("tail should read");
        assert_eq!(tail.len(), 2_000);
        assert_eq!(
            tail[0],
            format!("line {:08} padding padding pad", count - 1_999)
        );
        assert_eq!(
            tail[1_998],
            format!("line {:08} padding padding pad", count - 1)
        );
        assert_eq!(tail[1_999], "bad \u{fffd} byte");

        let whole = tail_lines(&log, usize::MAX).expect("whole file should read");
        assert_eq!(whole.len(), count + 1);
        assert_eq!(whole[0], "line 00000000 padding padding pad");
    }

    #[test]
    fn app_log_file_deserializes_only_the_two_known_logs() {
        assert_eq!(
            serde_json::from_str::<AppLogFile>("\"rust\"").expect("rust log should parse"),
            AppLogFile::Rust
        );
        assert_eq!(
            serde_json::from_str::<AppLogFile>("\"nativeCapture\"")
                .expect("native capture log should parse"),
            AppLogFile::NativeCapture
        );
        // No caller-supplied path can reach the reader.
        assert!(serde_json::from_str::<AppLogFile>("\"../../etc/passwd\"").is_err());
    }
}
