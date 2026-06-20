use crate::InactivityActivityMode;
use capture_metadata::{MetadataSettings, PrivacySettings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScreenResolutionPreset {
    Original,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "540p")]
    P540,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ScreenResolution {
    Preset { preset: ScreenResolutionPreset },
    Custom { width: u32, height: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoBitratePreset {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoBitrateMode {
    Preset,
    Custom,
}

pub fn default_video_bitrate_mode() -> VideoBitrateMode {
    VideoBitrateMode::Preset
}

pub fn default_video_bitrate_preset() -> Option<VideoBitratePreset> {
    Some(VideoBitratePreset::Medium)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VideoBitrateSettings {
    #[serde(default = "default_video_bitrate_mode")]
    pub mode: VideoBitrateMode,
    #[serde(default = "default_video_bitrate_preset")]
    pub preset: Option<VideoBitratePreset>,
    #[serde(default)]
    pub custom_mbps: Option<u32>,
}

pub fn default_screen_resolution() -> ScreenResolution {
    ScreenResolution::Preset {
        preset: ScreenResolutionPreset::Original,
    }
}

pub fn default_video_bitrate() -> VideoBitrateSettings {
    VideoBitrateSettings {
        mode: VideoBitrateMode::Preset,
        preset: Some(VideoBitratePreset::Medium),
        custom_mbps: None,
    }
}

pub fn default_pause_capture_on_inactivity() -> bool {
    true
}

pub fn default_idle_timeout_seconds() -> u64 {
    10
}

pub fn default_microphone_activity_sensitivity() -> u8 {
    50
}

pub fn default_system_audio_activity_sensitivity() -> u8 {
    50
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AudioSpeechDetector {
    Silero,
    Webrtc,
    Off,
}

pub type MicrophoneVadAdapter = AudioSpeechDetector;

pub fn default_audio_speech_detector() -> AudioSpeechDetector {
    AudioSpeechDetector::Silero
}

pub fn default_microphone_vad_adapter() -> AudioSpeechDetector {
    default_audio_speech_detector()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSpeechDetectionSettings {
    #[serde(default = "default_audio_speech_detector")]
    pub detector: AudioSpeechDetector,
}

pub fn default_audio_speech_detection_settings() -> AudioSpeechDetectionSettings {
    AudioSpeechDetectionSettings {
        detector: default_audio_speech_detector(),
    }
}

pub fn default_native_capture_debug_logging_enabled() -> bool {
    false
}

pub fn default_developer_options_enabled() -> bool {
    false
}

pub fn default_preview_cache_ttl_seconds() -> u64 {
    3600
}

pub fn default_follow_timeline_live() -> bool {
    false
}

pub fn default_metadata_settings() -> MetadataSettings {
    MetadataSettings::default()
}

pub fn default_privacy_settings() -> PrivacySettings {
    PrivacySettings::default()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    Never,
    #[serde(rename = "days_7", alias = "days7")]
    Days7,
    #[serde(rename = "days_14", alias = "days14")]
    Days14,
    #[serde(rename = "days_30", alias = "days30")]
    Days30,
}

pub fn default_retention_policy() -> RetentionPolicy {
    RetentionPolicy::Never
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceSetting {
    System,
    Light,
    Dark,
}

pub fn default_appearance() -> AppearanceSetting {
    AppearanceSetting::System
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrProvider {
    AppleVision,
    Tesseract,
    PaddleOcr,
}

pub fn default_ocr_provider() -> OcrProvider {
    OcrProvider::AppleVision
}

pub fn default_ocr_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OcrRecognitionMode {
    Fast,
    Accurate,
}

pub fn default_ocr_recognition_mode() -> OcrRecognitionMode {
    OcrRecognitionMode::Fast
}

pub fn default_ocr_language_correction() -> bool {
    false
}

pub fn default_ocr_model_id() -> Option<String> {
    None
}

pub fn default_ocr_language() -> Option<String> {
    None
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrTesseractPageSegmentationMode {
    Auto,
    SingleBlock,
    SingleLine,
    SingleWord,
    SparseText,
}

pub fn default_ocr_tesseract_page_segmentation_mode() -> OcrTesseractPageSegmentationMode {
    OcrTesseractPageSegmentationMode::SingleBlock
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OcrTesseractPreprocessMode {
    Grayscale,
    Thresholded,
}

pub fn default_ocr_tesseract_preprocess_mode() -> OcrTesseractPreprocessMode {
    OcrTesseractPreprocessMode::Grayscale
}

pub fn default_ocr_tesseract_upscale_factor() -> u8 {
    1
}

pub fn default_ocr_tesseract_char_whitelist() -> Option<String> {
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OcrSettings {
    #[serde(default = "default_ocr_enabled")]
    pub enabled: bool,
    #[serde(default = "default_ocr_provider")]
    pub provider: OcrProvider,
    #[serde(default = "default_ocr_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_ocr_language")]
    pub language: Option<String>,
    #[serde(default = "default_ocr_recognition_mode")]
    pub recognition_mode: OcrRecognitionMode,
    #[serde(default = "default_ocr_language_correction")]
    pub language_correction: bool,
    #[serde(default = "default_ocr_tesseract_page_segmentation_mode")]
    pub tesseract_page_segmentation_mode: OcrTesseractPageSegmentationMode,
    #[serde(default = "default_ocr_tesseract_preprocess_mode")]
    pub tesseract_preprocess_mode: OcrTesseractPreprocessMode,
    #[serde(default = "default_ocr_tesseract_upscale_factor")]
    pub tesseract_upscale_factor: u8,
    #[serde(default = "default_ocr_tesseract_char_whitelist")]
    pub tesseract_char_whitelist: Option<String>,
}

pub fn default_ocr_settings() -> OcrSettings {
    OcrSettings {
        enabled: default_ocr_enabled(),
        provider: default_ocr_provider(),
        model_id: default_ocr_model_id(),
        language: default_ocr_language(),
        recognition_mode: default_ocr_recognition_mode(),
        language_correction: default_ocr_language_correction(),
        tesseract_page_segmentation_mode: default_ocr_tesseract_page_segmentation_mode(),
        tesseract_preprocess_mode: default_ocr_tesseract_preprocess_mode(),
        tesseract_upscale_factor: default_ocr_tesseract_upscale_factor(),
        tesseract_char_whitelist: default_ocr_tesseract_char_whitelist(),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionProvider {
    LocalWhisper,
    AppleSpeechOnDevice,
    Parakeet,
}

pub fn default_audio_transcription_enabled() -> bool {
    true
}

pub fn default_audio_transcription_microphone_enabled() -> bool {
    true
}

pub fn default_audio_transcription_system_audio_enabled() -> bool {
    false
}

pub fn default_audio_transcription_provider() -> AudioTranscriptionProvider {
    AudioTranscriptionProvider::LocalWhisper
}

pub fn default_audio_transcription_model_id() -> Option<String> {
    Some("base".to_string())
}

pub fn default_audio_transcription_language() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioTranscriptionMemoryMode {
    Balanced,
    LowMemory,
    Performance,
}

pub fn default_audio_transcription_memory_mode() -> AudioTranscriptionMemoryMode {
    AudioTranscriptionMemoryMode::Balanced
}

pub fn default_audio_transcription_idle_unload_seconds() -> u64 {
    300
}

pub fn default_audio_transcription_chunk_seconds() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionSettings {
    #[serde(default = "default_audio_transcription_enabled")]
    pub enabled: bool,
    #[serde(default = "default_audio_transcription_microphone_enabled")]
    pub microphone_enabled: bool,
    #[serde(default = "default_audio_transcription_system_audio_enabled")]
    pub system_audio_enabled: bool,
    #[serde(default = "default_audio_transcription_provider")]
    pub provider: AudioTranscriptionProvider,
    #[serde(default = "default_audio_transcription_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_audio_transcription_language")]
    pub language: String,
    #[serde(default = "default_audio_transcription_memory_mode")]
    pub memory_mode: AudioTranscriptionMemoryMode,
    #[serde(default = "default_audio_transcription_idle_unload_seconds")]
    pub idle_unload_seconds: u64,
    #[serde(default = "default_audio_transcription_chunk_seconds")]
    pub chunk_seconds: u64,
}

pub fn default_audio_transcription_settings() -> AudioTranscriptionSettings {
    AudioTranscriptionSettings {
        enabled: default_audio_transcription_enabled(),
        microphone_enabled: default_audio_transcription_microphone_enabled(),
        system_audio_enabled: default_audio_transcription_system_audio_enabled(),
        provider: default_audio_transcription_provider(),
        model_id: default_audio_transcription_model_id(),
        language: default_audio_transcription_language(),
        memory_mode: default_audio_transcription_memory_mode(),
        idle_unload_seconds: default_audio_transcription_idle_unload_seconds(),
        chunk_seconds: default_audio_transcription_chunk_seconds(),
    }
}

impl Default for AudioTranscriptionSettings {
    fn default() -> Self {
        default_audio_transcription_settings()
    }
}

pub fn default_speaker_separation_enabled() -> bool {
    false
}

pub fn default_speaker_recognition_enabled() -> bool {
    false
}

pub fn default_speaker_analysis_provider() -> String {
    "sherpa_onnx".to_string()
}

pub fn default_speaker_analysis_model_id() -> Option<String> {
    Some("pyannote-3.0-nemo-titanet-small".to_string())
}

pub fn default_speaker_analysis_timeout_seconds() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisSettings {
    #[serde(default = "default_speaker_separation_enabled")]
    pub separate_speakers: bool,
    #[serde(default = "default_speaker_recognition_enabled")]
    pub recognize_saved_people: bool,
    #[serde(default = "default_speaker_analysis_provider")]
    pub provider: String,
    #[serde(default = "default_speaker_analysis_model_id")]
    pub model_id: Option<String>,
    #[serde(default = "default_speaker_analysis_timeout_seconds")]
    pub timeout_seconds: u64,
}

pub fn default_speaker_analysis_settings() -> SpeakerAnalysisSettings {
    SpeakerAnalysisSettings {
        separate_speakers: default_speaker_separation_enabled(),
        recognize_saved_people: default_speaker_recognition_enabled(),
        provider: default_speaker_analysis_provider(),
        model_id: default_speaker_analysis_model_id(),
        timeout_seconds: default_speaker_analysis_timeout_seconds(),
    }
}

impl Default for SpeakerAnalysisSettings {
    fn default() -> Self {
        default_speaker_analysis_settings()
    }
}

pub fn default_semantic_search_enabled() -> bool {
    // Default-on but model-gated: the feature is inert until a Semantic Search
    // Model is installed (ADR 0036). `enabled` lets the user turn it off even
    // when a model is present.
    true
}

pub fn default_semantic_search_provider() -> String {
    // The on-disk `{provider}/{model_id}` namespace for locally-run models. Kept
    // backend-neutral ("local"), not a runtime name — mirrors
    // `semantic_search::SEMANTIC_SEARCH_PROVIDER_ID`, which the desktop validator
    // checks this against.
    "local".to_string()
}

pub fn default_semantic_search_model_id() -> Option<String> {
    // English default tier: nomic-embed-text-v1.5 (768-dim, 8192-token, Apache-2.0).
    Some("nomic-embed-text-v1.5".to_string())
}

/// User-facing selection of the **Semantic Search Model** (a **Semantic Search
/// Model Tier**). This is the minimal shape the embedding runtime and its
/// model-gating need; the Settings slice extends it (guided tiers + Custom
/// picker) and wires it into `RecordingSettings`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchSettings {
    #[serde(default = "default_semantic_search_enabled")]
    pub enabled: bool,
    #[serde(default = "default_semantic_search_provider")]
    pub provider: String,
    #[serde(default = "default_semantic_search_model_id")]
    pub model_id: Option<String>,
}

pub fn default_semantic_search_settings() -> SemanticSearchSettings {
    SemanticSearchSettings {
        enabled: default_semantic_search_enabled(),
        provider: default_semantic_search_provider(),
        model_id: default_semantic_search_model_id(),
    }
}

impl Default for SemanticSearchSettings {
    fn default() -> Self {
        default_semantic_search_settings()
    }
}

impl Default for VideoBitrateSettings {
    fn default() -> Self {
        default_video_bitrate()
    }
}

impl Default for ScreenResolution {
    fn default() -> Self {
        default_screen_resolution()
    }
}

/// Default per-question Ask AI tool-call cap. `0` means "no cap" (unlimited
/// follow-up brokered queries), so the default is a bounded value rather than 0.
pub fn default_ask_ai_max_tool_calls() -> u32 {
    12
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccessSettings {
    #[serde(default)]
    pub ask_ai_enabled: bool,
    /// Maximum brokered tool calls Ask AI may issue per question. `0` disables
    /// the cap (unlimited). Bounds how much retained capture history a single
    /// conversation can pull through the broker.
    #[serde(default = "default_ask_ai_max_tool_calls")]
    pub ask_ai_max_tool_calls: u32,
    /// The Ask AI **model override** (ADR 0034): a bare rig-core model id that
    /// replaces the global default model's *model* for Quick Recall and
    /// unpinned Chat threads, riding on the default model's provider.
    /// `None`/empty means "use the global default model". NOTE this was
    /// historically a PI `provider:modelId` pair; it is now a bare model id.
    #[serde(default)]
    pub ask_ai_model: Option<String>,
}

impl Default for AccessSettings {
    fn default() -> Self {
        Self {
            ask_ai_enabled: false,
            ask_ai_max_tool_calls: default_ask_ai_max_tool_calls(),
            ask_ai_model: None,
        }
    }
}

/// Stable kind tag for one connected AI provider (ADR 0034). The serde
/// snake_case string is the provider's stable id everywhere: the OS-keychain
/// account in the Capture Index Key Store, the conversation engine-pin
/// `provider` string, and the `provider` tag on discovered models.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    Anthropic,
    Openai,
    OpenaiCompatible,
    Ollama,
    Llamafile,
}

impl AiProviderKind {
    /// The stable provider id (the serde snake_case form).
    pub fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::OpenaiCompatible => "openai_compatible",
            Self::Ollama => "ollama",
            Self::Llamafile => "llamafile",
        }
    }

    /// Parse a stable provider id back into the kind. `None` for an unknown id
    /// (e.g. a conversation pin recorded by a build with other providers).
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "anthropic" => Some(Self::Anthropic),
            "openai" => Some(Self::Openai),
            "openai_compatible" => Some(Self::OpenaiCompatible),
            "ollama" => Some(Self::Ollama),
            "llamafile" => Some(Self::Llamafile),
            _ => None,
        }
    }

    /// Local runtimes are reached on an endpoint and need no credential.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Ollama | Self::Llamafile)
    }

    /// Default endpoint for a local runtime whose provider config leaves
    /// `baseUrl` empty. Cloud providers default to their first-party endpoint
    /// inside the engine crate, so they have no default here.
    pub fn default_local_endpoint(self) -> Option<&'static str> {
        match self {
            Self::Ollama => Some("http://localhost:11434"),
            Self::Llamafile => Some("http://localhost:8080"),
            _ => None,
        }
    }
}

/// One connected AI provider (ADR 0034, amended by ADR 0035): the provider kind
/// plus its non-secret connection details. The credential (cloud API key) lives
/// ONLY in the OS keychain keyed by [`AiProviderConfig::id`]; never persisted here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderConfig {
    /// Stable per-instance id — the identity used everywhere a provider is
    /// referenced: the OS-keychain account, the `provider` tag on discovered
    /// models, the conversation engine-pin `provider`, and the default-model
    /// `provider`. Multiple instances of one [`kind`](Self::kind) coexist by
    /// carrying distinct ids; the first instance of a kind keeps `id ==
    /// kind.id()` so keys and pins recorded before instance ids existed still
    /// resolve. An empty id on a legacy settings file is backfilled to
    /// `kind.id()` at load.
    #[serde(default)]
    pub id: String,
    pub kind: AiProviderKind,
    /// Optional user-facing display name distinguishing same-kind instances
    /// (e.g. "llama-swap box"). Empty falls back to a kind+host label in the UI.
    #[serde(default)]
    pub label: String,
    /// Custom base URL / endpoint. Required for `openai_compatible`; ignored
    /// for the first-party cloud providers (which use their default endpoint);
    /// the local endpoint for `ollama`/`llamafile` (empty = the kind's default
    /// localhost endpoint).
    #[serde(default)]
    pub base_url: String,
}

impl AiProviderConfig {
    /// Backfill an empty [`id`](Self::id) to the kind id. A legacy settings file
    /// (saved before instance ids existed, at most one provider per kind) leaves
    /// `id` empty; `kind.id()` is exactly the identity its keychain key and pins
    /// were recorded under, so this keeps them resolving.
    fn with_backfilled_id(mut self) -> Self {
        if self.id.trim().is_empty() {
            self.id = self.kind.id().to_string();
        }
        self
    }
}

/// An engine identity `{provider, model}` (ADR 0034) — the same shape the
/// conversation engine pin uses. The global default model is one of these, and
/// every model decision resolves to one through the single precedence chain
/// (thread pin → feature override → global default).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiEngineRef {
    /// The connected provider **instance id** ([`AiProviderConfig::id`]). This
    /// was the provider kind before instance ids existed; a legacy value equal
    /// to a kind id still resolves because the first instance of a kind keeps
    /// that id.
    pub provider: String,
    pub model: String,
}

/// The non-secret AI **provider-centric** settings domain (ADR 0034): a master
/// switch, the flat list of connected providers, and ONE global default model
/// chosen from the merged pool. There is no privileged default engine and no
/// separate additional-engines list; a legacy engine-centric settings file
/// (engineKind/cloud*/local* + additionalEngines) still deserializes into this
/// shape via [`AiRuntimeSettingsWire`], and saves write only the new shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", from = "AiRuntimeSettingsWire")]
pub struct AiRuntimeSettings {
    pub enabled: bool,
    pub providers: Vec<AiProviderConfig>,
    pub default_model: Option<AiEngineRef>,
}

impl Default for AiRuntimeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            providers: Vec::new(),
            default_model: None,
        }
    }
}

// --- legacy engine-centric wire shape (pre-ADR-0034) ---------------------
// Kept deserialize-only so an old persisted `aiRuntime` block migrates into
// the provider list at load time. Mirrors the old serde defaults so a partial
// legacy file resolves exactly as it used to before converting.

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyAiEngineKind {
    Cloud,
    Local,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyAiCloudProvider {
    Anthropic,
    Openai,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyAiLocalKind {
    Ollama,
    Llamafile,
}

fn legacy_default_engine_kind() -> LegacyAiEngineKind {
    LegacyAiEngineKind::Cloud
}

fn legacy_default_cloud_provider() -> LegacyAiCloudProvider {
    LegacyAiCloudProvider::Anthropic
}

fn legacy_default_cloud_model() -> String {
    "claude-haiku-4-5".to_string()
}

fn legacy_default_local_kind() -> LegacyAiLocalKind {
    LegacyAiLocalKind::Ollama
}

fn legacy_default_local_endpoint() -> String {
    "http://localhost:11434".to_string()
}

/// One legacy configured engine: the old flat default-engine fields or one
/// entry of the old `additionalEngines` catalog.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyAiEngineProfile {
    #[serde(default = "legacy_default_engine_kind")]
    engine_kind: LegacyAiEngineKind,
    #[serde(default = "legacy_default_cloud_provider")]
    cloud_provider: LegacyAiCloudProvider,
    #[serde(default = "legacy_default_cloud_model")]
    cloud_model: String,
    #[serde(default)]
    cloud_base_url: String,
    #[serde(default = "legacy_default_local_kind")]
    local_kind: LegacyAiLocalKind,
    #[serde(default = "legacy_default_local_endpoint")]
    local_endpoint: String,
    #[serde(default)]
    local_model: String,
}

impl LegacyAiEngineProfile {
    /// The provider this engine actually used (only the active engine-kind
    /// side counts; the inactive side's fields were latent UI state).
    fn provider_kind(&self) -> AiProviderKind {
        match self.engine_kind {
            LegacyAiEngineKind::Cloud => match self.cloud_provider {
                LegacyAiCloudProvider::Anthropic => AiProviderKind::Anthropic,
                LegacyAiCloudProvider::Openai => AiProviderKind::Openai,
                LegacyAiCloudProvider::OpenaiCompatible => AiProviderKind::OpenaiCompatible,
            },
            LegacyAiEngineKind::Local => match self.local_kind {
                LegacyAiLocalKind::Ollama => AiProviderKind::Ollama,
                LegacyAiLocalKind::Llamafile => AiProviderKind::Llamafile,
            },
        }
    }

    fn base_url(&self) -> String {
        match self.engine_kind {
            LegacyAiEngineKind::Cloud => self.cloud_base_url.trim().to_string(),
            LegacyAiEngineKind::Local => self.local_endpoint.trim().to_string(),
        }
    }

    fn model(&self) -> String {
        match self.engine_kind {
            LegacyAiEngineKind::Cloud => self.cloud_model.trim().to_string(),
            LegacyAiEngineKind::Local => self.local_model.trim().to_string(),
        }
    }
}

/// Deserialization-time wire shape for [`AiRuntimeSettings`], accepting both
/// the provider-centric shape and the legacy engine-centric one. A `providers`
/// key marks the new shape; otherwise any legacy engine field triggers the
/// migration (old default engine + `additionalEngines` → providers list, with
/// the default engine's `{provider, model}` becoming the global default model).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiRuntimeSettingsWire {
    #[serde(default)]
    enabled: bool,
    // New provider-centric shape.
    providers: Option<Vec<AiProviderConfig>>,
    #[serde(default)]
    default_model: Option<AiEngineRef>,
    // Legacy engine-centric shape.
    engine_kind: Option<LegacyAiEngineKind>,
    cloud_provider: Option<LegacyAiCloudProvider>,
    cloud_model: Option<String>,
    cloud_base_url: Option<String>,
    local_kind: Option<LegacyAiLocalKind>,
    local_endpoint: Option<String>,
    local_model: Option<String>,
    #[serde(default)]
    additional_engines: Vec<LegacyAiEngineProfile>,
}

impl From<AiRuntimeSettingsWire> for AiRuntimeSettings {
    fn from(wire: AiRuntimeSettingsWire) -> Self {
        // New shape present → use it verbatim (a blank default model id is
        // treated as "no default chosen").
        if let Some(providers) = wire.providers {
            return Self {
                enabled: wire.enabled,
                providers: providers
                    .into_iter()
                    .map(AiProviderConfig::with_backfilled_id)
                    .collect(),
                default_model: wire
                    .default_model
                    .filter(|model| !model.model.trim().is_empty()),
            };
        }

        let has_legacy_fields = wire.engine_kind.is_some()
            || wire.cloud_provider.is_some()
            || wire.cloud_model.is_some()
            || wire.cloud_base_url.is_some()
            || wire.local_kind.is_some()
            || wire.local_endpoint.is_some()
            || wire.local_model.is_some()
            || !wire.additional_engines.is_empty();
        if !has_legacy_fields {
            return Self {
                enabled: wire.enabled,
                ..Self::default()
            };
        }

        // Legacy migration: the flat fields are the old default engine, with
        // the old serde defaults applied to whatever a partial file omitted.
        let default_engine = LegacyAiEngineProfile {
            engine_kind: wire.engine_kind.unwrap_or_else(legacy_default_engine_kind),
            cloud_provider: wire
                .cloud_provider
                .unwrap_or_else(legacy_default_cloud_provider),
            cloud_model: wire.cloud_model.unwrap_or_else(legacy_default_cloud_model),
            cloud_base_url: wire.cloud_base_url.unwrap_or_default(),
            local_kind: wire.local_kind.unwrap_or_else(legacy_default_local_kind),
            local_endpoint: wire
                .local_endpoint
                .unwrap_or_else(legacy_default_local_endpoint),
            local_model: wire.local_model.unwrap_or_default(),
        };

        let mut providers: Vec<AiProviderConfig> = Vec::new();
        for profile in std::iter::once(&default_engine).chain(wire.additional_engines.iter()) {
            let kind = profile.provider_kind();
            if providers.iter().any(|provider| provider.kind == kind) {
                continue;
            }
            providers.push(AiProviderConfig {
                // Pre-instance-id data had one provider per kind, so the kind id
                // is the instance id (and the account its keychain key lives at).
                id: kind.id().to_string(),
                kind,
                label: String::new(),
                base_url: profile.base_url(),
            });
        }

        let default_model = {
            let model = default_engine.model();
            (!model.is_empty()).then(|| AiEngineRef {
                provider: default_engine.provider_kind().id().to_string(),
                model,
            })
        };

        Self {
            enabled: wire.enabled,
            providers,
            default_model,
        }
    }
}

/// Double-Option deserializer so `defaultModel` can express all three patch
/// intents: field absent = leave unchanged, explicit `null` = clear, an object
/// = set.
fn deserialize_double_option_engine_ref<'de, D>(
    deserializer: D,
) -> Result<Option<Option<AiEngineRef>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<AiEngineRef>::deserialize(deserializer).map(Some)
}

