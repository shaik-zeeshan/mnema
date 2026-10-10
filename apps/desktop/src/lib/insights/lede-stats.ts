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
import { isoWeek } from "$lib/components/date-input";

/** Just the shape the lede footer swatch+label needs. */
export interface LedeTopCategory {
  label: string;
  colorVar: string;
  /** Share of the range's (clipped) activity time, 0..100. */
  pct: number;
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
  let sum = 0;
  for (const v of totals.values()) sum += v;
  const pct = (v: number) => (sum > 0 ? Math.round((v / sum) * 100) : 0);
  let best: LedeTopCategory | null = null;
  let bestValue = 0;
  for (const c of CATEGORY_ORDER) {
    const v = totals.get(c) ?? 0;
    if (v > bestValue) {
      bestValue = v;
      best = { label: categoryLabel(c), colorVar: CATEGORY_COLOR[c], pct: pct(v) };
    }
  }
  // Uncategorized wins only when STRICTLY larger than the best named category
  // (it sits last in Overview's segment list, so a tie keeps the named one).
  const uncat = totals.get("__uncat__") ?? 0;
  if (uncat > bestValue) {
    return { label: "Uncategorized", colorVar: UNCATEGORIZED_COLOR, pct: pct(uncat) };
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

// ── Overview figures (IN2) ─────────────────────────────────────────────
// Deep focus is BY ACTIVITY COUNT everywhere (lede stat, focus split, Focus
// modal) — one computation, ruled 2026-10-10.

export type FocusCounts = { deep: number; mixed: number; distracted: number; n: number };

export function focusCounts(acts: Activity[]): FocusCounts {
  const c = { deep: 0, mixed: 0, distracted: 0, n: 0 };
  for (const a of acts) {
    if (a.focus == null) continue;
    c[a.focus] += 1;
    c.n += 1;
  }
  return c;
}

const deepShare = (c: FocusCounts) => (c.n ? Math.round((c.deep / c.n) * 100) : null);

/** Deep-focus change vs the previous range, in percentage points. Null when
 *  either side has no focus-tagged activity (nothing to compare). */
export function deepFocusDelta(cur: Activity[], prev: Activity[]): number | null {
  const a = deepShare(focusCounts(cur));
  const b = deepShare(focusCounts(prev));
  return a == null || b == null ? null : a - b;
}

/** "6h 09m" · "47m" — the Overview's padded duration voice. */
export function fmtDuration(ms: number): string {
  const m = Math.max(0, Math.round(ms / 60000));
  return m >= 60 ? `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m` : `${m}m`;
}

const overlap = (a: Activity, s: number, e: number) =>
  Math.max(0, Math.min(a.endedAtMs, e) - Math.max(a.startedAtMs, s));

export interface CategoryRow {
  key: string; // category id or "__uncat__"
  label: string;
  colorVar: string;
  ms: number;
  pct: number;
}

/** Activity time per category inside [startMs, endMs), largest first. */
export function categoryRows(acts: Activity[], startMs: number, endMs: number): CategoryRow[] {
  const totals = new Map<string, number>();
  for (const a of acts) {
    const ms = overlap(a, startMs, endMs);
    if (ms > 0) totals.set(a.category ?? "__uncat__", (totals.get(a.category ?? "__uncat__") ?? 0) + ms);
  }
  let sum = 0;
  for (const v of totals.values()) sum += v;
  return [...CATEGORY_ORDER, "__uncat__" as const]
    .filter((k) => totals.has(k))
    .map((k) => ({
      key: k,
      label: k === "__uncat__" ? "Uncategorized" : categoryLabel(k),
      colorVar: k === "__uncat__" ? UNCATEGORIZED_COLOR : CATEGORY_COLOR[k],
      ms: totals.get(k)!,
      pct: Math.round((totals.get(k)! / sum) * 100),
    }))
    .sort((a, b) => b.ms - a.ms);
}

export type PeriodUnit = "day" | "week" | "month";

export interface Slot {
  startMs: number;
  endMs: number;
  /** "Mon" (week) · "5" (month) · "08" (day). */
  label: string;
  tip: string;
  live: boolean;
  future: boolean;
}

export interface Column extends Slot {
  parts: { key: string; label: string; colorVar: string; ms: number }[];
  totalMs: number;
}

const STACK: string[] = [...CATEGORY_ORDER, "__uncat__"];
const BLOCK_H = 2; // day mode: 2-hour blocks
const dayTip = (d: Date) => d.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
const hh = (h: number) => String(h).padStart(2, "0");

/** The columns of "Where the <unit> went": one per local day (week / month —
 *  empty days included, so a gap reads as a zero column), or one per 2-hour
 *  block (day). */
export function slots(startMs: number, endMs: number, unit: PeriodUnit, nowMs: number): Slot[] {
  const out: Slot[] = [];
  const s0 = new Date(startMs);
  for (let i = 0; ; i++) {
    const s = unit === "day"
      ? new Date(s0.getFullYear(), s0.getMonth(), s0.getDate(), i * BLOCK_H)
      : new Date(s0.getFullYear(), s0.getMonth(), s0.getDate() + i);
    const e = unit === "day"
      ? new Date(s0.getFullYear(), s0.getMonth(), s0.getDate(), (i + 1) * BLOCK_H)
      : new Date(s0.getFullYear(), s0.getMonth(), s0.getDate() + i + 1);
    if (+s >= endMs) break;
    out.push({
      startMs: +s,
      endMs: +e,
      label: unit === "day" ? hh(i * BLOCK_H) : unit === "week"
        ? s.toLocaleDateString("en-US", { weekday: "short" }) : String(s.getDate()),
      tip: unit === "day" ? `${hh(i * BLOCK_H)}:00–${hh((i + 1) * BLOCK_H)}:00` : dayTip(s),
      live: nowMs >= +s && nowMs < +e,
      future: +s > nowMs,
    });
  }
  return out;
}

/** Day mode keeps only the blocks from the first to the last with anything
 *  (through "now" when live); week/month keep every day. */
function trim<T extends Column>(cols: T[], unit: PeriodUnit): T[] {
  if (unit !== "day") return cols;
  const hit = cols.map((c, i) => (c.totalMs > 0 || c.live ? i : -1)).filter((i) => i >= 0);
  return hit.length ? cols.slice(hit[0], hit[hit.length - 1] + 1) : [];
}

/** Category-stacked activity time per slot. */
export function activityColumns(acts: Activity[], startMs: number, endMs: number, unit: PeriodUnit, nowMs: number): Column[] {
  return trim(
    slots(startMs, endMs, unit, nowMs).map((s) => {
      // Stacked in the fixed category order so a colour sits at the same height in every column.
      const parts = categoryRows(acts, s.startMs, s.endMs)
        .map(({ key, label, colorVar, ms }) => ({ key, label, colorVar, ms }))
        .sort((a, b) => STACK.indexOf(a.key) - STACK.indexOf(b.key));
      return { ...s, parts, totalMs: parts.reduce((t, p) => t + p.ms, 0) };
    }),
    unit,
  );
}

/** Capture-only columns: the range's app time spread over the slots by the
 *  usage heatmap's per-hour capture intensity.
 *  ponytail: get_usage_charts has no per-day app time, so this is a share of
 *  Σ app time, not a measurement; a per-day query if the bars must be exact. */
export function appTimeColumns(
  heatmap: { bucketStartMs: number; intensityCount: number }[],
  trackedMs: number, startMs: number, endMs: number, unit: PeriodUnit, nowMs: number,
): Column[] {
  const total = heatmap.reduce((t, b) => t + Math.max(0, b.intensityCount), 0);
  return trim(
    slots(startMs, endMs, unit, nowMs).map((s) => {
      const w = heatmap.reduce((t, b) => (b.bucketStartMs >= s.startMs && b.bucketStartMs < s.endMs ? t + Math.max(0, b.intensityCount) : t), 0);
      const ms = total > 0 ? (trackedMs * w) / total : 0;
      return { ...s, parts: ms > 0 ? [{ key: "app", label: "App time", colorVar: "--chart-grey-3", ms }] : [], totalMs: ms };
    }),
    unit,
  );
}

export type FocusBand = "deep" | "mixed" | "distracted";
export interface HeatRow {
  label: string;
  tip: string;
  /** 12 two-hour slots, 00–24 local; null = no activity there. */
  cells: ({ band: FocusBand; minutes: number } | null)[];
}

const FOCUS_W: Record<FocusBand, number> = { deep: 1, mixed: 0.5, distracted: 0 };

/** Focus heat: rows = captured days (day / week) or ISO weeks (month, "W41");
 *  a cell's band is the overlap-weighted mean focus, `minutes` the mean
 *  activity minutes per captured day in that slot. */
export function focusHeat(acts: Activity[], startMs: number, endMs: number, unit: PeriodUnit): HeatRow[] {
  const days = slots(startMs, endMs, unit === "day" ? "week" : unit, 0)
    .map((d) => ({ d, acts: acts.filter((a) => a.focus != null && overlap(a, d.startMs, d.endMs) > 0) }))
    .filter((x) => x.acts.length);
  const groups = new Map<string, { label: string; tip: string; days: typeof days }>();
  for (const x of days) {
    const date = new Date(x.d.startMs);
    const key = unit === "month" ? `W${isoWeek(date)}` : x.d.tip;
    const g = groups.get(key) ?? {
      label: unit === "month" ? key : date.toLocaleDateString("en-US", { weekday: "short" }),
      tip: unit === "month" ? `week ${isoWeek(date)}` : x.d.tip,
      days: [],
    };
    g.days.push(x);
    groups.set(key, g);
  }
  return [...groups.values()].map((g) => ({
    label: g.label,
    tip: g.tip,
    cells: Array.from({ length: 12 }, (_, i) => {
      let w = 0, m = 0;
      for (const { d, acts: da } of g.days) {
        const s0 = new Date(d.startMs);
        const s = +new Date(s0.getFullYear(), s0.getMonth(), s0.getDate(), i * 2);
        const e = +new Date(s0.getFullYear(), s0.getMonth(), s0.getDate(), i * 2 + 2);
        for (const a of da) {
          const o = overlap(a, s, e) / 60000;
          w += FOCUS_W[a.focus as FocusBand] * o;
          m += o;
        }
      }
      if (m <= 0) return null;
      const mean = w / m;
      return { band: mean >= 0.7 ? "deep" : mean >= 0.35 ? "mixed" : "distracted", minutes: m / g.days.length };
    }),
  }));
}

