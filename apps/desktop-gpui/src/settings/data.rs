//! Settings' one backend door. `load_local` then `load_slow` fill
//! [`SettingsData`] from `AppCore` on tokio; saves go through [`Backend`] on the
//! background executor, so tests and goldens swap in `fixture::FixtureBackend`.
//! The live calls are in `live.rs`.

use std::fmt::Display;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use app_core::recording_settings::{current_recording_settings, RecordingSettingsState};
use app_core::{AppCore, AppInfraState};
use capture_types::{AiProviderConfig, MicrophoneControllerState, RecordingSettings};
use serde::Serialize;
use serde_json::{json, Value};

use super::autosave::Unit;
use super::model::SettingsData;
use super::privacy_prompt::{Mark, PromptData};

fn to_json(v: impl Serialize) -> Option<Value> {
    serde_json::to_value(v).ok()
}

/// Bound on each vault, keychain or network read: one that hangs (a keychain
/// prompt nobody answers, a dead local endpoint) gives up instead of holding the rest.
pub const SLOW_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// One slow read, bounded. A failure or a timeout logs one warn line naming
/// the read (error codes and paths only, never a key) and answers `None`.
async fn slow<T, E: Display>(what: &str, read: impl Future<Output = Result<T, E>>) -> Option<T> {
    match tokio::time::timeout(SLOW_READ_TIMEOUT, read).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            log::warn!("settings: couldn't read {what}: {e}");
            None
        }
        Err(_) => {
            log::warn!("settings: {what} timed out after {SLOW_READ_TIMEOUT:?}");
            None
        }
    }
}

/// The cheap local reads (settings files, in-memory state, OS permission and
/// support queries): they land first, before any key store or network answers.
pub fn load_local(core: &Arc<AppCore>) -> SettingsData {
    let mut d = SettingsData::default();
    d.settings = core
        .try_state::<RecordingSettingsState>()
        .map(|s| current_recording_settings(&s));
    d.keyboard = to_json(app_core::keyboard_bindings::load_settings_from_disk(core));
    d.storage_usage = to_json(app_core::managed_storage_layout::get_storage_usage(
        core,
        Some(true),
    ));
    match app_core::app_infra::get_storage_location(core) {
        Ok(l) => d.storage_location = Some(l),
        Err(e) => d.storage_location_error = Some(e),
    }
    // A failed read keeps its reason for the section's "Couldn't load …" row.
    let mut status = |family: Family, read: Result<Value, String>| match read {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("settings: couldn't read the {family:?} model status: {e}");
            d.model_errors.insert(family, e);
            None
        }
    };
    let json = |v: Option<Value>| v.ok_or_else(|| "unreadable status".to_string());
    let ocr = app_core::ocr_models::get_ocr_model_status(core).and_then(|v| json(to_json(v)));
    let tx = app_core::audio_transcription_models::get_audio_transcription_model_status(core)
        .and_then(|v| json(to_json(v)));
    let spk = app_core::speaker_analysis_models::get_speaker_analysis_model_status(core)
        .and_then(|v| json(to_json(v)));
    let sem = app_core::semantic_search_models::get_semantic_search_model_status(core)
        .and_then(|v| json(to_json(v)));
    let (ocr, tx, spk, sem) = (
        status(Family::Ocr, ocr),
        status(Family::Transcription, tx),
        status(Family::Speaker, spk),
        status(Family::Semantic, sem),
    );
    d.ocr_models = ocr;
    d.transcription_models = tx;
    d.speaker_models = spk;
    d.semantic_models = sem;
    d.os_locale = crate::native::locale::preferred_language();
    d.log_status = app_core::general_app_log::status(core)
        .ok()
        .and_then(to_json);
    d.notices = to_json(app_core::third_party_notices::collect_third_party_notices());
    use app_core::native_capture::permissions as perms;
    match app_core::native_capture::microphone::controller::get_microphone_controller_state(core) {
        Ok(mic) => d.mic = Some(mic),
        Err(e) => log::warn!(
            "settings: couldn't read the microphone state: {}",
            e.message
        ),
    }
    d.capture_support = to_json(perms::get_capture_support());
    load_access(core, &mut d);
    d.update = app_core::updates::current_status(core).and_then(to_json);
    d
}

