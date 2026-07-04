// Pure dismissal policy for the dashboard audio drawer and its
// speaker-actions popover. The handlers in `routes/+page.svelte` translate a
// pointerdown/wheel event into this context and execute the returned action;
// keeping the decision DOM-free makes the branch ORDER (popover collapses
// before the drawer closes) testable under `bun test`.

export type AudioDrawerDismissAction =
  | "ignore"
  | "collapse-popover"
  | "switch"
  | "close-drawer";

export interface AudioDrawerDismissContext {
  /** The drawer is open (a segment is selected). */
  drawerOpen: boolean;
  /** Event target is inside the drawer element. */
  insideDrawer: boolean;
  /** Event target is inside the speaker-actions popover. */
  insidePopover: boolean;
  /** Event target is on a timeline audio bar (a SWITCH, never a close). */
  onAudioBar: boolean;
  /** A speaker-actions popover is currently open. */
  popoverOpen: boolean;
}

export function audioDrawerPointerDownAction(
  ctx: AudioDrawerDismissContext,
): AudioDrawerDismissAction {
  if (!ctx.drawerOpen) return "ignore";
  if (ctx.insideDrawer) return "ignore";
  // A click on another segment's bar switches the drawer, never closes it —
  // collapse a transient speaker-actions popover first, then let the bar's
  // click handler reselect.
  if (ctx.onAudioBar) return ctx.popoverOpen ? "collapse-popover" : "switch";
  if (ctx.popoverOpen) return "collapse-popover";
  return "close-drawer";
}

export function audioDrawerWheelAction(
  ctx: AudioDrawerDismissContext,
): AudioDrawerDismissAction {
  if (!ctx.drawerOpen) return "ignore";
  if (ctx.insidePopover) return "ignore";
  // The popover is anchored to viewport coordinates, so any wheel outside it
  // (including scrolling the transcript underneath) collapses it before it
  // can drift away from its chip.
  if (ctx.popoverOpen) return "collapse-popover";
  if (ctx.insideDrawer) return "ignore";
  return "close-drawer";
}
