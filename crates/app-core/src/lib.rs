//! Behaviour shared by the Tauri and GPUI desktop apps. S1 is the skeleton: the
//! host seam, paths, events, the runtime/state container and the fixtures. The
//! modules move in from `apps/desktop/src-tauri` in S2/S3.

pub mod clock_format;
mod core;
pub mod debug_log;
mod event;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod host;
pub mod licensing;
pub mod model_download_lock;
pub mod native_capture_inactivity;
pub mod native_capture_output;
pub mod native_capture_system_idle;
pub mod ocr_budget;
mod paths;
pub mod recording_settings;
pub mod speaker_analysis_runtime;
#[cfg(test)]
mod test_support;
pub mod third_party_notices;

pub use crate::core::AppCore;
pub use event::CoreEvent;
pub use host::{
    ConfirmDialog, DialogKind, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
pub use paths::Paths;

/// The app's one `AppInfra` (DB, jobs, pipeline), managed once it opens.
pub type AppInfraState = std::sync::Arc<app_infra::AppInfra>;
