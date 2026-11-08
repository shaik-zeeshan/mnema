//! What the shell shows, as plain structs plus the pure rules that turn capture,
//! license and engine state into words. Ported from `lib/shell/status-bar.ts`;
//! views only render these, and `data.rs` / `fixture.rs` fill them.

use capture_types::{
    AiProviderKind, AudioTranscriptionProvider, CapturePermissionState, RecordingSettings,
    RuntimeSourcesStatus,
};

/// The main window's four surfaces (placeholders until S8/S11/S12/S13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Timeline,
    Insights,
    Chat,
    Settings,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellModel {
    pub status: StatusBarModel,
    pub notifications: Vec<NotificationRow>,
    pub developer_options: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationRow {
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub message: String,
    pub created_at_ms: i64,
    /// The row's button label (`actionLabel` in NotificationsPop.svelte), if any.
    pub action: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Screen,
    Microphone,
    SystemAudio,
}

impl Lane {
    pub const ALL: [Lane; 3] = [Lane::Screen, Lane::Microphone, Lane::SystemAudio];

    pub fn label(self) -> &'static str {
        match self {
            Lane::Screen => "screen",
            Lane::Microphone => "mic",
            Lane::SystemAudio => "system",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lane::Screen => "Screen",
            Lane::Microphone => "Microphone",
            Lane::SystemAudio => "System audio",
        }
    }
}

/// `StatusInput` in status-bar.ts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusBarModel {
    pub running: bool,
    pub loading_start: bool,
    pub user_paused: bool,
    pub inactivity_paused: bool,
    pub low_disk: bool,
    pub runtime: Option<RuntimeSources>,
    /// Sources chosen for the next / current session, in [`Lane::ALL`] order.
    pub selected: [bool; 3],
    /// Screen Recording permission, when read.
    pub screen_permission: Option<CapturePermissionState>,
    pub start_error: Option<String>,
    pub read_only: bool,
    pub stopped_reason: Option<String>,
    /// Earliest source start of the running session (unix ms).
    pub session_start_ms: Option<i64>,
    pub engine: Option<EngineFlag>,
    pub tracked_ms: Option<i64>,
    pub storage_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub local_only: bool,
}

/// `RuntimeSourcesStatus` reduced to what the lanes read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeSource {
    pub requested: bool,
    pub paused: bool,
    pub live: bool,
    pub privacy_failed: bool,
}

pub type RuntimeSources = [RuntimeSource; 3];

