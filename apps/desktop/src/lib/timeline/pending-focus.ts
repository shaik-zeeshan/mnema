// Frontend-only cross-surface handoff: the Activity Receipt (on /insights) asks
// the Timeline (on /) to focus a specific frame after a route switch. Module
// state persists across the /insights→/ route change within the same window, so
// `initializeTimeline()` on the freshly-mounted Timeline page can consume it.
// ponytail: a one-slot singleton, no backend command — the receipt sets it,
// the timeline takes it exactly once.
let pending: { frameId: number } | null = null;

export function setPendingTimelineFocus(frameId: number): void {
  pending = { frameId };
}

export function takePendingTimelineFocus(): { frameId: number } | null {
  const p = pending;
  pending = null;
  return p;
}
