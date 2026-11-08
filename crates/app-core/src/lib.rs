//! Behaviour shared by the Tauri and GPUI desktop apps. S1 is the skeleton: the
//! host seam, paths, events, the runtime/state container and the fixtures. The
//! modules move in from `apps/desktop/src-tauri` in S2/S3.

pub mod chatgpt_auth;
pub mod clock_format;
mod core;
pub mod crl_refresh;
pub mod debug_log;
pub mod debug_pipeline;
pub mod debug_status;
pub mod disk_space;
mod event;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod general_app_log;
mod host;
pub mod keyboard_bindings;
pub mod licensing;
pub mod managed_storage_layout;
pub mod model_download_lock;
pub mod native_capture_inactivity;
pub mod native_capture_output;
pub mod native_capture_system_idle;
pub mod ocr_budget;
pub mod one_time_prompts;
mod paths;
pub mod recording_settings;
pub mod speaker_analysis_runtime;
#[cfg(test)]
mod test_support;
pub mod third_party_notices;
pub mod transcription_deepgram;
pub mod usage_charts;

pub use crate::core::AppCore;
pub use event::CoreEvent;
pub use host::{
    ConfirmDialog, DialogKind, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
pub use paths::Paths;

/// The app's one `AppInfra` (DB, jobs, pipeline), managed once it opens.
pub type AppInfraState = std::sync::Arc<app_infra::AppInfra>;
