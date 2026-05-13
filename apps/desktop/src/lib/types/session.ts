export type PermissionStatus = "granted" | "denied" | "not_determined" | "restricted" | "unsupported" | "unknown";

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
	accessibility: PermissionStatus;
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
