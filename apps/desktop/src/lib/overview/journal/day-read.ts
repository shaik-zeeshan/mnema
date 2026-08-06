// The Journal destination's two one-line readouts, as pure functions so the
// page component stays markup + loading and the copy has a runnable check.
//
// Round-4 **G8** binds both: every figure is a real read with a real
// denominator, and a fact the machine cannot answer produces NO sentence
// rather than a placeholder. `hours` comes from `DayCoverage.hours` (the local
// hours that actually hold capture — nothing estimated) and `gaps` from
// `buildJournalDay` (inter-frame silences inside the summarized region).

import type { JournalGap } from "$lib/insights/journal-day";

/** The longest away-gap of the day, or null when the day has none. */
export function biggestGap(gaps: JournalGap[]): JournalGap | null {
  let best: JournalGap | null = null;
  for (const gap of gaps) {
    if (!best || gap.endMs - gap.startMs > best.endMs - best.startMs) best = gap;
  }
  return best;
}

/**
 * The coverage strip's readout: which hours hold frames, and the day's biggest
 * absence. `null` when nothing was captured — the strip then says so on its own
 * rather than printing "0 hours".
 */
export function coverageReadout(
  litHours: number,
  gaps: JournalGap[],
  clock: (ms: number) => string,
): string | null {
  if (litHours <= 0) return null;
  const head = `${litHours} ${litHours === 1 ? "hour" : "hours"} hold frames`;
  const gap = biggestGap(gaps);
  if (!gap) return head;
  if (gaps.length === 1) return `${head} · one away-gap at ${clock(gap.startMs)}`;
  return `${head} · ${gaps.length} away-gaps, the longest at ${clock(gap.startMs)}`;
}

/** "2 min ago" / "3h ago" — the digest's freshness stamp, rounded coarsely (G8). */
export function relativeTime(ms: number, now: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "";
  const diff = now - ms;
  if (diff < 60_000) return "just now";
  const min = Math.floor(diff / 60_000);
  if (min < 60) return `${min} min ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  return `${Math.floor(hr / 24)}d ago`;
}
