//! The AI "Reasoning Engine" command surface and engine resolver.
//!
//! The actual engine — provider clients, structured extraction, reachability —
//! lives in the `ai-runtime` crate, aliased here as `ai_engine` so it does not
//! collide with this `crate::ai_runtime` module. These commands map the
//! provider-centric wire [`AiRuntimeSettings`] (ADR 0034: a flat provider list
//! plus ONE global default model) onto an [`ai_engine::EngineConfig`], read the
//! bring-your-own provider key from the OS keychain (never from settings), and
//! expose status/test round trips plus provider-tagged model discovery to the
//! Settings AI surface.

use capture_types::{
    AccessSettings, AiEngineRef, AiProviderConfig, AiProviderKind, AiRuntimeSettings,
};
use serde::{Deserialize, Serialize};

use crate::recording_settings::{current_recording_settings, RecordingSettingsState};
use crate::AppCore;

/// Availability snapshot for the configured engine, mirroring Ask AI's
/// "Ok with available:false + reason" shape for the normal not-ready cases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeStatus {
    enabled: bool,
    /// At least one provider was ever added — the Insights page pitches setup
    /// only when this is false, never for a broken-but-set-up engine.
    has_providers: bool,
    configured: bool,
    available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<AiEngineRef>,
    /// The default model's provider *kind* (cloud vs local), so the Context
    /// composer can say where authored statements go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_provider_kind: Option<AiProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Result of a single structured-extraction round trip against the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeTestResult {
    ok: bool,
    provider: String,
    model: String,
    message: String,
    raw_json: String,
}

/// One model id discovered from a connected provider's models route, tagged
/// with the provider it came from so the merged pool stays attributable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeModel {
    id: String,
    /// Stable provider id ([`AiProviderKind::id`]).
    provider: String,
    /// Provider-reported context-window size in tokens, when the listing route
    /// advertises one (many OpenAI-compatible vendors and the Fireworks catalog
    /// do; Anthropic/OpenAI don't expose it at all).
    context_window: Option<u64>,
}

/// One connected provider that failed to list its models, so the picker can
/// surface it (with a Retry) instead of silently showing a smaller pool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderFailure {
    /// The provider instance id that failed ([`AiProviderConfig::id`]).
    provider: String,
    /// A short, human-readable reason (`unreachable`, `missing API key`, …).
    reason: String,
}

/// The result of listing the merged model pool: the discovered models plus the
/// providers that failed to list. Best-effort listing means a failed provider
/// no longer just vanishes from the pool — it rides back here so the UI can
/// show it and offer a retry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeModelsResult {
    models: Vec<AiRuntimeModel>,
    failures: Vec<AiRuntimeProviderFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderKeyRequest {
    provider: String,
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeProviderRequest {
    provider: String,
}

/// Set/clear the single secret of an MCP tool connector, keyed by server id.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSecretRequest {
    id: String,
    secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRequest {
    id: String,
}

mod commands;
mod listing;
mod resolver;
pub use commands::*;
pub use listing::*;
pub use resolver::*;

#[cfg(test)]
mod tests;
