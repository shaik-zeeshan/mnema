//! The capture runtime both hosts drive. It moves in from the desktop crate's
//! `native_capture` in S3 parts: the leaves first, then the segment machinery,
//! then the Recording Lifecycle.

pub mod activity;
#[cfg(target_os = "macos")]
pub mod browser_url_ax;
pub mod capture_health;
pub mod frame_artifacts;
pub mod metadata;
pub mod microphone;
#[cfg(target_os = "macos")]
pub mod microphone_away;
pub mod privacy;
pub mod runtime;
#[cfg(target_os = "macos")]
pub mod system_audio;