/// What changes outside the app (System Settings): capture permissions, the
/// system-audio evidence hint, the browser-URL Accessibility grant. Local reads.
pub fn load_access(core: &Arc<AppCore>, d: &mut SettingsData) {
    use app_core::native_capture::permissions as perms;
    d.permissions = to_json(perms::get_capture_permissions(core).permissions);
    d.system_audio_blocked = perms::get_system_audio_access_hint(core).should_show;
    d.gecko = to_json(perms::get_browser_url_accessibility_status());
}

/// The vault, keychain, database and network reads on top of `d` (the local
/// answers), each bounded by [`SLOW_READ_TIMEOUT`]; `complete` marks them done.
pub async fn load_slow(core: &Arc<AppCore>, mut d: SettingsData) -> SettingsData {
    let infra = core.try_state::<AppInfraState>();
    if let Some(infra) = &infra {
        d.license = slow(
            "the license status",
            app_core::licensing::get_license_status(core, infra),
        )
        .await;
        // Debug builds answer a fixed dev licence (app-core's bypass, same test):
        // there is no real key to count devices for, and the count reads the
        // stored key from the keychain, so it never runs there.
        let dev_licence =
            cfg!(debug_assertions) && std::env::var_os("MNEMA_LICENSE_ENFORCE").is_none();
        if !dev_licence
            && matches!(
                d.license,
                Some(capture_types::LicenseStatus::Licensed { .. })
            )
        {
            let devices = app_core::licensing::reset::get_license_devices();
            d.license_devices = slow("the license devices", devices)
                .await
                .flatten()
                .map(|v| (v.used, v.cap));
        }
        d.user_context = slow(
            "the user context status",
            app_core::user_context::commands::get_user_context_status(core, infra),
        )
        .await
        .and_then(to_json);
    }
    // A failed read keeps its reason for the AI runtime row.
    match tokio::time::timeout(
        SLOW_READ_TIMEOUT,
        app_core::ai_runtime::get_ai_runtime_status(core),
    )
    .await
    {
        Ok(Ok(v)) => d.ai_status = to_json(v),
        Ok(Err(e)) => {
            log::warn!("settings: couldn't read the AI runtime status: {e}");
            // `humanizeError`: the first letter up.
            let e = e.to_string();
            let mut c = e.chars();
            d.ai_status_error = c
                .next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>());
        }
        Err(_) => {
            log::warn!("settings: the AI runtime status timed out after {SLOW_READ_TIMEOUT:?}");
            d.ai_status_error = Some("Timed out reading the AI runtime status.".into());
        }
    }
    if let Some(s) = d.settings.clone() {
        for p in &s.ai_runtime.providers {
            let req = || serde_json::from_value(json!({ "provider": p.id })).ok();
            if let Some(r) = req() {
                let has = app_core::ai_runtime::ai_runtime_has_provider_key(r);
                if slow("a provider key", has).await == Some(true) {
                    d.keys_saved.insert(p.id.clone());
                }
            }
            if let Some(r) = req() {
                let ending = app_core::ai_runtime::ai_runtime_provider_key_ending(r);
                if let Some(Some(end)) = slow("a provider key ending", ending).await {
                    d.key_endings.insert(p.id.clone(), end);
                }
            }
        }
        for m in &s.ai_runtime.mcp_servers {
            if let Ok(r) = serde_json::from_value(json!({ "id": m.id })) {
                let has = app_core::ai_runtime::mcp_has_server_secret(r);
                if slow("a connector secret", has).await == Some(true) {
                    d.mcp_secrets.insert(m.id.clone());
                }
            }
        }
    }
    d.cli_status = slow(
        "the CLI install",
        app_core::cli_access::get_cli_status_inner(),
    )
    .await
    .and_then(to_json);
    if let Some(file) = slow(
        "the CLI access grants",
        app_core::cli_access::list_cli_access_grants(core),
    )
    .await
    {
        d.grants = to_json(file)
            .and_then(|v| v["grants"].as_array().cloned())
            .unwrap_or_default();
    }
    if let Some(file) = slow(
        "the CLI access history",
        app_core::cli_access::list_cli_access_history(core),
    )
    .await
    {
        d.history = to_json(file)
            .and_then(|v| v["events"].as_array().cloned())
            .unwrap_or_default();
    }
    d.ask_ai = slow(
        "Ask AI availability",
        app_core::ask_ai::ask_ai_availability(core),
    )
    .await
    .and_then(to_json);
    if let Some(infra) = &infra {
        let people = slow(
            "the person profiles",
            app_core::app_infra::list_person_profiles(infra),
        )
        .await
        .and_then(to_json);
        d.person_count = people
            .as_ref()
            .and_then(|p| p.as_array())
            .map_or(0, Vec::len);
        d.voice_checked = true;
        let owner = app_core::voice_enrollment::get_account_owner_person_id(infra);
        if let Some(Some(owner)) = slow("the voice owner", owner).await {
            d.voice_owner_id = Some(owner);
            d.voice_owner = people
                .as_ref()
                .and_then(|p| p.as_array())
                .and_then(|p| p.iter().find(|p| p["id"].as_i64() == Some(owner)))
                .and_then(|p| p["displayName"].as_str().map(str::to_string))
                .or(Some("You".into()));
        }
    }
    let oauth = app_core::ask_ai::mcp::oauth_flow::mcp_oauth_statuses(core);
    if let Some(statuses) = slow("the connector sign-ins", oauth).await {
        d.mcp_oauth = to_json(statuses)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
    }
    let deepgram = app_core::transcription_deepgram::transcription_has_deepgram_key();
    d.deepgram_key = slow("the Deepgram key", deepgram).await == Some(true);
    if let Some(auth) =
        core.try_state::<app_core::transcription_deepgram::DeepgramAuthStatusState>()
    {
        let status = app_core::transcription_deepgram::transcription_deepgram_auth_status(&auth);
        d.deepgram_auth = slow("the Deepgram auth status", status).await.flatten();
    }
    if core.try_state::<RecordingSettingsState>().is_some() {
        let recs =
            app_core::sensitive_capture_recommendations::sensitive_capture_recommendations_now(
                core,
            );
        d.privacy_prompt = slow("the sensitive-app recommendations", recs)
            .await
            .map(|r| {
                let state = app_core::one_time_prompts::get_one_time_prompt_state(core);
                PromptData::from_core(&r, &state)
            });
    }
    d.complete = true;
    d
}

