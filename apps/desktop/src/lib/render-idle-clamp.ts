// The pure geometry behind the display-disconnect window clamp, split out of
// `render-idle.svelte.ts` so it is testable without runes, a window, or Tauri IPC.
//
// A display disconnect can park the window (almost) entirely outside every
// remaining screen. macOS keeps reporting such a window visible while a sliver is
// on-screen, so it never occludes, never goes `hidden`, and every repaint strands a
// non-purgeable IOSurface — the leak the render-idle gate exists to stop, arriving
// through a door the gate cannot see.

/** A monitor or window rectangle in the shared global physical-pixel space. */
export type Rect = {
  position: { x: number; y: number };
  size: { width: number; height: number };
};

/**
 * How much of the window must overlap some monitor, on BOTH axes, to count as
 * meaningfully visible. Physical pixels.
 */
export const MIN_VISIBLE_PX = 100;

/**
 * Where the window should be moved to, or `null` to leave it alone.
 *
 * `null` means either "already meaningfully visible somewhere" or "no monitors to
 * move it to" — both are no-ops for the caller.
 */
export function clampTarget(
  monitors: Rect[],
  pos: { x: number; y: number },
  size: { width: number; height: number },
): { x: number; y: number } | null {
  if (monitors.length === 0) return null;

  const meaningfullyVisible = monitors.some((m) => {
    // Plain interval intersection on each axis. Monitor origins are signed — a
    // display to the left of or above the primary has negative coordinates — so
    // this must never take an absolute value or assume the primary is at (0, 0).
    const overlapW =
      Math.min(pos.x + size.width, m.position.x + m.size.width) -
      Math.max(pos.x, m.position.x);
    const overlapH =
      Math.min(pos.y + size.height, m.position.y + m.size.height) -
      Math.max(pos.y, m.position.y);
    return overlapW >= MIN_VISIBLE_PX && overlapH >= MIN_VISIBLE_PX;
  });
  if (meaningfullyVisible) return null;

  // Centre on the first remaining monitor. `Math.max(0, …)` keeps a window LARGER
  // than the target monitor pinned to that monitor's origin instead of being pushed
  // to a negative offset that would park it offscreen again.
  const m = monitors[0];
  return {
    x: m.position.x + Math.max(0, Math.round((m.size.width - size.width) / 2)),
    y: m.position.y + Math.max(0, Math.round((m.size.height - size.height) / 2)),
  };
}
