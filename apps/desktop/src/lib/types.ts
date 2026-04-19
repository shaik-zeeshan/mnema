// ─── Types mirroring Rust structs ─────────────────────────────────────────

export type PermissionStatus = "granted" | "denied" | "not_determined" | "restricted";

export type MicrophonePreferenceMode = "default" | "specific_device";
export type MicrophoneDisconnectPolicy = "fallback_to_default" | "wait_for_same_device";

export interface MicrophoneDevice {
	id: string;
	name: string;
	isDefault: boolean;
}

export interface MicrophonePreference {
	mode: MicrophonePreferenceMode;
	deviceId: string | null;
}

export interface MicrophoneControllerState {
	devices: MicrophoneDevice[];
	preference: MicrophonePreference;
	disconnectPolicy: MicrophoneDisconnectPolicy;
	effectiveDevice: MicrophoneDevice | null;
}

export interface MicrophoneAutoDisconnectTransitionFailedEvent {
	context: string;
	code: string;
	message: string;
}

export interface SupportedSources {
	screen: boolean;
	microphone: boolean;
	systemAudio: boolean;
}

export interface CaptureSupport {
	platform: string;
	nativeCaptureSupported: boolean;
	supportedSources: SupportedSources;
}

export interface PermissionsMap {
	screen: PermissionStatus;
	microphone: PermissionStatus;
	systemAudio: PermissionStatus;
}

export interface RequestedSources {
	screen: boolean;
	microphone: boolean;
	systemAudio: boolean;
}

export interface CaptureOutputFiles {
	screenFile: string | null;
	screenFiles: string[];
	microphoneFile: string | null;
	microphoneFiles: string[];
	systemAudioFile: string | null;
	systemAudioFiles: string[];
}

export interface CaptureSession {
	isRunning: boolean;
	sessionId: string | null;
	startedAtUnixMs: number | null;
	requestedSources: RequestedSources | null;
	outputFiles: CaptureOutputFiles | null;
	/** Set by the backend when inactivity gating has paused capture. */
	isInactivityPaused: boolean;
}

export interface GetPermissionsResponse {
	permissions: PermissionsMap;
	session: CaptureSession | null;
}

export interface StartCaptureResponse {
	session: CaptureSession;
}

export interface StopCaptureResponse {
	session: CaptureSession;
}

/**
 * Controls which signals are considered "activity" for the inactivity-gating
 * feature.
 *
 * - `system_input_only`                   — only keyboard/mouse/pointer events count.
 * - `system_input_or_screen`              — keyboard/mouse/pointer OR visible on-screen
 *   changes (video, animations, calls) also count, preventing spurious pauses
 *   during calls or video playback with no direct user input.
 * - `system_input_or_screen_or_audio`     — all of the above PLUS microphone and system
 *   audio levels: if either audio source is above the configured sensitivity threshold,
 *   capture stays active.  Higher sensitivity means quieter audio counts as activity.
 */
export type ActivityMode =
	| "system_input_only"
	| "system_input_or_screen"
	| "system_input_or_screen_or_audio";

export type ActivitySourceKind =
	| "system_input"
	| "screen_capture"
	| "microphone_capture"
	| "system_audio_capture"
	| "internal_fallback";

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
	/** Whether native-capture debug logging is enabled. */
	nativeCaptureDebugLoggingEnabled: boolean;
	pauseCaptureOnInactivity: boolean;
	idleTimeoutSeconds: number;
	/** Which signals count as "activity" for the inactivity-gating feature. */
	activityMode: ActivityMode;
	/**
	 * Microphone activity sensitivity (0–100). Only relevant when `activityMode` is
	 * `system_input_or_screen_or_audio`. Higher values mean quieter audio is
	 * treated as activity (more sensitive). Lower values require louder audio
	 * before it is counted as activity.
	 */
	microphoneActivitySensitivity: number;
	/**
	 * System audio activity sensitivity (0–100). Only relevant when `activityMode` is
	 * `system_input_or_screen_or_audio`. Higher values mean quieter audio is
	 * treated as activity (more sensitive). Lower values require louder audio
	 * before it is counted as activity.
	 */
	systemAudioActivitySensitivity: number;
}