/// A model family's downloadable store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Ocr,
    Transcription,
    Speaker,
    Semantic,
}

/// `cancel_*_model_download` for the family's one in-flight download.
pub fn cancel_download(core: &Arc<AppCore>, family: Family) -> Result<(), String> {
    match family {
        Family::Ocr => app_core::ocr_models::cancel_ocr_model_download(core),
        Family::Transcription => {
            app_core::audio_transcription_models::cancel_audio_transcription_model_download(core)
        }
        Family::Speaker => {
            app_core::speaker_analysis_models::cancel_speaker_analysis_model_download(core)
        }
        Family::Semantic => {
            app_core::semantic_search_models::cancel_semantic_search_model_download(
                &core.state::<app_core::semantic_search_models::SemanticSearchModelDownloadState>(),
            )
        }
    }
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
        Family::Semantic => app_core::semantic_search_models::start_semantic_search_model_download(
            core,
            app_core::semantic_search_models::SemanticSearchModelDownloadRequestDto {
                provider,
                model_id,
            },
        )
        .map(|_| ()),
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
        Err("no app core".into())
    }
    /// `mark_one_time_prompt_shown` / `dismiss_` / `complete_one_time_prompt`.
    fn prompt(&self, id: &str, mark: Mark) -> Result<(), String> {
        let _ = (id, mark);
        Err("no app core".into())
    }
    /// Window focus: [`load_access`] only (`None` = nothing to re-read).
    fn recheck_access(&self) -> Option<SettingsData> {
        None
    }
    /// `update_microphone_controller`.
    fn save_mic(
        &self,
        state: &MicrophoneControllerState,
    ) -> Result<MicrophoneControllerState, String>;
    /// `verify_ai_provider`: list one provider's models live, against the
    /// draft list (an unsaved base URL counts). `Err` = the short reason.
    fn verify_provider(&self, id: &str, providers: &[AiProviderConfig]) -> Result<(), String> {
        let _ = (id, providers);
        Err("no app core".into())
    }
    /// `ai_runtime_set_provider_key` into the vault, by instance id; answers the
    /// stored key's last 4 (`ai_runtime_provider_key_ending`). Never logs the key.
    fn save_key(&self, provider: &str, key: &str) -> Result<Option<String>, String> {
        let _ = (provider, key);
        Err("no app core".into())
    }
    /// `ai_runtime_clear_provider_key` (also revokes a ChatGPT token set).
    fn clear_key(&self, provider: &str) -> Result<(), String> {
        let _ = provider;
        Err("no app core".into())
    }
    /// `ai_runtime_chatgpt_begin_login`: `(user code, verify URL)`. The outcome
    /// arrives later as `CoreEvent::ChatgptLoginUpdate`.
    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String> {
        let _ = provider;
        Err("no app core".into())
    }
    /// `ai_runtime_chatgpt_cancel_login` (best effort).
    fn chatgpt_cancel(&self, provider: &str) {
        let _ = provider;
    }
    /// `ai_runtime_test_connection`: the `AiRuntimeTestResult` JSON.
    fn test_connection(&self) -> Result<Value, String> {
        Err("no app core".into())
    }
    /// `list_semantic_search_supported_models`: the whole embedding catalog (DTO JSON).
    fn semantic_catalog(&self) -> Result<Vec<Value>, String> {
        Err("no app core".into())
    }
    /// `select_semantic_search_model`: switch models and clear the old vectors
    /// (the index rebuilds in the background); answers how many were cleared.
    fn select_semantic_model(&self, model_id: &str) -> Result<u64, String> {
        let _ = model_id;
        Err("no app core".into())
    }
    /// `record_bounded_microphone_clip`: opens the microphone for `ms`, blocks
    /// for the take, answers the clip path, `None` when [`Self::cancel_clip`]
    /// ended it (`Err` = what to do, in words).
    fn record_clip(&self, ms: u64) -> Result<Option<String>, String> {
        let _ = ms;
        Err("no app core".into())
    }
    /// End a running take now (Cancel, leaving the section, closing Settings).
    fn cancel_clip(&self) {}
    /// `get_microphone_activity_level` (the level meter; `None` = no sample yet).
    fn mic_level(&self) -> Option<f32> {
        None
    }
    /// `enroll_account_owner_voice` on a recorded take.
    fn enroll(&self, clip: &str) -> Result<crate::voice::Enrollment, String> {
        let _ = clip;
        Err("no app core".into())
    }
    /// A player on a recorded take (Play the take).
    fn open_clip(
        &self,
        clip: &str,
    ) -> Result<Box<dyn crate::timeline::audio::transport::Player + Send>, String> {
        let _ = clip;
        Err("no app core".into())
    }
    /// `mcp_list_server_tools`: connects the connector (an npx cold boot takes
    /// seconds) and answers its tools as `(name, description)`.
    fn mcp_tools(&self, id: &str) -> Result<Vec<(String, Option<String>)>, String> {
        let _ = id;
        Err("no app core".into())
    }
    /// `mcp_set_server_secret` into the vault, by connector id. Never logs the secret.
    fn mcp_set_secret(&self, id: &str, secret: &str) -> Result<(), String> {
        let _ = (id, secret);
        Err("no app core".into())
    }
    /// `mcp_clear_server_secret`.
    fn mcp_clear_secret(&self, id: &str) -> Result<(), String> {
        let _ = id;
        Err("no app core".into())
    }
    /// `mcp_oauth_begin`: app-core opens the browser at the consent page; the
    /// callback lands later as `McpAuthorizationChanged`.
    fn mcp_oauth_begin(&self, id: &str) -> Result<(), String> {
        let _ = id;
        Err("no app core".into())
    }
    /// `mcp_oauth_statuses`: `{ id, state }` per OAuth connector (`None` = unread).
    fn mcp_oauth_statuses(&self) -> Option<Vec<Value>> {
        None
    }
    /// `mcp_check_node`: `Ok(None)` = Node isn't on the login-shell PATH.
    fn mcp_check_node(&self) -> Result<Option<String>, String> {
        Err("no app core".into())
    }
}

