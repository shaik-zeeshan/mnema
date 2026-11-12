//! The capture runtime both hosts drive: the Recording Lifecycle and its
//! segment machinery, the start/stop/pause entry points, recording settings
//! saves, permissions, wake recovery and the not-recording reminder. Each host
//! keeps only its command adapters and the native notifier registrations.

pub mod activity;
pub mod app_candidates;
pub mod app_notifications;
pub mod auto_start;
#[cfg(target_os = "macos")]
pub mod browser_url_ax;
pub mod capture_health;
pub mod control;
pub mod frame_artifacts;
pub mod lifecycle;
mod log_format;
pub mod metadata;
pub mod microphone;
#[cfg(target_os = "macos")]
pub mod microphone_away;
pub mod model_warnings;
pub mod not_recording_reminder;
pub mod os_notifiers;
pub mod permissions;
pub mod privacy;
pub mod runtime;
mod segments;
pub mod settings;
pub mod settings_update;
#[cfg(target_os = "macos")]
pub mod system_audio;
#[cfg(target_os = "macos")]
mod system_audio_away;
pub mod system_wake;
#[cfg(test)]
mod tests;

use crate::{AppCore, CoreEvent};

/// The frontend event a committed audio segment raises ([`CoreEvent::AudioSegmentsChanged`]).
pub const AUDIO_SEGMENTS_CHANGED_EVENT: &str = "audio_segments_changed";
pub const SYSTEM_DID_WAKE_EVENT: &str = "system_did_wake";
// Screens-asleep is a render-idle signal for the frontend: WebKit strands a
// non-purgeable IOSurface for every layer repaint that happens while the
// window isn't composited to a lit display (FB16462982), so the frontend
// pauses its periodic DOM updates between these two events.
pub const SCREENS_DID_SLEEP_EVENT: &str = "screens_did_sleep";
pub const SCREENS_DID_WAKE_EVENT: &str = "screens_did_wake";
pub const RECORDING_SETTINGS_CHANGED_EVENT: &str = "recording_settings_changed";
pub const RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT: &str = "recording_settings_domain_changed";
pub const NATIVE_CAPTURE_SESSION_CHANGED_EVENT: &str = "native_capture_session_changed";

/// Broadcasts the session's state (`native_capture_session_changed`).
pub fn emit_native_capture_session_changed(
    core: &AppCore,
    session: &capture_types::NativeCaptureSession,
) {
    core.host().emit(CoreEvent::NativeCaptureSessionChanged(
        serde_json::to_value(session).unwrap_or(serde_json::Value::Null),
    ));
}
