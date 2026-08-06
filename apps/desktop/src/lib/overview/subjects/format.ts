// Shared readouts for the Subjects destination (index + drill-in).
//
// ONE format for confidence: `NN%`, everywhere. The old index printed a raw
// `0.86` while every detail surface printed `86%` for the same number — the
// only place in the app that showed a 0–1 float to a person.
//
// G8: a fact that isn't there renders NOTHING. `agoLabel` returns null rather
// than an em-dash placeholder, so the caller omits the slot instead of drawing
// an empty one.

/** 0–1 confidence as the whole percent the whole app prints. */
export function pct(confidence: number): number {
  return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
}

/** `86%`. */
export function pctLabel(confidence: number): string {
  return `${pct(confidence)}%`;
}

/** Coarse "2h ago" / "6w ago". Null for a missing or future-dated stamp. */
export function agoLabel(ms: number): string | null {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const diff = Date.now() - ms;
  if (diff < 0) return "just now";
  const min = Math.floor(diff / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d ago`;
  const wk = Math.floor(day / 7);
  // Weeks run to 8 rather than 4: a subject's whole life is measured in weeks
  // here, and "1mo ago" next to a "6 WEEKS AGO" axis label for the same day is
  // two names for one distance.
  if (wk < 9) return `${wk}w ago`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.floor(day / 365)}y ago`;
}

/** The same ladder without the "ago", for a span: "6 weeks", "3 days". */
export function spanLabel(fromMs: number, toMs: number): string | null {
  const diff = toMs - fromMs;
  if (!Number.isFinite(diff) || diff <= 0) return null;
  const min = Math.floor(diff / 60000);
  if (min < 60) return `${min} minute${min === 1 ? "" : "s"}`;
  const hr = Math.floor(min / 60);
  if (hr < 48) return `${hr} hour${hr === 1 ? "" : "s"}`;
  const day = Math.floor(hr / 24);
  if (day < 14) return `${day} day${day === 1 ? "" : "s"}`;
  const wk = Math.floor(day / 7);
  if (wk < 9) return `${wk} week${wk === 1 ? "" : "s"}`;
  const mo = Math.floor(day / 30);
  if (mo < 24) return `${mo} month${mo === 1 ? "" : "s"}`;
  const yr = Math.floor(day / 365);
  return `${yr} year${yr === 1 ? "" : "s"}`;
}

/** The movement chip's wording — the engine's ±0.04 dead-band, in words. */
export function trendLabel(trend: "up" | "steady" | "down"): string {
  return trend === "up"
    ? "▲ warming"
    : trend === "down"
      ? "▼ cooling"
      : "– steady";
}
