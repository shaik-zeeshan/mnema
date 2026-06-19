export type ActivityMode =
	| "system_input_only"
	| "system_input_or_screen"
	| "system_input_or_screen_or_audio";

export type AppearanceSetting = "system" | "light" | "dark";
export type AudioSpeechDetector = "silero" | "webrtc" | "off";
export type MicrophoneVadAdapter = AudioSpeechDetector;
export type RetentionPolicy = "never" | "days_7" | "days_14" | "days_30";
export type BrowserUrlMode = "off" | "sanitized" | "full";

export interface MetadataSettings {
	enabled: boolean;
	browserUrlMode: BrowserUrlMode;
}

export interface AccessSettings {
	askAiEnabled: boolean;
	/** Per-question Ask AI tool-call cap. `0` disables the cap (unlimited). */
	askAiMaxToolCalls: number;
	/**
	 * rig-core model id Quick Recall should use against the default Reasoning
	 * Engine (e.g. `claude-haiku-4-5`). `null`/empty lets the engine pick its
	 * configured default model. (Was historically a PI `provider:modelId` pair;
	 * on the rig-core engine the provider is fixed by the default engine, so this
	 * is now a bare model id.)
	 */
	askAiModel?: string | null;
}

export interface ExcludedAppEntry {
	id: string;
	enabled: boolean;
	bundleId: string;
	displayName: string;
}

export interface PrivacySettings {
	excludedApps: ExcludedAppEntry[];
}

/** Stable provider id, matching the Rust `AiProviderKind::id` values. */
export type AiProviderKind =
	| "anthropic"
	| "openai"
	| "openai_compatible"
	| "ollama"
	| "llamafile";

/**
 * One connected AI provider (ADR 0034, amended by ADR 0035): the provider kind
 * plus its non-secret connection details. The credential (cloud API key) lives
 * ONLY in the OS keychain keyed by the provider **instance id**; never here.
 */
export interface AiProviderConfig {
	/**
	 * Stable per-instance id — the identity used everywhere a provider is
	 * referenced (keychain account, model-pool `provider` tag, engine pin, and
	 * default-model `provider`). Multiple instances of one `kind` coexist by
	 * carrying distinct ids; the first instance of a kind keeps `id === kind`
	 * so keys/pins recorded before instance ids existed still resolve.
	 */
	id: string;
	kind: AiProviderKind;
	/**
	 * Optional user-facing display name distinguishing same-kind instances
	 * (e.g. "llama-swap box"). Empty falls back to a kind+host label.
	 */
	label: string;
	/**
	 * Custom base URL / endpoint. Required for `openai_compatible`; ignored for
	 * the first-party cloud providers; the local endpoint for `ollama` /
	 * `llamafile` (empty = the kind's default localhost endpoint).
	 */
	baseUrl: string;
}

/**
 * An engine identity `{provider, model}` (ADR 0034) — the same shape the
 * conversation engine pin uses. The global default model is one of these.
 * `provider` is the connected provider **instance id** (`AiProviderConfig.id`).
 */
export interface AiEngineRef {
	provider: string;
	model: string;
}

/**
 * The provider-centric AI settings domain (ADR 0034): a master switch, the
 * flat list of connected providers, and ONE global default model chosen from
 * the merged pool. Model resolution is thread pin → feature override → this
 * global default.
 */
export interface AiRuntimeSettings {
	enabled: boolean;
	providers: AiProviderConfig[];
	defaultModel: AiEngineRef | null;
}

export interface UpdateAiRuntimeSettingsRequest {
	enabled?: boolean;
	/** Replacement provider list; omitting leaves the list unchanged. */
	providers?: AiProviderConfig[];
	/** Tri-state: absent = unchanged, `null` = clear, object = set. */
	defaultModel?: AiEngineRef | null;
}

/** Named Derivation Budget intensity tier for a cloud Reasoning Engine. */
export type DerivationBudgetTier = "light" | "balanced" | "thorough";

/** Non-secret User Context derivation settings domain. */
export interface UserContextSettings {
	/**
	 * The continuous-derivation opt-in: whether the background User Context
	 * worker runs at all. Independent of Ask AI; the shared prerequisite is only
	 * that a usable Reasoning Engine is configured. Off by default.
	 */
	enabled: boolean;
	derivationBudgetTier: DerivationBudgetTier;
	backfillWindowDays: number;
	backfillGoDeeper: boolean;
}

