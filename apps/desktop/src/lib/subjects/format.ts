// Subjects formatting — the page's TWO number formats and its time wording.
//
// The split is deliberate (mockup page 09, "two numbers, deliberately"): a
// subject ROW shows the raw 0–1 measurement (`0.86`), and anything at the
// CONCLUSION level shows the percent claim (`86%`). Same quantity, two
// registers; keep them apart.
import type { Trend } from "$lib/insights/subjectsTiers";

function clamp01(v: number): number {
  return Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));
}

/** Row register: the measurement, two decimals on the 0–1 scale. */
export function conf(v: number): string {
  return clamp01(v).toFixed(2);
}

/** Conclusion register: the claim, whole percent. */
export function pct(v: number): number {
  return Math.round(clamp01(v) * 100);
}

/** "3h ago" — coarse by design, and past-tense. An unknown time is a dash, not
 *  an invented one; "now" never becomes "now ago".
 *
 *  ponytail: a local copy rather than `conversationStore`'s — that module
 *  instantiates a `$state` singleton on import, which makes this file (and its
 *  test) un-runnable under plain `bun test`. */
export function ago(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const diff = Date.now() - ms;
  const min = Math.floor(diff / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  const wk = Math.floor(day / 7);
  if (wk < 5) return `${wk}w ago`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.floor(day / 365)}y ago`;
}

export function trendLabel(t: Trend): string {
  return t === "up" ? "▲ warming" : t === "down" ? "▼ cooling" : "– steady";
}

/** "18 Jul" — the formation date, day + short month, locale-ordered. */
export function shortDate(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  return new Date(ms).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
  });
}

/** The conclusion card's sub-line. Three shapes, all in the row register:
 *  faded    → "0.38 → 0.12 · below floor"
 *  moved    → "0.42 → 0.86 · 3h ago"
 *  flat/one → "steady near 0.74 · 2d ago"
 *  A history shorter than two points has no arc to state, so it falls back to
 *  the steady wording rather than inventing a start value. */
export function deltaLine(opts: {
  history: readonly number[];
  confidence: number;
  faded: boolean;
  lastSupportedAtMs: number;
}): string {
  const { history, confidence, faded, lastSupportedAtMs } = opts;
  const n = history.length;
  const last = n > 0 ? history[n - 1] : confidence;
  const first = n > 1 ? history[0] : last;
  const moved = n > 1 && Math.abs(last - first) >= 0.01;
  if (faded) {
    return moved
      ? `${conf(first)} → ${conf(last)} · below floor`
      : `${conf(last)} · below floor`;
  }
  if (moved) return `${conf(first)} → ${conf(last)} · ${ago(lastSupportedAtMs)}`;
  return `steady near ${conf(last)} · ${ago(lastSupportedAtMs)}`;
}