pub struct CoreBackend(pub Arc<AppCore>);

impl Backend for CoreBackend {
    fn recheck_access(&self) -> Option<SettingsData> {
        let mut d = SettingsData::default();
        load_access(&self.0, &mut d);
        // A tool approved in the CLI Access Request window meanwhile: the list
        // has no change event, so focus re-reads it (`complete` = it was read).
        let core = &self.0;
        let grants = core.block_on(app_core::cli_access::list_cli_access_grants(core));
        let history = core.block_on(app_core::cli_access::list_cli_access_history(core));
        if let (Ok(g), Ok(h)) = (grants, history) {
            d.grants = to_json(g)
                .and_then(|v| v["grants"].as_array().cloned())
                .unwrap_or_default();
            d.history = to_json(h)
                .and_then(|v| v["events"].as_array().cloned())
                .unwrap_or_default();
            d.complete = true;
        }
        Some(d)
    }
    fn save_domain(
        &self,
        unit: Unit,
        draft: &RecordingSettings,
    ) -> Result<RecordingSettings, String> {
        super::live::save_domain(&self.0, unit, draft)
    }

    fn save_privacy(&self, next: &RecordingSettings) -> Result<RecordingSettings, String> {
        super::live::save_privacy(&self.0, next)
    }

    fn prompt(&self, id: &str, mark: Mark) -> Result<(), String> {
        use app_core::one_time_prompts as p;
        let id = id.to_string();
        match mark {
            Mark::Shown => p::mark_one_time_prompt_shown(id, &self.0),
            Mark::Dismissed => p::dismiss_one_time_prompt(id, &self.0),
            Mark::Completed => p::complete_one_time_prompt(id, &self.0),
        }
        .map(|_| ())
    }

