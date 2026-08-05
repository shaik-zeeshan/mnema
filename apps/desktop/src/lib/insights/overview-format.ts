// Pure formatting + shaping helpers for the Overview bento (redesign slice 10).
// Kept out of the component so the drop-ladder and label logic is bun-testable.

import type { Conclusion, DayConversation, RetentionPolicy } from "$lib/types/recording";

/** "38 min" under an hour, "1h 4m" over — the Conversations row duration. */
export function conversationDurationLabel(c: DayConversation): string {
	const ms = Math.max(0, c.displayEndedAtMs - c.startedAtMs);
	const totalMin = Math.max(1, Math.round(ms / 60000));
	if (totalMin < 60) return `${totalMin} min`;
	const h = Math.floor(totalMin / 60);
	const m = totalMin % 60;
	return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

export function speakersLabel(count: number): string {
	return count === 1 ? "1 speaker" : `${count} speakers`;
}

/** Local wall-clock "13:02" (24h, tabular) for strip captions and row stamps. */
export function clockHM(unixMs: number): string {
	const d = new Date(unixMs);
	return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "6h 42m" — the header's captured-coverage meta. */
export function coverageLabel(ms: number): string {
	if (!Number.isFinite(ms) || ms <= 0) return "0m";
	const totalMin = Math.round(ms / 60000);
	const h = Math.floor(totalMin / 60);
	const m = totalMin % 60;
	if (h <= 0) return `${m}m`;
	return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/** "6:42" — the Capture tile's one `--t-display` hero (hours:minutes). */
export function coverageHero(ms: number): string {
	if (!Number.isFinite(ms) || ms <= 0) return "0:00";
	const totalMin = Math.round(ms / 60000);
	return `${Math.floor(totalMin / 60)}:${String(totalMin % 60).padStart(2, "0")}`;
}

/** "≈ 8.1 GB / month at today's pace" from today's captured bytes. */
export function monthlyPaceLabel(bytesToday: number): string | null {
	if (!Number.isFinite(bytesToday) || bytesToday <= 0) return null;
	const monthly = bytesToday * 30;
	const label =
		monthly >= 1e9 ? `${(monthly / 1e9).toFixed(1)} GB` : `${Math.round(monthly / 1e6)} MB`;
	return `≈ ${label} / month at today's pace`;
}

export function retentionLabel(policy: RetentionPolicy): string {
	switch (policy) {
		case "days_7":
			return "keep 7 days";
		case "days_14":
			return "keep 14 days";
		case "days_30":
			return "keep 30 days";
		default:
			return "kept forever";
	}
}

/** The 800×600 ladder's one-sentence digest: first sentence of the narrative. */
export function firstSentence(text: string): string {
	const trimmed = text.trim();
	const match = /^[\s\S]*?[.!?](?=\s|$)/.exec(trimmed);
	return match ? match[0] : trimmed;
}

/** One Subjects-tile row: a subject's freshest visible belief + conviction. */
export interface SubjectRow {
	subject: string;
	statement: string;
	/** 1..5 filled conviction dots (round(confidence × 5), clamped). */
	dots: number;
	lastSupportedAtMs: number;
}

/**
 * Group visible conclusions by subject: each subject shows its
 * highest-confidence statement, subjects ordered by freshest support.
 */
export function subjectRows(conclusions: Conclusion[]): SubjectRow[] {
	const bySubject = new Map<string, SubjectRow>();
	for (const c of conclusions) {
		if (c.status !== "visible") continue;
		const existing = bySubject.get(c.subject);
		const dots = Math.min(5, Math.max(1, Math.round(c.confidence * 5)));
		if (!existing) {
			bySubject.set(c.subject, {
				subject: c.subject,
				statement: c.statement,
				dots,
				lastSupportedAtMs: c.lastSupportedAtMs,
			});
			continue;
		}
		existing.lastSupportedAtMs = Math.max(existing.lastSupportedAtMs, c.lastSupportedAtMs);
		if (dots > existing.dots) {
			existing.dots = dots;
			existing.statement = c.statement;
		}
	}
	return [...bySubject.values()].sort((a, b) => b.lastSupportedAtMs - a.lastSupportedAtMs);
}

/** Moments shown by width tier — 3 narrow, 5 default, 6 wide (frame 04). */
export function momentsShownCount(narrow: boolean, wide: boolean): number {
	if (narrow) return 3;
	return wide ? 6 : 5;
}
