//! Settings' one backend door. `load` fills [`SettingsData`] from `AppCore` on
//! tokio; saves go through [`Backend`] on the background executor, so tests and
//! goldens swap in `fixture::FixtureBackend`. Commands still in src-tauri answer
//! empty here, each with a `shortcut:` line naming the slice that moves it.

use std::sync::Arc;

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::{AppCore, AppInfraState};
use capture_types::{MicrophoneControllerState, RecordingSettings};
use serde::Serialize;
use serde_json::{json, Value};

use super::autosave::Unit;
use super::model::SettingsData;

fn to_json(v: impl Serialize) -> Option<Value> {
    serde_json::to_value(v).ok()
}

pub async fn load(core: &AppCore) -> SettingsData {
    let mut d = SettingsData::default();
    d.settings = core
        .try_state::<RecordingSettingsState>()
        .map(|s| current_recording_settings(&s));
    d.keyboard = to_json(app_core::keyboard_bindings::load_settings_from_disk(core));
    let infra = core.try_state::<AppInfraState>();
    if let Some(infra) = &infra {
        d.license = app_core::licensing::get_license_status(core, infra)
            .await
            .ok();
        d.user_context = app_core::user_context::commands::get_user_context_status(core, infra)
            .await
            .ok()
            .and_then(to_json);
    }
    d.storage_usage = to_json(app_core::managed_storage_layout::get_storage_usage(
        core,
        Some(true),
    ));
    d.storage_location = app_core::app_infra::get_storage_location(core).ok();
    d.ai_status = app_core::ai_runtime::get_ai_runtime_status(core)
        .await
        .ok()
        .and_then(to_json);
    if let Some(s) = &d.settings {
        for p in &s.ai_runtime.providers {
            let req = || serde_json::from_value(json!({ "provider": p.id })).ok();
            if let Some(r) = req() {
                if app_core::ai_runtime::ai_runtime_has_provider_key(r).await == Ok(true) {
                    d.keys_saved.insert(p.id.clone());
                }
            }
            if let Some(r) = req() {
                if let Ok(Some(end)) = app_core::ai_runtime::ai_runtime_provider_key_ending(r).await
                {
                    d.key_endings.insert(p.id.clone(), end);
                }
            }
        }
        for m in &s.ai_runtime.mcp_servers {
            if let Ok(r) = serde_json::from_value(json!({ "id": m.id })) {
                if app_core::ai_runtime::mcp_has_server_secret(r).await == Ok(true) {
                    d.mcp_secrets.insert(m.id.clone());
                }
            }
        }
    }
    d.ocr_models = app_core::ocr_models::get_ocr_model_status(core)
        .ok()
        .and_then(to_json);
    d.transcription_models =
        app_core::audio_transcription_models::get_audio_transcription_model_status(core)
            .ok()
            .and_then(to_json);
    d.speaker_models = app_core::speaker_analysis_models::get_speaker_analysis_model_status(core)
        .ok()
        .and_then(to_json);
    d.semantic_models = app_core::semantic_search_models::get_semantic_search_model_status(core)
        .ok()
        .and_then(to_json);
    d.cli_status = app_core::cli_access::get_cli_status_inner()
        .await
        .ok()
        .and_then(to_json);
    if let Ok(file) = app_core::cli_access::list_cli_access_grants(core).await {
        d.grants = to_json(file)
            .and_then(|v| v["grants"].as_array().cloned())
            .unwrap_or_default();
    }
    if let Ok(file) = app_core::cli_access::list_cli_access_history(core).await {
        d.history = to_json(file)
            .and_then(|v| v["events"].as_array().cloned())
            .unwrap_or_default();
    }
    d.log_status = app_core::general_app_log::status(core)
        .ok()
        .and_then(to_json);
    d.notices = to_json(app_core::third_party_notices::collect_third_party_notices());
    // shortcut: get_microphone_controller_state, get_capture_permissions, get_capture_support
    // and get_system_audio_access_hint are native_capture (src-tauri) until S3; the mic
    // section shows "Couldn't load microphone state" and the permission rows hide.
    // shortcut: ask_ai_availability is src-tauri ask_ai until S3; Ask AI reads "Checking…".
    // shortcut: get_app_update_status needs the updater runtime (src-tauri app_updates.rs)
    // until S16; Updates shows "checking…".
    // shortcut: get_account_owner_person_id is src-tauri voice_enrollment until S3.
    // shortcut: list_privacy_app_candidates is native_capture until S3 (Add app… lists nothing).
    d
}

