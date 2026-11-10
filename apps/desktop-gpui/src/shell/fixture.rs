//! Parity fixtures -> shell models. The P0 harness serves
//! `apps/desktop/parity/fixtures/<surface>/<state>.json` (command -> JSON) to the
//! Svelte stub; goldens build the GPUI models from the same maps, through the
//! real Rust types, so both sides show the same numbers.

use std::path::PathBuf;

use anyhow::Context as _;
use app_infra::StorageUsage;
use capture_types::{
    CapturePermissionState, LicenseStatus, NativeCaptureSession, RecordingSettings,
    RuntimeSourcesStatus, UsageChartsResponse,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::model::{self, ShellModel};

/// The fixture map for `surface` / `state`, with `$REPO` resolved like `shoot.ts`.
pub fn load(surface: &str, state: &str) -> anyhow::Result<Value> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = repo.canonicalize().unwrap_or(repo);
    let path = repo.join(format!(
        "apps/desktop/parity/fixtures/{surface}/{state}.json"
    ));
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let text = text.replace("$REPO", &repo.to_string_lossy());
    Ok(serde_json::from_str(&text)?)
}

/// JS writes whole numbers as floats (`1819702920000.0`); Rust's integer fields
/// refuse those, so turn integral floats back into integers first.
fn integral(v: &Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9e15 => Value::from(f as i64),
            _ => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(integral).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), integral(v))).collect())
        }
        _ => v.clone(),
    }
}

pub(crate) fn get<T: DeserializeOwned>(map: &Value, path: &[&str]) -> anyhow::Result<Option<T>> {
    let mut v = map;
    for key in path {
        match v.get(key) {
            Some(next) => v = next,
            None => return Ok(None),
        }
    }
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(integral(v))
        .map(Some)
        .with_context(|| format!("fixture {}", path.join(".")))
}

pub fn shell_model(map: &Value) -> anyhow::Result<ShellModel> {
    let settings: Option<RecordingSettings> = get(map, &["get_recording_settings"])?;
    let settings_ref = settings.clone();
    let screen_permission: Option<CapturePermissionState> =
        get(map, &["get_capture_permissions", "permissions", "screen"])?;
    let session: Option<NativeCaptureSession> = get(map, &["get_capture_permissions", "session"])?;
    let runtime: Option<RuntimeSourcesStatus> = get(map, &["get_idle_debug", "runtimeSources"])?;
    let license: Option<LicenseStatus> = get(map, &["get_license_status"])?;
    let usage: Option<UsageChartsResponse> = get(map, &["get_usage_charts"])?;
    let storage: Option<StorageUsage> = get(map, &["get_storage_usage"])?;
    let free_bytes: Option<u64> = match get(map, &["get_capture_health", "disk", "freeBytes"])? {
        Some(free) => Some(free),
        None => get(map, &["get_storage_usage", "freeBytes"])?,
    };
    Ok(super::data::build(super::data::Inputs {
        settings,
        screen_permission,
        session,
        runtime,
        license,
        usage,
        storage,
        free_bytes,
        engine: get::<crate::insights::model::AiStatus>(map, &["get_ai_runtime_status"])?
            .and_then(|status| model::engine_flag(&status, settings_ref.as_ref())),
        notifications: map
            .get("get_app_notifications")
            .map(model::notification_rows)
            .unwrap_or_default(),
        health: get(map, &["get_capture_health"])?,
        backlog: backlog(map)?,
    }))
}

/// The popover's "Catching up" reads, when the map carries any of them.
fn backlog(map: &Value) -> anyhow::Result<Option<super::health::Backlog>> {
    let keys = [
        "get_quick_recall_index_status",
        "get_processing_pipeline_status",
        "transcription_deepgram_auth_status",
    ];
    if keys.iter().all(|k| map.get(k).is_none()) {
        return Ok(None);
    }
    Ok(Some(super::health::Backlog {
        index: get(map, &[keys[0]])?,
        pipeline: get(map, &[keys[1]])?,
        deepgram_auth_error: get(map, &[keys[2]])?,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::model::Severity;
    use super::*;

    #[test]
    fn populated_fixture_matches_the_tauri_status_bar() {
        let m = shell_model(&load("shell", "populated").unwrap()).unwrap();
        let s = &m.status;
        assert!(s.running);
        assert_eq!(model::rec_view(s).label, "REC");
        // 14:32:00 fixture now - 12:52:53 start = the Tauri shot's 01:39:07.
        assert_eq!(
            model::format_clock(1_793_782_920_000 - s.session_start_ms.unwrap(), true),
            "01:39:07"
        );
        assert_eq!(model::format_tracked(s.tracked_ms.unwrap()), "4h 41m");
        assert_eq!(model::format_bytes(s.storage_bytes.unwrap()), "2.4 GB");
        assert!(s.local_only && s.engine.is_none());
        assert_eq!(m.notifications.len(), 1);
        assert_eq!(m.notifications[0].severity, Severity::Warning);
    }

    #[test]
    fn empty_fixture_matches_the_tauri_status_bar() {
        let m = shell_model(&load("shell", "empty").unwrap()).unwrap();
        let s = &m.status;
        assert!(!s.running);
        assert_eq!(model::rec_view(s).label, "NOT RECORDING");
        assert_eq!(s.engine.as_ref().unwrap().label, "not set up");
        assert_eq!(model::format_tracked(s.tracked_ms.unwrap()), "0m");
        assert_eq!(model::format_bytes(s.storage_bytes.unwrap()), "1.1 GB");
        assert!(m.notifications.is_empty());
    }
}
