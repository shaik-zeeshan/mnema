use std::sync::Arc;

use app_core::managed_storage_layout::{self as storage, StorageProbe, StorageUsageDto};
use app_core::AppCore;
use tauri::State;

/// `(async)` on a non-async body is Tauri's "run this on the blocking
/// threadpool" marker, and it is load-bearing: a plain `#[tauri::command]` runs
/// the body on the macOS main thread inside the WKWebView IPC handler, and the
/// probe blocks on a directory the *user* picked (see
/// [`storage::probe_storage_path`]). That is a whole-app freeze for a readout.
#[tauri::command(async)]
pub fn probe_storage_path(path: Option<String>) -> StorageProbe {
    storage::probe_storage_path(path)
}

/// `(async)`: walks the disk off the main thread (see `probe_storage_path`).
#[tauri::command(async)]
pub fn get_storage_usage(core: State<'_, Arc<AppCore>>, refresh: Option<bool>) -> StorageUsageDto {
    storage::get_storage_usage(&core, refresh)
}
