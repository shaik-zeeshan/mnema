//! Behaviour shared by the Tauri and GPUI desktop apps. S1 is the skeleton: the
//! host seam, paths, events, the runtime/state container and the fixtures. The
//! modules move in from `apps/desktop/src-tauri` in S2/S3.

mod core;
pub mod debug_log;
mod event;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod host;
pub mod model_download_lock;
mod paths;
pub mod recording_settings;
#[cfg(test)]
mod test_support;

pub use crate::core::AppCore;
pub use event::CoreEvent;
pub use host::{
    ConfirmDialog, DialogKind, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
pub use paths::Paths;

/// The app's one `AppInfra` (DB, jobs, pipeline), managed once it opens.
pub type AppInfraState = std::sync::Arc<app_infra::AppInfra>;
