// Developer-window formatters (plain strings only; no CSS class emitters).

import { formatClock } from "$lib/time-format";

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