/// Distinguish an absent field (`None`) from an explicit `null` (`Some(None)`)
/// for a patch string. Present-and-set is `Some(Some(value))`.
fn deserialize_optional_optional_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAiRuntimeSettingsRequest {
    pub enabled: Option<bool>,
    /// Replacement provider list. `None` leaves the existing list unchanged;
    /// `Some` replaces it wholesale.
    pub providers: Option<Vec<AiProviderConfig>>,
    /// Global default model. Absent = unchanged; explicit `null` = clear.
    #[serde(default, deserialize_with = "deserialize_double_option_engine_ref")]
    pub default_model: Option<Option<AiEngineRef>>,
}

/// The named **Derivation Budget** intensity tier for a cloud Reasoning Engine
/// (CONTEXT.md "Derivation Budget"). Local engines ignore the tier (fixed
/// resource pacing only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DerivationBudgetTier {
    Light,
    Balanced,
    Thorough,
}

pub fn default_derivation_budget_tier() -> DerivationBudgetTier {
    DerivationBudgetTier::Balanced
}

pub fn default_backfill_window_days() -> u32 {
    30
}

/// The non-secret **User Context** derivation settings domain (CONTEXT.md
/// "Derivation Budget" / "History Backfill"). Persisted like every other
/// `RecordingSettings` domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserContextSettings {
    /// The **continuous-derivation opt-in**: whether the background User Context
    /// worker (Activity/Conclusion/digest derivation) runs at all. This is
    /// independent of the interactive Ask AI opt-in — the shared prerequisite is
    /// only that a usable Reasoning Engine is configured (the `AiRuntime` master
    /// toggle + a resolvable engine). Off by default, so an existing user who
    /// turned on the engine for Ask AI does NOT silently start continuous
    /// background derivation until they opt in here.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_derivation_budget_tier")]
    pub derivation_budget_tier: DerivationBudgetTier,
    #[serde(default = "default_backfill_window_days")]
    pub backfill_window_days: u32,
    #[serde(default)]
    pub backfill_go_deeper: bool,
}

