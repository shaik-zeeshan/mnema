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
	 * PI model id (`provider:modelId`) Quick Recall should use. `null`/empty lets
	 * the PI runtime pick its configured default model.
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

export type AiEngineKind = "cloud" | "local";
export type AiCloudProvider = "anthropic" | "openai" | "openai_compatible";
export type AiLocalKind = "ollama" | "llamafile";

export interface AiRuntimeSettings {
	enabled: boolean;
	engineKind: AiEngineKind;
	cloudProvider: AiCloudProvider;
	cloudModel: string;
	cloudBaseUrl: string;
	localKind: AiLocalKind;
	localEndpoint: string;
	localModel: string;
}

export interface UpdateAiRuntimeSettingsRequest {
	enabled?: boolean;
	engineKind?: AiEngineKind;
	cloudProvider?: AiCloudProvider;
	cloudModel?: string;
	cloudBaseUrl?: string;
	localKind?: AiLocalKind;
	localEndpoint?: string;
	localModel?: string;
}

/** Named Derivation Budget intensity tier for a cloud Reasoning Engine. */
export type DerivationBudgetTier = "light" | "balanced" | "thorough";

/** Non-secret User Context derivation settings domain. */
export interface UserContextSettings {
	derivationBudgetTier: DerivationBudgetTier;
	backfillWindowDays: number;
	backfillGoDeeper: boolean;
}

export interface UpdateUserContextSettingsRequest {
	derivationBudgetTier?: DerivationBudgetTier;
	backfillWindowDays?: number;
	backfillGoDeeper?: boolean;
}

/** Reasoning Engine availability snapshot, mirroring the Rust `AiRuntimeStatus`. */
export interface AiRuntimeStatus {
	enabled: boolean;
	engineKind: string;
	configured: boolean;
	available: boolean;
	hasCloudKey: boolean;
	reason?: string | null;
}

/** Reasoning Engine test-connection round-trip result, mirroring `AiRuntimeTestResult`. */
export interface AiRuntimeTestResult {
	ok: boolean;
	engineKind: string;
	model: string;
	message: string;
	rawJson: string;
}

/** Fixed v1 Activity taxonomy (engine-tier; may be absent on a tracer). */
export type ActivityCategory =
	| "coding"
	| "research"
	| "communication"
	| "design"
	| "testing"
	| "personal"
	| "distractions";

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
	/** Selected Quick Recall model (`provider:modelId`); empty clears to default. */
	askAiModel: string;
}

/** One PI model selectable for Quick Recall, reported by `ask_ai_list_models`. */
export interface AskAiModel {
	/** Stable `provider:modelId` value persisted in settings. */
	value: string;
	provider: string;
	id: string;
	name: string;
}

/** One model id discovered from the Reasoning Engine's `/models` route. */
export interface AiRuntimeModel {
	id: string;
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
	search: string;
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
