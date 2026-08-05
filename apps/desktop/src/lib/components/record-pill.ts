// Pure state mapping for the title-bar recording pill (design frame 11).
// Nine states; recording red is a STATE, never an error — transient liveness
// tints warn/info, never danger. Kept free of Svelte/Tauri imports so the
// mapping is unit-testable with `bun test`.

import type { PermissionsMap, RuntimeSourcesStatus } from "$lib/types";

export type PillStateId =
	| "idle"
	| "starting"
	| "stopping"
	| "recording"
	| "paused-user"
	| "paused-inactivity"
	| "low-disk"
	| "display-unavailable"
	| "source-degraded"
	| "permission-missing";

export interface PillInputs {
	running: boolean;
	starting: boolean;
	stopping: boolean;
	settingsLoading: boolean;
	userPaused: boolean;
	inactivityPaused: boolean;
	lowDisk: boolean;
	/** System idle time (ms) while inactivity-paused; null when unknown. */
	idleMs: number | null;
	sources: RuntimeSourcesStatus | null;
	permissions: PermissionsMap | null;
	/** Sources selected for the next session (settings). */
	selected: { screen: boolean; microphone: boolean; systemAudio: boolean };
}

export interface PillView {
	state: PillStateId;
	/** "button" renders the idle Record button — a state pill needs a session. */
	kind: "button" | "pill";
	tone: "record" | "quiet" | "warn";
	dot: "rec" | "idle" | "warn" | "spinner";
	showTime: boolean;
	showCost: boolean;
	word: string | null;
	wordTone: "plain" | "info" | "warn";
}

function degraded(src: {
	requested: boolean;
	paused: boolean;
	sessionActive: boolean | null;
	writerActive: boolean | null;
}): boolean {
	return (
		src.requested &&
		!src.paused &&
		(src.sessionActive === false || src.writerActive === false)
	);
}

const PERMISSION_BLOCKED: readonly string[] = ["denied", "restricted"];

/** The source (screen/mic) selected for next session whose permission is
 *  denied — system audio is excluded by design: its tap has no permission
 *  query, only inferred evidence (ADR 0052). */
export function blockedPermissionSource(
	permissions: PermissionsMap | null,
	selected: PillInputs["selected"],
): "screen" | "microphone" | null {
	if (!permissions) return null;
	if (selected.screen && PERMISSION_BLOCKED.includes(permissions.screen)) return "screen";
	if (selected.microphone && PERMISSION_BLOCKED.includes(permissions.microphone)) {
		return "microphone";
	}
	return null;
}

export function pillView(i: PillInputs): PillView {
	const base = {
		kind: "pill" as const,
		tone: "record" as const,
		dot: "rec" as const,
		showTime: false,
		showCost: false,
		word: null,
		wordTone: "plain" as const,
	};

	if (!i.running) {
		if (i.starting) {
			return { ...base, state: "starting", tone: "quiet", dot: "spinner", word: "Starting" };
		}
		if (blockedPermissionSource(i.permissions, i.selected)) {
			const source = blockedPermissionSource(i.permissions, i.selected);
			return {
				...base,
				state: "permission-missing",
				tone: "warn",
				dot: "warn",
				word: `${source === "microphone" ? "mic" : "screen"} not allowed`,
			};
		}
		return { ...base, state: "idle", kind: "button" };
	}

	if (i.stopping) {
		return { ...base, state: "stopping", tone: "quiet", dot: "spinner", word: "Stopping" };
	}
	// Low-disk keeps precedence over the pause words (ADR 0040), matching the
	// shipping status label; a manual pause outranks the automatic one.
	if (i.lowDisk) {
		return { ...base, state: "low-disk", tone: "warn", dot: "warn", word: "Low disk" };
	}
	if (i.userPaused) {
		return { ...base, state: "paused-user", tone: "quiet", dot: "idle", word: "Paused", showTime: true };
	}
	if (i.inactivityPaused) {
		const minutes = i.idleMs === null ? null : Math.max(1, Math.floor(i.idleMs / 60_000));
		return {
			...base,
			state: "paused-inactivity",
			tone: "quiet",
			dot: "idle",
			word: minutes === null ? "Idle" : `Idle ${minutes}m`,
		};
	}

	const screen = i.sources?.screen ?? null;
	if (screen?.reason === "privacy_recovery_restart_required") {
		return { ...base, state: "source-degraded", tone: "warn", dot: "warn", word: "restart screen" };
	}
	// Display-unavailable is info, not error: the dot stays red, the timer
	// runs, mic + system audio keep recording (ADR 0021).
	if (screen?.reason === "capture_display_unavailable") {
		return { ...base, state: "display-unavailable", showTime: true, word: "screen asleep", wordTone: "info" };
	}
	if (screen?.reason === "privacy_filter_apply_failed") {
		return { ...base, state: "source-degraded", showTime: true, word: "screen retrying", wordTone: "warn" };
	}
	const degradedWord =
		i.sources && degraded(i.sources.microphone)
			? "mic lost"
			: i.sources && degraded(i.sources.systemAudio)
				? "audio lost"
				: screen && degraded(screen)
					? "screen lost"
					: null;
	if (degradedWord) {
		return { ...base, state: "source-degraded", showTime: true, word: degradedWord, wordTone: "warn" };
	}

	return { ...base, state: "recording", showTime: true, showCost: true };
}

/** "2:14:07" — hours never padded, minutes/seconds always two digits. */
export function formatElapsed(ms: number): string {
	const total = Math.max(0, Math.floor(ms / 1000));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

/** Compact byte capsule per frame 11 ("270 MB", "1.4 GB") — decimal/SI like
 *  the app's one byte formatter, but whole megabytes so the pill stays tight. */
export function formatPillBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) return "";
	if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
	return `${Math.round(bytes / 1e6)} MB`;
}
