// Pure formatters for the Overview bento. No runes, no `invoke` — unit-tested
// by `format.test.ts`. The byte formatter is NOT here: the app already has one
// (`$lib/settings/state/format` → `formatBytes`), and one byte formatter is the
// rule.

import type { DayCoverage } from "$lib/types/app-infra";

/** `YYYY-MM-DD` in the viewer's local timezone — the key shape
 *  `list_day_coverage` returns (its days are local, not UTC). Built by hand
 *  rather than through a locale so no locale can reorder the parts. */
export function localDayKey(d: Date): string {
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Unix ms at local midnight of `d`'s day. */
export function startOfLocalDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "6h 42m" / "42m" — the header's captured-today phrase. */
export function formatHoursMinutes(ms: number): string {
  const mins = Math.floor(Math.max(0, ms) / 60_000);
  const h = Math.floor(mins / 60);
  return h > 0 ? `${h}h ${String(mins % 60).padStart(2, "0")}m` : `${mins}m`;
}

/** "6:42" — the Capture tile's hero, the screen's one display-size number. */
export function formatHoursColon(ms: number): string {
  const mins = Math.floor(Math.max(0, ms) / 60_000);
  return `${Math.floor(mins / 60)}:${String(mins % 60).padStart(2, "0")}`;
}

/** "2:14:07" — live session elapsed, always h:mm:ss so the width never jumps. */
export function formatElapsed(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${Math.floor(total / 3600)}:${p(Math.floor(total / 60) % 60)}:${p(total % 60)}`;
}

/** "09:14" — local 24h wall clock for a moment chip / row accessory. */
export function formatClock(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "38 min" — conversation length. Rounded, never seconds (G8: round coarsely). */
export function formatMinutes(ms: number): string {
  return `${Math.max(1, Math.round(Math.max(0, ms) / 60_000))} min`;
}

/** Conviction meter fill, 0–1 confidence → 1–5 dots. Any positive confidence
 *  lights at least one dot; the meter is never empty for a live belief. */
export function convictionDots(confidence: number): number {
  if (!Number.isFinite(confidence) || confidence <= 0) return 0;
  return Math.min(5, Math.max(1, Math.round(confidence * 5)));
}

export interface WeekBar {
  /** `YYYY-MM-DD` local. */
  key: string;
  /** Two-letter weekday, e.g. "Mo". */
  label: string;
  coveredMs: number;
  isToday: boolean;
}

const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/** Seven local days ending today, oldest first. Days absent from
 *  `list_day_coverage` had no recording and draw a zero bar. */
export function weekBars(coverage: DayCoverage[], now: Date): WeekBar[] {
  const byDay = new Map(coverage.map((d) => [d.day, d.coveredMs]));
  const bars: WeekBar[] = [];
  for (let back = 6; back >= 0; back -= 1) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - back);
    const key = localDayKey(d);
    bars.push({
      key,
      label: WEEKDAYS[d.getDay()],
      coveredMs: byDay.get(key) ?? 0,
      isToday: back === 0,
    });
  }
  return bars;
}

/** The week's busiest weekday label, or null when nothing was captured. */
export function busiestDay(bars: WeekBar[]): string | null {
  const best = bars.reduce<WeekBar | null>(
    (acc, b) => (b.coveredMs > 0 && (!acc || b.coveredMs > acc.coveredMs) ? b : acc),
    null,
  );
  return best?.label ?? null;
}

/** Captured ms on a given local day, or 0 when that day holds no recording. */
export function coveredOn(coverage: DayCoverage[], day: string): number {
  return coverage.find((d) => d.day === day)?.coveredMs ?? 0;
}

/** Retention policy → days kept, or null for "keep everything" (no projection
 *  is possible, so the Storage tile draws no bar). */
export function retentionDays(policy: string): number | null {
  switch (policy) {
    case "days_7":
      return 7;
    case "days_14":
      return 14;
    case "days_30":
      return 30;
    default:
      return null;
  }
}