pub fn runtime_sources(status: &RuntimeSourcesStatus) -> RuntimeSources {
    [&status.screen, &status.microphone, &status.system_audio].map(|s| RuntimeSource {
        requested: s.requested,
        paused: s.paused,
        live: s.session_active == Some(true) && s.writer_active == Some(true),
        privacy_failed: matches!(
            s.reason.as_deref(),
            Some("privacy_filter_apply_failed" | "privacy_recovery_restart_required")
        ),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecMod {
    Off,
    Starting,
    Running,
    Paused,
    Suspended,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecView {
    pub kind: RecMod,
    pub label: String,
    /// The session clock follows the label.
    pub elapsed_clock: bool,
}

pub fn rec_view(m: &StatusBarModel) -> RecView {
    let v = |kind, label: &str, elapsed_clock| RecView {
        kind,
        label: label.into(),
        elapsed_clock,
    };
    if !m.running {
        if m.loading_start {
            return v(RecMod::Starting, "STARTING…", false);
        }
        if let Some(reason) = &m.stopped_reason {
            return v(RecMod::Stopped, &format!("STOPPED · {reason}"), false);
        }
        return v(RecMod::Off, "NOT RECORDING", false);
    }
    if m.low_disk && !m.user_paused {
        return v(RecMod::Suspended, "PAUSED · low disk", false);
    }
    // shortcut: no pause instant on the session, so "PAUSED" has no clock until
    // S3 moves capture state (status-bar.svelte counts from when it saw the pause).
    if m.user_paused {
        return v(RecMod::Paused, "PAUSED", false);
    }
    let all_idle = m.runtime.is_none_or(|rt| {
        let requested: Vec<_> = rt.iter().filter(|s| s.requested).collect();
        !requested.is_empty() && requested.iter().all(|s| s.paused)
    });
    if m.inactivity_paused && all_idle {
        return v(RecMod::Paused, "PAUSED · idle", false);
    }
    v(RecMod::Running, "REC", true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneMod {
    On,
    Off,
    Paused,
    Starting,
    Error,
}

/// Display-only lanes while starting / recording.
// shortcut: the `waiting` (display asleep, tap reconnecting) lanes read
// `get_capture_health`, which is not in app-core until S3; wire it then.
pub fn live_lanes(m: &StatusBarModel) -> [(LaneMod, Option<&'static str>); 3] {
    let running = rec_view(m).kind == RecMod::Running;
    std::array::from_fn(|i| match (m.running, m.runtime) {
        (true, Some(rt)) => {
            let src = rt[i];
            if !src.requested {
                (LaneMod::Off, None)
            } else if src.privacy_failed {
                (LaneMod::Error, Some("stopped"))
            } else if src.paused {
                (LaneMod::Paused, running.then_some("idle"))
            } else if src.live {
                (LaneMod::On, None)
            } else {
                (LaneMod::Starting, None)
            }
        }
        _ if m.selected[i] => (LaneMod::Starting, None),
        _ => (LaneMod::Off, None),
    })
}

pub fn screen_permission_missing(m: &StatusBarModel) -> bool {
    !m.running && m.selected[0] && m.screen_permission == Some(CapturePermissionState::Denied)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Warn,
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusFlag {
    pub tone: Tone,
    pub text: &'static str,
    pub action: String,
}

/// At most one flag. Recording: privacy > low disk. Stopped: start failure >
/// read-only > stopped by the system > permission.
// shortcut: "disk almost full" reads capture health thresholds (S3).
pub fn status_flag(m: &StatusBarModel) -> Option<StatusFlag> {
    let flag = |tone, text, action: &str| {
        Some(StatusFlag {
            tone,
            text,
            action: action.into(),
        })
    };
    if m.running {
        if m.runtime
            .is_some_and(|rt| rt.iter().any(|s| s.requested && s.privacy_failed))
        {
            return flag(Tone::Warn, "privacy filter failed ·", "retrying");
        }
        if m.low_disk {
            return flag(Tone::Warn, "free up space ·", "resumes on its own");
        }
        return None;
    }
    if m.start_error.is_some() {
        return flag(Tone::Danger, "couldn't start ·", "Retry");
    }
    if m.read_only {
        return flag(Tone::Danger, "read-only ·", "buy a license");
    }
    if m.stopped_reason.as_deref() == Some("disk full") {
        return flag(Tone::Danger, "", "Open Storage");
    }
    if screen_permission_missing(m) {
        return flag(Tone::Warn, "Screen Recording off ·", "Open System Settings");
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineFlag {
    pub label: &'static str,
    pub error: bool,
}

/// The engine pill from `get_ai_runtime_status`: nothing while healthy.
// shortcut: a reduced `engineState` (insights/engine-state.ts) over the status
// JSON, because `AiRuntimeStatus` lives in src-tauri until S2; use the type then.
pub fn engine_flag(status: &serde_json::Value) -> Option<EngineFlag> {
    let flag = |label, error| Some(EngineFlag { label, error });
    if status.get("hasProviders").and_then(|v| v.as_bool()) == Some(false) {
        return flag("not set up", false);
    }
    match status.get("reason").and_then(|v| v.as_str()) {
        Some("vault_denied") => flag("keychain locked", true),
        Some(r) if r == "local_endpoint_unreachable" || r.starts_with("provider_unreachable:") => {
            flag("offline", true)
        }
        Some(r) if r.starts_with("needs_reconnect:") => flag("needs reconnect", true),
        _ if status.get("configured").and_then(|v| v.as_bool()) == Some(false) => {
            flag("not set up", false)
        }
        _ => None,
    }
}

/// "local only" is a claim: drop it once audio or questions leave the Mac.
pub fn is_local_only(settings: &RecordingSettings) -> bool {
    let t = &settings.transcription;
    if t.enabled && t.provider == AudioTranscriptionProvider::Deepgram {
        return false;
    }
    let ai = &settings.ai_runtime;
    match (ai.enabled, &ai.default_model) {
        (true, Some(model)) => ai
            .providers
            .iter()
            .find(|p| p.id == model.provider)
            .is_some_and(|p| matches!(p.kind, AiProviderKind::Ollama | AiProviderKind::Llamafile)),
        _ => true,
    }
}

/// "4h 41m" · "12m".
pub fn format_tracked(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    let h = minutes / 60;
    if h > 0 {
        format!("{h}h {}m", minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// "01:39:07" (with hours) or "00:12" (mm:ss until an hour has passed).
pub fn format_clock(ms: i64, with_hours: bool) -> String {
    let s = ms.max(0) / 1000;
    let (hh, mm, ss) = (s / 3600, (s % 3600) / 60, s % 60);
    if with_hours || hh > 0 {
        format!("{hh:02}:{mm:02}:{ss:02}")
    } else {
        format!("{mm:02}:{ss:02}")
    }
}

/// Decimal units, one decimal past bytes: "2.4 GB" (`formatBytes`).
pub fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if value == 0 {
        return "unknown size".into();
    }
    let (mut size, mut unit) = (value as f64, 0);
    while size >= 1000. && unit < UNITS.len() - 1 {
        size /= 1000.;
        unit += 1;
    }
    if unit == 0 {
        format!("{size:.0} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// The system's "Recording stopped — …" notification as a short reason, unless a
/// session started after it.
pub fn stopped_reason_from(
    rows: &[NotificationRow],
    last_session_start: Option<i64>,
) -> Option<String> {
    rows.iter().find_map(|n| {
        let reason = match n.id.as_str() {
            "capture_disk_full_stopped" => "disk full",
            "capture_trial_ended_stopped" => "trial ended",
            "capture_license_revoked_stopped" => "license revoked",
            _ => return None,
        };
        last_session_start
            .is_none_or(|start| n.created_at_ms > start)
            .then(|| reason.to_string())
    })
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
