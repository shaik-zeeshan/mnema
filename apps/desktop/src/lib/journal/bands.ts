// Journal bands (direction 01 — Bento Native, mockup 08). One time-of-day band
// is one 4×1 tile, so the band needs two things the shared river model doesn't
// carry: the live-edge pending row folded into the band it actually falls in
// (the mockup draws it as the last row of Afternoon, not floating below the
// grid), and the band's own header meta — "N activities · 9:04 – 11:44".
//
// Pure functions on top of `$lib/insights/journal-view` (which stays untouched);
// unit-checked in bands.test.ts.

import type { JournalCardSlot, JournalGap, JournalPending } from "$lib/insights/journal-day";
import {
  bandOf,
  bandRiver,
  buildRiver,
  type BandLabel,
  type RiverRow,
} from "$lib/insights/journal-view";

/** A band row: the river's card/gap rows, plus the live-edge pending slot. */
export type BandRow = RiverRow | { kind: "pending"; atMs: number };

export interface JournalBand {
  label: BandLabel;
  rows: BandRow[];
  /** Activity cards in this band — gaps and the pending row are not activities. */
  count: number;
  /** Earliest covered instant in the band. */
  startMs: number;
  /** Latest covered instant, or `null` when the band runs into the live edge. */
  endMs: number | null;
}

/** Key an `{#each}` over band rows. Cards key off the Activity id (two
 *  activities can share a start), gaps off their start, pending is a singleton. */
export function bandRowKey(row: BandRow): string {
  if (row.kind === "card") return `card${row.slot.activity.id}`;
  if (row.kind === "gap") return `gap${row.atMs}`;
  return "pending";
}

/**
 * The page's bands: the chronological river grouped by time-of-day, with the
 * pending slot appended to the band its watermark falls in (a new trailing band
 * when that band doesn't exist yet — e.g. capture that only started at 5pm).
 */
export function buildBands(
  slots: JournalCardSlot[],
  gaps: JournalGap[],
  pending: JournalPending,
): JournalBand[] {
  const bands = bandRiver(buildRiver(slots, gaps)).map((b) => ({
    label: b.label,
    rows: [...b.rows] as BandRow[],
  }));

  if (pending.active && pending.sinceMs !== null) {
    const label = bandOf(pending.sinceMs);
    const row: BandRow = { kind: "pending", atMs: pending.sinceMs };
    const last = bands[bands.length - 1];
    if (last && last.label === label) last.rows.push(row);
    else bands.push({ label, rows: [row] });
  }

  return bands.map(summarize);
}

function summarize(band: { label: BandLabel; rows: BandRow[] }): JournalBand {
  let count = 0;
  let startMs = Number.POSITIVE_INFINITY;
  let endMs = Number.NEGATIVE_INFINITY;
  let gapStartMs = Number.POSITIVE_INFINITY;
  let gapEndMs = Number.NEGATIVE_INFINITY;
  let live = false;
  for (const row of band.rows) {
    if (row.kind === "card") {
      count += 1;
      startMs = Math.min(startMs, row.slot.activity.startedAtMs);
      endMs = Math.max(endMs, row.slot.activity.endedAtMs);
    } else if (row.kind === "gap") {
      // A gap sits INSIDE the band's span; it never extends it (a band whose
      // last row is a gap into the next band would otherwise claim that band's
      // hours). Only a band that is nothing but gaps takes its span from them.
      gapStartMs = Math.min(gapStartMs, row.gap.startMs);
      gapEndMs = Math.max(gapEndMs, row.gap.endMs);
    } else {
      live = true;
      startMs = Math.min(startMs, row.atMs);
    }
  }
  if (!Number.isFinite(startMs)) startMs = gapStartMs;
  if (!Number.isFinite(endMs) && !live) endMs = gapEndMs;
  return {
    label: band.label,
    rows: band.rows,
    count,
    startMs: Number.isFinite(startMs) ? startMs : band.rows[0].atMs,
    endMs: live || !Number.isFinite(endMs) ? null : endMs,
  };
}
