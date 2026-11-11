//! What Insights reads, as plain data: the page status (engine + context), and
//! each tab's load. `data.rs` fills these from AppCore; `fixture.rs` from the
//! parity map. The engine reading is `engine-state.ts`'s four-step page rule.

use std::collections::HashMap;

use capture_types::{
    Activity, ActivityMeta, AiEngineRef, AiProviderKind, Conclusion, ConfidenceSnapshot,
    DismissedView, RecordingSettings, SubjectView, UsageChartsResponse, UserContextStatus,
};
use serde::Deserialize;

use crate::chat::format::{provider_label_by_id, url_host};

/// `AiRuntimeStatus` (app-core keeps its fields private; the serde shape is the contract).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AiStatus {
    pub enabled: bool,
    pub has_providers: bool,
    pub configured: bool,
    pub available: bool,
    pub default_model: Option<AiEngineRef>,
    pub default_provider_kind: Option<AiProviderKind>,
    pub reason: Option<String>,
}

/// The page-wide reads every tab shares (`loadEngineStatus`).
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub loaded: bool,
    pub ai: Option<AiStatus>,
    pub ctx: Option<UserContextStatus>,
    pub settings: Option<RecordingSettings>,
    /// `list_user_context_authored` length (the Context tab's count).
    pub authored: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Overview,
    Journal,
    Subjects,
    Context,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Overview, Tab::Journal, Tab::Subjects, Tab::Context];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Journal => "Journal",
            Tab::Subjects => "Subjects",
            Tab::Context => "Context",
        }
    }
}

/// `EngineState` (engine-state.ts).
#[derive(Clone, Debug, PartialEq)]
pub enum Engine {
    Loading,
    /// Never set up: the pitch.
    Pitch,
    /// Off on purpose.
    Off,
    On,
    Unreachable(String),
    /// A named fix; `Some(id)` = a rejected ChatGPT login.
    Fix(String, Option<String>),
}

fn label(settings: Option<&RecordingSettings>, id: &str) -> String {
    let providers = settings
        .map(|s| s.ai_runtime.providers.as_slice())
        .unwrap_or(&[]);
    provider_label_by_id(providers, id)
}

/// "Ollama at localhost:11434", named from the default model's provider.
fn local_endpoint_name(settings: Option<&RecordingSettings>) -> String {
    let Some(s) = settings else {
        return "the local engine".into();
    };
    let id = s
        .ai_runtime
        .default_model
        .as_ref()
        .map(|m| m.provider.as_str());
    let Some(p) = s
        .ai_runtime
        .providers
        .iter()
        .find(|p| Some(p.id.as_str()) == id)
    else {
        return "the local engine".into();
    };
    let kind = crate::chat::format::kind_label(p.kind);
    let default_url = match p.kind {
        AiProviderKind::Ollama => "http://localhost:11434",
        AiProviderKind::Llamafile => "http://localhost:8080",
        _ => "",
    };
    let base = if p.base_url.trim().is_empty() {
        default_url
    } else {
        &p.base_url
    };
    let host = url_host(base);
    if host.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} at {host}")
    }
}

/// `reasonCopy`: a reason code that isn't plain on/off.
pub fn reason_copy(reason: &str, settings: Option<&RecordingSettings>) -> Engine {
    if reason == "local_endpoint_unreachable" {
        return Engine::Unreachable(format!(
            "Can't reach {}. Make sure it's running.",
            local_endpoint_name(settings)
        ));
    }
    if let Some(id) = reason.strip_prefix("provider_unreachable:") {
        return Engine::Unreachable(format!(
            "Can't reach {} right now — your sign-in is fine.",
            label(settings, id)
        ));
    }
    if reason == "vault_denied" {
        return Engine::Unreachable(
            "Mnema couldn't read your saved keys from the keychain. Allow access, then try again."
                .into(),
        );
    }
    if let Some(id) = reason.strip_prefix("needs_reconnect:") {
        return Engine::Fix(
            format!(
                "{} signed you out. Sign in again to keep going.",
                label(settings, id)
            ),
            Some(id.to_string()),
        );
    }
    Engine::Fix(reason_label(reason, settings), None)
}

/// `aiRuntimeReasonLabelFor`: plain copy for a resolve code.
pub fn reason_label(reason: &str, settings: Option<&RecordingSettings>) -> String {
    let code = reason.to_lowercase();
    let after = |prefix: &str| label(settings, &reason[prefix.len()..]);
    let prefixed: [(&str, fn(String) -> String); 5] = [
        ("no_provider_key:", |n| format!("No API key saved for {n}.")),
        ("provider_not_connected:", |n| {
            format!("The default model's provider ({n}) is not connected.")
        }),
        ("needs_reconnect:", |n| {
            format!("{n} needs to be reconnected — sign in with ChatGPT again.")
        }),
        ("provider_unreachable:", |n| {
            format!("Couldn't reach {n} — check your connection and try again.")
        }),
        ("base_url_host_mismatch:", |n| {
            format!("The base URL for {n} doesn't point at that provider. Fix it in Settings.")
        }),
    ];
    for (prefix, f) in prefixed {
        if code.starts_with(prefix) {
            return f(after(prefix));
        }
    }
    match code.as_str() {
        "user_context_disabled" => "Continuous derivation is turned off.",
        "ai_runtime_disabled" => "AI features are turned off.",
        "no_providers" => "No AI providers connected yet.",
        "no_default_model" => "Choose a global default model.",
        "no_base_url" => "Add the base URL for the OpenAI-compatible provider.",
        "invalid_base_url" => "The provider's base URL isn't a valid URL. Fix it in Settings.",
        "invalid_base_url_scheme" => "The provider's base URL must start with http:// or https://.",
        "vault_denied" => "Mnema couldn't read your saved keys from the keychain.",
        "local_endpoint_unreachable" => "The local endpoint could not be reached.",
        // Svelte echoes the raw code here; a code is never shown to the user.
        _ => "The AI engine isn't available right now.",
    }
    .to_string()
}

