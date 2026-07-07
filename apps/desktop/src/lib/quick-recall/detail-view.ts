// Pure view-model helpers for the Quick Recall detail pane (bun-tested):
// per-result cache keying, waveform bars + real match-position math, transcript
// match-turn resolution, and small formatting helpers. No Svelte reactivity,
// no tauri imports — the detail store / component own those.

import { parseCapturedAt } from "$lib/format-time";
import type { SelectedSearchResult } from "./searchStore.svelte";

// Cache key for a selection's fetched detail data. Frame results key on BOTH
// the representative frame (the OCR subject) and the thumbnail frame (the hero
// preview) — equivalent-reuse results can pair the same representative with
// different thumbnails. Audio results key on segment + match-span start, since
// two matches inside one segment want different transcript anchors/markers.
export function detailCacheKey(selected: SelectedSearchResult): string {
  return selected.kind === "frame"
    ? `frame:${selected.frame.representativeFrame.id}:${selected.frame.thumbnailFrameId}`
    : `audio:${selected.audio.audioSegment.id}:${selected.audio.spanStartMs}`;
}

// ── Waveform (mockup `.wavebox`: 64 bars, marker at the REAL match position) ──

export const WAVE_BAR_COUNT = 64;

export type WaveBar = { x: number; y: number; h: number; on: boolean };

// Fraction of the way through the segment where the match span starts.
// Clamped to [0, 1]; a zero/invalid duration yields 0 rather than NaN.
export function matchFraction(spanStartMs: number, durationMs: number): number {
  if (
    !Number.isFinite(spanStartMs) ||
    !Number.isFinite(durationMs) ||
    durationMs <= 0
  ) {
    return 0;
  }
  return Math.min(1, Math.max(0, spanStartMs / durationMs));
}

function matchBarIndex(frac: number): number {
  return Math.min(
    WAVE_BAR_COUNT - 1,
    Math.max(0, Math.round(frac * (WAVE_BAR_COUNT - 1))),
  );
}

// Deterministic bars from the result's group key (same Lehmer PRNG as the row
// tile, so the pane visually echoes the row), with the highlighted ±2-bar
// cluster placed at the REAL match position — unlike the row's decorative one.
// Bar geometry is in the mockup's `0 0 448 20` viewBox space.
export function waveBars(key: string, matchFrac: number): WaveBar[] {
  let s = 0;
  for (let i = 0; i < key.length; i++) s = (s * 31 + key.charCodeAt(i)) >>> 0;
  s = (s % 2147483646) + 1; // Lehmer seed must be in [1, 2147483646]
  const at = matchBarIndex(matchFrac);
  const bars: WaveBar[] = [];
  for (let i = 0; i < WAVE_BAR_COUNT; i++) {
    s = (s * 16807) % 2147483647;
    const h = 4 + ((s % 1000) / 1000) * 14;
    bars.push({ x: i * 7, y: (20 - h) / 2, h, on: Math.abs(i - at) <= 2 });
  }
  return bars;
}

// x of the vertical match-marker line, centered on the match bar (mockup: at*7+2).
export function waveMarkerX(matchFrac: number): number {
  return matchBarIndex(matchFrac) * 7 + 2;
}

// ── Transcript ────────────────────────────────────────────────────────────────

// Index of the turn the match span starts inside, else the turn whose
// [startMs, endMs] interval is nearest to the span start (transcription and
// diarization timelines can disagree by a little). -1 only for an empty list.
export function matchTurnIndex(
  turns: readonly { startMs: number; endMs: number }[],
  spanStartMs: number,
): number {
  let best = -1;
  let bestDistance = Infinity;
  turns.forEach((turn, index) => {
    const distance = Math.max(
      turn.startMs - spanStartMs,
      spanStartMs - turn.endMs,
      0,
    );
    if (distance < bestDistance) {
      bestDistance = distance;
      best = index;
    }
  });
  return best;
}

// ── Formatting ────────────────────────────────────────────────────────────────

export function segmentDurationMs(segment: {
  startedAt: string;
  endedAt: string;
}): number {
  const start = parseCapturedAt(segment.startedAt).getTime();
  const end = parseCapturedAt(segment.endedAt).getTime();
  return Number.isFinite(start) && Number.isFinite(end)
    ? Math.max(0, end - start)
    : 0;
}

// "M:SS" duration, mirroring the result row's wording.
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const total = Math.round(seconds);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

// Wall-clock stamp for a transcript turn ("15:03:41"), from the segment's
// absolute start plus the turn's in-segment offset.
export function formatTurnClock(
  segmentStartedAt: string,
  offsetMs: number,
): string {
  const base = parseCapturedAt(segmentStartedAt);
  if (isNaN(base.getTime())) return "";
  return new Date(base.getTime() + offsetMs).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

// "captured 15:00 – 15:05" (mockup time-row range), null when unparseable.
export function capturedRangeLabel(
  startAt: string,
  endAt: string,
): string | null {
  const start = parseCapturedAt(startAt);
  const end = parseCapturedAt(endAt);
  if (isNaN(start.getTime()) || isNaN(end.getTime())) return null;
  const fmt = (d: Date) =>
    d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  return `captured ${fmt(start)} – ${fmt(end)}`;
}

// ── App icon (mockup `.appicon`: colored initials square) ────────────────────
// Deterministic decoration from the app name — search results carry no icon.

export function appIconLabel(appName: string | null): string {
  const first = (appName ?? "").trim().split(/\s+/)[0] ?? "";
  return first.slice(0, 2) || "?";
}

export function appIconColor(appName: string | null): string {
  const name = appName ?? "";
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360}deg 42% 38%)`;
}
