// Hand-mirrored from `capture-types/src/session.rs` (no codegen — the serde
// wire-pin test there is what keeps these spellings honest).
//
// `assumed_working` / `possibly_blocked` are system audio's, and only system
// audio's: a Core Audio process tap has no authorization query, so its state is
// inferred from whether a tap has ever delivered sound, never read (ADR 0052).
export type PermissionStatus =
	| "granted"
	| "denied"
	| "not_determined"
	| "restricted"
	| "unsupported"
	| "unknown"
	| "assumed_working"
	| "possibly_blocked";

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
	/**
	 * Sources the user turned off mid-session — a user-scoped pause on one
	 * source, always a subset of `requestedSources`. What is actually recording
	 * is `requestedSources` minus this.
	 */
	maskedSources: RequestedSources;
	outputFiles: CaptureOutputFiles | null;
	sourceSessions: SourceSessions | null;
	isInactivityPaused: boolean;
	isUserPaused: boolean;
	isLowDiskSuspended: boolean;
}

export interface GetPermissionsResponse {
	permissions: PermissionsMap;
	session: CaptureSession | null;
}

export interface GeckoBrowserInfo {
	bundleId: string;
	displayName: string;
	installed: boolean;
}

export interface BrowserUrlAccessibilityStatus {
	trusted: boolean;
	geckoBrowsers: GeckoBrowserInfo[];
}

export interface StartCaptureResponse {
	session: CaptureSession;
}

export interface StopCaptureResponse {
	session: CaptureSession;
}
