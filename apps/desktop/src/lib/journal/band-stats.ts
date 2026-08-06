// Pure helpers for the Journal surface (direction 04). Everything the river's
// band headers and the lede's "read written N ago" need that isn't already in
// `$lib/insights/journal-view` / `journal-day` / `lede-stats` — those are
// imported, never re-implemented.

import type { RiverRow } from "$lib/insights/journal-view";

export interface BandStats {
	/** Activity cards in the band (gap rows don't count). */
	count: number;
	/** Summed card durations, ms. */
	totalMs: number;
}

/** Per-band header numbers: "4 activities · 3h 12m". Both counted, never estimated. */
export function bandStats(rows: RiverRow[]): BandStats {
	let count = 0;
	let totalMs = 0;
	for (const row of rows) {
		if (row.kind !== "card") continue;
		count += 1;
		totalMs += Math.max(0, row.slot.activity.endedAtMs - row.slot.activity.startedAtMs);
	}
	return { count, totalMs };
}

/** "12 min ago" / "just now" — coarse by design (G8: no minute-precise claims
 *  past the hour). Empty string for a missing/absurd timestamp. */
export function relativeAgo(ms: number, nowMs: number = Date.now()): string {
	if (!Number.isFinite(ms) || ms <= 0) return "";
	const diff = nowMs - ms;
	if (diff < 60_000) return "just now";
	const min = Math.floor(diff / 60_000);
	if (min < 60) return `${min} min ago`;
	const hr = Math.floor(min / 60);
	if (hr < 24) return `${hr}h ago`;
	return `${Math.floor(hr / 24)}d ago`;
}
