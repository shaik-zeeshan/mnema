mod conversation;
mod inactivity;
mod licensing;
mod logs;
mod microphone;
mod recording;
mod session;
mod usage_charts;
mod user_context;

use serde::Serialize;

pub use capture_metadata::{BrowserUrlMode, ExcludedAppEntry, MetadataSettings, PrivacySettings};
pub use conversation::*;
pub use inactivity::*;
pub use licensing::*;
pub use logs::*;
pub use microphone::*;
pub use recording::*;
pub use session::*;
pub use usage_charts::*;
pub use user_context::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureErrorResponse {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
