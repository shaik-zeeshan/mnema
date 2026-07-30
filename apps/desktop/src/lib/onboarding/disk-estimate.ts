// Daily disk cost of capture, from a MEASURED anchor (issue #195, slice 1).
//
// Anchor: 270 MB/day total — one snapshot every 3 s, 720p, medium bitrate,
// pause-on-inactivity ON — measured over a complete 14-day window
// (3.00 GB recordings + 773 MB index). n=1: one machine, one set of habits.
//
// Storage scales LINEARLY with frame rate: `compute_effective_screen_bitrate_bps`
// multiplies the preset bitrate by the frame rate, and OCR (the index half) runs
// once per frame. So the whole ladder derives from the one anchor rather than
// from invented preset tiers — the rate itself is the app's existing log-spaced
// slider (`$lib/components/capture-rate`), and this module keys off its stops.
import { DEFAULT_CAPTURE_INTERVAL_S } from "../components/capture-rate";

/** Seconds between snapshots at which the anchor was measured. */
export const ANCHOR_INTERVAL_S = 3;
/** MB/day at `ANCHOR_INTERVAL_S`, recordings + index. */
export const ANCHOR_MB_PER_DAY = 270;

/**
 * Estimated MB/day at `intervalSeconds` between snapshots. Linear in frame rate,
 * so halving the interval doubles the figure: the 2 s default returns 405.
 * A non-positive interval falls back to the slider's default.
 */
export function estimateDailyStorageMb(intervalSeconds: number): number {
  const interval =
    intervalSeconds > 0 ? intervalSeconds : DEFAULT_CAPTURE_INTERVAL_S;
  return round1((ANCHOR_MB_PER_DAY * ANCHOR_INTERVAL_S) / interval);
}

/**
 * Estimated MB held at steady state under a `days`-long retention window — the
 * consequence line shown next to the selected Retention option. `Never` has no
 * steady state, so the caller shows nothing rather than passing a number here.
 */
export function estimateWindowStorageMb(
  intervalSeconds: number,
  days: number,
): number {
  return round1(estimateDailyStorageMb(intervalSeconds) * Math.max(days, 0));
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}
