// Pure date math for DateInput.svelte (kit.js "dates" section, re-expressed).
// Days are local-calendar Dates pinned to 12:00 so DST shifts never move them
// across a midnight; the wire format is ISO `YYYY-MM-DD`.

export type PeriodUnit = "day" | "week" | "month";
export type Unit = PeriodUnit | "span";
export type Mode = "day" | "period" | "range";
export interface DateRange {
  start: string;
  end: string;
  unit: Unit;
}
type Span = [Date, Date];

const DAY_MS = 864e5;

export const parseIso = (s: string): Date => {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d, 12);
};
export const toIso = (d: Date): string =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
export const addDays = (d: Date, n: number): Date =>
  new Date(d.getFullYear(), d.getMonth(), d.getDate() + n, 12);
export const daysBetween = (a: Date, b: Date): number => Math.round((+b - +a) / DAY_MS);
export const monday = (d: Date): Date => addDays(d, -((d.getDay() + 6) % 7));
export const monthOf = (d: Date, k = 0): Span => [
  new Date(d.getFullYear(), d.getMonth() + k, 1, 12),
  new Date(d.getFullYear(), d.getMonth() + k + 1, 0, 12),
];

/** The whole day / Mon–Sun week / calendar month containing `d`. */
export function periodOf(d: Date, unit: Unit): Span {
  if (unit === "week") return [monday(d), addDays(monday(d), 6)];
  if (unit === "month") return monthOf(d);
  return [d, d];
}

/** ISO-8601 week number (weeks start Monday; week 1 holds the first Thursday). */
export function isoWeek(d: Date): number {
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  t.setUTCDate(t.getUTCDate() + 4 - (t.getUTCDay() || 7));
  return Math.ceil(((+t - Date.UTC(t.getUTCFullYear(), 0, 1)) / DAY_MS + 1) / 7);
}

/** Step `[a, b]` by `k` units. `null` when the move would leave [min, max]:
 *  a period never steps past the one holding `max` (now). */
export function stepSpan(a: Date, b: Date, unit: Unit, k: number, min: Date, max: Date): Span | null {
  let na: Date, nb: Date;
  if (unit === "day") na = nb = addDays(a, k);
  else if (unit === "week") [na, nb] = [addDays(a, 7 * k), addDays(b, 7 * k)];
  else if (unit === "month") [na, nb] = monthOf(a, k);
  else {
    const len = daysBetween(a, b) + 1;
    [na, nb] = [addDays(a, len * k), addDays(b, len * k)];
  }
  const out = unit === "day" ? na < min || na > max : na > max || nb < min;
  return out ? null : [na, nb];
}

/** Home/End: the newest span of the same unit (a custom span keeps its length). */
export function latestSpan(a: Date, b: Date, unit: Unit, max: Date): Span {
  if (unit === "span") return [addDays(max, -daysBetween(a, b)), max];
  return periodOf(max, unit);
}

/** Whole periods between `a`'s period and today's (0 = the current one). */
export function periodsAgo(a: Date, unit: Unit, today: Date): number {
  if (unit === "month")
    return (today.getFullYear() - a.getFullYear()) * 12 + today.getMonth() - a.getMonth();
  return Math.round(daysBetween(periodOf(a, unit)[0], periodOf(today, unit)[0]) / (unit === "week" ? 7 : 1));
}

/** Period mode's D·W·M switch: lands on "now" if the current period was showing,
 *  else on the period holding the old start (clamped up to `min`). */
export function switchUnit(a: Date, from: Unit, to: PeriodUnit, today: Date, min: Date): Span {
  const focus = periodsAgo(a, from, today) === 0 ? today : a < min ? min : a;
  return periodOf(focus, to);
}

/** Which unit a committed range is, so stepping follows the selection. */
export function unitOf(s: Date, e: Date): Unit {
  if (+s === +e) return "day";
  if (+s === +monday(s) && daysBetween(s, e) === 6) return "week";
  const [m0, m1] = monthOf(s);
  return +s === +m0 && +e === +m1 ? "month" : "span";
}

