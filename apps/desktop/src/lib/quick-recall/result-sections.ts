// Pure section-slicing / show-more / selection-clamping logic for the Quick
// Recall results list (Slice 4 of the search redesign). One `search_capture`
// call fetches up to 24 frame / 12 audio results; each section initially
// renders a capped slice with a "↓ show N more" row revealing the fetched
// remainder client-side (mockup `.more-row`, a collapse/expand toggle).
// Plain TS so it's bun-testable without Svelte.

// Per-call fetch limits (PLAN.md: fetch once, expand client-side; anything the
// backend reports beyond these via hasMoreFrames/Audio is deliberately ignored).
export const FRAME_FETCH_LIMIT = 24;
export const AUDIO_FETCH_LIMIT = 12;

// Initial (collapsed) per-section render caps, per the mockup's CAPS.
export const FRAME_VISIBLE_CAP = 8;
export const AUDIO_VISIBLE_CAP = 3;

// How many rows of a section are rendered given its collapse state.
export function visibleCount(total: number, cap: number, expanded: boolean): number {
  return expanded ? total : Math.min(total, cap);
}

// The show-more/show-less toggle row label, or null when the section fits its
// cap (no row rendered). Wording is the mockup's exactly:
//   "↓ show 16 more screen results" / "↑ show less".
export function moreRowLabel(
  total: number,
  cap: number,
  expanded: boolean,
  noun: string,
): string | null {
  if (total <= cap) {
    return null;
  }
  return expanded ? "↑ show less" : `↓ show ${total - cap} more ${noun} results`;
}

// Selection lives in the flattened VISIBLE row space (visible frames first,
// then visible audio). When a section's visible count changes (show-less
// collapse, or any resize), remap the selected index so the SAME result stays
// selected when it is still visible; when the collapse hid it, clamp to the
// nearest visible row. -1 (nothing selected) is preserved; an empty next
// state yields -1.
export function remapSelection(
  selected: number,
  prev: { frames: number; audio: number },
  next: { frames: number; audio: number },
): number {
  if (selected < 0) {
    return -1;
  }
  const nextTotal = next.frames + next.audio;
  if (nextTotal === 0) {
    return -1;
  }
  if (selected < prev.frames) {
    // A frame row: same position if still visible, else the nearest visible
    // row (last visible frame, or row 0 when the frame section emptied).
    return selected < next.frames ? selected : Math.max(next.frames - 1, 0);
  }
  const audioPos = Math.min(selected - prev.frames, prev.audio - 1);
  if (audioPos >= 0 && audioPos < next.audio) {
    return next.frames + audioPos;
  }
  // Hidden (or out-of-range) audio row: nearest visible is the last row.
  return nextTotal - 1;
}