impl Default for UserContextSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            derivation_budget_tier: default_derivation_budget_tier(),
            backfill_window_days: default_backfill_window_days(),
            backfill_go_deeper: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserContextSettingsRequest {
    pub enabled: Option<bool>,
    pub derivation_budget_tier: Option<DerivationBudgetTier>,
    pub backfill_window_days: Option<u32>,
    pub backfill_go_deeper: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSettings {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
    pub segment_duration_seconds: u64,
    pub screen_frame_rate: u32,
    #[serde(default = "default_screen_resolution")]
    pub screen_resolution: ScreenResolution,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate: VideoBitrateSettings,
    pub save_directory: String,
    pub auto_start: bool,
    #[serde(default = "default_native_capture_debug_logging_enabled")]
    pub native_capture_debug_logging_enabled: bool,
    #[serde(default = "default_developer_options_enabled")]
    pub developer_options_enabled: bool,
    #[serde(default = "default_preview_cache_ttl_seconds")]
    pub preview_cache_ttl_seconds: u64,
    #[serde(default = "default_follow_timeline_live")]
    pub follow_timeline_live: bool,
    #[serde(default = "default_retention_policy")]
    pub retention_policy: RetentionPolicy,
    #[serde(default = "default_appearance")]
    pub appearance: AppearanceSetting,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_audio_transcription_settings")]
    pub transcription: AudioTranscriptionSettings,
    #[serde(default = "default_speaker_analysis_settings")]
    pub speaker_analysis: SpeakerAnalysisSettings,
    #[serde(default = "default_audio_speech_detection_settings")]
    pub audio_speech_detection: AudioSpeechDetectionSettings,
    #[serde(default = "default_metadata_settings")]
    pub metadata: MetadataSettings,
    #[serde(default = "default_privacy_settings")]
    pub privacy: PrivacySettings,
    #[serde(default)]
    pub access: AccessSettings,
    #[serde(default)]
    pub ai_runtime: AiRuntimeSettings,
    #[serde(default)]
    pub user_context: UserContextSettings,
    #[serde(default = "default_semantic_search_settings")]
    pub semantic_search: SemanticSearchSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(
        default = "default_audio_speech_detector",
        alias = "microphoneVadAdapter",
        skip_serializing
    )]
    pub microphone_vad_adapter: AudioSpeechDetector,
    #[serde(
        default = "crate::default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SettingsOwnershipDomain {
    CaptureSources,
    CaptureTiming,
    Video,
    Storage,
    Display,
    Metadata,
    AppPrivacyExclusion,
    Inactivity,
    Processing,
    Developer,
    KeyboardBindings,
    MicrophoneController,
    AppUpdate,
    Access,
    AiRuntime,
    UserContext,
    SemanticSearch,
    OneTimePromptState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSettingsDomainUpdateResponse {
    pub domain: SettingsOwnershipDomain,
    pub settings: RecordingSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecordingSettingsRequest {
    pub capture_screen: bool,
    pub capture_microphone: bool,
    pub capture_system_audio: bool,
    pub segment_duration_seconds: u64,
    pub screen_frame_rate: u32,
    #[serde(default = "default_screen_resolution")]
    pub screen_resolution: ScreenResolution,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate: VideoBitrateSettings,
    pub save_directory: String,
    pub auto_start: bool,
    #[serde(default = "default_native_capture_debug_logging_enabled")]
    pub native_capture_debug_logging_enabled: bool,
    #[serde(default = "default_developer_options_enabled")]
    pub developer_options_enabled: bool,
    #[serde(default = "default_preview_cache_ttl_seconds")]
    pub preview_cache_ttl_seconds: u64,
    #[serde(default = "default_follow_timeline_live")]
    pub follow_timeline_live: bool,
    #[serde(default = "default_retention_policy")]
    pub retention_policy: RetentionPolicy,
    #[serde(default = "default_appearance")]
    pub appearance: AppearanceSetting,
    #[serde(default = "default_ocr_settings")]
    pub ocr: OcrSettings,
    #[serde(default = "default_audio_transcription_settings")]
    pub transcription: AudioTranscriptionSettings,
    #[serde(default = "default_speaker_analysis_settings")]
    pub speaker_analysis: SpeakerAnalysisSettings,
    #[serde(default = "default_audio_speech_detection_settings")]
    pub audio_speech_detection: AudioSpeechDetectionSettings,
    #[serde(default = "default_metadata_settings")]
    pub metadata: MetadataSettings,
    #[serde(default = "default_privacy_settings")]
    pub privacy: PrivacySettings,
    #[serde(default)]
    pub access: AccessSettings,
    #[serde(default)]
    pub ai_runtime: AiRuntimeSettings,
    #[serde(default)]
    pub user_context: UserContextSettings,
    #[serde(default = "default_semantic_search_settings")]
    pub semantic_search: SemanticSearchSettings,
    #[serde(default = "default_pause_capture_on_inactivity")]
    pub pause_capture_on_inactivity: bool,
    #[serde(default = "default_idle_timeout_seconds")]
    pub idle_timeout_seconds: u64,
    #[serde(default = "default_microphone_activity_sensitivity")]
    pub microphone_activity_sensitivity: u8,
    #[serde(default = "default_system_audio_activity_sensitivity")]
    pub system_audio_activity_sensitivity: u8,
    #[serde(
        default = "default_audio_speech_detector",
        alias = "microphoneVadAdapter",
        skip_serializing
    )]
    pub microphone_vad_adapter: AudioSpeechDetector,
    #[serde(
        default = "crate::default_inactivity_activity_mode",
        rename = "activityMode",
        alias = "inactivityActivityMode"
    )]
    pub inactivity_activity_mode: InactivityActivityMode,
}

