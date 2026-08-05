// ── Overview bento — the pure arithmetic behind the two read-only readouts ──
//
// Direction 05 "Tactile Instruments": on Overview an instrument READS; you turn
// it in Settings. The two faces here are the 24-hour coverage strip and the
// day-budget gauge, plus the This Week bars (G11).
//
// Round-4 decision **G8** governs every number: a figure that the machine
// cannot actually answer returns `null` and the tile renders NO number — not a
// placeholder, not a zero. Nothing here invents a denominator, and nothing is
// rounded finer than its source supports.
//
// No runes and no `invoke` in this file — pure and unit-tested. Types are
// imported type-only on purpose: `bun test` does not resolve the `$lib` alias
// for real values (see `timeline/jumper-coverage.ts`'s note).

import type { DayCoverage } from "$lib/types/app-infra";
import type { SystemFacts } from "$lib/types";

const DAY_MS = 86_400_000;

export type CoverageIndex = Map<string, DayCoverage>;

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** `YYYY-MM-DD` in local time — the key `list_day_coverage` rows carry. */
export function dayKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

export function indexDays(days: DayCoverage[]): CoverageIndex {
  return new Map(days.map((day) => [day.day, day]));
}

/**
 * One cell per hour of the day, lit when that hour holds capture. The strip is
 * a reading of `DayCoverage.hours`, which is exactly "the local hours that hold
 * capture" — so there is nothing to estimate.
 */
export function hourCells(hours: number[] | undefined): boolean[] {
  const cells = new Array<boolean>(24).fill(false);
  for (const h of hours ?? []) {
    if (Number.isInteger(h) && h >= 0 && h <= 23) cells[h] = true;
  }
  return cells;
}

/** The hero reading: `6:42` from captured milliseconds. `null` when nothing was
 *  captured — a zero hero would claim a measurement that did not happen. */
export function heroHours(coveredMs: number | undefined): string | null {
  if (!coveredMs || coveredMs < 60_000) return null;
  const minutes = Math.floor(coveredMs / 60_000);
  return `${Math.floor(minutes / 60)}:${pad2(minutes % 60)}`;
}

/** "6h 42m" · "47m" · null (nothing). Rounds down to the minute. */
export function capturedLabel(coveredMs: number | undefined): string | null {
  if (!coveredMs || coveredMs < 60_000) return null;
  const minutes = Math.floor(coveredMs / 60_000);
  const hours = Math.floor(minutes / 60);
  return hours === 0 ? `${minutes}m` : `${hours}h ${pad2(minutes % 60)}m`;
}

export interface WeekBar {
  key: string;
  /** Localised short weekday ("Mon"). */
  label: string;
  coveredMs: number;
  /** 0–1 against the busiest day of the seven; 0 when nothing was captured. */
  fraction: number;
  isToday: boolean;
}

/**
 * Seven days ending today, oldest first — the same `list_day_coverage` read the
 * jump menu uses (G6/G11, one query family). A day with no row is a real zero
 * here: the backend omits days with no recording, and "no recording" is a fact.
 */
export function weekBars(coverage: CoverageIndex, today: Date, count = 7): WeekBar[] {
  const days: { key: string; label: string; coveredMs: number; isToday: boolean }[] = [];
  for (let i = count - 1; i >= 0; i--) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - i);
    const key = dayKey(date);
    days.push({
      key,
      label: date.toLocaleDateString(undefined, { weekday: "short" }),
      coveredMs: coverage.get(key)?.coveredMs ?? 0,
      isToday: i === 0,
    });
  }
  const peak = Math.max(...days.map((d) => d.coveredMs), 0);
  return days.map((d) => ({ ...d, fraction: peak > 0 ? d.coveredMs / peak : 0 }));
}

/** Total captured time across the week's bars. */
export function weekTotalMs(bars: WeekBar[]): number {
  return bars.reduce((sum, bar) => sum + bar.coveredMs, 0);
}

/** The busiest of the seven, or `null` when the week is empty. */
export function busiestBar(bars: WeekBar[]): WeekBar | null {
  let best: WeekBar | null = null;
  for (const bar of bars) {
    if (bar.coveredMs > 0 && (best === null || bar.coveredMs > best.coveredMs)) best = bar;
  }
  return best;
}

/** Horizon the gauge's bar covers — a month of capture at the measured rate. */
export const GAUGE_HORIZON_DAYS = 30;

/**
 * The day-budget gauge.
 *
 * **G8 deviation from the mockup, stated out loud.** The mockup draws "today's
 * footprint against a 500 MB/day budget". Neither figure exists: `system_facts`
 * measures only COMPLETE day directories (today is deliberately skipped, being
 * incomplete) and Mnema has no per-day budget setting. Inventing either is
 * exactly the fabricated denominator G8 forbids.
 *
 * What the machine can answer: `measuredBytesPerDay` (× `measuredDays` = the
 * literal summed size of those day directories) and `diskFreeBytes`. So the
 * gauge keeps its shape and swaps in the real quantities:
 *
 *   scale = the free space — the denominator, exactly as the brief asks
 *   fill  = a MONTH of capture at the measured rate (a projection, said so)
 *   notch = what the last N measured days actually took — the 7-day-average mark
 *
 * Reading: "does the coming month of capture fit in what's left, and how far in
 * did the last week get me?" `null` until there is a complete measured day AND
 * a free-space reading; the tile then renders its no-data face.
 */
export interface StorageGauge {
  perDayBytes: number;
  measuredDays: number;
  /** What the last `measuredDays` days of capture took. */
  windowBytes: number;
  /** What `GAUGE_HORIZON_DAYS` more days cost at that rate. */
  horizonBytes: number;
  freeBytes: number;
  fillPct: number;
  notchPct: number;
  /** Under a fortnight of free space left at this rate — the gauge goes warn. */
  tight: boolean;
}

export function storageGauge(facts: SystemFacts | null): StorageGauge | null {
  if (!facts) return null;
  // `Number.isFinite` and not `=== null`: an absent field must fail the same
  // way an explicit null does, or the gauge draws a NaN.
  const perDayBytes = facts.measuredBytesPerDay ?? NaN;
  const freeBytes = facts.diskFreeBytes ?? NaN;
  if (!Number.isFinite(perDayBytes) || perDayBytes <= 0) return null;
  if (!Number.isFinite(freeBytes) || freeBytes < 0) return null;
  if (freeBytes <= 0) return null;
  const measuredDays = Number.isFinite(facts.measuredDays)
    ? Math.max(1, facts.measuredDays)
    : 1;
  const windowBytes = perDayBytes * measuredDays;
  const horizonBytes = perDayBytes * GAUGE_HORIZON_DAYS;
  return {
    perDayBytes,
    measuredDays,
    windowBytes,
    horizonBytes,
    freeBytes,
    // A sliver still has to be visible: a gauge that reads empty when it is not
    // is a lie in the other direction.
    fillPct: Math.min(100, Math.max(1.5, (horizonBytes / freeBytes) * 100)),
    notchPct: Math.min(100, (windowBytes / freeBytes) * 100),
    tight: freeBytes / perDayBytes < 14,
  };
}

/** Local midnight-to-midnight window for the day-scoped reads. */
export function dayWindow(now: Date): { startMs: number; endMs: number } {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  return { startMs: start, endMs: start + DAY_MS };
}

/** "38 min" — conversation length, rounded to the minute it was measured in. */
export function minutesLabel(ms: number): string {
  return `${Math.max(1, Math.round(ms / 60_000))} min`;
}

/** "13:02" local, the stamp every row and citation carries. */
export function clockLabel(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}
