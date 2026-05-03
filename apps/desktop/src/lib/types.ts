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

export interface SourceSessionMeta {
	sessionId: string;
	startedAtUnixMs: number;
}

export interface SourceSessions {
	screen: SourceSessionMeta | null;
	microphone: SourceSessionMeta | null;
	systemAudio: SourceSessionMeta | null;
}

export interface CaptureSession {
	isRunning: boolean;
	requestedSources: RequestedSources | null;
	outputFiles: CaptureOutputFiles | null;
	sourceSessions: SourceSessions | null;
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
	/**
	 * In-memory TTL (in seconds) for cached frame/image previews. `0` disables
	 * caching entirely; otherwise entries expire automatically after the
	 * configured duration. Defaults to 3600 (1 hour).
	 */
	previewCacheTtlSeconds: number;
	/**
	 * When true, developer-only surfaces (the Debug page and its nav entry)
	 * are exposed in the UI. When false, the Debug page is hidden from
	 * navigation and direct visits to `/` are redirected to `/timeline`.
	 */
	developerOptionsEnabled: boolean;
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
	 * Effective idle time (ms) used by the combined inactivity policy for the current
	 * activity mode. The value is derived from whichever source the policy selects
	 * (e.g. min of system input and screen in hybrid mode, audio level in audio mode)
	 * and is the value compared against idleTimeoutSeconds.
	 */
	effectiveIdleMs: number;
	/**
	 * The activity source the backend selected to produce effectiveIdleMs.
	 * "system_input" — system keyboard/mouse idle is driving the decision.
	 * "screen_capture" — screen-change detection is driving the decision.
	 * "microphone_capture" — microphone audio level is driving the decision.
	 * "system_audio_capture" — system audio level is driving the decision.
	 * "internal_fallback" — no usable reading from any probe; using internal timer.
	 */
	effectiveActivitySource: ActivitySourceKind;
	/** Screen-family effective idle time (ms) evaluated using screen-specific source selection. */
	screenEffectiveIdleMs: number;
	/** The activity source driving the screen-family effective idle reading. */
	screenEffectiveActivitySource: ActivitySourceKind;
	/** Whether the screen capture family is currently paused due to inactivity. */
	screenPaused: boolean;
	/** Microphone-family effective idle time (ms) evaluated using microphone-specific source selection. */
	microphoneEffectiveIdleMs: number;
	/** The activity source driving the microphone-family effective idle reading. */
	microphoneEffectiveActivitySource: ActivitySourceKind;
	/** Whether the microphone capture is currently paused due to inactivity. */
	microphonePaused: boolean;
	/** System-audio-family effective idle time (ms) evaluated using system-audio-specific source selection. */
	systemAudioEffectiveIdleMs: number;
	/** The activity source driving the system-audio-family effective idle reading. */
	systemAudioEffectiveActivitySource: ActivitySourceKind;
	/** Whether the system audio capture is currently paused due to inactivity. */
	systemAudioPaused: boolean;
	/** Per-source idle samples used for policy evaluation. */
	activitySources: IdleDebugActivitySource[];
	/**
	 * Configured microphone activity sensitivity (0–100). Higher = more sensitive.
	 */
	microphoneActivitySensitivity: number;
	/**
	 * Configured system audio activity sensitivity (0–100). Higher = more sensitive.
	 */
	systemAudioActivitySensitivity: number;
	/**
	 * Derived microphone activity threshold (normalised 0–1 level below which audio is
	 * considered "silent"). Computed from sensitivity by the backend.
	 */
	microphoneActivityThreshold: number;
	/**
	 * Derived system audio activity threshold (normalised 0–1 level below which audio is
	 * considered "silent"). Computed from sensitivity by the backend.
	 */
	systemAudioActivityThreshold: number;
	/**
	 * Unix timestamp (ms) of the last raw microphone audio sample, or null if no sample
	 * has been received yet. This is a raw sample timestamp, not a threshold-qualified
	 * activity event.
	 */
	microphoneActivityLastUnixMs: number | null;
	/** Milliseconds of idle since the last threshold-qualified microphone activity event, or null if no such event has occurred. */
	microphoneActivityIdleMs: number | null;
	/** Latest normalised microphone level (0–1), or null if unavailable. */
	microphoneActivityLevel: number | null;
	/** Whether microphone audio activity detection is currently enabled. */
	microphoneActivityEnabled: boolean;
	/** Unix timestamp (ms) of the last raw system audio sample, or null if no sample
	 * has been received yet. This is a raw sample timestamp, not a threshold-qualified
	 * activity event.
	 */
	systemAudioActivityLastUnixMs: number | null;
	/** Milliseconds of idle since the last threshold-qualified system-audio activity event, or null if no such event has occurred. */
	systemAudioActivityIdleMs: number | null;
	/** Latest normalised system audio level (0–1), or null if unavailable. */
	systemAudioActivityLevel: number | null;
	/** Whether system audio activity detection is currently enabled. */
	systemAudioActivityEnabled: boolean;
	/**
	 * Operational truth for each capture source family — what is requested,
	 * whether the underlying capture session/writer is currently attached, and
	 * the on-disk output path when known. Distinguishes "requested but paused"
	 * from "session running" from "writer attached/active".
	 */
	runtimeSources: RuntimeSourcesStatus;
}

