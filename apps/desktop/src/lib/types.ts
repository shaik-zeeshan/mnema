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
