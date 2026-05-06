import type { ActivityMode } from "./recording";

export type ActivitySourceKind =
	| "system_input"
	| "screen_capture"
	| "microphone_capture"
	| "system_audio_capture"
	| "internal_fallback";

export interface AudioActivitySample {
	lastUnixMs: number | null;
	level: number | null;
}

export interface AudioActivityDecision {
	enabled: boolean;
	idleMs: number | null;
	activityThreshold: number | null;
	detector: string | null;
}

export interface MicrophoneVadStatus {
	configuredAdapter: string;
	effectiveAdapter: string;
	fallbackReason: string | null;
}

export interface IdleDebugInfo {
	systemIdleMs: number | null;
	systemIdleAvailable: boolean;
	inactivityEnabled: boolean;
	idleTimeoutSeconds: number;
	isInactivityPaused: boolean;
	detectorSource: string;
	activityMode: ActivityMode;
	microphoneActivitySensitivity: number;
	systemAudioActivitySensitivity: number;
	screenActivityLastUnixMs: number | null;
	screenActivityIdleMs: number | null;
	microphoneActivitySample: AudioActivitySample;
	microphoneActivityDecision: AudioActivityDecision;
	systemAudioActivitySample: AudioActivitySample;
	systemAudioActivityDecision: AudioActivityDecision;
	microphoneVad: MicrophoneVadStatus;
	effectiveIdleMs: number;
	effectiveActivitySource: ActivitySourceKind;
	screenEffectiveIdleMs: number;
	screenEffectiveActivitySource: ActivitySourceKind;
	screenPaused: boolean;
	microphoneEffectiveIdleMs: number;
	microphoneEffectiveActivitySource: ActivitySourceKind;
	microphonePaused: boolean;
	systemAudioEffectiveIdleMs: number;
	systemAudioEffectiveActivitySource: ActivitySourceKind;
	systemAudioPaused: boolean;
	activitySources: IdleDebugActivitySource[];
	runtimeSources: RuntimeSourcesStatus;
}

export interface RuntimeSourcesStatus {
	screen: RuntimeSourceStatus;
	microphone: RuntimeSourceStatus;
	systemAudio: RuntimeSourceStatus;
}

export interface RuntimeSourceStatus {
	requested: boolean;
	paused: boolean;
	sessionActive: boolean | null;
	writerActive: boolean | null;
	outputPath: string | null;
	reason: string | null;
}

export interface IdleDebugActivitySource {
	kind: ActivitySourceKind;
	available: boolean;
	idleMs: number | null;
	selected: boolean;
	enabled: boolean;
	latestNormalizedLevel: number | null;
	activityThreshold: number | null;
}
