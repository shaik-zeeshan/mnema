// ── Timeline zoom — span-only (round-4 decision G5) ──────────────────────────
// Hour / Day / Week, no Month level (the jump menu's month grid covers
// month-scale navigation). Zoom owns SPAN; the position pill owns POSITION.
//
// The rail is a frame browser: one fixed-width slot per loaded frame, so span
// is set by how many pixels each frame gets. We aim the slot width at the
// requested span using the capture cadence (one frame ≈ one capture interval),
// then clamp.
//
// ponytail: the clamp is a real ceiling, not tidiness. At the default 0.5 fps
// cadence an hour is 1,800 frames and a week is 302,400 — past both the pixel
// budget and the rail's 5,000-frame load cap — so Day and Week bottom out at
// their floor and show less than they name. The floors keep the three levels
// visibly distinct and monotonic, and the span readout always prints what is
// really on screen. Honouring Day/Week literally needs the rail to render from
// coverage aggregates instead of frames — a rail data-model change, not a knob.
export type TimelineZoom = "hour" | "day" | "week";

export const TIMELINE_ZOOMS: readonly TimelineZoom[] = ["hour", "day", "week"];

const TARGET_SPAN_MS: Record<TimelineZoom, number> = {
  hour: 3_600_000,
  day: 86_400_000,
  week: 604_800_000,
};

/** Narrowest slot per level. `day` keeps the rail's shipped 8 px density. */
const MIN_SLOT_PX: Record<TimelineZoom, number> = {
  hour: 16,
  day: 8,
  week: 3,
};

const MAX_SLOT_PX = 48;

/** Capture interval when the recording settings aren't loaded yet (0.5 fps). */
export const DEFAULT_FRAME_INTERVAL_MS = 2_000;

export function frameIntervalMs(fps: number | undefined | null): number {
  if (!fps || fps <= 0 || !isFinite(fps)) return DEFAULT_FRAME_INTERVAL_MS;
  return 1_000 / fps;
}

/** Slot width (px per frame) for a zoom level, rounded to a half pixel. */
export function slotWidthForZoom(
  zoom: TimelineZoom,
  viewportPx: number,
  intervalMs: number,
): number {
  const viewport = viewportPx > 0 ? viewportPx : 1_200;
  const interval = intervalMs > 0 ? intervalMs : DEFAULT_FRAME_INTERVAL_MS;
  const framesInSpan = Math.max(1, TARGET_SPAN_MS[zoom] / interval);
  const ideal = viewport / framesInSpan;
  const clamped = Math.min(MAX_SLOT_PX, Math.max(MIN_SLOT_PX[zoom], ideal));
  return Math.round(clamped * 2) / 2;
}

/**
 * The span actually on screen, measured from the loaded frames rather than
 * assumed from the cadence — so an overnight gap inside the visible window
 * reads as the hours it really is. Returns null when there is nothing to
 * measure.
 */
export function visibleSpanMs(
  capturedAtMs: number[],
  activeIndex: number,
  visibleCount: number,
): number | null {
  if (capturedAtMs.length < 2 || visibleCount < 2) return null;
  const half = Math.floor(visibleCount / 2);
  const newest = Math.max(0, Math.min(capturedAtMs.length - 1, activeIndex - half));
  const oldest = Math.max(0, Math.min(capturedAtMs.length - 1, activeIndex + half));
  if (newest === oldest) return null;
  return Math.abs(capturedAtMs[newest] - capturedAtMs[oldest]);
}

/** "4h 12m" · "18m" · "42s". Coarse by design (G8: no minute-precise claims). */
export function formatSpan(ms: number): string {
  if (!isFinite(ms) || ms <= 0) return "0s";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `${hours}h ${String(rest).padStart(2, "0")}m`;
}
