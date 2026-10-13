// The one-sentence verdict over the health rollup, plus the plain names and
// home tabs of the health rows. Pure, so it is bun-testable.
//
// Rows are typed structurally with `severity: string` / `feature: string` so
// this compiles against both the old and the 12-row health contract.

import type { DevTab } from "./state/controller.svelte";

export type Tone = "danger" | "warn" | "info" | "off" | "ok";

export interface VerdictRow {
	feature: string;
	severity: string;
	reason: string;
}

export interface Verdict {
	tone: Tone;
	text: string;
}

const CAPTURE_FEATURES = ["screen", "microphone", "systemAudio"];

/** Severity → UI tone. `error` is the only rename; anything unknown reads as ok. */
export function toneOf(severity: string): Tone {
	if (severity === "error") return "danger";
	if (severity === "warn" || severity === "info" || severity === "off") return severity;
	return "ok";
}

export function verdict(rows: VerdictRow[]): Verdict {
	const bad = rows.filter((r) => r.severity === "warn" || r.severity === "error");
	if (bad.length > 0) {
		return {
			tone: bad.some((r) => r.severity === "error") ? "danger" : "warn",
			text: `${bad.length} ${bad.length === 1 ? "thing needs" : "things need"} a look`,
		};
	}
	const info = rows.find((r) => r.severity === "info");
	// Checked before the info row: when nothing records, the info rows are the
	// queues draining, and the mockup's sentence says exactly that.
	const capture = rows.filter((r) => CAPTURE_FEATURES.includes(r.feature));
	if (capture.length > 0 && capture.every((r) => r.severity === "off")) {
		return { tone: "off", text: info ? "Not recording · finishing earlier work" : "Not recording" };
	}
	if (info) return { tone: "info", text: info.reason };
	return { tone: "ok", text: "Everything is running" };
}

/** "2h 14m" / "14m" — how long the current recording has run. */
export function formatRunning(ms: number): string {
	const minutes = Math.max(0, Math.floor(ms / 60_000));
	const h = Math.floor(minutes / 60);
	return h > 0 ? `${h}h ${minutes % 60}m` : `${minutes}m`;
}

/** The plain name and home tab of each health row (the 12-row contract). */
export const FEATURE_ROWS: Record<string, { name: string; tab: DevTab }> = {
	screen: { name: "Screen", tab: "capture" },
	microphone: { name: "Microphone", tab: "capture" },
	systemAudio: { name: "System audio", tab: "capture" },
	reading: { name: "Reading", tab: "pipeline" },
	transcription: { name: "Transcription", tab: "pipeline" },
	speakers: { name: "Speakers", tab: "pipeline" },
	meaningIndex: { name: "Meaning index", tab: "pipeline" },
	aiEngine: { name: "AI engine", tab: "ai" },
	userContext: { name: "User Context", tab: "ai" },
	disk: { name: "Disk", tab: "capture" },
	cliAccess: { name: "CLI access", tab: "ai" },
	startup: { name: "Startup", tab: "logs" },
};
