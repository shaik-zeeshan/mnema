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

use super::model::{self, NotificationRow, Severity, ShellModel, StatusBarModel};

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

fn get<T: DeserializeOwned>(map: &Value, path: &[&str]) -> anyhow::Result<Option<T>> {
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

fn action_label(action: &Value) -> Option<String> {
    let tab = match action.get("type")?.as_str()? {
        "open_settings_tab" => action.get("tab").and_then(Value::as_str).unwrap_or(""),
        "not_recording_reminder" => return Some("Start".into()),
        _ => return None,
    };
    Some(
        match tab {
            "about" => "Open update settings",
            "processing" => "Open OCR settings",
            "transcription" => "Open transcription settings",
            "speakers" => "Open speaker settings",
            "shortcuts" => "Open shortcut settings",
            _ => "Open settings",
        }
        .into(),
    )
}

// shortcut: `AppNotification` lives in src-tauri (`native_capture.rs`) until S3,
// so rows are read field by field here; deserialize the type once it moves.
fn notifications(map: &Value) -> Vec<NotificationRow> {
    let rows = map.get("get_app_notifications").and_then(Value::as_array);
    rows.into_iter()
        .flatten()
        .map(|n| NotificationRow {
            id: n["id"].as_str().unwrap_or_default().into(),
            severity: match n["severity"].as_str() {
                Some("error") => Severity::Error,
                Some("warning") => Severity::Warning,
                _ => Severity::Info,
            },
            title: n["title"].as_str().unwrap_or_default().into(),
            message: n["message"].as_str().unwrap_or_default().into(),
            created_at_ms: n["createdAtUnixMs"].as_f64().unwrap_or_default() as i64,
            action: action_label(&n["action"]),
        })
        .collect()
}

pub fn shell_model(map: &Value) -> anyhow::Result<ShellModel> {
    let settings: Option<RecordingSettings> = get(map, &["get_recording_settings"])?;
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
    let notifications = notifications(map);

    let running = session.as_ref().is_some_and(|s| s.is_running);
    let session_start_ms = session
        .as_ref()
        .and_then(|s| s.source_sessions.as_ref())
        .and_then(|s| {
            [&s.screen, &s.microphone, &s.system_audio]
                .into_iter()
                .flatten()
                .map(|m| m.started_at_unix_ms as i64)
                .min()
        });
    let selected = settings.as_ref().map_or([false; 3], |s| {
        [
            s.capture_screen,
            s.capture_microphone,
            s.capture_system_audio,
        ]
    });
    let status = StatusBarModel {
        running,
        loading_start: false,
        user_paused: session.as_ref().is_some_and(|s| s.is_user_paused),
        inactivity_paused: session.as_ref().is_some_and(|s| s.is_inactivity_paused),
        low_disk: session.as_ref().is_some_and(|s| s.is_low_disk_suspended),
        runtime: runtime.as_ref().map(model::runtime_sources),
        selected,
        screen_permission,
        start_error: None,
        read_only: matches!(
            license,
            Some(LicenseStatus::ReadOnly | LicenseStatus::Revoked)
        ),
        stopped_reason: model::stopped_reason_from(&notifications, session_start_ms),
        session_start_ms,
        engine: map
            .get("get_ai_runtime_status")
            .and_then(model::engine_flag),
        tracked_ms: usage.map(|u| u.time_per_app.iter().map(|a| a.active_ms).sum()),
        storage_bytes: storage.map(|u| {
            u.screen_bytes
                + u.microphone_bytes
                + u.system_audio_bytes
                + u.index_bytes
                + u.models_bytes
        }),
        free_bytes,
        local_only: settings.as_ref().is_none_or(model::is_local_only),
    };
    Ok(ShellModel {
        status,
        notifications,
        developer_options: settings.is_some_and(|s| s.developer_options_enabled),
    })
}

#[cfg(test)]
mod tests {
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
