// Frontend-only cross-surface handoff: the Activity Receipt (on /insights) asks
// the Timeline (on /) to focus a specific frame OR audio segment after a route
// switch. Module state persists across the /insights→/ route change within the
// same window, so `initializeTimeline()` on the freshly-mounted Timeline page
// can consume it.
// ponytail: a one-slot singleton, no backend command — the receipt sets it,
// the timeline takes it exactly once.
export type PendingTimelineFocus = { frameId: number } | { audioSegmentId: number };

let pending: PendingTimelineFocus | null = null;

export function setPendingTimelineFocus(focus: PendingTimelineFocus): void {
  pending = focus;
}

export function takePendingTimelineFocus(): PendingTimelineFocus | null {
  const p = pending;
  pending = null;
  return p;
}