/// Partial update of the **Semantic Search Model Tier** selection. A model
/// switch is a deliberate, confirmed action in the UI (it re-derives every
/// **Semantic Search Vector**); this is the patch the desktop command applies
/// once the user confirms (ADR 0036).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSemanticSearchSettingsRequest {
    pub enabled: Option<bool>,
    pub provider: Option<String>,
    /// Double-`Option`: `Some(None)` clears the selected model, `Some(Some(id))`
    /// selects a model, `None` leaves the selection unchanged.
    #[serde(default, deserialize_with = "deserialize_optional_optional_string")]
    pub model_id: Option<Option<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCaptureSourceSettingsRequest {
    pub capture_screen: Option<bool>,
    pub capture_microphone: Option<bool>,
    pub capture_system_audio: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCaptureTimingSettingsRequest {
    pub segment_duration_seconds: Option<u64>,
    pub auto_start: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVideoSettingsRequest {
    pub screen_frame_rate: Option<u32>,
    pub screen_resolution: Option<ScreenResolution>,
    pub video_bitrate: Option<VideoBitrateSettings>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStorageSettingsRequest {
    pub save_directory: Option<String>,
    pub retention_policy: Option<RetentionPolicy>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDisplaySettingsRequest {
    pub appearance: Option<AppearanceSetting>,
    pub follow_timeline_live: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadataSettingsRequest {
    pub enabled: Option<bool>,
    pub browser_url_mode: Option<capture_metadata::BrowserUrlMode>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInactivitySettingsRequest {
    pub pause_capture_on_inactivity: Option<bool>,
    pub idle_timeout_seconds: Option<u64>,
    pub microphone_activity_sensitivity: Option<u8>,
    pub system_audio_activity_sensitivity: Option<u8>,
    pub audio_speech_detection: Option<AudioSpeechDetectionSettings>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProcessingSettingsRequest {
    pub ocr: Option<OcrSettings>,
    pub transcription: Option<AudioTranscriptionSettings>,
    pub speaker_analysis: Option<SpeakerAnalysisSettings>,
    pub preview_cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAccessSettingsRequest {
    pub ask_ai_enabled: Option<bool>,
    /// New per-question tool-call cap (`0` = no cap). `None` leaves it unchanged.
    pub ask_ai_max_tool_calls: Option<u32>,
    /// New Quick Recall model id — a rig-core model id used against the default
    /// Reasoning Engine (not a PI `provider:modelId` pair). `None` leaves it
    /// unchanged; an empty string clears the selection back to the engine's
    /// default model.
    pub ask_ai_model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDeveloperSettingsRequest {
    pub developer_options_enabled: Option<bool>,
    pub native_capture_debug_logging_enabled: Option<bool>,
}
