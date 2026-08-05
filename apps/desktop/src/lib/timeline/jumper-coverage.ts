// ── Timeline Jumper — per-day coverage helpers (rune-free) ───────────────────
// Pure logic behind the jump menu's quick targets, seven day rows and the
// month grid's disabled days (round-4 decision G6). Fed by the backend's
// `list_day_coverage` (ONE GROUP BY over `capture_segments`, cached there).
//
// Everything here is date-math + formatting so it can be exercised under
// `bun test`; the component keeps only the reactive plumbing.
import type { DayCoverage } from "$lib/types/app-infra";
import type { CalendarFields } from "./jumper-cache-core";

// Local copy rather than importing from `jumper-cache-core` — that module pulls
// in `$lib/*`, which `bun test` cannot resolve. Two characters of zero-padding
// is not worth a test-only path alias.
function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** Number of cells in a day row's coverage bar (24 h / 8 = 3 h per cell). */
export const COVERAGE_CELLS = 8;

export type CoverageIndex = Map<string, DayCoverage>;

export type DayRow = {
  /** "YYYY-MM-DD" local. */
  key: string;
  /** "Today" / "Yesterday" / "Saturday, Aug 1". */
  label: string;
  date: CalendarFields;
  /** One boolean per coverage-bar cell — true when that slice holds capture. */
  cells: boolean[];
  /** "6h 42m", or "nothing" when the day is empty. */
  hoursLabel: string;
  /** No recording that day: the row is disabled (G6 — never land on empty). */
  disabled: boolean;
};

export function indexCoverage(days: DayCoverage[]): CoverageIndex {
  return new Map(days.map((d) => [d.day, d]));
}

export function dayKeyOfDate(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

export function calendarFieldsOfDate(d: Date): CalendarFields {
  return { year: d.getFullYear(), month: d.getMonth() + 1, day: d.getDate() };
}

/** "6h 42m" · "47m" · "nothing" (0 or unknown). Rounds down to the minute. */
export function formatCapturedHours(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes <= 0) return "nothing";
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest}m`;
  return `${hours}h ${pad2(rest)}m`;
}

/**
 * Collapse the day's captured hours into the coverage bar's cells. Each cell
 * covers `24 / cells` hours and lights up when ANY of them holds capture.
 */
export function coverageCells(
  hours: number[],
  cells: number = COVERAGE_CELLS,
): boolean[] {
  const perCell = 24 / cells;
  const out = new Array<boolean>(cells).fill(false);
  for (const h of hours) {
    if (h < 0 || h > 23) continue;
    out[Math.min(cells - 1, Math.floor(h / perCell))] = true;
  }
  return out;
}

/** Weekday-and-date label; the two most recent days get relative names. */
export function dayRowLabel(date: Date, offsetFromToday: number): string {
  if (offsetFromToday === 0) return "Today";
  if (offsetFromToday === 1) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

/**
 * The `count` most recent local days ending at `today`, newest first. Days with
 * no coverage still render — as disabled rows reading "nothing" — so the menu
 * shows an honest, gap-free week rather than silently skipping empty days.
 */
export function buildDayRows(
  coverage: CoverageIndex,
  today: Date,
  count = 7,
): DayRow[] {
  const rows: DayRow[] = [];
  for (let i = 0; i < count; i++) {
    const date = new Date(
      today.getFullYear(),
      today.getMonth(),
      today.getDate() - i,
    );
    const key = dayKeyOfDate(date);
    const day = coverage.get(key);
    rows.push({
      key,
      label: dayRowLabel(date, i),
      date: calendarFieldsOfDate(date),
      cells: coverageCells(day?.hours ?? []),
      hoursLabel: formatCapturedHours(day?.coveredMs ?? 0),
      disabled: !day || day.hours.length === 0,
    });
  }
  return rows;
}

/** True when the local day holds capture — the month grid's enable predicate. */
export function dayHasCoverage(
  coverage: CoverageIndex,
  d: CalendarFields,
): boolean {
  const day = coverage.get(`${d.year}-${pad2(d.month)}-${pad2(d.day)}`);
  return !!day && day.hours.length > 0;
}

/** Whether any of `hours` (inclusive range) was captured on that local day. */
export function hasCoverageInHours(
  coverage: CoverageIndex,
  d: CalendarFields,
  fromHour: number,
  toHour: number,
): boolean {
  const day = coverage.get(`${d.year}-${pad2(d.month)}-${pad2(d.day)}`);
  if (!day) return false;
  return day.hours.some((h) => h >= fromHour && h <= toHour);
}
