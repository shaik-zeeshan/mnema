//! The status-bar (tray) menu as data: item ids, labels, enabled/checked
//! state and tooltip for the current capture state. Pure, so both hosts build
//! the same menu (Tauri in `src-tauri/src/status_bar.rs`, GPUI in
//! `apps/desktop-gpui/src/native/tray.rs`) and it is unit-tested once.

use capture_types::{CaptureSources, RecordingSettings};

use crate::updates::AppUpdateState;

#[cfg(test)]
mod tests;

pub const TRAY_ID: &str = "mnema-status-bar";
pub const COMPLETE_SETUP_ID: &str = "tray_complete_setup";
pub const STATUS_HEADER_ID: &str = "tray_status_header";
pub const RECORDING_TOGGLE_ID: &str = "tray_recording_toggle";
pub const PAUSE_TOGGLE_ID: &str = "tray_pause_toggle";
pub const DELETE_LAST_1_MINUTE_ID: &str = "tray_delete_recent_60";
pub const DELETE_LAST_5_MINUTES_ID: &str = "tray_delete_recent_300";
pub const DELETE_LAST_15_MINUTES_ID: &str = "tray_delete_recent_900";
pub const EXCLUDE_CURRENT_APP_ID: &str = "tray_exclude_current_app";
pub const SOURCE_SCREEN_ID: &str = "tray_source_screen";
pub const SOURCE_MICROPHONE_ID: &str = "tray_source_microphone";
pub const SOURCE_SYSTEM_AUDIO_ID: &str = "tray_source_system_audio";
pub const UPDATE_ID: &str = "tray_install_update";
pub const OPEN_MAIN_ID: &str = "tray_open_main";
pub const OPEN_SETTINGS_ID: &str = "tray_open_settings";
pub const QUIT_ID: &str = "tray_quit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBarOperation {
    Idle,
    Starting,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItemModel {
    pub id: &'static str,
    pub label: &'static str,
    pub checked: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBarMenuModel {
    pub onboarding_complete: bool,
    /// A non-actionable status header shown at the top of the menu. Present only
    /// for the low-disk-suspended state ("Paused — low disk"), which is surfaced
    /// as a disabled label so the tray never reads as plainly "Recording" while
    /// capture is held by the low-disk liveness suspension (ADR 0040).
    pub status_label: Option<&'static str>,
    pub recording_label: Option<&'static str>,
    pub recording_enabled: bool,
    pub pause_label: Option<&'static str>,
    pub pause_enabled: bool,
    pub source_items: Vec<SourceItemModel>,
    /// Label + enabled for the update row, or `None` when there's nothing to
    /// install. Built by [`tray_update_menu_item`] from the updater state.
    pub update_item: Option<(String, bool)>,
    pub tooltip: &'static str,
}

impl Default for StatusBarOperation {
    fn default() -> Self {
        Self::Idle
    }
}

fn any_source_enabled(sources: &CaptureSources) -> bool {
    sources.screen || sources.microphone || sources.system_audio
}

fn effective_checked_sources(
    settings: &RecordingSettings,
    _support: &CaptureSources,
) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

fn supported_sources_only(sources: &CaptureSources, support: &CaptureSources) -> CaptureSources {
    CaptureSources {
        screen: sources.screen && support.screen,
        microphone: sources.microphone && support.microphone,
        system_audio: sources.system_audio && support.system_audio,
    }
}

// System audio is an independent capture family on a Core Audio process tap
// (ADR 0052), so none of the three sources gates another here: turning the
// screen off leaves audio recording, and system audio toggles on its own. The
// only rule left is the one that always applied — a session needs at least one
// source.
pub fn computed_toggle_sources(current: CaptureSources, source_id: &str) -> Option<CaptureSources> {
    let mut next = current;
    match source_id {
        SOURCE_SCREEN_ID => next.screen = !next.screen,
        SOURCE_MICROPHONE_ID => next.microphone = !next.microphone,
        SOURCE_SYSTEM_AUDIO_ID => next.system_audio = !next.system_audio,
        _ => return None,
    }

    any_source_enabled(&next).then_some(next)
}

fn source_item_enabled(
    source_id: &str,
    checked: bool,
    current: &CaptureSources,
    support: &CaptureSources,
    operation: StatusBarOperation,
    recording: bool,
) -> bool {
    if recording || operation != StatusBarOperation::Idle {
        return false;
    }

    let supported = match source_id {
        SOURCE_SCREEN_ID => support.screen,
        SOURCE_MICROPHONE_ID => support.microphone,
        SOURCE_SYSTEM_AUDIO_ID => support.system_audio,
        _ => false,
    };
    if !supported {
        return false;
    }
    if checked {
        let Some(next) = computed_toggle_sources(current.clone(), source_id) else {
            return false;
        };
        if !any_source_enabled(&supported_sources_only(&next, support)) {
            return false;
        }
    }

    true
}

pub fn build_menu_model(
    onboarding_complete: bool,
    recording: bool,
    user_paused: bool,
    low_disk_suspended: bool,
    settings: &RecordingSettings,
    support: &CaptureSources,
    operation: StatusBarOperation,
) -> StatusBarMenuModel {
    if !onboarding_complete {
        return StatusBarMenuModel {
            onboarding_complete: false,
            status_label: None,
            recording_label: None,
            recording_enabled: false,
            pause_label: None,
            pause_enabled: false,
            source_items: Vec::new(),
            update_item: None,
            tooltip: "Mnema",
        };
    }

    let checked_sources = effective_checked_sources(settings, support);
    let recording_label = match operation {
        StatusBarOperation::Idle if recording => "Stop Recording",
        StatusBarOperation::Idle => "Start Recording",
        StatusBarOperation::Starting => "Starting...",
        StatusBarOperation::Stopping => "Stopping...",
    };
    // The low-disk suspension keeps the session alive (recording == true), so it
    // must take precedence over the generic recording/paused tooltip — otherwise
    // the tray would read "Recording" while capture is actually held (ADR 0040).
    let status_label = (operation == StatusBarOperation::Idle && low_disk_suspended)
        .then_some("Paused — low disk");
    let tooltip = match operation {
        StatusBarOperation::Idle if low_disk_suspended => "Mnema — Paused (low disk)",
        StatusBarOperation::Idle if recording => "Mnema - Recording",
        StatusBarOperation::Idle => "Mnema",
        StatusBarOperation::Starting => "Mnema - Starting...",
        StatusBarOperation::Stopping => "Mnema - Stopping...",
    };

    let source_items = [
        (SOURCE_SCREEN_ID, "Screen", checked_sources.screen),
        (
            SOURCE_MICROPHONE_ID,
            "Microphone",
            checked_sources.microphone,
        ),
        (
            SOURCE_SYSTEM_AUDIO_ID,
            "System Audio",
            checked_sources.system_audio,
        ),
    ]
    .into_iter()
    .map(|(id, label, checked)| SourceItemModel {
        id,
        label,
        checked,
        enabled: source_item_enabled(id, checked, &checked_sources, support, operation, recording),
    })
    .collect();

    StatusBarMenuModel {
        onboarding_complete: true,
        status_label,
        recording_label: Some(recording_label),
        recording_enabled: operation == StatusBarOperation::Idle,
        pause_label: recording.then_some(if user_paused {
            "Resume Recording"
        } else {
            "Pause Recording"
        }),
        pause_enabled: recording && operation == StatusBarOperation::Idle,
        source_items,
        update_item: None,
        tooltip,
    }
}

/// Read-Only Mode (licensing) is a distinct condition from the low-disk liveness
/// suspension: it does NOT self-heal (cleared only by buying a license) and it
/// blocks *starting* new capture rather than holding a live session. When active
/// and idle, surface its own status header and grey out Start Recording so the
/// tray never offers a start the capture gate will refuse. Gating on the
/// "Start Recording" label scopes this to the idle-and-not-recording state only.
pub fn apply_read_only_status(
    model: &mut StatusBarMenuModel,
    blocked: bool,
    status: Option<&capture_types::LicenseStatus>,
) {
    if blocked && model.recording_label == Some("Start Recording") {
        model.recording_enabled = false;
        let (label, tooltip) = match status {
            Some(capture_types::LicenseStatus::Revoked) => (
                "Read-Only — license revoked",
                "Mnema — Read-Only (license revoked)",
            ),
            // A paid key whose once-per-machine activation Lapsed is NOT a trial —
            // mirror `capture_refusal_copy`'s distinct "finish activation" copy so
            // the tray never tells a paying customer their trial ended.
            Some(capture_types::LicenseStatus::Licensed {
                activation: capture_types::Activation::Lapsed,
                ..
            }) => (
                "Read-Only — activation needed",
                "Mnema — Read-Only (activation needed)",
            ),
            _ => ("Read-Only — trial ended", "Mnema — Read-Only (trial ended)"),
        };
        model.status_label = Some(label);
        model.tooltip = tooltip;
    }
}

/// Label + enabled state for the tray's update item, or `None` when the menu
/// should carry no update row at all. Out-of-window builds
/// (`AvailableOutOfWindow`) are deliberately absent: they aren't installable, so
/// the tray would offer a button that can only fail — Settings does the directing.
///
/// `Failed` DOES keep a row. A failed download or bundle swap leaves
/// `pending_update` intact and is retryable, which is why the update window and
/// Settings → About both keep their install button live for it; dropping the
/// tray row instead stranded the windowless user the row exists for.
///
/// `installable` is `pending_update.is_some()`, NOT the presence of a version:
/// they are different fields and they come apart. A failed *check* clears
/// `update` (no version, no row), but `install_app_update`'s no-pending branch
/// and `restart_after_app_update`'s error path both set `Failed` while LEAVING
/// `update` populated — keying the row on the version alone would draw an
/// enabled "Retry" whose only possible outcome is the failure alert. That is
/// exactly the "button that can only fail" reasoning that keeps
/// `AvailableOutOfWindow` off the tray.
pub fn tray_update_menu_item(
    state: AppUpdateState,
    version: Option<&str>,
    installable: bool,
) -> Option<(String, bool)> {
    match state {
        AppUpdateState::RestartRequired => Some(("Restart to Finish Update".to_string(), true)),
        AppUpdateState::Downloading | AppUpdateState::Installing => {
            Some(("Updating\u{2026}".to_string(), false))
        }
        AppUpdateState::Available if installable => {
            version.map(|version| (format!("Install Update {version}\u{2026}"), true))
        }
        AppUpdateState::Failed if installable => {
            version.map(|version| (format!("Retry Update {version}\u{2026}"), true))
        }
        _ => None,
    }
}