/// A model family's downloadable store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Ocr,
    Transcription,
    Speaker,
}

/// `start_*_model_download`. Progress arrives as `*_download_progress` events.
pub fn start_download(
    core: &Arc<AppCore>,
    family: Family,
    provider: String,
    model_id: String,
) -> Result<(), String> {
    match family {
        Family::Ocr => app_core::ocr_models::start_ocr_model_download(
            core,
            app_core::ocr_models::OcrModelDownloadRequestDto { provider, model_id },
        )
        .map(|_| ()),
        Family::Transcription => {
            let infra = core
                .try_state::<AppInfraState>()
                .ok_or("app infrastructure isn't ready")?;
            app_core::audio_transcription_models::start_audio_transcription_model_download(
                core,
                app_core::audio_transcription_models::AudioTranscriptionModelDownloadRequestDto {
                    provider,
                    model_id,
                },
                &infra,
            )
            .map(|_| ())
        }
        Family::Speaker => {
            app_core::speaker_analysis_models::start_speaker_analysis_model_download(
                core,
                app_core::speaker_analysis_models::SpeakerAnalysisModelDownloadRequestDto {
                    provider,
                    model_id,
                },
            )
            .map(|_| ())
        }
    }
}

/// Settings' writes. Synchronous: the view calls them off the main thread.
pub trait Backend: Send + Sync + 'static {
    /// One autosave unit's persist; answers the canonical settings.
    fn save_domain(
        &self,
        unit: Unit,
        draft: &RecordingSettings,
    ) -> Result<RecordingSettings, String>;
    /// `update_keyboard_bindings_settings`; answers the validated bindings.
    fn save_keyboard(&self, bindings: &Value) -> Result<Value, String>;
    /// A privacy command (`set_privacy_excluded_app_enabled`, `remove_privacy_excluded_app`,
    /// `set_privacy_filter_system_audio`): `next` is the settings after the app-core mutation.
    fn save_privacy(&self, next: &RecordingSettings) -> Result<RecordingSettings, String> {
        let _ = next;
        // shortcut: privacy commands apply through native_capture (the capture filter)
        // in src-tauri until S3; the mutation itself is app-core's.
        Err("privacy commands aren't in app-core until S3".into())
    }
    /// `update_microphone_controller`.
    fn save_mic(
        &self,
        state: &MicrophoneControllerState,
    ) -> Result<MicrophoneControllerState, String>;
}

pub struct CoreBackend(pub Arc<AppCore>);

impl Backend for CoreBackend {
    fn save_domain(&self, unit: Unit, _: &RecordingSettings) -> Result<RecordingSettings, String> {
        // shortcut: update_*_settings apply through native_capture_settings.rs (capture
        // runtime, workers, events) in src-tauri until S3 moves it; until then every
        // recording-domain save fails honestly ("Couldn't save" + Retry).
        Err(format!("{} isn't in app-core until S3", unit.command()))
    }

    fn save_keyboard(&self, bindings: &Value) -> Result<Value, String> {
        use app_core::keyboard_bindings::{persist_settings, KeyboardBindingsSettings};
        let request: KeyboardBindingsSettings =
            serde_json::from_value(bindings.clone()).map_err(|e| e.to_string())?;
        let settings = request.validated_for_update()?;
        persist_settings(&self.0, &settings)?;
        let payload = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
        // shortcut: global hotkey re-registration is S7 (native layer); the in-app
        // keymap rebinds from this event.
        self.0
            .host()
            .emit(app_core::CoreEvent::KeyboardBindingsSettingsChanged(
                payload.clone(),
            ));
        Ok(payload)
    }

    fn save_mic(&self, _: &MicrophoneControllerState) -> Result<MicrophoneControllerState, String> {
        // shortcut: update_microphone_controller drives the capture runtime (src-tauri) until S3.
        Err("update_microphone_controller isn't in app-core until S3".into())
    }
}

/// No `AppCore` (a window without one): every save fails.
pub struct NoBackend;

impl Backend for NoBackend {
    fn save_domain(&self, unit: Unit, _: &RecordingSettings) -> Result<RecordingSettings, String> {
        Err(format!("{}: no app core", unit.command()))
    }
    fn save_keyboard(&self, _: &Value) -> Result<Value, String> {
        Err("no app core".into())
    }
    fn save_mic(&self, _: &MicrophoneControllerState) -> Result<MicrophoneControllerState, String> {
        Err("no app core".into())
    }
}
