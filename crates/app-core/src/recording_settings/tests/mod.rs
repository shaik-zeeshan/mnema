use super::*;
use capture_types::{
    default_inactivity_activity_mode, default_semantic_search_model_id,
    default_semantic_search_provider, default_speaker_analysis_model_id,
    AudioTranscriptionProvider, SemanticSearchSettings, SpeakerAnalysisSettings,
    UpdateAccessSettingsRequest, UpdateAiRuntimeSettingsRequest,
    UpdateCaptureSourceSettingsRequest, UpdateDeveloperSettingsRequest,
    UpdateInactivitySettingsRequest, UpdateProcessingSettingsRequest, UpdateStorageSettingsRequest,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod domain_patch;
mod load;
mod validate;

// The app log names this module, not `debug_log`, for lines logged from here.
#[test]
fn lines_logged_here_carry_this_modules_path_as_target() {
    assert_eq!(
        crate::debug_log::caller_target(file!()),
        "app_core::recording_settings::tests"
    );
}

pub(super) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    pub(super) fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("desktop-settings-{label}-{unique}"));

        fs::create_dir_all(&path).expect("test directory should be created");

        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn apply_domain_patch_for_test(
    mut settings: RecordingSettings,
    patch: RecordingSettingsDomainPatch,
) -> Result<RecordingSettings, CaptureErrorResponse> {
    apply_domain_patch_to_settings(&mut settings, patch)?;
    validate_recording_settings_with_resolution_support(
        recording_settings_request_from_settings(settings),
        true,
    )
}

/// Provider-centric AI runtime settings with the master switch ON, for the
/// wipe-flips-switch and AiRuntime patch tests.
pub(super) fn enabled_ai_runtime_settings() -> AiRuntimeSettings {
    AiRuntimeSettings {
        enabled: true,
        providers: vec![capture_types::AiProviderConfig {
            id: "anthropic".to_string(),
            kind: capture_types::AiProviderKind::Anthropic,
            label: String::new(),
            base_url: String::new(),
        }],
        default_model: Some(capture_types::AiEngineRef {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5".to_string(),
        }),
        mcp_servers: Vec::new(),
        favorite_models: Vec::new(),
        recent_models: Vec::new(),
    }
}
