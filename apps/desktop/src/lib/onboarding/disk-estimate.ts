// Daily disk cost of capture, from a MEASURED anchor (issue #195, slice 1).
//
// Anchor: 270 MB/day total — one snapshot every 3 s, 720p, medium bitrate,
// pause-on-inactivity ON — measured over a complete 14-day window
// (3.00 GB recordings + 773 MB index). n=1: one machine, one set of habits.
//
// Two axes move the figure off the anchor:
//  · RATE — storage scales LINEARLY with frame rate: `compute_effective_screen_
//    bitrate_bps` multiplies the preset bitrate by the frame rate, and OCR (the
//    index half) runs once per frame. The rate itself is the app's existing
//    log-spaced slider (`$lib/components/capture-rate`).
//  · RESOLUTION — only the VIDEO share scales, and it scales with PIXELS per
//    frame, not with the bitrate setting: at snapshot frame rates every preset's
//    computed bitrate sits under the backend's 500 kbps floor
//    (`MIN_EFFECTIVE_VIDEO_BITRATE_BPS`), so the cap is the same number whatever
//    the user picks — the anchor's measured usage is ~1/10 of it. What actually
//    grows a VBR-encoded frame is how many pixels it has. The OCR/audio/
//    transcript shares don't move with resolution at all.
import { DEFAULT_CAPTURE_INTERVAL_S } from "../components/capture-rate";
import type { ResolutionMode, ResolutionPreset } from "../types/recording";

/** Seconds between snapshots at which the anchor was measured. */
export const ANCHOR_INTERVAL_S = 3;
/** MB/day at `ANCHOR_INTERVAL_S`, recordings + index. */
export const ANCHOR_MB_PER_DAY = 270;
/** Pixels per captured frame at which the anchor was measured (720p — which is
 *  also the app's default capture resolution, so the no-argument estimate is
 *  exact at defaults). */
export const ANCHOR_VIDEO_PIXELS = 1280 * 720;
/** The video slice of the anchor — the only share that moves with resolution.
 *  `feature-cost.ts` uses this same constant as its screen row. */
export const ANCHOR_VIDEO_MB = 168;

const PRESET_PIXELS: Record<ResolutionPreset, number> = {
  "1080p": 1920 * 1080,
  "720p": 1280 * 720,
  "540p": 960 * 540,
};

/** Stand-in when the display's real pixel count can't be read. */
const FALLBACK_PIXELS = 1920 * 1080;

/**
 * Pixels per captured frame under the draft resolution settings. `original`
 * records the display's true backing pixels — the caller reads them where a
 * `window` exists (`screen.width × height × devicePixelRatio²`) and passes
 * them in; a mid-edit invalid custom size falls back like an unknown display.
 */
export function draftVideoPixels(
  draft: {
    resolutionMode: ResolutionMode;
    resolutionPreset: ResolutionPreset;
    customWidth: number | null;
    customHeight: number | null;
  },
  nativePixels: number | null,
): number {
  switch (draft.resolutionMode) {
    case "preset":
      return PRESET_PIXELS[draft.resolutionPreset];
    case "custom":
      return draft.customWidth && draft.customHeight
        ? draft.customWidth * draft.customHeight
        : (nativePixels ?? FALLBACK_PIXELS);
    default:
      return nativePixels ?? FALLBACK_PIXELS;
  }
}

/**
 * Estimated MB/day at `intervalSeconds` between snapshots and `videoPixels` per
 * frame. Linear in frame rate, so halving the interval doubles the figure; the
 * video share additionally scales with the pixel ratio against the 720p anchor.
 * A non-positive interval falls back to the slider's default; omitted pixels
 * price the anchor (= default) resolution.
 */
export function estimateDailyStorageMb(
  intervalSeconds: number,
  videoPixels: number = ANCHOR_VIDEO_PIXELS,
): number {
  const interval =
    intervalSeconds > 0 ? intervalSeconds : DEFAULT_CAPTURE_INTERVAL_S;
  const video = ANCHOR_VIDEO_MB * (videoPixels / ANCHOR_VIDEO_PIXELS);
  const rest = ANCHOR_MB_PER_DAY - ANCHOR_VIDEO_MB;
  return round1(((video + rest) * ANCHOR_INTERVAL_S) / interval);
}

/**
 * Estimated MB held at steady state under a `days`-long retention window — the
 * consequence line shown next to the selected Retention option. `Never` has no
 * steady state, so the caller shows nothing rather than passing a number here.
 */
export function estimateWindowStorageMb(
  intervalSeconds: number,
  days: number,
  videoPixels?: number,
): number {
  return round1(
    estimateDailyStorageMb(intervalSeconds, videoPixels) * Math.max(days, 0),
  );
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}