export function presets(today: Date): [string, Date, Date, Unit][] {
  const m0 = monday(today);
  return [
    ["Today", today, today, "day"],
    ["Yesterday", addDays(today, -1), addDays(today, -1), "day"],
    ["This week", m0, addDays(m0, 6), "week"],
    ["Last week", addDays(m0, -7), addDays(m0, -1), "week"],
    ["Last 7 days", addDays(today, -6), today, "span"],
    ["Last 30 days", addDays(today, -29), today, "span"],
    ["This month", ...monthOf(today), "month"],
    ["Last month", ...monthOf(today, -1), "month"],
  ];
}

export const fmt = (d: Date, o: Intl.DateTimeFormatOptions): string => d.toLocaleDateString("en-US", o);
export function spanLabel(a: Date, b: Date): string {
  const md = { month: "short", day: "numeric" } as const;
  if (+a === +b) return fmt(a, md);
  return a.getMonth() === b.getMonth() ? `${fmt(a, md)} – ${b.getDate()}` : `${fmt(a, md)} – ${fmt(b, md)}`;
}

export const cap = (s: string) => s[0].toUpperCase() + s.slice(1);
export const resetLabel = (unit: Unit): string => (unit === "day" ? "Today" : `This ${unit}`);

/** Capsule bold line: `Fri, Oct 9` · `Oct 5 – 11` · `October 2026`. */
export function capsuleLabel(mode: Mode, unit: Unit, a: Date, b: Date, today: Date): string {
  if (mode === "day") return fmt(a, { weekday: "short", month: "short", day: "numeric" });
  if (mode === "range" || unit === "week") return spanLabel(a, b);
  if (unit === "month") return fmt(a, { month: "long", year: "numeric" });
  return fmt(a, { month: "short", day: "numeric", ...(a.getFullYear() !== today.getFullYear() && { year: "numeric" }) });
}

/** Capsule small line: `today`, `last week`, `3 months ago`, `Last 7 days`… */
export function relativeLabel(mode: Mode, unit: Unit, a: Date, b: Date, today: Date): string {
  if (mode === "period") {
    const k = periodsAgo(a, unit, today);
    if (k === 0) return resetLabel(unit).toLowerCase();
    if (k === 1) return unit === "day" ? "yesterday" : `last ${unit}`;
    return `${k} ${unit}s ago`;
  }
  if (mode === "day") {
    const k = daysBetween(a, today);
    return k === 0 ? "today" : k === 1 ? "yesterday" : `${k} days ago`;
  }
  const p = presets(today).find(([, s, e]) => +s === +a && +e === +b);
  if (p) return p[0];
  if (unit === "month") return fmt(a, { month: "long" });
  if (unit === "week") return `week of ${fmt(a, { month: "short", day: "numeric" })}`;
  const n = daysBetween(a, b) + 1;
  return n === 1 ? "1 day" : `${n} days`;
}

/** Per-day capture density 0..3 from `get_usage_charts`' `activityHeatmap`
 *  (UTC-hour buckets): level = local hours with any capture that day.
 *  ponytail: fixed hour thresholds (<2h · <5h · ≥5h) and a bucket counts for the
 *  local day its start falls on (a half-hour UTC offset splits one hour across
 *  midnight); make it relative or minute-accurate only if the dots read wrong. */
export function densityFromHeatmap(buckets: { bucketStartMs: number; intensityCount: number }[]): Record<string, number> {
  const hours: Record<string, number> = {};
  for (const { bucketStartMs, intensityCount } of buckets) {
    if (intensityCount <= 0) continue;
    const day = toIso(new Date(bucketStartMs));
    hours[day] = (hours[day] ?? 0) + 1;
  }
  const out: Record<string, number> = {};
  for (const [day, h] of Object.entries(hours)) out[day] = h < 2 ? 1 : h < 5 ? 2 : 3;
  return out;
}