    fn save_keyboard(&self, bindings: &Value) -> Result<Value, String> {
        use app_core::keyboard_bindings::{persist_settings, KeyboardBindingsSettings};
        let request: KeyboardBindingsSettings =
            serde_json::from_value(bindings.clone()).map_err(|e| e.to_string())?;
        let settings = request.validated_for_update()?;
        persist_settings(&self.0, &settings)?;
        let payload = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
        // The native layer re-registers the global shortcuts from this event.
        self.0
            .host()
            .emit(app_core::CoreEvent::KeyboardBindingsSettingsChanged(
                payload.clone(),
            ));
        Ok(payload)
    }

    fn save_mic(
        &self,
        state: &MicrophoneControllerState,
    ) -> Result<MicrophoneControllerState, String> {
        super::live::save_mic(&self.0, state)
    }

    fn verify_provider(&self, id: &str, providers: &[AiProviderConfig]) -> Result<(), String> {
        let req = serde_json::from_value(json!({ "provider": id, "providers": providers }))
            .map_err(|e| e.to_string())?;
        self.0
            .block_on(app_core::ai_runtime::verify_ai_provider(&self.0, req))
            .map(|_| ())
    }

    fn save_key(&self, provider: &str, key: &str) -> Result<Option<String>, String> {
        use app_core::ai_runtime::{ai_runtime_provider_key_ending, ai_runtime_set_provider_key};
        let request = serde_json::from_value(json!({ "provider": provider, "key": key }))
            .map_err(|_| "couldn't build the key request".to_string())?;
        self.0.block_on(ai_runtime_set_provider_key(request))?;
        let ending = serde_json::from_value(json!({ "provider": provider }))
            .ok()
            .and_then(|r| self.0.block_on(ai_runtime_provider_key_ending(r)).ok())
            .flatten();
        Ok(ending)
    }

    fn clear_key(&self, provider: &str) -> Result<(), String> {
        let request =
            serde_json::from_value(json!({ "provider": provider })).map_err(|e| e.to_string())?;
        self.0
            .block_on(app_core::ai_runtime::ai_runtime_clear_provider_key(request))
    }

    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String> {
        let request =
            serde_json::from_value(json!({ "provider": provider })).map_err(|e| e.to_string())?;
        let p = self
            .0
            .block_on(app_core::ai_runtime::ai_runtime_chatgpt_begin_login(
                &self.0, request,
            ))?;
        Ok((p.user_code, p.verify_url))
    }

