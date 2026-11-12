//! Onboarding's one backend door. Synchronous: the view runs every call on the
//! background executor. `CoreBackend` calls app-core directly (no IPC);
//! `fixture::FixtureBackend` serves the parity JSON and records calls.

use std::sync::Arc;

use app_core::{AppCore, AppInfraState};
use capture_types::{
    AiProviderConfig, CapturePermissionState, CapturePermissions, RecordingSettings,
    StartNativeCaptureRequest,
};
use serde_json::{json, Value};

use super::flow::{ModelFacts, Subsystem, WorkItem};
use super::storage::Probe;

/// Everything the flow reads once at load.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub ever_saved: bool,
    pub settings: RecordingSettings,
    pub perms: CapturePermissions,
    /// Accessibility (Gecko browsers' URLs): `(trusted, installed browser names)`.
    pub gecko: Option<(bool, Vec<String>)>,
    pub models: Vec<(Subsystem, ModelFacts)>,
    /// `transcription` status providers, for the Engines tab: `(id, name, model ids+bytes)`.
    pub engines: Vec<Engine>,
    /// Semantic Search catalog: `(model id, display name, approx bytes, installed, multilingual)`.
    pub semantic: Vec<(String, String, u64, bool, bool)>,
    pub owner_enrolled: bool,
    /// The sensitive-app catalog to exclude on a first run: `(bundle id, name)`.
    pub recommended: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engine {
    pub provider: String,
    pub name: String,
    /// `(model id, display name, download bytes, installed)`.
    pub models: Vec<(Option<String>, String, u64, bool)>,
}

/// The newest frame and what OCR read off it (the Finale's evidence).
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub frame_id: i64,
    pub captured_at_ms: i64,
    pub image: Option<std::path::PathBuf>,
    pub ocr: Option<String>,
}

pub use crate::voice::Enrollment;

pub trait Backend: Send + Sync + 'static {
    fn load(&self) -> Result<Loaded, String>;
    fn permissions(&self) -> Result<CapturePermissions, String>;
    /// Raise the macOS prompt for `kind` (`screen` / `microphone` / `systemAudio`).
    fn request_permission(&self, kind: &str) -> Result<CapturePermissions, String>;
    fn accessibility(&self) -> Option<(bool, Vec<String>)>;
    fn request_accessibility(&self) -> Option<(bool, Vec<String>)>;
    fn start_sys_audio_probe(&self) {}
    fn sys_audio_level(&self) -> Option<f32> {
        None
    }
    fn probe(&self, path: &str) -> Result<Probe, String>;
    /// Installed / running apps for the "Never recorded" Add: `(bundle id, name)`.
    fn app_candidates(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// `resolve_app_icons`: `(bundle id, icon file)` for each one that has an icon.
    fn app_icons(&self, _bundle_ids: &[String]) -> Vec<(String, std::path::PathBuf)> {
        Vec::new()
    }
    fn start_download(&self, item: &WorkItem) -> Result<(), String>;
    fn cancel_download(&self, item: &WorkItem);
    fn mic_level(&self) -> Option<f32> {
        None
    }
    /// Records the bounded clip (blocks for its length); answers the clip path,
    /// `None` when [`Self::cancel_clip`] ended it.
    fn record_clip(&self, ms: u64) -> Result<Option<String>, String>;
    /// End a running take now (leaving the step, closing the window).
    fn cancel_clip(&self);
    fn enroll(&self, clip: &str) -> Result<Enrollment, String>;
    /// The atomic commit: save the whole recording config, mark onboarding done,
    /// then (optionally) start capture. `Err((completed, why))`: `completed` says
    /// whether onboarding got marked done before the failure.
    fn finish(
        &self,
        settings: &RecordingSettings,
        start: Option<StartNativeCaptureRequest>,
    ) -> Result<(), (bool, String)>;
    /// The newest frame newer than `after`, with its OCR once it lands.
    fn evidence(&self, after: Option<i64>) -> Option<Evidence>;
    fn newest_frame_id(&self) -> Option<i64>;
    fn relaunch(&self);

    // AI provider setup. Keys live in the app vault, keyed by instance id.
    /// Store a typed API key in the vault. The key is never logged or kept.
    fn save_key(&self, provider: &str, key: &str) -> Result<(), String>;
    /// Revoke an instance's credential (an API key or a ChatGPT token set).
    fn clear_key(&self, provider: &str) -> Result<(), String>;
    fn has_key(&self, provider: &str) -> bool;
    /// `verify_ai_provider` against the DRAFT list: the chat models it listed
    /// just now and the round trip in ms, or the short failure reason
    /// (`unreachable`, `authentication failed`, `needs_reconnect:<id>`, …).
    fn verify(
        &self,
        provider: &str,
        providers: &[AiProviderConfig],
    ) -> Result<(Vec<String>, u64), String>;
    /// Start the ChatGPT device login: `(user code, verify url)`. The outcome
    /// arrives later as `CoreEvent::ChatgptLoginUpdate`.
    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String>;
    fn chatgpt_cancel(&self, provider: &str);
    fn open_url(&self, url: &str);
}

