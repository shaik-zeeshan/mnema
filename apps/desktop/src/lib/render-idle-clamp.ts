// The pure geometry behind the display-disconnect window clamp, split out of
// `render-idle.svelte.ts` so it is testable without runes, a window, or Tauri IPC.
//
// A display disconnect can park the window (almost) entirely outside every
// remaining screen. macOS keeps reporting such a window visible while a sliver is
// on-screen, so it never occludes, never goes `hidden`, and every repaint strands a
// non-purgeable IOSurface — the leak the render-idle gate exists to stop, arriving
// through a door the gate cannot see.

/** A rectangle in physical pixels, as tauri reports monitors and windows. */
export type Rect = {
  position: { x: number; y: number };
  size: { width: number; height: number };
};

/** A monitor as `availableMonitors()` returns it. */
export type Monitor = Rect & { scaleFactor: number };

/**
 * How much of the window must overlap some monitor, on BOTH axes, to count as
 * meaningfully visible. Logical points, the backend's `parked_view`
 * `MIN_VISIBLE_PT`.
 */
export const MIN_VISIBLE_PT = 50;

/**
 * Where the window should be moved to, in global logical points, or `null` to
 * leave it alone.
 *
 * Tauri's physical rects are each `points × that rect's own scale factor`, so a
 * Retina monitor beside a 1x one share no pixel space. Every rect is divided by
 * its own scale first; AppKit's global point space is the shared one.
 *
 * `null` means either "already meaningfully visible somewhere" or "no monitors to
 * move it to" — both are no-ops for the caller.
 */
export function clampTarget(
  monitors: Monitor[],
  pos: { x: number; y: number },
  size: { width: number; height: number },
  scaleFactor: number,
): { x: number; y: number } | null {
  if (monitors.length === 0) return null;

  const w = { x: pos.x / scaleFactor, y: pos.y / scaleFactor };
  const ws = { width: size.width / scaleFactor, height: size.height / scaleFactor };
  const screens = monitors.map((m) => ({
    x: m.position.x / m.scaleFactor,
    y: m.position.y / m.scaleFactor,
    width: m.size.width / m.scaleFactor,
    height: m.size.height / m.scaleFactor,
  }));

  const meaningfullyVisible = screens.some((m) => {
    // Plain interval intersection on each axis. Monitor origins are signed — a
    // display to the left of or above the primary has negative coordinates — so
    // this must never take an absolute value or assume the primary is at (0, 0).
    const overlapW = Math.min(w.x + ws.width, m.x + m.width) - Math.max(w.x, m.x);
    const overlapH = Math.min(w.y + ws.height, m.y + m.height) - Math.max(w.y, m.y);
    return overlapW >= MIN_VISIBLE_PT && overlapH >= MIN_VISIBLE_PT;
  });
  if (meaningfullyVisible) return null;

  // Centre on the first remaining monitor. `Math.max(0, …)` keeps a window LARGER
  // than the target monitor pinned to that monitor's origin instead of being pushed
  // to a negative offset that would park it offscreen again.
  const m = screens[0];
  return {
    x: m.x + Math.max(0, Math.round((m.width - ws.width) / 2)),
    y: m.y + Math.max(0, Math.round((m.height - ws.height) / 2)),
  };
}
