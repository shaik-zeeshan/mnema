//! The capture runtime both hosts drive. It moves in from the desktop crate's
//! `native_capture` in S3 parts: the leaves first, then the segment machinery
//! and the Recording Lifecycle, then the root's commands.

pub mod activity;
pub mod app_notifications;
#[cfg(target_os = "macos")]
pub mod browser_url_ax;
pub mod capture_health;
pub mod frame_artifacts;
pub mod lifecycle;
pub mod metadata;
pub mod microphone;
#[cfg(target_os = "macos")]
pub mod microphone_away;
pub mod privacy;
pub mod runtime;
mod segments;
#[cfg(target_os = "macos")]
pub mod system_audio;
#[cfg(test)]
mod tests;

use crate::{AppCore, CoreEvent};

/// The frontend event a committed audio segment raises ([`CoreEvent::AudioSegmentsChanged`]).
pub const AUDIO_SEGMENTS_CHANGED_EVENT: &str = "audio_segments_changed";

/// Broadcasts the session's state (`native_capture_session_changed`).
pub fn emit_native_capture_session_changed(
    core: &AppCore,
    session: &capture_types::NativeCaptureSession,
) {
    core.host().emit(CoreEvent::NativeCaptureSessionChanged(
        serde_json::to_value(session).unwrap_or(serde_json::Value::Null),
    ));
}