// ─── Native Capture Debug Log ──────────────────────────────────────────────

/** Status of the native-capture debug log file, returned by Tauri commands. */
export interface NativeCaptureDebugLogStatus {
	enabled: boolean;
	path: string;
	exists: boolean;
}

// ─── General Application Log ───────────────────────────────────────────────

/** Status of the general application log file, returned by Tauri commands. */
export interface GeneralAppLogStatus {
	path: string;
	exists: boolean;
}

// ─── Video Bitrate ──────────────────────────────────────────────────────────

/** Named bitrate presets. */
export type VideoBitratePreset = "low" | "medium" | "high";

/**
 * Union mirroring the Rust `VideoBitrate` enum.
 *
 * - `preset` mode selects a named preset (low / medium / high).
 * - `custom` mode uses an exact integer Mbps value; `customMbps` must be a
 *   whole number (no decimals) and is null when the mode is `preset`.
 */
export type VideoBitrate =
	| { mode: "preset"; preset: VideoBitratePreset; customMbps: null }
	| { mode: "custom"; preset: null; customMbps: number };

/** UI-only discriminant used by the settings draft. */
export type VideoBitrateMode = "preset" | "custom";

// ─── Screen Resolution ─────────────────────────────────────────────────────

/** Named resolution presets (height-based labels). */
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

/** UI-only mode split that keeps "original" as a top-level option. */
export type ResolutionMode = "original" | "preset" | "custom";
export type ResolutionPreset = Exclude<ScreenResolutionPreset, "original">;

// ─── Idle Debug ─────────────────────────────────────────────────────────────

