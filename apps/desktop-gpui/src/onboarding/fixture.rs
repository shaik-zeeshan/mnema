//! `FixtureBackend`: onboarding over `apps/desktop/parity/fixtures/onboarding/
//! <state>.json` (Tauri command -> JSON, the map the Svelte stub serves), so
//! goldens, parity and tests see the same data. Records every call.

use std::sync::Mutex;

use capture_types::{
    CapturePermissionState as P, CapturePermissions, RecordingSettings, StartNativeCaptureRequest,
};
use serde_json::Value;

use super::data::{self, Backend, Enrollment, Evidence, Loaded};
use super::flow::WorkItem;
use super::storage::Probe;

/// Golden / parity states: `(state, fixture file)`.
pub const STATES: [(&str, &str); 19] = [
    ("welcome", "default"),
    ("permissions", "default"),
    ("permissions-granted", "granted"),
    ("permissions-denied", "denied"),
    ("capture", "granted"),
    ("capture-blocked", "tiny"),
    ("settings", "granted"),
    ("change", "granted"),
    // Change settings › Models over the full eight-model embedding catalog.
    ("change-models", "granted"),
    ("setup", "granted"),
    ("setup-done", "granted"),
    ("voice", "installed"),
    ("finale", "finale"),
    // Change settings › AI features (S14b).
    ("ai-choose", "granted"),
    ("ai-key", "granted"),
    ("ai-verifying", "granted"),
    ("ai-ready", "granted"),
    ("ai-invalid", "ai-invalid"),
    ("ai-chatgpt", "granted"),
];

pub struct FixtureBackend {
    map: Value,
    calls: Mutex<Vec<String>>,
    /// What `enroll` answers.
    pub enrollment: Mutex<Enrollment>,
    /// `finish` fails with this, when set.
    pub finish_error: Mutex<Option<String>>,
    /// Instance ids with a key "in the vault".
    keys: Mutex<Vec<String>>,
}

impl FixtureBackend {
    pub fn new(map: Value) -> Self {
        Self {
            map,
            calls: Mutex::new(Vec::new()),
            enrollment: Mutex::new(Enrollment::Enrolled),
            finish_error: Mutex::new(None),
            keys: Mutex::new(Vec::new()),
        }
    }