/// `askAiReasonLabel`: Ask AI's own gate plus the engine codes (Settings › Ask AI).
pub fn ask_ai_reason_label(reason: Option<&str>, settings: Option<&RecordingSettings>) -> String {
    match reason.unwrap_or("") {
        "" => "Ask AI is unavailable.".into(),
        "ask_ai_disabled" => "Ask AI is turned off.".into(),
        "ai_runtime_disabled" => {
            "AI features are turned off \u{2014} enable them in the Providers card above.".into()
        }
        "no_providers" => "Connect an AI provider in the Providers card above.".into(),
        "no_default_model" => "Choose a global default model in the Providers card above.".into(),
        r => reason_label(r, settings),
    }
}

#[cfg(test)]
mod reason_tests {
    use super::*;

    #[test]
    fn settings_never_show_a_raw_reason_code() {
        // Real app, AI off (capture 41): both read as raw codes.
        assert_eq!(
            reason_label("ai_runtime_disabled", None),
            "AI features are turned off."
        );
        assert_eq!(
            ask_ai_reason_label(Some("ask_ai_disabled"), None),
            "Ask AI is turned off."
        );
        assert!(ask_ai_reason_label(Some("ai_runtime_disabled"), None)
            .starts_with("AI features are turned off"));
        assert_eq!(ask_ai_reason_label(None, None), "Ask AI is unavailable.");
        for code in ["some_new_code", "engine_gone:abc"] {
            assert_eq!(
                reason_label(code, None),
                "The AI engine isn't available right now."
            );
            assert!(!ask_ai_reason_label(Some(code), None).contains(code));
        }
    }
}

/// The four-step page rule (independent of `configured`).
pub fn engine_state(s: &Status) -> Engine {
    if !s.loaded {
        return Engine::Loading;
    }
    let Some(ai) = &s.ai else {
        return Engine::Unreachable("Couldn't check the engine's status.".into());
    };
    if !ai.has_providers {
        return Engine::Pitch;
    }
    if !ai.enabled {
        return Engine::Off;
    }
    match (&ai.reason, ai.available) {
        (Some(r), false) => reason_copy(r, s.settings.as_ref()),
        _ => Engine::On,
    }
}

/// The Overview's period load (`loadFree` + `loadPrev` + `loadEngine` + `loadDigest`).
#[derive(Clone, Debug, Default)]
pub struct OverviewData {
    pub usage: Option<UsageChartsResponse>,
    pub usage_error: Option<String>,
    pub prev_usage: Option<UsageChartsResponse>,
    /// `capture_presence`: (screen, audio).
    pub presence: Option<(bool, bool)>,
    pub activities: Vec<Activity>,
    pub prev_activities: Vec<Activity>,
    pub conclusions: Vec<Conclusion>,
    pub engine_error: Option<String>,
    /// A period step's placeholder: only `capture_hours` is real (skeletons).
    pub stale: bool,
    /// Capture hours (local hour starts) for the picker's density.
    pub capture_hours: Vec<i64>,
    /// subject → conclusion id → history (the feed's spark lines).
    pub histories: Histories,
}

pub type Histories = HashMap<String, HashMap<i64, Vec<ConfidenceSnapshot>>>;

/// One local day for the Journal (`loadRange` + `loadUsage` + `loadDigest`).
#[derive(Clone, Debug, Default)]
pub struct JournalData {
    pub activities: Vec<Activity>,
    pub week_activities: Vec<Activity>,
    pub meta: Vec<ActivityMeta>,
    /// Frame capture instants (ms) in the day.
    pub frames: Vec<(i64, i64)>,
    /// Audio segments as `(start, end)` ms.
    pub audio: Vec<(i64, i64)>,
    pub failed: Vec<(i64, i64)>,
    pub usage: Option<UsageChartsResponse>,
    pub history: Option<UsageChartsResponse>,
    /// A day step's placeholder: only `history` is real (skeletons).
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectsData {
    pub conclusions: Vec<Conclusion>,
    pub histories: Histories,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ContextData {
    pub authored: Option<Vec<app_core::user_context::commands::AuthoredContextRow>>,
    pub inferred: Option<Vec<Conclusion>>,
    pub dismissed: Vec<DismissedView>,
    pub error: Option<String>,
}

/// One subject's page (`get_user_context_subject` + the cited activities).
#[derive(Clone, Debug, Default)]
pub struct SubjectData {
    pub view: Option<SubjectView>,
    pub activities: HashMap<i64, Activity>,
    pub error: Option<String>,
}
