// Window-level side of the developer window's strip: keep on top, and the
// 300×120 always-on-top mini mode. Pure frontend window APIs; the capability
// grants allow-set-always-on-top / allow-set-min-size / allow-set-size.
//
// Module state on purpose: the full strip unmounts while mini is showing, and
// the pin it remounts with must be the one it left with.

import { getCurrentWindow, LogicalSize, type PhysicalSize } from "@tauri-apps/api/window";

const MINI = new LogicalSize(300, 120);
const FULL_MIN = new LogicalSize(440, 480);
const FULL_DEFAULT = new LogicalSize(520, 640);

let pinned = false;
let restoreSize: PhysicalSize | null = null;

export const isPinned = (): boolean => pinned;

export async function setPinned(next: boolean): Promise<void> {
	await getCurrentWindow().setAlwaysOnTop(next);
	pinned = next;
}

/** Shrink to mini. Lower the min size before shrinking, or the shrink is clamped. */
export async function enterMini(): Promise<void> {
	const win = getCurrentWindow();
	// Inner, not outer: `setSize` sets the inner size (the same on this undecorated window).
	restoreSize = await win.innerSize();
	await win.setMinSize(MINI);
	await win.setSize(MINI);
	await win.setAlwaysOnTop(true);
}

/** Back to full. Restore the size before raising the min, then restore the pin. */
export async function exitMini(): Promise<void> {
	const win = getCurrentWindow();
	await win.setSize(restoreSize ?? FULL_DEFAULT);
	await win.setMinSize(FULL_MIN);
	await win.setAlwaysOnTop(pinned);
	restoreSize = null;
}
