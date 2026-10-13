// Developer-window formatters (plain strings only; no CSS class emitters).

import { formatClock, formatClockRange } from "$lib/time-format";
import type {
	DebugSeverity,
	OcrAdmissionSignals,
	PermissionStatus,
	SegmentWorkspaceCleanupDisposition,
} from "$lib/types";

export function formatDebugList(values: Array<string | number> | null | undefined): string {
	if (!values || values.length === 0) return "none";
	return values.join(", ");
}

export function formatIdleMs(ms: number | null | undefined): string {
	if (ms == null) return "unavailable";
	if (ms < 1000) return `${ms} ms`;
	return `${(ms / 1000).toFixed(1)} s`;
}

export function formatOptionalMs(ms: number | null | undefined): string {
	return ms == null ? "—" : `${ms} ms`;
}

export function formatDebugTime(value: string): string {
	const parsed = Date.parse(value);
	return Number.isNaN(parsed) ? value : formatClock(parsed, { seconds: true });
}

export function truncateDebugText(value: string | null | undefined, max = 80): string {
	if (!value) return "—";
	return value.length <= max ? value : `${value.slice(0, max - 1)}…`;
}

export function formatTimestamp(ms: number): string {
	return formatClock(ms, { seconds: true });
}

export function formatSourceStartedAt(ms: number | null | undefined): string {
	return ms != null ? formatTimestamp(ms) : "—";
}

export function activeSignalBadges(signals: OcrAdmissionSignals): string[] {
	const badges: string[] = [];
	if (signals.firstCandidateInScope) badges.push("first");
	if (signals.contextChanged) badges.push("context");
	if (signals.lowQueuePressure) badges.push("low-q");
	if (signals.highQueuePressure) badges.push("high-q");
	if (signals.representativeDue) badges.push("repr");
	if (signals.fingerprintNovelInScope) badges.push("novel");
	if (signals.noveltyAdmissionAvailable) badges.push("novelty-ok");
	return badges;
}

/** Human-readable label for the activity mode, clarifying hybrid behaviour. */
export function formatActivityMode(mode: string): string {
	if (mode === "system_input_only") return "input-only";
	if (mode === "system_input_or_screen") return "hybrid (input + screen)";
	if (mode === "system_input_or_screen_or_audio") return "audio (input + screen + audio)";
	return mode;
}

/** Human-readable label for the effective idle source. */
export function formatEffectiveSource(src: string): string {
	if (src === "system_input") return "system input";
	if (src === "screen_capture") return "screen activity";
	if (src === "microphone_capture") return "microphone audio";
	if (src === "system_audio_capture") return "system audio";
	if (src === "internal_fallback") return "internal fallback";
	return src;
}

export function sourceKindLabel(src: string): string {
	return formatEffectiveSource(src);
}

export function shortenPath(p: string | null | undefined, max = 48): string {
	if (!p) return "—";
	if (p.length <= max) return p;
	const head = p.slice(0, 12);
	const tail = p.slice(-(max - 12 - 1));
	return `${head}…${tail}`;
}

export function sourceDecisionSummary(available: boolean, selected: boolean, enabled?: boolean): string {
	if (selected) return "selected";
	if (!available) return "unavailable";
	if (enabled === false) return "available, disabled";
	return "available, not selected";
}

/** Compact runtime status word for a source family. */
export function runtimeStateWord(src: {
	requested: boolean;
	paused: boolean;
	sessionActive: boolean | null;
	writerActive: boolean | null;
	reason: string | null;
}): { word: string; cls: string } {
	if (!src.requested) return { word: "off", cls: "rs-state rs-state--off" };
	if (src.sessionActive === null) return { word: src.reason ?? "unknown", cls: "rs-state rs-state--unknown" };
	if (src.paused) return { word: "paused", cls: "rs-state rs-state--paused" };
	if (src.sessionActive && src.writerActive) return { word: "running", cls: "rs-state rs-state--running" };
	if (src.sessionActive && !src.writerActive) return { word: "session only", cls: "rs-state rs-state--partial" };
	return { word: "idle", cls: "rs-state rs-state--idle" };
}

export function formatPermission(status: PermissionStatus | undefined): string {
	if (!status) return "unknown";
	return status.replace(/_/g, " ");
}

export function dispositionLabel(d: SegmentWorkspaceCleanupDisposition): string {
	switch (d) {
		case "referenced_by_incomplete_batch": return "referenced by incomplete batch";
		case "referenced_by_nonterminal_ocr": return "referenced by non-terminal OCR";
		case "missing_visible_segment_sibling": return "missing visible segment sibling";
		case "dead_segment_without_artifacts": return "dead segment without artifacts";
		case "pending_frame_artifacts": return "pending frame artifacts";
		case "completed_only": return "completed only";
		case "no_references": return "no references";
		default: return d;
	}
}

/** A plain word for a health severity. */
export function severityLabel(severity: DebugSeverity | null): string {
	if (severity === "ok") return "healthy";
	if (severity === "warn") return "degraded";
	if (severity === "error") return "failing";
	return "unknown";
}

/** A wall-clock time from epoch ms, or an em-dash when absent. */
export function formatOptionalTime(ms: number | null | undefined): string {
	return ms == null ? "—" : formatClock(ms, { seconds: true });
}

/** `12:00–14:00` for a derivation run's window, or an em-dash when open-ended. */
export function formatWindow(startMs: number | null, endMs: number | null): string {
	if (startMs == null || endMs == null) return "—";
	return formatClockRange(startMs, endMs, { seconds: true });
}

/** Human-readable byte size (`4.2 GB`), or an em-dash when absent. */
export function formatBytes(bytes: number | null | undefined): string {
	if (bytes == null) return "—";
	if (bytes < 1024) return `${bytes} B`;
	const units = ["KB", "MB", "GB", "TB"];
	let value = bytes;
	let unit = 0;
	do {
		value /= 1024;
		unit += 1;
	} while (value >= 1024 && unit < units.length);
	return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit - 1]}`;
}

/** Thousands-separated count — job/vector counts get long enough to need it. */
export function formatCount(n: number | null | undefined): string {
	return n == null ? "—" : n.toLocaleString();
}

function normalizeJobTsForDate(ts: string): string {
	const trimmed = ts.trim();
	// SQLite CURRENT_TIMESTAMP is typically "YYYY-MM-DD HH:MM:SS" in UTC.
	// Convert that shape to a browser-safe ISO-8601 string before parsing.
	if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(trimmed)) {
		return trimmed.replace(" ", "T") + "Z";
	}
	// If a timestamp already includes a timezone or is already ISO-like,
	// preserve it and only normalize the date/time separator when needed.
	if (trimmed.includes(" ") && /(?:Z|[+-]\d{2}:\d{2})$/.test(trimmed)) {
		return trimmed.replace(" ", "T");
	}
	return trimmed;
}

export function formatJobTs(ts: string | null | undefined): string {
	if (!ts) return "—";
	const d = new Date(normalizeJobTsForDate(ts));
	return isNaN(d.getTime()) ? ts : formatClock(d, { seconds: true });
}

/**
 * A job timestamp as epoch ms, or `null` when absent/unparseable. Same
 * normalisation as `formatJobTs` (SQLite writes naive-UTC "YYYY-MM-DD
 * HH:MM:SS") — the detail view needs the number, not the label, to say how far
 * off a `next_attempt_at` is.
 */
export function parseJobTs(ts: string | null | undefined): number | null {
	if (!ts) return null;
	const ms = new Date(normalizeJobTsForDate(ts)).getTime();
	return isNaN(ms) ? null : ms;
}
