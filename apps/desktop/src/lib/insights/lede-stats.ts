// Shared lede-stat primitives for the Insights lede (Overview + Journal). The
// plan requires "same label ⇒ same computation": the three figures the lede
// footer shows must be derived identically on both surfaces. Extracted verbatim
// from Overview's derivations (`summary.totalMs`, `summary.deepPct`,
// `topCategory` via `categorySegments`) so Overview's rendered output is
// unchanged — see the equivalence note on `topCategory` below.
//
// Pure functions only (no Svelte / no invoke) so it is unit-testable and the two
// surfaces can't drift.

import type { Activity } from "$lib/types/recording";
import {
  CATEGORY_COLOR,
  CATEGORY_ORDER,
  UNCATEGORIZED_COLOR,
  categoryLabel,
} from "$lib/insights/activity-helpers";

/** Just the shape the lede footer swatch+label needs. */
export interface LedeTopCategory {
  label: string;
  colorVar: string;
}

export interface LedeStats {
  /** Sum of `timePerApp[].activeMs` — the "tracked" figure (usage-charts basis). */
  trackedMs: number;
  /** Round(deep / counted × 100) over range activities with a non-null focus;
   *  null when the engine is off or nothing was counted. */
  deepPct: number | null;
  /** Busiest category in the range (activity basis), or null when none. */
  topCategory: LedeTopCategory | null;
}

export interface LedeStatsInput {
  /** `usage.timePerApp` (only `activeMs` is read). */
  timePerApp: { activeMs: number }[];
  /** Activities already filtered to the active range (Overview's `rangeActivities`). */
  rangeActivities: Activity[];
  rangeStartMs: number;
  rangeEndMs: number;
  /** Deep-focus % is an engine-tier stat; null when the engine is off. */
  engineOn: boolean;
}

/**
 * The busiest category, computed the SAME way Overview does today.
 *
 * Overview derives `topCategory` from `categorySegments`, which sums each
 * category's duration CLIPPED to the range, folds all-but-the-top-5 named
 * categories into a synthetic "Other" bucket, appends an "Uncategorized" bucket,
 * then picks the max after dropping "Other". Because the "Other" fold only ever
 * collects the SMALLEST named categories (the list is sorted desc), the max is
 * always a real category — never the fold — so it is equivalent to: argmax of
 * the clipped per-category totals over {named categories ∪ uncategorized}. That
 * is what this computes, preserving Overview's tie-breaks exactly (named
 * categories in `CATEGORY_ORDER` order win ties among themselves; Uncategorized
 * only wins when STRICTLY larger than every named category).
 */
function computeTopCategory(
  rangeActivities: Activity[],
  startMs: number,
  endMs: number,
): LedeTopCategory | null {
  const totals = new Map<string, number>();
  for (const a of rangeActivities) {
    const clippedStart = Math.max(a.startedAtMs, startMs);
    const clippedEnd = Math.min(a.endedAtMs, endMs);
    const dur = Math.max(0, clippedEnd - clippedStart);
    if (dur <= 0) continue;
    const key = a.category ?? "__uncat__";
    totals.set(key, (totals.get(key) ?? 0) + dur);
  }
  // Best named category — iterate CATEGORY_ORDER with a strict `>` so ties
  // resolve to the earlier category (matches Overview's stable desc sort).
  let best: LedeTopCategory | null = null;
  let bestValue = 0;
  for (const c of CATEGORY_ORDER) {
    const v = totals.get(c) ?? 0;
    if (v > bestValue) {
      bestValue = v;
      best = { label: categoryLabel(c), colorVar: CATEGORY_COLOR[c] };
    }
  }
  // Uncategorized wins only when STRICTLY larger than the best named category
  // (it sits last in Overview's segment list, so a tie keeps the named one).
  const uncat = totals.get("__uncat__") ?? 0;
  if (uncat > bestValue) {
    return { label: "Uncategorized", colorVar: UNCATEGORIZED_COLOR };
  }
  return best;
}

export function computeLedeStats(input: LedeStatsInput): LedeStats {
  const { timePerApp, rangeActivities, rangeStartMs, rangeEndMs, engineOn } =
    input;

  const trackedMs = timePerApp.reduce((acc, a) => acc + a.activeMs, 0);

  let deepPct: number | null = null;
  if (engineOn) {
    let deep = 0;
    let counted = 0;
    for (const a of rangeActivities) {
      const focus = a.focus;
      if (focus == null) continue;
      counted += 1;
      if (focus === "deep") deep += 1;
    }
    deepPct = counted > 0 ? Math.round((deep / counted) * 100) : null;
  }

  const topCategory = computeTopCategory(rangeActivities, rangeStartMs, rangeEndMs);

  return { trackedMs, deepPct, topCategory };
}
