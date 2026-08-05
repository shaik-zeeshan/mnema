// Pure day-window and label math for the Overview bento.
//
// No runes and no runtime `$lib` imports — the same constraint
// `lib/timeline/jumper-coverage.ts` documents, so `bun test` can exercise this
// without a path alias. The type import below is erased at build time.
//
// Every formatter here refuses to invent: a duration it cannot express honestly
// comes back as the empty-ish case its caller renders as nothing (round-4
// decision **G8**).

import type { DayCoverage } from "$lib/types/app-infra";

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** `YYYY-MM-DD` in the user's local calendar — the same key `list_day_coverage` returns. */
export function dayKeyOf(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** Local midnight of a `YYYY-MM-DD` key. */
export function parseDayKey(key: string): Date {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, (m ?? 1) - 1, d ?? 1);
}

/** The half-open `[startMs, endMs)` local window every day-scoped read takes. */
export function dayWindow(key: string): { startMs: number; endMs: number } {
  const start = parseDayKey(key);
  const end = new Date(start.getFullYear(), start.getMonth(), start.getDate() + 1);
  return { startMs: start.getTime(), endMs: end.getTime() };
}

export function shiftDayKey(key: string, days: number): string {
  const d = parseDayKey(key);
  return dayKeyOf(new Date(d.getFullYear(), d.getMonth(), d.getDate() + days));
}

/** "Monday, August 3" — the pane's one title. */
export function formatDayTitle(key: string): string {
  return parseDayKey(key).toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

/** "Mon, Aug 3" — the tool strip's date control. */
export function formatDayShort(key: string): string {
  return parseDayKey(key).toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

/**
 * Local wall clock, `14:32`. `null` when there is no usable stamp.
 *
 * The guard is here rather than at each of the nine call sites: a record whose
 * timestamp field is missing used to render the literal string `NaN:NaN`, which
 * is exactly the failure **G8** exists to prevent — a number on screen that no
 * fact backs. Svelte renders a `null` interpolation as nothing, so callers get
 * the right behaviour (no number at all) for free; only the two callers that
 * embed this in a template literal have to check.
 */
export function formatClock(ms: number | null | undefined): string | null {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return null;
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/**
 * The hero readout, `6:42`. Hours and minutes only — the source is a coverage
 * estimate summed per segment, so seconds would be false precision.
 * `null` for a day with nothing captured: the tile then renders no number.
 */
export function formatHeroHours(ms: number | null): string | null {
  if (ms === null || ms < 60_000) return null;
  const minutes = Math.floor(ms / 60_000);
  return `${Math.floor(minutes / 60)}:${pad2(minutes % 60)}`;
}

/** "38 min" · "1h 12m" · "under a minute". Coarse by construction (G8). */
export function formatSpan(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return "under a minute";
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)}h ${pad2(minutes % 60)}m`;
}

/** "32h 08m" — the week total. */
export function formatLongHours(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  return `${Math.floor(minutes / 60)}h ${pad2(minutes % 60)}m`;
}

export interface WeekBar {
  key: string;
  /** Two-letter weekday, the axis label under the bar. */
  label: string;
  coveredMs: number;
  /** 0–1 against the week's busiest day; 0 when nothing was captured all week. */
  ratio: number;
  isAnchor: boolean;
}

/**
 * Seven bars ending at `anchorKey`, oldest first. A day absent from
 * `list_day_coverage` genuinely had no capture, so it draws a zero bar rather
 * than being skipped — a gap in the week is a fact about the week.
 */
export function weekBars(days: DayCoverage[], anchorKey: string): WeekBar[] {
  const index = new Map(days.map((d) => [d.day, d.coveredMs]));
  const bars: WeekBar[] = [];
  for (let i = 6; i >= 0; i--) {
    const key = shiftDayKey(anchorKey, -i);
    bars.push({
      key,
      label: parseDayKey(key).toLocaleDateString(undefined, { weekday: "short" }).slice(0, 2),
      coveredMs: index.get(key) ?? 0,
      ratio: 0,
      isAnchor: i === 0,
    });
  }
  const peak = Math.max(...bars.map((b) => b.coveredMs));
  if (peak > 0) for (const b of bars) b.ratio = b.coveredMs / peak;
  return bars;
}

export function weekTotalMs(bars: WeekBar[]): number {
  return bars.reduce((sum, b) => sum + b.coveredMs, 0);
}

/** "busiest Fri", or `null` when the week holds no capture at all. */
export function busiestDayPhrase(bars: WeekBar[]): string | null {
  let best: WeekBar | null = null;
  for (const b of bars) if (b.coveredMs > 0 && (!best || b.coveredMs > best.coveredMs)) best = b;
  if (!best) return null;
  return `busiest ${parseDayKey(best.key).toLocaleDateString(undefined, { weekday: "short" })}`;
}
