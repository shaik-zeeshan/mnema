// subjects-format.ts — the handful of pure helpers the Subjects destination
// (direction 01, mockup 09) needs on top of the shared, already-tested
// machinery in `$lib/insights/subjectsTiers` + `$lib/insights/subjectTimeline`.
// Nothing here re-derives a threshold or a trend: those live in the shared
// modules and are imported, never copied.

/** "2h ago" — the page's stamp. The shared `relativeTime` in
 *  conversationStore.svelte.ts returns the bare "2h" form for tight chrome and
 *  lives in a `.svelte.ts` module (runes — not importable from a plain unit
 *  test), so the ladder is spelled out here with the "ago" suffix this page
 *  always wants. Same rungs, same wording. */
export function ago(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const diff = Date.now() - ms;
  const min = Math.floor(diff / 60000);
  if (diff < 0 || min < 1) return "just now";
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

/** 0..1 confidence → whole percent, clamped. */
export function pct(confidence: number): number {
  return Math.round(clamp01(confidence) * 100);
}

/** 0..1 confidence → the page's two-place figure ("0.78"). */
export function conf2(confidence: number): string {
  return clamp01(confidence).toFixed(2);
}

export function clamp01(v: number): number {
  return Math.max(0, Math.min(1, v));
}

/** "over 21 days" / "over 6 hours" — the trajectory's span, rounded coarsely
 *  (G8: no minute-precise claims). Returns null below an hour, so the caller
 *  simply drops the clause rather than printing "over 0 hours". */
export function spanLabel(ms: number): string | null {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const hours = Math.round(ms / 3_600_000);
  if (hours < 1) return null;
  if (hours < 48) return `over ${hours} ${hours === 1 ? "hour" : "hours"}`;
  const days = Math.round(hours / 24);
  return `over ${days} ${days === 1 ? "day" : "days"}`;
}

// ── sparkline / area geometry ───────────────────────────────────────────────
// X is the POINT INDEX, never time (mockup 09's own rule: a time axis would
// imply a window the engine's decay beat does not have). Y is confidence with a
// 2px pad top and bottom so a 1.00 or a 0.00 still draws inside the box.
export const SPARK_PAD = 2;

export function sparkY(confidence: number, height: number): number {
  return SPARK_PAD + (1 - clamp01(confidence)) * (height - SPARK_PAD * 2);
}

/** `points` attribute for one conclusion's trajectory across a `width`×`height`
 *  box. A single point is flattened into a flat 2-point line (a lone point
 *  draws nothing). Empty input yields "". */
export function sparkPoints(
  values: readonly number[],
  width: number,
  height: number,
): string {
  if (values.length === 0) return "";
  const pts = values.length === 1 ? [values[0], values[0]] : values;
  const step = pts.length > 1 ? width / (pts.length - 1) : 0;
  return pts
    .map((v, i) => `${(i * step).toFixed(1)},${sparkY(v, height).toFixed(1)}`)
    .join(" ");
}

/** The belief tile's filled area chart: the same polyline plus a closing path
 *  down to the baseline. Null when there is nothing to draw (< 2 points — a
 *  single snapshot is a floating dot, not a trajectory). */
export function areaPaths(
  values: readonly number[],
  width: number,
  height: number,
): { line: string; fill: string } | null {
  if (values.length < 2) return null;
  const step = width / (values.length - 1);
  const coords = values.map(
    (v, i) => `${(i * step).toFixed(1)},${sparkY(v, height).toFixed(1)}`,
  );
  const line = `M${coords.join("L")}`;
  return { line, fill: `${line}L${width},${height}L0,${height}Z` };
}