export interface UpdateUserContextSettingsRequest {
	enabled?: boolean;
	derivationBudgetTier?: DerivationBudgetTier;
	backfillWindowDays?: number;
	backfillGoDeeper?: boolean;
}

/** Reasoning Engine availability snapshot, mirroring the Rust `AiRuntimeStatus`. */
export interface AiRuntimeStatus {
	enabled: boolean;
	configured: boolean;
	available: boolean;
	defaultModel?: AiEngineRef | null;
	reason?: string | null;
}

/** Reasoning Engine test-connection round-trip result, mirroring `AiRuntimeTestResult`. */
export interface AiRuntimeTestResult {
	ok: boolean;
	/** Stable provider id of the global default model's provider. */
	provider: string;
	model: string;
	message: string;
	rawJson: string;
}

/** Fixed v1 Activity taxonomy (engine-tier; may be absent on a tracer). */
export type ActivityCategory =
	| "creating"
	| "communication"
	| "meetings"
	| "research"
	| "learning"
	| "organizing"
	| "personal"
	| "entertainment";

/** A raw-capture evidence reference grounding an Activity. */
export interface ActivityEvidenceRef {
	subjectType: string;
	subjectId: number;
	capturedAtMs?: number | null;
}

/** How engaged the user was during an Activity (issue #109 focus correction). */
export type ActivityFocus = "deep" | "mixed" | "distracted";

/** A derived episode of what the user did and how (the evidence layer). */
export interface Activity {
	id: number;
	title: string;
	summary: string;
	category?: ActivityCategory | null;
	focus?: ActivityFocus | null;
	startedAtMs: number;
	endedAtMs: number;
	createdAtMs: number;
	evidence: ActivityEvidenceRef[];
}

/** One user-authored standing context statement (issue #107 backend DTO). */
export interface AuthoredContext {
	id: number;
	text: string;
	topic: string | null;
	createdAtMs: number;
	updatedAtMs: number;
}

/** Whether a piece of evidence supports or contradicts a Conclusion. */
export type EvidenceStance = "support" | "contradict";

/** Visibility status of a Conclusion (`faded` = below the display floor). */
export type ConclusionStatus = "visible" | "faded" | "dismissed";

/** A reference from a Conclusion to the Activity that is its evidence. */
export interface ConclusionEvidenceRef {
	activityId: number;
	stance: EvidenceStance;
	activityTitle?: string | null;
	activityStartedAtMs?: number | null;
}

/** A distilled, plain-language belief about the user, grounded in Activities. */
export interface Conclusion {
	id: number;
	subject: string;
	statement: string;
	confidence: number;
	status: ConclusionStatus;
	pinned: boolean;
	formedAtMs: number;
	lastSupportedAtMs: number;
	updatedAtMs: number;
	evidence: ConclusionEvidenceRef[];
}

/** A single point on a Conclusion's confidence-over-time line. */
export interface ConfidenceSnapshot {
	confidence: number;
	snapshotAtMs: number;
}

/** A single Conclusion's confidence trajectory for the Subject page. */
export interface SubjectTrajectory {
	conclusionId: number;
	statement: string;
	history: ConfidenceSnapshot[];
}

/** The Subject page: every Conclusion about a Subject plus its trajectories. */
export interface SubjectView {
	subject: string;
	conclusions: Conclusion[];
	trajectories: SubjectTrajectory[];
}

/** Aggregated (estimated) token usage across derivation runs. */
export interface UserContextTokenUsage {
	inputTokens: number;
	outputTokens: number;
	totalTokens: number;
	runCount: number;
}

/**
 * The most recent completed Conclusion-distillation pass with its per-gate
 * withheld counts (the "why is my dossier thin?" readout line).
 */
export interface UserContextDistillationSummary {
	atMs: number;
	conclusionsDerived: number;
	ungrounded: number;
	guardrailSuppressed: number;
	belowFormationBar: number;
	resurfaceBlocked: number;
}

