// receipt-clock — wall-clock formatters for the Activity Receipt. Evidence
// answers WHEN a frame/turn happened (absolute time), not elapsed duration. Pure,
// shared by ActivityReceipt.svelte and its extracted ReceiptViewer child.

/** Hour:minute with AM/PM — the receipt's default wall clock. */
export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", hour12: true });
}

/** Hour:minute:second — the frame-meta / counter clock (per-frame precision). */
export function clockSec(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit", hour12: true });
}

/** The wall clock without the AM/PM suffix — lane/transcript time (matches the mockup). */
export function clockShort(ms: number): string {
  return clock(ms).replace(/\s?[AP]M$/i, "");
}
