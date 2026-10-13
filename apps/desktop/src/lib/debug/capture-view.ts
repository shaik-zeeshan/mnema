// Pure view helpers for the developer window's Capture tab: lane words and
// details, the level meter, plain rebuild reasons and the disk headroom scale.

import type { BrowserUrlMode } from "$lib/types";
import type { CaptureSourceHealth, CaptureSourceState, SystemAudioTapRebuildReason } from "$lib/types/debug";
import { formatRunning } from "./verdict";

export type LaneTone = "info" | "warn" | "danger" | null;

export const LANE_WORD: Record<CaptureSourceState, { word: string; tone: LaneTone }> = {
	recording: { word: "Recording", tone: null },
	paused: { word: "Paused", tone: "info" },
	waiting: { word: "Waiting", tone: "info" },
	reconnecting: { word: "Reconnecting", tone: "warn" },
	failed: { word: "Failed", tone: "danger" },
	off: { word: "Off", tone: null },
};

// Raw reason code (suspension reason, pause cause or the tap's snake_case
// rebuild reason) → plain lowercase phrase. This is what makes "display asleep"
// reach the screen lane on macOS whatever the session flags say.
const REASON: Record<string, string> = {
	capture_display_unavailable: "display asleep",
	privacy_filter_apply_failed: "privacy filter retrying",
	privacy_recovery_restart_required: "privacy filter needs a restart",
	capture_low_disk: "disk almost full",
	not_requested: "turned off in Settings",
	user_paused: "paused by you",
	inactivity: "nothing happening",
	default_output_device_changed: "output device changed",
	device_sample_rate_changed: "sample rate changed",
	device_died: "output device went away",
	exclude_list_moved: "kept-out apps moved",
	zero_watchdog: "no sound for a while",
};

export function reasonPhrase(code: string | null | undefined): string | null {
	return code ? (REASON[code] ?? null) : null;
}

const REBUILD: Record<SystemAudioTapRebuildReason, string> = {
	deviceChanged: "output device changed",
	deviceDied: "output device went away",
	sampleRateChanged: "sample rate changed",
	excludeListMoved: "kept-out apps moved",
	silenceWatchdog: "no sound for a while",
};

/** "Sample rate changed" — the spec row's sentence-case rebuild reason. */
export function rebuildReasonLabel(reason: SystemAudioTapRebuildReason | null): string {
	if (!reason) return "—";
	const text = REBUILD[reason];
	return text[0].toUpperCase() + text.slice(1);
}

/** Lane detail: device name or the plain reason, plus "48 kHz" for system audio. */
export function laneDetail(src: CaptureSourceHealth, sampleRateHz: number | null = null): string {
	const parts = [src.deviceName ?? reasonPhrase(src.reason)];
	if (sampleRateHz && src.state !== "off") parts.push(`${Math.round(sampleRateHz / 1000)} kHz`);
	return parts.filter(Boolean).join(" · ");
}

/** Lit bars on the 12-bar meter: clamp((dB + 60) / 60 · 12). */
export function meterBars(levelDb: number | null | undefined): number {
	if (levelDb == null) return 0;
	return Math.max(0, Math.min(12, Math.round(((levelDb + 60) / 60) * 12)));
}

/** "6 min" under an hour, else "2h 14m". */
export function formatWait(ms: number): string {
	return ms < 3_600_000 ? `${Math.max(0, Math.floor(ms / 60_000))} min` : formatRunning(ms);
}

/** "30 s" / "2 min" for the silence-check cadence and countdown. */
export function formatSeconds(ms: number): string {
	const s = Math.max(0, Math.round(ms / 1000));
	return s < 120 ? `${s} s` : `${Math.round(s / 60)} min`;
}

export const WEB_ADDRESS: Record<BrowserUrlMode, string> = {
	off: "not saved",
	sanitized: "saved without query strings",
	full: "saved in full",
};

const GB = 1024 ** 3; // binary, like the backend thresholds and formatBytes
export const DISK_SCALE_BYTES = 25 * GB;

/** Bare GB the way the mockup writes them: "182", "41.2", "5" (no unit, no ".0"). */
export function gb(bytes: number): string {
	const v = bytes / GB;
	return String(v >= 100 ? Math.round(v) : Math.round(v * 10) / 10);
}

/** Fraction (0–1) of the 0–25 GB headroom scale. */
export function diskAt(bytes: number): number {
	return Math.max(0, Math.min(bytes / DISK_SCALE_BYTES, 1));
}