/** Availability + counts + token usage for the User Context settings surface. */
export interface UserContextStatus {
	engineAvailable: boolean;
	reason?: string | null;
	activityCount: number;
	conclusionCount: number;
	lastDerivedAtMs?: number | null;
	backfilling: boolean;
	tokenUsage: UserContextTokenUsage;
	budgetTier: DerivationBudgetTier;
	lastDistillation?: UserContextDistillationSummary | null;
}

/** Result of a manual "Run derivation now" pass, mirroring the Rust DTO. */
export interface UserContextDerivationRunResult {
	activitiesDerived: number;
	conclusionsDerived: number;
	windowStartMs: number;
	windowEndMs: number;
	itemsRead: number;
	message: string;
}

/** The engine-written narrative lede for an Overview range (story feed). */
export interface UserContextDigest {
	rangeKind: string;
	rangeStartMs: number;
	rangeEndMs: number;
	narrative: string;
	generatedAtMs: number;
	/** One-line generated headline above the narrative; absent on old cached digests. */
	headline?: string | null;
}

export interface RecordingSettings {
	captureScreen: boolean;
	captureMicrophone: boolean;
	captureSystemAudio: boolean;
	segmentDurationSeconds: number;
	screenFrameRate: number;
	saveDirectory: string;
	autoStart: boolean;
	screenResolution: ScreenResolution;
	videoBitrate: VideoBitrate;
	nativeCaptureDebugLoggingEnabled: boolean;
	pauseCaptureOnInactivity: boolean;
	idleTimeoutSeconds: number;
	activityMode: ActivityMode;
	microphoneActivitySensitivity: number;
	systemAudioActivitySensitivity: number;
	microphoneVadAdapter?: MicrophoneVadAdapter;
	audioSpeechDetection: AudioSpeechDetectionSettings;
	metadata: MetadataSettings;
	privacy: PrivacySettings;
	access: AccessSettings;
	aiRuntime: AiRuntimeSettings;
	userContext: UserContextSettings;
	semanticSearch: SemanticSearchSettings;
	previewCacheTtlSeconds: number;
	followTimelineLive: boolean;
	retentionPolicy: RetentionPolicy;
	appearance: AppearanceSetting;
	ocr: OcrSettings;
	transcription: AudioTranscriptionSettings;
	speakerAnalysis: SpeakerAnalysisSettings;
	developerOptionsEnabled: boolean;
}

export type SettingsOwnershipDomain =
	| "capture_sources"
	| "capture_timing"
	| "video"
	| "storage"
	| "display"
	| "metadata"
	| "app_privacy_exclusion"
	| "inactivity"
	| "processing"
	| "developer"
	| "keyboard_bindings"
	| "microphone_controller"
	| "app_update"
	| "access"
	| "ai_runtime"
	| "user_context"
	| "semantic_search"
	| "one_time_prompt_state";

export interface RecordingSettingsDomainUpdateResponse {
	domain: SettingsOwnershipDomain;
	settings: RecordingSettings;
}

export type UpdateCaptureSourceSettingsRequest = Partial<
	Pick<RecordingSettings, "captureScreen" | "captureMicrophone" | "captureSystemAudio">
>;

export type UpdateCaptureTimingSettingsRequest = Partial<
	Pick<RecordingSettings, "segmentDurationSeconds" | "autoStart">
>;

export type UpdateVideoSettingsRequest = Partial<
	Pick<RecordingSettings, "screenFrameRate" | "screenResolution" | "videoBitrate">
>;

export type UpdateStorageSettingsRequest = Partial<
	Pick<RecordingSettings, "saveDirectory" | "retentionPolicy">
>;

export type UpdateDisplaySettingsRequest = Partial<
	Pick<RecordingSettings, "appearance" | "followTimelineLive">
>;

export type UpdateMetadataSettingsRequest = Partial<MetadataSettings>;

export type UpdateInactivitySettingsRequest = Partial<
	Pick<
		RecordingSettings,
		| "pauseCaptureOnInactivity"
		| "idleTimeoutSeconds"
		| "microphoneActivitySensitivity"
		| "systemAudioActivitySensitivity"
		| "audioSpeechDetection"
	>
>;

export type UpdateProcessingSettingsRequest = Partial<
	Pick<RecordingSettings, "ocr" | "transcription" | "speakerAnalysis" | "previewCacheTtlSeconds">
>;

export type UpdateDeveloperSettingsRequest = Partial<
	Pick<RecordingSettings, "developerOptionsEnabled" | "nativeCaptureDebugLoggingEnabled">
