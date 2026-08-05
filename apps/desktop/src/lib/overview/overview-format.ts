// Overview (direction 01 — Bento Native) — pure formatting + shaping helpers.
//
// Everything here is date-math, byte-math and list-shaping so it can be
// exercised under `bun test`; the tile components keep only the reactive
// plumbing and the `invoke` calls. Type-only `$lib/*` imports are erased by the
// transpiler, so they are safe here (the same rule `jumper-coverage.ts` follows;
// a VALUE import from `$lib/*` would break `bun test`).
//
// Round-4 decision G8 is the standing rule in this file: a number is rendered
// only where the value is real on this machine. Every formatter therefore takes
// `number | null | undefined` and returns `null` for "render no number" — never
// a zero standing in for an unknown.
import type { DayCoverage } from "$lib/types/app-infra";
import type { Conclusion } from "$lib/types/recording";

const DAY_MS = 86_400_000;

/** Half-open `[startMs, endMs)` for the local calendar day containing `now`. */
export function localDayWindow(now: Date): { startMs: number; endMs: number } {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const end = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  return { startMs: start.getTime(), endMs: end.getTime() };
}

/** `YYYY-MM-DD` in local time — the key shape `list_day_coverage` returns. */
export function localDayKey(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** "6:42" — the Capture tile's one `--t-display` hero. Null below one minute. */
export function heroHours(ms: number | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 60_000) return null;
  const minutes = Math.floor(ms / 60_000);
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
}

/** "6h 42m" · "47m" · null when nothing was captured. */
export function capturedLabel(ms: number | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 60_000) return null;
  const minutes = Math.floor(ms / 60_000);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return hours === 0 ? `${rest}m` : `${hours}h ${String(rest).padStart(2, "0")}m`;
}

/** "2:14:07" — running capture elapsed, always H:MM:SS. */
export function elapsedClock(startMs: number, nowMs: number): string {
  const total = Math.max(0, Math.floor((nowMs - startMs) / 1000));
  const p = (n: number) => String(n).padStart(2, "0");
  return `${Math.floor(total / 3600)}:${p(Math.floor(total / 60) % 60)}:${p(total % 60)}`;
}

/** "13:02" local, from unix ms. */
export function clock(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** "38 min" · "1h 02m" — a conversation's spoken length. */
export function minutesLabel(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/**
 * "34.2 GB" · "270 MB" · "8.1 GB". Null in, null out — G8: an unmeasured
 * quantity renders no number rather than a zero.
 */
export function bytesLabel(bytes: number | null | undefined): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit >= 3 && value < 100 ? 1 : 0;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

/** One bar of the This Week tile. `ms` is honest zero for a day with no capture. */
export interface WeekBar {
  key: string;
  /** "Mo" / "Tu" — two-letter weekday initial. */
  label: string;
  ms: number;
  isToday: boolean;
  /** 0–1 against the week's busiest day; 0 when the whole week is empty. */
  fraction: number;
}

/**
 * The last seven local days ending at `today`, OLDEST first (the mockup reads
 * left-to-right into today). Days absent from `list_day_coverage` are absent
 * because they hold no capture — they draw a zero bar, never a gap.
 */
export function weekBars(days: DayCoverage[], today: Date): WeekBar[] {
  const byDay = new Map(days.map((d) => [d.day, d]));
  const bars: WeekBar[] = [];
  for (let i = 6; i >= 0; i--) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - i);
    const key = localDayKey(date);
    bars.push({
      key,
      label: date.toLocaleDateString(undefined, { weekday: "short" }).slice(0, 2),
      ms: byDay.get(key)?.coveredMs ?? 0,
      isToday: i === 0,
      fraction: 0,
    });
  }
  const peak = Math.max(...bars.map((b) => b.ms));
  if (peak > 0) for (const bar of bars) bar.fraction = bar.ms / peak;
  return bars;
}

/** Total capture across the seven bars — the This Week tile's header meta. */
export function weekTotalMs(bars: WeekBar[]): number {
  return bars.reduce((sum, bar) => sum + bar.ms, 0);
}

/** One Subjects-tile row: a Subject with its most recently supported belief. */
export interface SubjectRow {
  subject: string;
  statement: string;
  /** 0–1, the top belief's confidence — drawn as filled `.conv` dots. */
  confidence: number;
  lastSupportedAtMs: number;
}

/**
 * Group Conclusions into Subject rows, most-recently-supported first. Pinned
 * beliefs win the row for their Subject, then the highest confidence — the same
 * "one line per Subject" reduction the Subjects surface makes, done here in ten
 * lines rather than imported, because that surface's version also carries tiers,
 * trends and trajectories this tile has no room for.
 */
export function subjectRows(conclusions: Conclusion[], limit = 2): SubjectRow[] {
  const best = new Map<string, Conclusion>();
  for (const c of conclusions) {
    const key = c.subject.toLowerCase();
    const held = best.get(key);
    if (
      !held ||
      (c.pinned && !held.pinned) ||
      (c.pinned === held.pinned && c.confidence > held.confidence)
    ) {
      best.set(key, c);
    }
  }
  return [...best.values()]
    .sort((a, b) => b.lastSupportedAtMs - a.lastSupportedAtMs || b.confidence - a.confidence)
    .slice(0, limit)
    .map((c) => ({
      subject: c.subject,
      statement: c.statement,
      confidence: c.confidence,
      lastSupportedAtMs: c.lastSupportedAtMs,
    }));
}

/** The newest belief's statement — the Context tile's "Newest:" line. */
export function newestStatement(conclusions: Conclusion[]): string | null {
  let newest: Conclusion | null = null;
  for (const c of conclusions) {
    if (!newest || c.formedAtMs > newest.formedAtMs) newest = c;
  }
  return newest?.statement ?? null;
}

/** Filled dots out of `of` for a 0–1 confidence. At least one when held at all. */
export function confidenceDots(confidence: number, of = 5): number {
  if (!Number.isFinite(confidence) || confidence <= 0) return 0;
  return Math.min(of, Math.max(1, Math.round(confidence * of)));
}

/**
 * "in the last 7 days" window for the day-relative labels the tiles use for a
 * conversation-history row: "13:58" today, "Yesterday", else a short date.
 */
export function historyStamp(ms: number, now: Date): string {
  const today = localDayWindow(now);
  if (ms >= today.startMs) return clock(ms);
  if (ms >= today.startMs - DAY_MS) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
