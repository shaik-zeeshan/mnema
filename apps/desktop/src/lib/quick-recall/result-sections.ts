// Pure merge / show-more logic for the Quick Recall results list: one ranked
// list (best moment + rows) built from the frame and audio lists that one
// `search_capture` call returns. Plain TS so it's bun-testable without Svelte.
import type { AudioSearchResultDto, FrameSearchResultDto } from "../types/app-infra";
import { formatClock } from "$lib/time-format";

// Per-call fetch limits (fetch once, expand client-side; anything the backend
// reports beyond these via hasMoreFrames/Audio is deliberately ignored).
export const FRAME_FETCH_LIMIT = 24;
export const AUDIO_FETCH_LIMIT = 12;

// The one collapsed cap: the best moment + 7 rows, per the mockup.
export const VISIBLE_CAP = 8;

export type RankedResult =
  | { kind: "frame"; frame: FrameSearchResultDto }
  | { kind: "audio"; audio: AudioSearchResultDto };

// Two-pointer merge of the two rank-ordered lists — the broker's rule
// (`frame_outranks_audio`, brokered_access.rs): LOWER rank first; on an exact
// tie the newer start wins, and a frame wins a full tie.
export function mergeByRank(
  frames: FrameSearchResultDto[],
  audio: AudioSearchResultDto[],
): RankedResult[] {
  const out: RankedResult[] = [];
  let f = 0;
  let a = 0;
  while (f < frames.length || a < audio.length) {
    const frame = frames[f];
    const clip = audio[a];
    const takeFrame =
      clip === undefined ||
      (frame !== undefined &&
        (frame.rank !== clip.rank
          ? frame.rank < clip.rank
          : frame.groupStartAt >= clip.absoluteStartAt));
    if (takeFrame) out.push({ kind: "frame", frame: frames[f++] });
    else out.push({ kind: "audio", audio: audio[a++] });
  }
  return out;
}

// How many rows are rendered given the collapse state.
export function visibleCount(total: number, expanded: boolean): number {
  return expanded ? total : Math.min(total, VISIBLE_CAP);
}

// The show-more/show-less toggle row label, or null when the list fits.
export function moreRowLabel(total: number, expanded: boolean): string | null {
  if (total <= VISIBLE_CAP) return null;
  return expanded ? "↑ show less" : `↓ show ${total - VISIBLE_CAP} more`;
}

// A row's time (mockup `short`): "14:12" / "2:12 PM" today, "Wed 17:10" within the week,
// "Oct 6" before that.
export function momentTime(at: Date, now: Date = new Date()): string {
  const time = formatClock(at);
  const days = Math.round(
    (new Date(now).setHours(0, 0, 0, 0) - new Date(at).setHours(0, 0, 0, 0)) / 86_400_000,
  );
  if (days === 0) return time;
  if (days > 0 && days < 7) {
    return `${at.toLocaleDateString(undefined, { weekday: "short" })} ${time}`;
  }
  return at.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
