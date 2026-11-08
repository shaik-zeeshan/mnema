//! Behaviour shared by the Tauri and GPUI desktop apps. S1 is the skeleton: the
//! host seam, paths, events, the runtime/state container and the fixtures. The
//! modules move in from `apps/desktop/src-tauri` in S2/S3.

pub mod ai_runtime;
pub mod app_infra;
pub mod ask_ai;
pub mod audio_transcription_backfill;
pub mod audio_transcription_models;
pub mod broker_authorization_channel;
pub mod chatgpt_auth;
pub mod cli_access;
pub mod clock_format;
pub mod conversation;
mod core;
pub mod crl_refresh;
pub mod debug_health;
pub mod debug_log;
pub mod debug_pipeline;
pub mod debug_status;
pub mod disk_space;
mod event;
pub mod exit;
#[cfg(feature = "fixtures")]
pub mod fixtures;
pub mod general_app_log;
mod host;
pub mod keyboard_bindings;
pub mod licensing;
pub mod managed_storage_layout;
pub mod model_download_lock;
pub mod native_capture;
pub mod native_capture_inactivity;
pub mod native_capture_output;
pub mod native_capture_system_idle;
pub mod ocr_budget;
pub mod ocr_models;
pub mod onboarding_state;
pub mod one_time_prompts;
mod paths;
pub mod privacy_redaction_sources;
pub mod recording_settings;
pub mod semantic_search_models;
pub mod semantic_search_query;
pub mod semantic_search_worker;
pub mod sensitive_capture_recommendations;
pub mod speaker_analysis_models;
pub mod speaker_analysis_runtime;
#[cfg(test)]
mod test_support;
pub mod third_party_notices;
pub mod transcription_deepgram;
pub mod updates;
pub mod usage_charts;
pub mod user_context;
pub mod voice_enrollment;

pub use crate::core::AppCore;
pub use event::CoreEvent;
pub use host::{
    ConfirmDialog, DialogKind, ExitKind, Host, MessageDialog, WindowRequest, WindowSignal,
};
pub use paths::Paths;

/// The app's one `AppInfra` (DB, jobs, pipeline), managed once it opens.
pub type AppInfraState = std::sync::Arc<::app_infra::AppInfra>;