    pub fn load(file: &str) -> anyhow::Result<Self> {
        Ok(Self::new(crate::shell::fixture::load("onboarding", file)?))
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn log(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }

    fn perms(&self) -> CapturePermissions {
        serde_json::from_value(self.map["get_capture_permissions"]["permissions"].clone())
            .unwrap_or(CapturePermissions {
                screen: P::NotDetermined,
                microphone: P::NotDetermined,
                system_audio: P::NotDetermined,
            })
    }
}

impl Backend for FixtureBackend {
    fn load(&self) -> Result<Loaded, String> {
        self.log("load");
        let m = &self.map;
        let settings: RecordingSettings =
            serde_json::from_value(m["get_recording_settings"].clone())
                .unwrap_or_else(|_| app_core::recording_settings::default_recording_settings());
        let t = &m["get_audio_transcription_model_status"];
        let recommended = m["get_sensitive_capture_recommendations"]["recommendedApps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some((
                    a["bundleId"].as_str()?.to_string(),
                    a["displayName"].as_str()?.to_string(),
                ))
            })
            .collect();
        Ok(Loaded {
            recommended,
            ever_saved: m["get_onboarding_state"]["recordingSettingsEverSaved"]
                .as_bool()
                .unwrap_or(false),
            models: data::models_from(
                &settings,
                t,
                &m["get_speaker_analysis_model_status"],
                &m["get_semantic_search_model_status"],
            ),
            engines: data::engines_from_status(t),
            semantic: data::semantic_from_status(
                &m["get_semantic_search_model_status"],
                &m["list_semantic_search_supported_models"],
            ),
            perms: self.perms(),
            gecko: self.accessibility(),
            owner_enrolled: m["get_account_owner_person_id"].is_i64(),
            settings,
        })
    }

    fn permissions(&self) -> Result<CapturePermissions, String> {
        self.log("permissions");
        Ok(self.perms())
    }

    fn request_permission(&self, kind: &str) -> Result<CapturePermissions, String> {
        self.log(format!("request_permission:{kind}"));
        Ok(self.perms())
    }

    fn accessibility(&self) -> Option<(bool, Vec<String>)> {
        let v = &self.map["get_browser_url_accessibility_status"];
        let names = v["geckoBrowsers"]
            .as_array()?
            .iter()
            .filter(|b| b["installed"].as_bool() == Some(true))
            .filter_map(|b| b["displayName"].as_str().map(str::to_string))
            .collect();
        Some((v["trusted"].as_bool().unwrap_or(false), names))
    }

    fn request_accessibility(&self) -> Option<(bool, Vec<String>)> {
        self.log("request_accessibility");
        self.accessibility()
    }

    fn app_candidates(&self) -> Vec<(String, String)> {
        self.log("app_candidates");
        self.map["list_privacy_app_candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some((
                    a["bundleId"].as_str()?.to_string(),
                    a["displayName"].as_str()?.to_string(),
                ))
            })
            .collect()
    }

    fn probe(&self, path: &str) -> Result<Probe, String> {
        self.log(format!("probe:{path}"));
        let p = &self.map["probe_storage_path"];
        if p.is_null() {
            return Err("no probe in fixture".into());
        }
        Ok(Probe {
            path: p["path"].as_str().unwrap_or(path).to_string(),
            exists: p["exists"].as_bool().unwrap_or(false),
            writable: p["writable"].as_bool().unwrap_or(false),
            free: p["freeBytes"].as_f64(),
        })
    }

    fn start_download(&self, item: &WorkItem) -> Result<(), String> {
        self.log(format!("start_download:{}", item.model_id));
        Ok(())
    }

    fn cancel_download(&self, item: &WorkItem) {
        self.log(format!("cancel_download:{}", item.model_id));
    }

    fn record_clip(&self, ms: u64) -> Result<String, String> {
        self.log(format!("record_clip:{ms}"));
        Ok("/tmp/mnema-fixture-clip.wav".into())
    }

    fn enroll(&self, clip: &str) -> Result<Enrollment, String> {
        self.log(format!("enroll:{clip}"));
        Ok(self.enrollment.lock().unwrap().clone())
    }

    fn finish(
        &self,
        settings: &RecordingSettings,
        start: Option<StartNativeCaptureRequest>,
    ) -> Result<(), (bool, String)> {
        let start = start.map_or("none".to_string(), |s| {
            format!(
                "screen={},mic={},sys={}",
                s.capture_screen, s.capture_microphone, s.capture_system_audio
            )
        });
        self.log(format!(
            "finish:fps={}:start={start}",
            settings.screen_frame_rate
        ));
        match self.finish_error.lock().unwrap().clone() {
            Some(e) => Err((true, e)),
            None => Ok(()),
        }
    }

    fn evidence(&self, after: Option<i64>) -> Option<Evidence> {
        let f = self.map["list_frames"].as_array()?.first()?;
        let id = f["id"].as_i64()?;
        if Some(id) == after {
            return None;
        }
        let captured_at_ms = time::OffsetDateTime::parse(
            f["capturedAt"].as_str()?,
            &time::format_description::well_known::Rfc3339,
        )
        .map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64)
        .ok()?;
        Some(Evidence {
            frame_id: id,
            captured_at_ms,
            image: self.map["get_frame_preview"]["filePath"]
                .as_str()
                .map(Into::into),
            ocr: f["ocrText"].as_str().map(str::to_string),
        })
    }

    fn newest_frame_id(&self) -> Option<i64> {
        None
    }

    fn relaunch(&self) {
        self.log("relaunch");
    }

    // The key itself is never recorded: only that one was saved, and for whom.
    fn save_key(&self, provider: &str, _key: &str) -> Result<(), String> {
        self.log(format!("save_key:{provider}"));
        self.keys.lock().unwrap().push(provider.to_string());
        Ok(())
    }

    fn clear_key(&self, provider: &str) -> Result<(), String> {
        self.log(format!("clear_key:{provider}"));
        self.keys.lock().unwrap().retain(|k| k != provider);
        Ok(())
    }

    fn has_key(&self, provider: &str) -> bool {
        self.keys.lock().unwrap().iter().any(|k| k == provider)
    }

    /// `verify_ai_provider` answers every provider the same; `{"__reject": why}`
    /// fails it (the stub's convention, so the Tauri side fails the same way).
    fn verify(
        &self,
        provider: &str,
        _: &[capture_types::AiProviderConfig],
    ) -> Result<(Vec<String>, u64), String> {
        self.log(format!("verify:{provider}"));
        let v = &self.map["verify_ai_provider"];
        if let Some(why) = v["__reject"].as_str() {
            return Err(why.to_string());
        }
        if v.is_null() {
            return Err("unreachable".into());
        }
        Ok(data::verified_from(v))
    }

    fn chatgpt_begin(&self, provider: &str) -> Result<(String, String), String> {
        self.log(format!("chatgpt_begin:{provider}"));
        let v = &self.map["ai_runtime_chatgpt_begin_login"];
        match (v["userCode"].as_str(), v["verifyUrl"].as_str()) {
            (Some(c), Some(u)) => Ok((c.into(), u.into())),
            _ => Err("no device code in fixture".into()),
        }
    }

    fn chatgpt_cancel(&self, provider: &str) {
        self.log(format!("chatgpt_cancel:{provider}"));
    }

    fn open_url(&self, url: &str) {
        self.log(format!("open_url:{url}"));
    }
}
