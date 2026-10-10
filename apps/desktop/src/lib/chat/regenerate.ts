// Regenerate / Retry replace the latest answer in place (IMPLEMENTATION.md §5,
// §8 #14): delete the trailing turn's saved row, then re-send the same question,
// which Ask AI numbers by row count back into the freed turn_index. The backend's
// `delete_last_turn` does NOT check phase, so only a settled turn may re-run — a
// live stream would write the deleted row back.

/** What re-running `turn` takes, or null when it can't re-run: not the trailing
 *  turn, still streaming, `stopping` (stopped locally, but the backend's terminal
 *  op — sent only after the row is final — hasn't arrived), or `busy`.
 *  `deleteFirst` is false for a local-only error (send() threw before the
 *  backend saved a row), which has nothing to delete. */
export function regeneratePlan(
  turn: { turnIndex: number; phase: string; localOnly?: boolean; stopping?: boolean },
  turnCount: number,
  busy: boolean,
): { deleteFirst: boolean } | null {
  if (busy || turn.stopping || turn.turnIndex !== turnCount - 1) return null;
  if (turn.phase !== "done" && turn.phase !== "error") return null;
  return { deleteFirst: turn.localOnly !== true };
}
