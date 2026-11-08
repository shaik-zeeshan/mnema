//! Behaviour shared by the Tauri and GPUI desktop apps. S1 is the skeleton: the
//! host seam, paths, events, the runtime/state container and the fixtures. The
//! modules move in from `apps/desktop/src-tauri` in S2/S3.

mod core;
mod event;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod host;
mod paths;

pub use crate::core::AppCore;
pub use event::CoreEvent;
pub use host::{
    ConfirmDialog, DialogKind, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
pub use paths::Paths;