>;

export interface UpdateAccessSettingsRequest {
	askAiEnabled: boolean;
	askAiMaxToolCalls: number;
	/**
	 * Selected Quick Recall model — a rig-core model id used against the default
	 * Reasoning Engine (not a PI `provider:modelId` pair); empty clears to the
	 * engine default.
	 */
	askAiModel: string;
}

/** One model discovered from a connected provider's models route
 *  (`ai_runtime_list_models`), tagged with the provider it came from. The
 *  merged pool feeds the default-model picker, the Ask AI override picker,
 *  and the Chat thread picker. */
export interface AiRuntimeModel {
	id: string;
	/** Stable provider id (`AiProviderKind`). */
	provider: string;
}

/** One connected provider that failed to list its models, surfaced so the
 *  picker can show it (with a Retry) instead of silently showing fewer models. */
export interface AiRuntimeProviderFailure {
	/** The provider instance id that failed. */
	provider: string;
	/** Short, human-readable reason (`unreachable`, `missing API key`, …). */
	reason: string;
}

/** The result of `ai_runtime_list_models`: the discovered models plus the
 *  providers that failed to list (best-effort listing never drops a provider
 *  silently — a transiently-down endpoint rides back here). */
export interface AiRuntimeModelsResult {
	models: AiRuntimeModel[];
	failures: AiRuntimeProviderFailure[];
}

export interface KeyboardBindingsSettings {
	schemaVersion: number;
	globalShortcuts: GlobalShortcutsSettings;
	appShortcuts: AppShortcutBindings;
	dashboardShortcuts: DashboardShortcutBindings;
	audioDrawerShortcuts: AudioDrawerShortcutBindings;
}

export interface GlobalShortcutsSettings {
	enabled: boolean;
	bindings: GlobalShortcutBindings;
}

export interface GlobalShortcutBindings {
	toggleRecording: string;
	pauseResumeRecording: string;
	toggleMainWindow: string;
	quickRecall: string;
}

export interface AppShortcutBindings {
	openSettings: string;
	openDebug: string;
	toggleSourceScreen: string;
	toggleSourceMicrophone: string;
	toggleSourceSystemAudio: string;
	toggleShortcutsHelp: string;
}

export interface DashboardShortcutBindings {
	openJumpPicker: string;
	jumpLatest: string;
	toggleOcr: string;
	refreshTimeline: string;
	copyFrame: string;
	downloadFrame: string;
}

export interface AudioDrawerShortcutBindings {
	playPause: string;
	seekBack: string;
	seekForward: string;
	seekBackFast: string;
	seekForwardFast: string;
}

export type VideoBitratePreset = "low" | "medium" | "high";

export type VideoBitrate =
	| { mode: "preset"; preset: VideoBitratePreset; customMbps: null }
	| { mode: "custom"; preset: null; customMbps: number };

export type VideoBitrateMode = "preset" | "custom";

export type ScreenResolutionPreset = "original" | "1080p" | "720p" | "540p";

export type ScreenResolution =
	| {
			mode: "preset";
			preset: ScreenResolutionPreset;
	  }
	| {
			mode: "custom";
			width: number;
			height: number;
	  };

export type ResolutionMode = "original" | "preset" | "custom";
export type ResolutionPreset = Exclude<ScreenResolutionPreset, "original">;

export type OcrProvider = "apple_vision" | "tesseract";
export type OcrRecognitionMode = "fast" | "accurate";
export type OcrTesseractPageSegmentationMode =
	| "auto"
	| "single_block"
	| "single_line"
	| "single_word"
	| "sparse_text";
export type OcrTesseractPreprocessMode = "grayscale" | "thresholded";

export interface OcrSettings {
	enabled: boolean;
	provider: OcrProvider;
	modelId: string | null;
	language: string | null;
	recognitionMode: OcrRecognitionMode;
	languageCorrection: boolean;
	tesseractPageSegmentationMode: OcrTesseractPageSegmentationMode;
	tesseractPreprocessMode: OcrTesseractPreprocessMode;
	tesseractUpscaleFactor: number;
	tesseractCharWhitelist: string | null;
}

export type AudioTranscriptionProvider =
	| "local_whisper"
	| "apple_speech_on_device"
	| "parakeet";