    fn chatgpt_cancel(&self, provider: &str) {
        if let Ok(request) = serde_json::from_value(json!({ "provider": provider })) {
            let _ = self
                .0
                .block_on(app_core::ai_runtime::ai_runtime_chatgpt_cancel_login(
                    request,
                ));
        }
    }

    fn test_connection(&self) -> Result<Value, String> {
        let r = self
            .0
            .block_on(app_core::ai_runtime::ai_runtime_test_connection(&self.0))?;
        serde_json::to_value(r).map_err(|e| e.to_string())
    }

    fn semantic_catalog(&self) -> Result<Vec<Value>, String> {
        app_core::semantic_search_models::list_semantic_search_supported_models()
            .map(|models| models.into_iter().filter_map(to_json).collect())
    }

    fn select_semantic_model(&self, model_id: &str) -> Result<u64, String> {
        use app_core::semantic_search_models::select_semantic_search_model;
        let infra = self
            .0
            .try_state::<AppInfraState>()
            .ok_or("app infrastructure isn't ready")?;
        let settings = self
            .0
            .try_state::<RecordingSettingsState>()
            .ok_or("settings aren't loaded")?;
        self.0.block_on(select_semantic_search_model(
            model_id.into(),
            &self.0,
            &infra,
            &settings,
        ))
    }

    fn record_clip(&self, ms: u64) -> Result<Option<String>, String> {
        crate::voice::live::record(&self.0, ms)
    }

    fn cancel_clip(&self) {
        crate::voice::live::cancel();
    }

    fn mic_level(&self) -> Option<f32> {
        app_core::native_capture::microphone::bounded_clip::get_microphone_activity_level()
    }

    fn enroll(&self, clip: &str) -> Result<crate::voice::Enrollment, String> {
        let infra = self
            .0
            .try_state::<AppInfraState>()
            .ok_or("app infrastructure isn't ready")?;
        crate::voice::live::enroll(&self.0, &infra, clip)
    }

    fn open_clip(
        &self,
        clip: &str,
    ) -> Result<Box<dyn crate::timeline::audio::transport::Player + Send>, String> {
        crate::native::audio_player::AudioPlayer::open(std::path::Path::new(clip))
            .map(|p| Box::new(p) as Box<dyn crate::timeline::audio::transport::Player + Send>)
    }

    fn mcp_tools(&self, id: &str) -> Result<Vec<(String, Option<String>)>, String> {
        use app_core::ask_ai::mcp::{manager, node_check};
        let core = &self.0;
        match core.block_on(manager::mcp_list_server_tools(core, id.to_string())) {
            Ok(tools) => Ok(tools.into_iter().map(|t| (t.name, t.description)).collect()),
            // Local (stdio) connectors run through npx: say so when Node is missing.
            Err(e) => Err(match core.block_on(node_check::mcp_check_node()) {
                Some(_) => e,
                None => {
                    format!("{e} Node.js (npx) wasn't found on this Mac; local connectors need it.")
                }
            }),
        }
    }

    fn mcp_set_secret(&self, id: &str, secret: &str) -> Result<(), String> {
        super::live::mcp_set_secret(&self.0, id, secret).map(|_| ())
    }

    fn mcp_clear_secret(&self, id: &str) -> Result<(), String> {
        super::live::mcp_clear_secret(&self.0, id).map(|_| ())
    }

    fn mcp_oauth_begin(&self, id: &str) -> Result<(), String> {
        use app_core::ask_ai::mcp::oauth_flow::mcp_oauth_begin;
        self.0.block_on(mcp_oauth_begin(&self.0, id.to_string()))
    }

    fn mcp_oauth_statuses(&self) -> Option<Vec<Value>> {
        use app_core::ask_ai::mcp::oauth_flow::mcp_oauth_statuses;
        let statuses = self.0.block_on(mcp_oauth_statuses(&self.0)).ok()?;
        to_json(statuses).and_then(|v| v.as_array().cloned())
    }

    fn mcp_check_node(&self) -> Result<Option<String>, String> {
        Ok(self
            .0
            .block_on(app_core::ask_ai::mcp::node_check::mcp_check_node()))
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
