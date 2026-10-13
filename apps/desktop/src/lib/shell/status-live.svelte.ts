// The status bar's and status popover's live feed: a ref-counted 2 s
// `get_capture_health` poll (display asleep, system audio reconnecting, free
// space vs the pause threshold) plus the popover's one open flag.
import { invoke } from "@tauri-apps/api/core";
import { renderIdle } from "$lib/render-idle.svelte";
import type { CaptureHealthDto } from "$lib/types/debug";

const POLL_MS = 2000;
const _health = $state<{ value: CaptureHealthDto | null }>({ value: null });
let refCount = 0;
let handle: ReturnType<typeof setInterval> | null = null;
let inFlight = false;

export const captureHealth: { readonly value: CaptureHealthDto | null } = {
  get value() {
    return _health.value;
  },
};

/** The status popover's open state (SP3 renders it; the bar's triggers set it). */
export const statusPop = $state({ open: false });

async function refresh(): Promise<void> {
  // Skip while nothing can render: each repaint then strands an IOSurface
  // (see $lib/render-idle.svelte). The interval keeps ticking and catches up.
  if (renderIdle() || inFlight) return;
  inFlight = true;
  const next = await invoke<CaptureHealthDto>("get_capture_health").catch(() => null);
  inFlight = false;
  // Released while in flight: don't resurrect a stale value.
  if (refCount > 0) _health.value = next;
}

/** Start (or join) the poll; the returned fn releases it. The last release clears the value. */
export function subscribeCaptureHealth(): () => void {
  refCount += 1;
  if (handle === null) {
    void refresh();
    handle = setInterval(refresh, POLL_MS);
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    refCount = Math.max(0, refCount - 1);
    if (refCount === 0 && handle !== null) {
      clearInterval(handle);
      handle = null;
      _health.value = null;
    }
  };
}