export type AudioTranscriptionModelStatusKind =
	| "installed"
	| "missing"
	| "downloading"
	| "failed"
	| "os_managed";

export type AudioTranscriptionModelManagement = "app_managed" | "os_managed";

export type AppleSpeechOnDeviceAvailabilityStatus =
	| "available"
	| "unsupported_platform"
	| "framework_unavailable"
	| "permission_not_determined"
	| "permission_denied"
	| "permission_restricted"
	| "recognizer_unavailable"
	| "on_device_recognition_unavailable";

export type AudioTranscriptionMemoryMode = "balanced" | "low_memory" | "performance";

export interface AudioTranscriptionSettings {
	enabled: boolean;
	microphoneEnabled: boolean;
	systemAudioEnabled: boolean;
	provider: AudioTranscriptionProvider;
	modelId: string | null;
	language: string;
	memoryMode: AudioTranscriptionMemoryMode;
	idleUnloadSeconds: number;
	chunkSeconds: number;
}

export interface AudioSpeechDetectionSettings {
	detector: AudioSpeechDetector;
}

export interface SpeakerAnalysisSettings {
	separateSpeakers: boolean;
	recognizeSavedPeople: boolean;
	provider: "sherpa_onnx" | string;
	modelId: string | null;
	timeoutSeconds: number;
}

export type SpeakerAnalysisModelStatusKind =
	| "not_installed"
	| "installed"
	| "incomplete"
	| "failed"
	| "downloading";

export interface SpeakerAnalysisModelStatusResponse {
	modelsDirectory: string;
	providers: SpeakerAnalysisProviderStatus[];
}

export interface SpeakerAnalysisProviderStatus {
	provider: string;
	displayName: string;
	models: SpeakerAnalysisModelStatus[];
}

export interface SpeakerAnalysisModelStatus {
	provider: string;
	modelId: string | null;
	displayName: string;
	description: string;
	status: SpeakerAnalysisModelStatusKind;
	available: boolean;
	installPath: string;
	missingFiles: string[];
	failureMessage: string | null;
	licenseLabel: string | null;
	sourceUrl: string | null;
	download: SpeakerAnalysisModelDownload | null;
}

export interface SpeakerAnalysisModelDownload {
	url: string;
	byteSize: number;
	sha256: string | null;
	shape: unknown;
}

export type SpeakerAnalysisModelDownloadStatus =
	| "starting"
	| "downloading"
	| "installing"
	| "completed"
	| "failed"
	| "cancelled";

export interface SpeakerAnalysisModelDownloadProgress {
	provider: string;
	modelId: string;
	status: SpeakerAnalysisModelDownloadStatus;
	downloadedBytes: number;
	totalBytes: number | null;
	message: string | null;
}

export interface AudioTranscriptionModelStatusResponse {
	modelsDirectory: string;
	providers: AudioTranscriptionProviderStatus[];
}

export interface AudioTranscriptionProviderStatus {
	provider: AudioTranscriptionProvider;
	displayName: string;
	models: AudioTranscriptionModelStatus[];
}

export interface AudioTranscriptionModelStatus {
	provider: AudioTranscriptionProvider;
	modelId: string | null;
	displayName: string;
	description: string;
	management: AudioTranscriptionModelManagement;
	status: AudioTranscriptionModelStatusKind;
	available: boolean;
	availabilityStatus: AppleSpeechOnDeviceAvailabilityStatus | null;
	installPath: string | null;
	missingFiles: string[];
	failureMessage: string | null;
	licenseLabel: string | null;
	sourceUrl: string | null;
	download: AudioTranscriptionModelDownload | null;
}

export interface AudioTranscriptionModelDownload {
	url: string;
	byteSize: number;
	sha256: string;
	shape: unknown;
}

export type AudioTranscriptionModelDownloadStatus =
	| "starting"
	| "downloading"
	| "installing"
	| "completed"
	| "failed"
	| "cancelled";

export interface AudioTranscriptionModelDownloadProgress {
	provider: AudioTranscriptionProvider;
	modelId: string;
	status: AudioTranscriptionModelDownloadStatus;
	downloadedBytes: number;
	totalBytes: number | null;
	message: string | null;
}

