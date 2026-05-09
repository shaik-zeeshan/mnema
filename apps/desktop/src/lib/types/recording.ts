export type ActivityMode =
	| "system_input_only"
	| "system_input_or_screen"
	| "system_input_or_screen_or_audio";

export type AppearanceSetting = "system" | "light" | "dark";
export type MicrophoneVadAdapter = "silero" | "webrtc" | "off";

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
	microphoneVadAdapter: MicrophoneVadAdapter;
	previewCacheTtlSeconds: number;
	followTimelineLive: boolean;
	appearance: AppearanceSetting;
	ocr: OcrSettings;
	transcription: AudioTranscriptionSettings;
	speakerAnalysis: SpeakerAnalysisSettings;
	developerOptionsEnabled: boolean;
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
	provider: AudioTranscriptionProvider;
	modelId: string | null;
	language: string;
	memoryMode: AudioTranscriptionMemoryMode;
	idleUnloadSeconds: number;
	chunkSeconds: number;
}

export interface SpeakerAnalysisSettings {
	separateSpeakers: boolean;
	recognizeSavedPeople: boolean;
	provider: "sherpa_onnx" | string;
	modelId: string | null;
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