/** Mirrors the Rust `RuntimeSourcesStatus` struct. */
export interface RuntimeSourcesStatus {
	screen: RuntimeSourceStatus;
	microphone: RuntimeSourceStatus;
	systemAudio: RuntimeSourceStatus;
}

/** Mirrors the Rust `RuntimeSourceStatus` struct. */
export interface RuntimeSourceStatus {
	/** Source was requested by the active recording. */
	requested: boolean;
	/** Source family is currently inactivity-paused. */
	paused: boolean;
	/**
	 * Native capture session for this source is currently attached/running.
	 * `null` when the platform cannot report this (e.g. non-macOS).
	 */
	sessionActive: boolean | null;
	/**
	 * Output writer for this source is currently attached and accepting samples
	 * (session running AND not paused AND output file resolved). `null` when
	 * the platform cannot report this.
	 */
	writerActive: boolean | null;
	/** Last known on-disk output path for the active segment, when available. */
	outputPath: string | null;
	/**
	 * Short machine-readable reason when truth is unavailable
	 * (e.g. `"non_macos"`, `"not_requested"`). `null` when normal.
	 */
	reason: string | null;
}

export interface IdleDebugActivitySource {
	kind: ActivitySourceKind;
	available: boolean;
	idleMs: number | null;
	selected: boolean;
	/** Whether this source is requested/enabled for evaluation (not necessarily selected or contributing). */
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

/**
 * Mirrors the Rust `FrameDto` struct returned by `list_frames` / `get_frame`.
 *
 * `filePath` is an absolute path on disk; render via Tauri's
 * `convertFileSrc` to obtain a URL safe for `<img src>`.
 */
export interface FrameDto {
	id: number;
	sessionId: string;
	filePath: string;
	capturedAt: string;
	width: number | null;
	height: number | null;
	contentFingerprint: string | null;
	createdAt: string;
	updatedAt: string;
}

/**
 * Discriminator for where a frame preview's bytes came from.
 *
 * Mirrors the Rust `FramePreviewSourceKindDto` enum (snake_case wire values).
 *
 * - `original_frame` — bytes were read directly from the captured frame image
 *   on disk for this exact frame.
 * - `segment_frame_fallback` — bytes were read from another captured frame
 *   image in the same hidden segment workspace when the exact frame image was
 *   unavailable.
 * - `video_fallback` — bytes were re-decoded from the segment video as a
 *   fallback when the original frame image was unavailable.
 */
export type FramePreviewSourceKind =
	| "original_frame"
	| "segment_frame_fallback"
	| "video_fallback";

/**
 * Mirrors the Rust `FramePreviewDto` returned by
 * `invoke('get_frame_preview', { request: { frameId } })`.
 *
 * The frontend builds an `<img src>` URL via
 * `data:${mimeType};base64,${dataBase64}`.
 */
export interface FramePreviewDto {
	mimeType: string;
	dataBase64: string;
	sourceKind: FramePreviewSourceKind;
}

/** Request body for `invoke('get_frame_preview', { request })`. */
export interface GetFramePreviewRequest {
	frameId: number;
}

/** Request body for `invoke('get_timeline_window_around_frame', { request })`. */
export interface GetTimelineWindowAroundFrameRequest {
	frameId: number;
	newerLimit: number;
	olderLimit: number;
}

/** Focused newest-first window centered around a target frame. */
export interface FocusedFrameWindowDto {
	frames: FrameDto[];
	targetIndex: number;
	hasNewer: boolean;
	hasOlder: boolean;
}

/**
 * Lightweight frame summary returned by `list_frame_summaries_in_range`.
 * Carries only the fields the timeline date-jump UI needs to populate the
 * calendar / time picker without paying for full `FrameDto` payloads.
 */
export interface FrameSummaryDto {
	id: number;
	capturedAt: string;
}

/**
 * Request body for `invoke('list_frame_summaries_in_range', { request })`
 * and `invoke('get_latest_frame_in_range', { request })`. Both bounds are
 * ISO 8601 timestamps; the backend treats them as a fully-closed
 * `[start, end]` window (inclusive start, inclusive end) and returns either
 * summaries or the latest matching frame.
 */
export interface FrameRangeRequest {
	capturedAtStart: string;
	capturedAtEnd: string;
}

/** Request body for `invoke('list_frames', { request })`. All fields optional. */
export interface ListFramesRequest {
	sessionId?: string | null;
	limit?: number | null;
	offset?: number | null;
	/**
	 * Cursor for stable pagination: only return frames with `id < beforeId`.
	 * Prefer this over `offset` when paging through a list that may have new
	 * rows inserted at the head between pages.
	 */
	beforeId?: number | null;
}

// ─── Audio Segments ────────────────────────────────────────────────────────

/**
 * Discriminator for which capture source produced an audio segment.
 *
 * Mirrors the Rust `AudioSegmentSourceKind` enum (snake_case wire values).
 * Each kind corresponds to its own independent source session, distinct
 * from the screen/frame session.
 */
export type AudioSegmentSourceKind = "microphone" | "system_audio";

/**
 * Mirrors the Rust `AudioSegmentDto` struct returned by `list_audio_segments`.
 *
 * `filePath` is an absolute path on disk to the segment's `.m4a` file.
 * `startedAt` / `endedAt` are ISO 8601 timestamps marking the segment's
 * captured time range. `sourceSessionId` identifies the per-source capture
 * session (independent from any screen/frame session id).
 */
export interface AudioSegmentDto {
	id: number;
	sourceKind: AudioSegmentSourceKind;
	sourceSessionId: string;
	segmentIndex: number;
	filePath: string;
	startedAt: string;
	endedAt: string;
	createdAt: string;
	updatedAt: string;
}

/**
 * Request body for `invoke('list_audio_segments', { request })`. Both bounds
 * are ISO 8601 timestamps; the backend treats them as a fully-closed
 * `[start, end]` window matching `FrameRangeRequest`.
 */
export interface ListAudioSegmentsRequest {
	capturedAtStart: string;
	capturedAtEnd: string;
}

/** Request body for `invoke('get_audio_segment_media', { request })`. */
export interface GetAudioSegmentMediaRequest {
	audioSegmentId: number;
}

/**
 * Audio segment bytes returned by `get_audio_segment_media`.
 * The frontend builds an `<audio src>` via
 * `data:${mimeType};base64,${dataBase64}`.
 */
export interface AudioSegmentMediaDto {
	mimeType: string;
	dataBase64: string;
}

/** Mirrors the Rust `FrameBatchStatus` enum (snake_case wire values). */
export type FrameBatchStatus = "open" | "closed" | "processing" | "completed" | "failed";

/** Mirrors the Rust `ProcessingJobStatus` enum (snake_case wire values). */
export type ProcessingJobStatus = "queued" | "running" | "completed" | "failed";

/** Mirrors the Rust `SegmentWorkspaceCleanupDisposition` enum (snake_case wire values). */
export type SegmentWorkspaceCleanupDisposition =
	| "referenced_by_incomplete_batch"
	| "referenced_by_nonterminal_ocr"
	| "missing_visible_segment_sibling"
	| "completed_only"
	| "no_references";

/** Mirrors the Rust `HiddenSegmentWorkspacePathsDto`. */
export interface HiddenSegmentWorkspacePathsDto {
	workspaceDir: string;
	framesDir: string;
	visibleSegmentPath: string;
}

/** Mirrors the Rust `SegmentWorkspaceBatchReferenceDto`. */
export interface SegmentWorkspaceBatchReferenceDto {
	batchId: number;
	status: FrameBatchStatus;
}

/** Mirrors the Rust `SegmentWorkspaceOcrReferenceDto`. */
export interface SegmentWorkspaceOcrReferenceDto {
	frameId: number;
	jobId: number;
	status: ProcessingJobStatus;
}

/**
 * Mirrors the Rust `SegmentWorkspaceCleanupDebugInfoDto` returned by
 * `classify_hidden_segment_workspace`. The command returns `null` when the
 * supplied path does not look like a hidden segment workspace directory.
 */
export interface SegmentWorkspaceCleanupDebugInfoDto {
	paths: HiddenSegmentWorkspacePathsDto;
	disposition: SegmentWorkspaceCleanupDisposition;
	safeToRemove: boolean;
	visibleSegmentExists: boolean;
	frameCount: number;
	batchReferences: SegmentWorkspaceBatchReferenceDto[];
	nonterminalOcrReferences: SegmentWorkspaceOcrReferenceDto[];
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