export interface DeletedAudioTranscriptionModel {
	provider: AudioTranscriptionProvider;
	modelId: string;
	displayName: string;
	installPath: string;
}

export interface DeleteUnusedAudioTranscriptionModelsResponse {
	deleted: DeletedAudioTranscriptionModel[];
	skippedActiveDownloads: DeletedAudioTranscriptionModel[];
	skippedProcessingJobs: DeletedAudioTranscriptionModel[];
	retargetedProcessingJobs: number;
}

export type OcrModelStatusKind =
	| "installed"
	| "missing"
	| "downloading"
	| "failed"
	| "os_managed";

export type OcrModelManagement = "app_managed" | "os_managed";

export interface OcrModelStatusResponse {
	modelsDirectory: string;
	providers: OcrProviderStatus[];
}

export interface OcrProviderStatus {
	provider: OcrProvider;
	displayName: string;
	models: OcrModelStatus[];
}

export interface OcrModelStatus {
	provider: OcrProvider;
	modelId: string | null;
	displayName: string;
	description: string;
	management: OcrModelManagement;
	status: OcrModelStatusKind;
	available: boolean;
	installPath: string | null;
	missingFiles: string[];
	failureMessage: string | null;
	licenseLabel: string | null;
	sourceUrl: string | null;
	download: OcrModelDownload | null;
	runtimeMessage: string | null;
}

export interface OcrModelDownload {
	url: string;
	byteSize: number;
	sha256: string;
	shape: unknown;
}

export type OcrModelDownloadStatus =
	| "starting"
	| "downloading"
	| "installing"
	| "completed"
	| "failed"
	| "cancelled";

export interface OcrModelDownloadProgress {
	provider: OcrProvider;
	modelId: string;
	status: OcrModelDownloadStatus;
	downloadedBytes: number;
	totalBytes: number | null;
	message: string | null;
}

export interface DeletedOcrModel {
	provider: OcrProvider;
	modelId: string;
	displayName: string;
	installPath: string;
}

export interface DeleteUnusedOcrModelsResponse {
	deleted: DeletedOcrModel[];
	skippedActiveDownloads: DeletedOcrModel[];
	skippedProcessingJobs: DeletedOcrModel[];
	retargetedProcessingJobs: number;
}

// --- Semantic Search Model Tier (issue #125) ---

export type SemanticSearchModelTier = "english" | "multilingual" | "custom";

export type SemanticSearchModelStatusKind = "installed" | "missing";

export interface SemanticSearchModelStatusResponse {
	modelsDirectory: string;
	models: SemanticSearchModelStatus[];
}

export interface SemanticSearchModelStatus {
	provider: string;
	modelId: string;
	displayName: string;
	description: string;
	tier: SemanticSearchModelTier;
	dimension: number;
	maxTokens: number;
	modelCode: string;
	approxDownloadBytes: number;
	licenseLabel: string | null;
	status: SemanticSearchModelStatusKind;
	available: boolean;
	installPath: string;
	missingFiles: string[];
}

export type SemanticSearchModelDownloadStatus =
	| "starting"
	| "downloading"
	| "installing"
	| "completed"
	| "failed"
	| "cancelled";

export interface SemanticSearchModelDownloadProgress {
	provider: string;
	modelId: string;
	status: SemanticSearchModelDownloadStatus;
	downloadedBytes: number;
	totalBytes: number | null;
	message: string | null;
}

// One entry in the Custom-picker catalog returned by
// `list_semantic_search_supported_models` — the curated set of candle-supported
// on-device models (gated models excluded server-side). Hand-mirrored to the Rust
// serde shape (camelCase). `approxDownloadBytes` may be null when the size is unknown.
export interface SemanticSearchSupportedModel {
	modelId: string;
	displayName: string;
	modelCode: string;
	dimension: number;
	description: string;
	multilingual: boolean;
	approxDownloadBytes: number | null;
}

// Mirrors `capture_types::SemanticSearchSettings` (camelCase serde).
export interface SemanticSearchSettings {
	enabled: boolean;
	provider: string;
	modelId: string | null;
	// Retained for serde compatibility but dormant under the candle backend: candle
	// has no ONNX intra-op thread pool, so this field's only consumer was removed.
	// Kept (not deleted) to preserve the on-wire shape; left at its existing value.
	embedThreads: number;
}
