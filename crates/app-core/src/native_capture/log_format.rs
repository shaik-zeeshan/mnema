//! How the capture root describes itself in the app log: source flags, session
//! ids, settings overviews and diffs, and the support/permission snapshots it
//! logs only when they change.

use std::sync::OnceLock;

use capture_types::{
    CaptureOutputFiles, CapturePermissionState, CapturePermissions, CaptureSources,
    CaptureSupportResponse, InactivityActivityMode, RecordingSettings, ScreenResolution,
    ScreenResolutionPreset, StartNativeCaptureRequest, VideoBitrateMode, VideoBitratePreset,
    VideoBitrateSettings,
};

use crate::debug_log;
use crate::native_capture::runtime;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureSupportSnapshot {
    platform: String,
    native_capture_supported: bool,
    supported_sources: CaptureSources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturePermissionsSnapshot {
    screen: &'static str,
    microphone: &'static str,
    system_audio: &'static str,
}

fn capture_support_log_snapshot_state() -> &'static std::sync::Mutex<Option<CaptureSupportSnapshot>>
{
    static LAST_CAPTURE_SUPPORT_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CaptureSupportSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_SUPPORT_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

fn capture_permissions_log_snapshot_state(
) -> &'static std::sync::Mutex<Option<CapturePermissionsSnapshot>> {
    static LAST_CAPTURE_PERMISSIONS_SNAPSHOT: OnceLock<
        std::sync::Mutex<Option<CapturePermissionsSnapshot>>,
    > = OnceLock::new();

    LAST_CAPTURE_PERMISSIONS_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(None))
}

pub(crate) fn reset_capture_log_snapshots() {
    *capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned") = None;
    *capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned") = None;
}

pub(crate) fn capture_sources_from_settings(settings: &RecordingSettings) -> CaptureSources {
    CaptureSources {
        screen: settings.capture_screen,
        microphone: settings.capture_microphone,
        system_audio: settings.capture_system_audio,
    }
}

pub(crate) fn capture_sources_from_start_request(
    request: &StartNativeCaptureRequest,
) -> CaptureSources {
    CaptureSources {
        screen: request.capture_screen,
        microphone: request.capture_microphone,
        system_audio: request.capture_system_audio,
    }
}

pub(crate) fn format_capture_source_flags(sources: &CaptureSources) -> String {
    format!(
        "screen={}, microphone={}, system_audio={}",
        sources.screen, sources.microphone, sources.system_audio
    )
}

pub(crate) fn format_optional_capture_source_flags(sources: Option<&CaptureSources>) -> String {
    sources
        .map(format_capture_source_flags)
        .unwrap_or_else(|| "screen=unknown, microphone=unknown, system_audio=unknown".to_string())
}

pub(crate) fn runtime_log_session_id(runtime: &runtime::NativeCaptureRuntime) -> &str {
    runtime
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|session| session.session_id.as_str())
        .unwrap_or("unknown")
}

pub(crate) fn session_log_session_id(session: &capture_types::NativeCaptureSession) -> &str {
    session
        .source_sessions
        .as_ref()
        .and_then(|sessions| sessions.screen.as_ref())
        .map(|source| source.session_id.as_str())
        .unwrap_or("unknown")
}

pub(crate) fn permission_state_label(state: &CapturePermissionState) -> &'static str {
    match state {
        CapturePermissionState::Granted => "granted",
        CapturePermissionState::Denied => "denied",
        CapturePermissionState::NotDetermined => "not_determined",
        CapturePermissionState::Unsupported => "unsupported",
        CapturePermissionState::Unknown => "unknown",
        CapturePermissionState::AssumedWorking => "assumed_working",
        CapturePermissionState::PossiblyBlocked => "possibly_blocked",
    }
}

pub(crate) fn format_screen_resolution(resolution: &ScreenResolution) -> String {
    match resolution {
        ScreenResolution::Preset { preset } => match preset {
            ScreenResolutionPreset::Original => "original".to_string(),
            ScreenResolutionPreset::P1080 => "1080p".to_string(),
            ScreenResolutionPreset::P720 => "720p".to_string(),
            ScreenResolutionPreset::P540 => "540p".to_string(),
        },
        ScreenResolution::Custom { width, height } => format!("{width}x{height}"),
    }
}