pub struct CoreBackend(pub Arc<AppCore>);

fn to_json(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The selected model's facts out of a status JSON (`providers[].models[]`).
pub fn facts_from_status(status: &Value, provider: &str, model_id: Option<&str>) -> ModelFacts {
    let models = status["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| p["models"].as_array().cloned().unwrap_or_default())
        .chain(status["models"].as_array().cloned().unwrap_or_default());
    for m in models {
        if m["provider"].as_str() == Some(provider) && m["modelId"].as_str() == model_id {
            let bytes = m["download"]["byteSize"]
                .as_u64()
                .or_else(|| m["approxDownloadBytes"].as_u64())
                .unwrap_or(0);
            return ModelFacts {
                provider: provider.into(),
                model_id: model_id.map(str::to_string),
                bytes,
                installed: m["available"].as_bool().unwrap_or(false),
            };
        }
    }
    ModelFacts {
        provider: provider.into(),
        model_id: model_id.map(str::to_string),
        bytes: 0,
        installed: false,
    }
}

/// The engines (providers) of the transcription status, Deepgram excluded:
/// cloud transcription is Settings-only, behind its own consent gate (ADR 0047).
pub fn engines_from_status(status: &Value) -> Vec<Engine> {
    status["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["provider"].as_str() != Some("deepgram"))
        .map(|p| Engine {
            provider: p["provider"].as_str().unwrap_or_default().into(),
            name: p["displayName"].as_str().unwrap_or_default().into(),
            models: p["models"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| {
                    (
                        m["modelId"].as_str().map(str::to_string),
                        m["displayName"].as_str().unwrap_or_default().into(),
                        m["download"]["byteSize"].as_u64().unwrap_or(0),
                        m["available"].as_bool().unwrap_or(false),
                    )
                })
                .collect(),
        })
        .collect()
}

/// The status's models, each with the catalog's (`list_semantic_search_supported_models`)
/// `multilingual` flag; a model the catalog doesn't list falls back to its tier.
pub fn semantic_from_status(
    status: &Value,
    catalog: &Value,
) -> Vec<(String, String, u64, bool, bool)> {
    status["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["modelId"].as_str()?;
            let listed = catalog
                .as_array()
                .and_then(|c| c.iter().find(|c| c["modelId"].as_str() == Some(id)));
            Some((
                id.to_string(),
                m["displayName"].as_str().unwrap_or_default().to_string(),
                m["approxDownloadBytes"].as_u64().unwrap_or(0),
                m["available"].as_bool().unwrap_or(false),
                match listed {
                    Some(c) => c["multilingual"].as_bool().unwrap_or(false),
                    None => m["tier"].as_str() == Some("multilingual"),
                },
            ))
        })
        .collect()
}

/// The three selected models' facts, from the four status JSONs.
pub fn models_from(
    s: &RecordingSettings,
    transcription: &Value,
    speaker: &Value,
    semantic: &Value,
) -> Vec<(Subsystem, ModelFacts)> {
    let provider =
        app_core::audio_transcription_models::provider_id_for_settings(s.transcription.provider);
    vec![
        (
            Subsystem::Speaker,
            facts_from_status(
                speaker,
                &s.speaker_analysis.provider,
                s.speaker_analysis.model_id.as_deref(),
            ),
        ),
        (
            Subsystem::Transcription,
            facts_from_status(transcription, provider, s.transcription.model_id.as_deref()),
        ),
        (
            Subsystem::Semantic,
            facts_from_status(semantic, "local", s.semantic_search.model_id.as_deref()),
        ),
    ]
}

impl CoreBackend {
    fn infra(&self) -> Result<Arc<AppInfraState>, String> {
        self.0
            .try_state::<AppInfraState>()
            .ok_or_else(|| "app infrastructure isn't ready".into())
    }
}

impl Backend for CoreBackend {
    fn load(&self) -> Result<Loaded, String> {
        let core = &self.0;
        let store = core.state::<app_core::onboarding_state::OnboardingStateStore>();
        let state = to_json(app_core::onboarding_state::get_onboarding_state(
            core, &store,
        ));
        let settings = app_core::native_capture::settings_update::get_recording_settings(core);
        let transcription =
            app_core::audio_transcription_models::get_audio_transcription_model_status(core)
                .map(to_json)
                .unwrap_or_default();
        let speaker = app_core::speaker_analysis_models::get_speaker_analysis_model_status(core)
            .map(to_json)
            .unwrap_or_default();
        let semantic = app_core::semantic_search_models::get_semantic_search_model_status(core)
            .map(to_json)
            .unwrap_or_default();
        let owner_enrolled = self
            .infra()
            .ok()
            .and_then(|infra| {
                core.run(async move {
                    app_core::voice_enrollment::get_account_owner_person_id(&infra).await
                })
                .ok()
            })
            .flatten()
            .is_some();
        let c = core.clone();
        let recommended = core
            .run(async move {
                app_core::sensitive_capture_recommendations::sensitive_capture_recommendations_now(
                    &c,
                )
                .await
            })
            .map(|r| {
                r.recommended_apps
                    .into_iter()
                    .map(|a| (a.bundle_id, a.display_name))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Loaded {
            recommended,
            ever_saved: state["recordingSettingsEverSaved"]
                .as_bool()
                .unwrap_or(false),
            models: models_from(&settings, &transcription, &speaker, &semantic),
            engines: engines_from_status(&transcription),
            semantic: semantic_from_status(
                &semantic,
                &app_core::semantic_search_models::list_semantic_search_supported_models()
                    .map(to_json)
                    .unwrap_or_default(),
            ),
            perms: self.permissions()?,
            gecko: self.accessibility(),
            settings,
            owner_enrolled,
        })
    }

    fn permissions(&self) -> Result<CapturePermissions, String> {
        Ok(app_core::native_capture::permissions::get_capture_permissions(&self.0).permissions)
    }

    fn request_permission(&self, kind: &str) -> Result<CapturePermissions, String> {
        let (core, kind) = (self.0.clone(), kind.to_string());
        self.0
            .run(async move {
                app_core::native_capture::permissions::request_capture_permission(kind, &core).await
            })
            .map(|r| r.permissions)
    }

    fn accessibility(&self) -> Option<(bool, Vec<String>)> {
        gecko(to_json(
            app_core::native_capture::permissions::get_browser_url_accessibility_status(),
        ))
    }

    fn request_accessibility(&self) -> Option<(bool, Vec<String>)> {
        gecko(to_json(
            app_core::native_capture::permissions::request_browser_url_accessibility(),
        ))
    }

    fn start_sys_audio_probe(&self) {
        let core = self.0.clone();
        if let Err(e) = self.0.run(async move {
            app_core::native_capture::microphone::bounded_clip::start_system_audio_level_probe(
                &core,
            )
            .await
        }) {
            log::warn!("onboarding: the system-audio level probe didn't start: {e:?}");
        }
    }

    fn sys_audio_level(&self) -> Option<f32> {
        app_core::native_capture::microphone::bounded_clip::get_system_audio_probe_level()
    }

    fn probe(&self, path: &str) -> Result<Probe, String> {
        let p = app_core::managed_storage_layout::probe_storage_path(Some(path.into()));
        Ok(Probe {
            path: p.path,
            exists: p.exists,
            writable: p.writable,
            free: p.free_bytes.map(|b| b as f64),
        })
    }

    fn app_candidates(&self) -> Vec<(String, String)> {
        self.0
            .run(app_core::native_capture::app_candidates::list_privacy_app_candidates())
            .map(|c| {
                c.into_iter()
                    .map(|a| (a.bundle_id, a.display_name))
                    .collect()
            })
            .unwrap_or_else(|e| {
                log::warn!("onboarding: couldn't list apps to exclude: {e}");
                Vec::new()
            })
    }

    fn app_icons(&self, bundle_ids: &[String]) -> Vec<(String, std::path::PathBuf)> {
        use crate::timeline::data::Source as _;
        crate::timeline::data::CoreSource(self.0.clone()).app_icons(bundle_ids)
    }

    fn start_download(&self, item: &WorkItem) -> Result<(), String> {
        let core = &self.0;
        let (provider, model_id) = (item.provider.clone(), item.model_id.clone());
        match item.subsystem {
            Subsystem::Speaker => crate::settings::data::start_download(
                core,
                crate::settings::data::Family::Speaker,
                provider,
                model_id,
            ),
            Subsystem::Transcription => crate::settings::data::start_download(
                core,
                crate::settings::data::Family::Transcription,
                provider,
                model_id,
            ),
            Subsystem::Semantic => {
                let request =
                    app_core::semantic_search_models::SemanticSearchModelDownloadRequestDto {
                        provider,
                        model_id,
                    };
                app_core::semantic_search_models::start_semantic_search_model_download(
                    core, request,
                )
                .map(|_| ())
            }
        }
    }

    fn cancel_download(&self, item: &WorkItem) {
        let core = &self.0;
        // Cancelling a download that already settled is not worth a message.
        let _ = match item.subsystem {
            Subsystem::Speaker => {
                app_core::speaker_analysis_models::cancel_speaker_analysis_model_download(core)
            }
            Subsystem::Transcription => {
                app_core::audio_transcription_models::cancel_audio_transcription_model_download(
                    core,
                )
            }
            Subsystem::Semantic => app_core::semantic_search_models::cancel_semantic_search_model_download(
                &core.state::<app_core::semantic_search_models::SemanticSearchModelDownloadState>(),
            ),
        };
    }

    fn mic_level(&self) -> Option<f32> {
        app_core::native_capture::microphone::bounded_clip::get_microphone_activity_level()
    }

    fn record_clip(&self, ms: u64) -> Result<Option<String>, String> {
        crate::voice::live::record(&self.0, ms)
    }

    fn cancel_clip(&self) {
        crate::voice::live::cancel();
    }

    fn enroll(&self, clip: &str) -> Result<Enrollment, String> {
        crate::voice::live::enroll(&self.0, &*self.infra()?, clip)
    }

    fn finish(
        &self,
        settings: &RecordingSettings,
        start: Option<StartNativeCaptureRequest>,
    ) -> Result<(), (bool, String)> {
        use app_core::onboarding_state::{
            persist_onboarding_state, OnboardingState, OnboardingStateStore,
        };
        let core = &self.0;
        // Nothing fails silently: the Finale shows `why`, the log keeps it.
        let fail = |completed: bool, step: &str, why: String| {
            log::error!("onboarding: finish failed at {step}: {why}");
            (completed, why)
        };
        // Privacy first, through its own domain: the full save refuses any
        // privacy change ("must be changed with dedicated privacy commands"),
        // and the draft's exclusions (the seeded recommendations) are one.
        let mut settings = settings.clone();
        let saved = app_core::native_capture::settings_update::get_recording_settings(core);
        if settings.privacy != saved.privacy {
            let wanted = settings.privacy.clone();
            let applied = app_core::native_capture::settings_update::apply_recording_settings_domain_mutation_from_core(
                core,
                capture_types::SettingsOwnershipDomain::AppPrivacyExclusion,
                move |s| {
                    s.privacy = wanted;
                    Ok(())
                },
            )
            .map_err(|e| fail(false, "the privacy exclusions", e.message))?;
            settings.privacy = applied.settings.privacy;
        }
        let request =
            app_core::recording_settings::recording_settings_request_from_settings(settings);
        app_core::native_capture::settings_update::update_recording_settings(request, core)
            .map_err(|e| fail(false, "the settings save", e.message))?;
        // Completion BEFORE the side-effecting start: a failed start must not
        // leave capture live with onboarding still incomplete.
        persist_onboarding_state(
            core,
            &core.state::<OnboardingStateStore>(),
            OnboardingState::completed_now(),
        )
        .map_err(|e| fail(false, "marking onboarding complete", e))?;
        core.host().refresh_tray();
        if let Some(start) = start {
            // Off this 512 KB GPUI worker: capture start needs an ordinary thread's stack.
            std::thread::scope(|s| {
                s.spawn(|| app_core::native_capture::control::start_native_capture(start, core))
                    .join()
            })
            .map_err(|_| fail(true, "the capture start", "capture start panicked".into()))?
            .map_err(|e| fail(true, "the capture start", e.message))?;
        }
        Ok(())
    }

    fn newest_frame_id(&self) -> Option<i64> {
        let infra = self.infra().ok()?;
        let request = serde_json::from_value(json!({ "limit": 1 })).ok()?;
        let frames = self
            .0
            .run(async move { app_core::app_infra::list_frames(Some(request), &infra).await })
            .ok()?;
        frames.first().map(|f| f.id)
    }

    fn evidence(&self, after: Option<i64>) -> Option<Evidence> {
        let infra = self.infra().ok()?;
        let request = serde_json::from_value(json!({ "limit": 1 })).ok()?;
        let i = infra.clone();
        let frame = self
            .0
            .run(async move { app_core::app_infra::list_frames(Some(request), &i).await })
            .ok()?
            .into_iter()
            .next()
            .filter(|f| Some(f.id) != after)?;
        let captured_at_ms = time::OffsetDateTime::parse(
            &frame.captured_at,
            &time::format_description::well_known::Rfc3339,
        )
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .unwrap_or(0);
        let frame_id = frame.id;
        let ocr = self
            .0
            .run(async move {
                infra
                    .list_processing_results_for_subject(&app_infra::ProcessingSubject::frame(
                        frame_id,
                    ))
                    .await
            })
            .ok()
            .and_then(|results| {
                results
                    .into_iter()
                    .find_map(|r| r.result_text.filter(|t| !t.trim().is_empty()))
            });
        let preview = serde_json::from_value(json!({ "frameId": frame.id })).ok();
        let image = preview
            .and_then(|req| {
                let core = self.0.clone();
                self.0
                    .run(async move {
                        app_core::app_infra::frame_preview::get_frame_preview(&core, req).await
                    })
                    .ok()
            })
            .flatten()
            .map(|p| std::path::PathBuf::from(p.file_path));
        Some(Evidence {
            frame_id: frame.id,
            captured_at_ms,
            image,
            ocr,
        })
    }

    fn relaunch(&self) {
        self.0.shutdown(app_core::ExitKind::Restart);
    }

    fn save_key(&self, provider: &str, key: &str) -> Result<(), String> {
        let request = serde_json::from_value(json!({ "provider": provider, "key": key }))
            .map_err(|_| "couldn't build the key request".to_string())?;
        self.0
            .run(app_core::ai_runtime::ai_runtime_set_provider_key(request))
    }

    fn clear_key(&self, provider: &str) -> Result<(), String> {
        let request =
            serde_json::from_value(json!({ "provider": provider })).map_err(|e| e.to_string())?;
        self.0
            .run(app_core::ai_runtime::ai_runtime_clear_provider_key(request))
    }

    fn has_key(&self, provider: &str) -> bool {
        serde_json::from_value(json!({ "provider": provider }))
            .ok()
            .is_some_and(|r| {
                self.0
                    .run(app_core::ai_runtime::ai_runtime_has_provider_key(r))
                    == Ok(true)
            })
    }

    fn verify(
        &self,
        provider: &str,
        providers: &[AiProviderConfig],
    ) -> Result<(Vec<String>, u64), String> {
        let request =
            serde_json::from_value(json!({ "provider": provider, "providers": providers }))
                .map_err(|e| e.to_string())?;
        let core = self.0.clone();
        let result = self
            .0
            .run(async move { app_core::ai_runtime::verify_ai_provider(&core, request).await })?;
        Ok(verified_from(&to_json(result)))
    }

    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String> {
        let request =
            serde_json::from_value(json!({ "provider": provider })).map_err(|e| e.to_string())?;
        let core = self.0.clone();
        let p = self.0.run(async move {
            app_core::ai_runtime::ai_runtime_chatgpt_begin_login(&core, request).await
        })?;
        Ok((p.user_code, p.verify_url))
    }

    fn chatgpt_cancel(&self, provider: &str) {
        if let Ok(request) = serde_json::from_value(json!({ "provider": provider })) {
            let _ = self
                .0
                .run(app_core::ai_runtime::ai_runtime_chatgpt_cancel_login(
                    request,
                ));
        }
    }

    fn open_url(&self, url: &str) {
        if let Err(e) = self.0.host().open_url(url) {
            log::warn!("onboarding: couldn't open the ChatGPT sign-in page: {e}");
        }
    }
}

/// `AiRuntimeVerifyResult`'s camelCase JSON: `(models, latencyMs)`.
pub fn verified_from(v: &Value) -> (Vec<String>, u64) {
    let models = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_str().map(str::to_string))
        .collect();
    (models, v["latencyMs"].as_u64().unwrap_or(0))
}

fn gecko(v: Value) -> Option<(bool, Vec<String>)> {
    let names = v["geckoBrowsers"]
        .as_array()?
        .iter()
        .filter(|b| b["installed"].as_bool() == Some(true))
        .filter_map(|b| b["displayName"].as_str().map(str::to_string))
        .collect();
    Some((v["trusted"].as_bool().unwrap_or(false), names))
}

/// `permissionHelperGranted`-style read for the ledger: granted, assumed or unsupported.
pub fn perm(p: &CapturePermissions, key: &str) -> CapturePermissionState {
    match key {
        "microphone" => p.microphone,
        "systemAudio" => p.system_audio,
        _ => p.screen,
    }
}
