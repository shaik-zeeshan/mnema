// receipt-lane — pure, DOM-free selection for the Receipt's synced transcript
// reader (Receipt redesign). The reader row a user clicks is keyed by
// TurnView.key; the row it HIGHLIGHTS is the turn under the current playhead
// (activeKeyAt). This module owns the shared key→index "bus" the reader resolves
// through, the initial selection, and the time→turn resolver. No Svelte, no
// DOM — unit-tested in bun. The view models come from buildTurnViews in
// receipt-audio.ts.

import type { TurnView } from "./receipt-audio";

/** Shared selection resolver — the "bus". The reader row selects by
 *  TurnView.key; this maps key → index (-1 if absent/null). */
export function selectionIndex(turns: TurnView[], key: string | null): number {
  if (key == null) return -1;
  return turns.findIndex((t) => t.key === key);
}

/** Initial selection: the headline turn, else the first cited, else the first
 *  (null for an empty set). */
export function defaultSelectedKey(turns: TurnView[]): string | null {
  if (turns.length === 0) return null;
  return (
    turns.find((t) => t.isHeadline)?.key ??
    turns.find((t) => t.cited)?.key ??
    turns[0].key
  );
}

/** The turn under a wall-clock instant: the last turn that has started at or
 *  before `ms` (karaoke "current line"), or null if none/empty. Turns are
 *  ascending by startMs (buildTurnViews guarantees it), so scan until the first
 *  start past `ms`. Pure. */
export function activeKeyAt(turns: TurnView[], ms: number | null): string | null {
  if (ms == null) return null;
  let key: string | null = null;
  for (const t of turns) {
    if (t.startMs > ms) break;
    key = t.key;
  }
  return key;
}

/** The turn whose window [startMs, endMs] contains the wall-clock `ms` — the
 *  segment to relive when the user clicks the scrub bar at that instant. Turns
 *  are start-ordered, so the first containing turn wins and an overlapping
 *  mic/system pair resolves to the earlier-starting one. null when `ms` falls in
 *  a gap with no captured audio (the caller keeps the frame-only scrub). Pure. */
export function turnAtMs(turns: TurnView[], ms: number | null): TurnView | null {
  if (ms == null) return null;
  for (const t of turns) {
    if (ms >= t.startMs && ms <= t.endMs) return t;
  }
  return null;
}

/** Auto-advance target after `endedSegmentId`'s audio finishes: the first turn
 *  of the NEXT distinct segment, in chronological (first-appearance) order.
 *  Advancing by segment — not turn — is why consecutive same-segment turns play
 *  continuously inside one clip; we only hop when the segment changes. null when
 *  the ended segment was the last, so playback stops at the span's end. Pure.
 *  ponytail: overlapping mic + system-audio segments play back-to-back (each
 *  replays its shared window) — the honest way to hear both mono sides through
 *  one <audio>; revisit only if replaying the overlap ever feels wrong. */
export function nextClipTurn(turns: TurnView[], endedSegmentId: number | null): TurnView | null {
  if (endedSegmentId == null) return null;
  const order: number[] = [];
  const firstTurnBySeg = new Map<number, TurnView>();
  for (const t of turns) {
    if (!firstTurnBySeg.has(t.audioSegmentId)) {
      firstTurnBySeg.set(t.audioSegmentId, t);
      order.push(t.audioSegmentId);
    }
  }
  const i = order.indexOf(endedSegmentId);
  if (i < 0 || i + 1 >= order.length) return null;
  return firstTurnBySeg.get(order[i + 1]) ?? null;
}