pub(crate) fn format_video_bitrate(settings: &VideoBitrateSettings) -> String {
    match settings.mode {
        VideoBitrateMode::Preset => {
            let preset = settings
                .preset
                .clone()
                .unwrap_or(VideoBitratePreset::Medium);
            let label = match preset {
                VideoBitratePreset::Low => "low",
                VideoBitratePreset::Medium => "medium",
                VideoBitratePreset::High => "high",
            };

            format!("preset:{label}")
        }
        VideoBitrateMode::Custom => format!("custom:{}mbps", settings.custom_mbps.unwrap_or(0)),
    }
}

pub(crate) fn inactivity_activity_mode_label(mode: &InactivityActivityMode) -> &'static str {
    match mode {
        InactivityActivityMode::SystemInputOnly => "system_input_only",
        InactivityActivityMode::SystemInputOrScreen => "system_input_or_screen",
        InactivityActivityMode::SystemInputOrScreenOrAudio => "system_input_or_screen_or_audio",
    }
}

pub(crate) fn recording_settings_overview(settings: &RecordingSettings) -> String {
    format!(
        "sources={}, auto_start={}, save_directory='{}', preview_cache_ttl_seconds={}, follow_timeline_live={}, segment_duration_seconds={}, screen_frame_rate={}, screen_resolution={}, video_bitrate={}, pause_on_inactivity={}, idle_timeout_seconds={}, microphone_activity_sensitivity={}, system_audio_activity_sensitivity={}, activity_mode={}",
        format_capture_source_flags(&capture_sources_from_settings(settings)),
        settings.auto_start,
        settings.save_directory,
        settings.preview_cache_ttl_seconds,
        settings.follow_timeline_live,
        settings.segment_duration_seconds,
        settings.screen_frame_rate,
        format_screen_resolution(&settings.screen_resolution),
        format_video_bitrate(&settings.video_bitrate),
        settings.pause_capture_on_inactivity,
        settings.idle_timeout_seconds,
        settings.microphone_activity_sensitivity,
        settings.system_audio_activity_sensitivity,
        inactivity_activity_mode_label(&settings.inactivity_activity_mode)
    )
}

pub(crate) fn describe_recording_settings_changes(
    previous: &RecordingSettings,
    next: &RecordingSettings,
) -> Vec<String> {
    let mut changes = Vec::new();
    let previous_sources = capture_sources_from_settings(previous);
    let next_sources = capture_sources_from_settings(next);

    if previous_sources != next_sources {
        changes.push(format!(
            "sources {} -> {}",
            format_capture_source_flags(&previous_sources),
            format_capture_source_flags(&next_sources)
        ));
    }

    if previous.auto_start != next.auto_start {
        changes.push(format!(
            "auto_start {} -> {}",
            previous.auto_start, next.auto_start
        ));
    }

    if previous.preview_cache_ttl_seconds != next.preview_cache_ttl_seconds {
        changes.push(format!(
            "preview_cache_ttl_seconds {} -> {}",
            previous.preview_cache_ttl_seconds, next.preview_cache_ttl_seconds
        ));
    }

    if previous.follow_timeline_live != next.follow_timeline_live {
        changes.push(format!(
            "follow_timeline_live {} -> {}",
            previous.follow_timeline_live, next.follow_timeline_live
        ));
    }

    if previous.segment_duration_seconds != next.segment_duration_seconds {
        changes.push(format!(
            "segment_duration_seconds {} -> {}",
            previous.segment_duration_seconds, next.segment_duration_seconds
        ));
    }

    if previous.screen_frame_rate != next.screen_frame_rate {
        changes.push(format!(
            "screen_frame_rate {} -> {}",
            previous.screen_frame_rate, next.screen_frame_rate
        ));
    }

    if previous.screen_resolution != next.screen_resolution {
        changes.push(format!(
            "screen_resolution {} -> {}",
            format_screen_resolution(&previous.screen_resolution),
            format_screen_resolution(&next.screen_resolution)
        ));
    }

    if previous.video_bitrate != next.video_bitrate {
        changes.push(format!(
            "video_bitrate {} -> {}",
            format_video_bitrate(&previous.video_bitrate),
            format_video_bitrate(&next.video_bitrate)
        ));
    }

    if previous.pause_capture_on_inactivity != next.pause_capture_on_inactivity {
        changes.push(format!(
            "pause_on_inactivity {} -> {}",
            previous.pause_capture_on_inactivity, next.pause_capture_on_inactivity
        ));
    }

    if previous.idle_timeout_seconds != next.idle_timeout_seconds {
        changes.push(format!(
            "idle_timeout_seconds {} -> {}",
            previous.idle_timeout_seconds, next.idle_timeout_seconds
        ));
    }

    if previous.inactivity_activity_mode != next.inactivity_activity_mode {
        changes.push(format!(
            "activity_mode {} -> {}",
            inactivity_activity_mode_label(&previous.inactivity_activity_mode),
            inactivity_activity_mode_label(&next.inactivity_activity_mode)
        ));
    }

    if previous.microphone_activity_sensitivity != next.microphone_activity_sensitivity {
        changes.push(format!(
            "microphone_activity_sensitivity {} -> {}",
            previous.microphone_activity_sensitivity, next.microphone_activity_sensitivity
        ));
    }

    if previous.system_audio_activity_sensitivity != next.system_audio_activity_sensitivity {
        changes.push(format!(
            "system_audio_activity_sensitivity {} -> {}",
            previous.system_audio_activity_sensitivity, next.system_audio_activity_sensitivity
        ));
    }

    changes
}

