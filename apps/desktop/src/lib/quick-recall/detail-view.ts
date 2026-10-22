// Pure helpers for Quick Look's audio side (bun-tested): which transcript
// turn the match sits in, and the clocks around it. No Svelte, no tauri.

import { parseCapturedAt } from "$lib/format-time";
import { formatClock } from "$lib/time-format";

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
    // End-exclusive: a span starting exactly where one turn ends and the next
    // begins belongs to the next turn.
    const distance = Math.max(
      turn.startMs - spanStartMs,
      spanStartMs - turn.endMs + 1,
      0,
    );
    if (distance < bestDistance) {
      bestDistance = distance;
      best = index;
    }
  });
  return best;
}

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

// Wall-clock stamp for a transcript turn ("15:03:41" / "3:03:41 PM"), from the segment's
// absolute start plus the turn's in-segment offset.
export function formatTurnClock(
  segmentStartedAt: string,
  offsetMs: number,
): string {
  const base = parseCapturedAt(segmentStartedAt);
  if (isNaN(base.getTime())) return "";
  return formatClock(base.getTime() + offsetMs, { seconds: true });
}