/** Mirrors the Rust `IdleDebugInfo` struct returned by `get_idle_debug`. */
export interface IdleDebugInfo {
	/** Current system-level idle time in milliseconds, or null if unavailable. */
	systemIdleMs: number | null;
	/** Whether the native idle probe returned a valid reading (probe availability). */
	systemIdleAvailable: boolean;
	/** Whether the inactivity gating feature is enabled in current settings. */
	inactivityEnabled: boolean;
	/** Configured inactivity timeout in seconds (0 when feature is disabled). */
	idleTimeoutSeconds: number;
	/** Whether capture is currently paused due to inactivity. */
	isInactivityPaused: boolean;
	/** Detector source: "core_graphics" on macOS (valid probe), "core_graphics_unavailable" (invalid probe), or "unavailable" elsewhere. */
	detectorSource: string;
	/**
	 * The configured activity mode as reported by the backend at runtime.
	 * "system_input_only" — only keyboard/mouse idle is considered.
	 * "system_input_or_screen" — hybrid mode: min(system input idle, screen idle) is used.
	 * "system_input_or_screen_or_audio" — audio mode: microphone and system audio levels
	 *   also contribute; audio above the sensitivity threshold counts as activity.
	 */
	activityMode: ActivityMode;
	/**
	 * Unix timestamp (ms) of the last observed on-screen activity sample, if any.
	 * null when no screen frame has been sampled yet (e.g. screen capture not running).
	 */
	screenActivityLastUnixMs: number | null;
	/** Current screen-activity idle derived by backend from latest sample, if any. */
	screenActivityIdleMs: number | null;
	/**
	 * Effective idle time (ms) used by the inactivity policy to decide whether to pause.
	 * In hybrid mode this is min(systemIdleMs, screen idle ms). This is the value compared
	 * against idleTimeoutSeconds — systemIdleMs alone does NOT trigger pause in hybrid mode
	 * if the screen is still active.
	 */
	effectiveIdleMs: number;
	/**
	 * The activity source the backend actually selected for the effective idle reading.
	 * "system_input" — system keyboard/mouse idle is driving the decision.
	 * "screen_capture" — screen-change detection is driving the decision (hybrid mode,
	 *   screen is less idle than system input).
	 * "microphone_capture" — microphone audio level is driving the decision (audio mode).
	 * "system_audio_capture" — system audio level is driving the decision (audio mode).
	 * "internal_fallback" — neither probe returned a usable reading; using internal timer.
	 */
	effectiveActivitySource: ActivitySourceKind;
	/** Screen-family effective idle time (ms) evaluated using screen-specific source selection. */
	screenEffectiveIdleMs: number;
	/** The activity source driving the screen-family effective idle reading. */
	screenEffectiveActivitySource: ActivitySourceKind;
	/** Whether the screen capture family is currently paused due to inactivity. */
	screenPaused: boolean;
	/** Audio-family effective idle time (ms) evaluated using audio-specific source selection. */
	audioEffectiveIdleMs: number;
	/** The activity source driving the audio-family effective idle reading. */
	audioEffectiveActivitySource: ActivitySourceKind;
	/** Whether the audio capture family is currently paused due to inactivity. */
	audioPaused: boolean;
	/** Whether the microphone capture is currently paused due to inactivity. */
	microphonePaused: boolean;
	/** Whether the system audio capture is currently paused due to inactivity. */
	systemAudioPaused: boolean;
	/** Per-source idle samples used for policy evaluation. */
	activitySources: IdleDebugActivitySource[];
	/**
	 * Configured microphone activity sensitivity (0–100). Higher = more sensitive.
	 */
	microphoneActivitySensitivity: number | null;
	/**
	 * Configured system audio activity sensitivity (0–100). Higher = more sensitive.
	 */
	systemAudioActivitySensitivity: number | null;
	/**
	 * Derived microphone activity threshold (normalised 0–1 level below which audio is
	 * considered "silent"). Computed from sensitivity by the backend.
	 */
	microphoneActivityThreshold: number | null;
	/**
	 * Derived system audio activity threshold (normalised 0–1 level below which audio is
	 * considered "silent"). Computed from sensitivity by the backend.
	 */
	systemAudioActivityThreshold: number | null;
	/** Unix timestamp (ms) of the last microphone activity detection, or null. */
	microphoneActivityLastUnixMs: number | null;
	/** Milliseconds since last microphone activity (derived from last timestamp). */
	microphoneActivityIdleMs: number | null;
	/** Latest normalised microphone level (0–1), or null if unavailable. */
	microphoneActivityLevel: number | null;
	/** Whether microphone audio activity detection is currently enabled. */
	microphoneActivityEnabled: boolean;
	/** Unix timestamp (ms) of the last system audio activity detection, or null. */
	systemAudioActivityLastUnixMs: number | null;
	/** Milliseconds since last system audio activity (derived from last timestamp). */
	systemAudioActivityIdleMs: number | null;
	/** Latest normalised system audio level (0–1), or null if unavailable. */
	systemAudioActivityLevel: number | null;
	/** Whether system audio activity detection is currently enabled. */
	systemAudioActivityEnabled: boolean;
}

export interface IdleDebugActivitySource {
	kind: ActivitySourceKind;
	available: boolean;
	idleMs: number | null;
	selected: boolean;
	/** Whether this source is actively contributing to activity detection. */
	enabled: boolean;
	/** Latest normalised signal level for audio sources (0–1); null for non-audio sources. */
	latestNormalizedLevel: number | null;
	/** Activity threshold for this source (normalised 0–1); null for non-audio sources. */
	activityThreshold: number | null;
}

// ─── App Infra ──────────────────────────────────────────────────────────────

/** Mirrors the Rust `BackgroundJobStatus` enum (snake_case wire values). */
export type BackgroundJobStatus = "queued" | "running" | "completed" | "failed";

/** Mirrors the Rust `JobCounts` struct returned inside `AppInfraStatus`. */
export interface JobCounts {
	total: number;
	queued: number;
	running: number;
	completed: number;
	failed: number;
}

/** Mirrors the Rust `AppInfraStatus` struct returned by `get_app_infra_status`. */
export interface AppInfraStatus {
	databasePath: string;
	migrationsRan: boolean;
	workerThreadCount: number;
	jobCounts: JobCounts;
}

/** Mirrors the Rust `AppJobDto` struct returned by job-related commands. */
export interface AppJobDto {
	id: number;
	kind: string;
	status: BackgroundJobStatus;
	payloadJson: string | null;
	resultText: string | null;
	attemptCount: number;
	lastError: string | null;
	createdAt: string;
	updatedAt: string;
	startedAt: string | null;
	finishedAt: string | null;
}