pub(crate) fn format_output_file_counts(output_files: Option<&CaptureOutputFiles>) -> String {
    output_files
        .map(|output_files| {
            format!(
                "screen_files={}, microphone_files={}, system_audio_files={}",
                output_files.screen_files.len(),
                output_files.microphone_files.len(),
                output_files.system_audio_files.len()
            )
        })
        .unwrap_or_else(|| "screen_files=0, microphone_files=0, system_audio_files=0".to_string())
}

pub(crate) fn log_capture_support_if_changed(response: &CaptureSupportResponse) {
    let snapshot = CaptureSupportSnapshot {
        platform: response.platform.clone(),
        native_capture_supported: response.native_capture_supported,
        supported_sources: response.supported_sources.clone(),
    };
    let mut last_snapshot = capture_support_log_snapshot_state()
        .lock()
        .expect("capture support log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log_info(format!(
        "observed native capture support (platform='{}', native_supported={}, supported_sources={})",
        snapshot.platform,
        snapshot.native_capture_supported,
        format_capture_source_flags(&snapshot.supported_sources)
    ));
}

pub(crate) fn log_capture_permissions_if_changed(permissions: &CapturePermissions) {
    let snapshot = CapturePermissionsSnapshot {
        screen: permission_state_label(&permissions.screen),
        microphone: permission_state_label(&permissions.microphone),
        system_audio: permission_state_label(&permissions.system_audio),
    };
    let mut last_snapshot = capture_permissions_log_snapshot_state()
        .lock()
        .expect("capture permissions log snapshot poisoned");

    if last_snapshot.as_ref() == Some(&snapshot) {
        return;
    }

    *last_snapshot = Some(snapshot.clone());

    debug_log::log_info(format!(
        "observed native capture permissions (screen={}, microphone={}, system_audio={})",
        snapshot.screen, snapshot.microphone, snapshot.system_audio
    ));
}

pub(crate) fn log_loaded_recording_settings(source: &str, settings: &RecordingSettings) {
    debug_log::log_info(format!(
        "loaded recording settings from {source} ({})",
        recording_settings_overview(settings)
    ));
}

pub(crate) fn log_recording_settings_changes(
    previous: &RecordingSettings,
    next: &RecordingSettings,
) {
    let changes = describe_recording_settings_changes(previous, next);

    if changes.is_empty() {
        return;
    }

    debug_log::log_info(format!(
        "updated recording settings ({})",
        changes.join(", ")
    ));
}
