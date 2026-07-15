// Debug-page formatters + badge-class helpers.
//
// A 1:1 move of the page-local helpers that used to live in the
// routes/debug/+page.svelte <script> block, re-homed so several section
// components can share them. Behaviour is unchanged.

import type {
	BackgroundJobStatus,
	DebugSeverity,
	FrameBatchStatus,
	OcrAdmissionSignals,
	PermissionStatus,
	ProcessingJobStatus,
	SegmentWorkspaceCleanupDisposition,
} from "$lib/types";

/**
 * One cell of a feature card's stat grid (mockup A). Lives here rather than in
 * `StatGrid.svelte` so sections can annotate a `$derived.by` without a
 * `<script module>` export — `tone` would otherwise widen to `string`.
 */
export type DebugStat = {
	/** `{#each}` key. */
	key: string;
	label: string;
	value: string | number;
	/** Small trailing unit rendered next to the value (e.g. "ms", "tok"). */
	unit?: string;
	/** The dim line under the value. */
	sub?: string | null;
	/** Colours the value; omit for the default (neutral) tone. */
	tone?: "ok" | "warn" | "err";
	/**
	 * Tag the label with mockup A's "new" chip. Honesty rule: only for a readout
	 * that genuinely did not exist before this redesign — never decoration.
	 */
	isNew?: boolean;
};

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
	return Number.isNaN(parsed) ? value : new Date(parsed).toLocaleTimeString();
}

export function truncateDebugText(value: string | null | undefined, max = 80): string {
	if (!value) return "—";
	return value.length <= max ? value : `${value.slice(0, max - 1)}…`;
}

export function formatTimestamp(ms: number): string {
	return new Date(ms).toLocaleTimeString();
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

// ─── Badge classes ─────────────────────────────────────────────────────────

export function permissionBadgeClass(status: PermissionStatus | undefined): string {
	if (!status) return "badge badge--neutral";
	if (status === "granted") return "badge badge--ok";
	if (status === "denied" || status === "restricted") return "badge badge--err";
	return "badge badge--neutral";
}

export function supportBadge(val: boolean): string {
	return val ? "badge badge--ok" : "badge badge--err";
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

export function dispositionBadgeClass(d: SegmentWorkspaceCleanupDisposition): string {
	switch (d) {
		case "completed_only":
		case "no_references":
		case "dead_segment_without_artifacts":
			return "badge badge--ok badge--sm";
		case "referenced_by_incomplete_batch":
		case "referenced_by_nonterminal_ocr":
		case "pending_frame_artifacts":
			return "badge badge--warn badge--sm";
		case "missing_visible_segment_sibling":
			return "badge badge--err badge--sm";
		default:
			return "badge badge--neutral badge--sm";
	}
}

export function batchStatusBadgeClass(status: FrameBatchStatus): string {
	if (status === "completed") return "badge badge--ok badge--sm";
	if (status === "failed") return "badge badge--err badge--sm";
	if (status === "processing") return "badge badge--running badge--sm";
	return "badge badge--neutral badge--sm";
}

export function ocrStatusBadgeClass(status: ProcessingJobStatus): string {
	if (status === "completed") return "badge badge--ok badge--sm";
	if (status === "failed") return "badge badge--err badge--sm";
	if (status === "running") return "badge badge--running badge--sm";
	return "badge badge--neutral badge--sm";
}

/**
 * The status badge a feature card wears in its group title, from the dock's
 * health rollup. `null` (feature missing from the rollup, or not polled yet)
 * reads as neutral — absence is normal, never an error (see `sections.ts` on
 * the 9-vs-11 split).
 *
 * Full-size, not `badge--sm`: mockup A puts this badge inline in the group
 * title, where it is the card's headline status, not a dense inline tag.
 */
export function severityBadgeClass(severity: DebugSeverity | null): string {
	if (severity === "ok") return "badge badge--ok";
	if (severity === "warn") return "badge badge--warn";
	if (severity === "error") return "badge badge--err";
	return "badge badge--neutral";
}

/**
 * The strip / dock dot for a severity. Same three tones as the badge, so a
 * feature's dot and its badge never disagree.
 */
export function severityDotClass(severity: DebugSeverity | null): string {
	if (severity === "ok") return "health-dot health-dot--ok";
	if (severity === "warn") return "health-dot health-dot--warn";
	if (severity === "error") return "health-dot health-dot--err";
	return "health-dot health-dot--idle";
}

/**
 * The severity-tinted accent hairline a card wears (mockup A's `card--warn` /
 * `card--danger`), passed to `<SettingGroup cardClass=…>`. `ok`/unknown get the
 * default neutral hairline SettingGroup already paints.
 */
export function severityCardClass(severity: DebugSeverity | null): string {
	if (severity === "warn") return "setting-group__card--warn";
	if (severity === "error") return "setting-group__card--danger";
	return "";
}

/** The word inside that badge. */
export function severityLabel(severity: DebugSeverity | null): string {
	if (severity === "ok") return "healthy";
	if (severity === "warn") return "degraded";
	if (severity === "error") return "failing";
	return "unknown";
}

/** A wall-clock time from epoch ms, or an em-dash when absent. */
export function formatOptionalTime(ms: number | null | undefined): string {
	return ms == null ? "—" : new Date(ms).toLocaleTimeString();
}

/** `12:00–14:00` for a derivation run's window, or an em-dash when open-ended. */
export function formatWindow(startMs: number | null, endMs: number | null): string {
	if (startMs == null || endMs == null) return "—";
	return `${new Date(startMs).toLocaleTimeString()}–${new Date(endMs).toLocaleTimeString()}`;
}

/** Thousands-separated count — job/vector counts get long enough to need it. */
export function formatCount(n: number | null | undefined): string {
	return n == null ? "—" : n.toLocaleString();
}

export function jobStatusBadgeClass(status: BackgroundJobStatus): string {
	if (status === "completed") return "badge badge--ok badge--sm";
	if (status === "failed") return "badge badge--err badge--sm";
	if (status === "running") return "badge badge--running badge--sm";
	return "badge badge--neutral badge--sm";
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
	return isNaN(d.getTime()) ? ts : d.toLocaleTimeString();
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
