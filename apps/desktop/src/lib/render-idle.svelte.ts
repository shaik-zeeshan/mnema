// Render-idle: true while the window's pixels can't reach a lit display —
// screens asleep (`screens_did_sleep` from NSWorkspace) or the document
// hidden (miniaturized / fully occluded). WebKit strands a non-purgeable
// IOSurface for every layer repaint in that state (Apple FB16462982,
// ~4–26 MB each, unreclaimable until the process exits), so periodic DOM
// updaters skip their tick while `renderIdle()` is true, and event-driven
// reloaders that skipped work re-run on `renderResumeTick()`.
//
// Interval-driven pollers need no resume wiring: their timer keeps firing
// and the first tick after resume passes the gate.
import { untrack } from "svelte";
import { listen } from "@tauri-apps/api/event";

import { setFramePreviewRenderGate } from "$lib/frame-preview";

const _state = $state({ screensAsleep: false, resumeTick: 0 });

export function renderIdle(): boolean {
  return (
    _state.screensAsleep ||
    (typeof document !== "undefined" && document.visibilityState !== "visible")
  );
}

/** Bumps whenever rendering becomes possible again (screens wake, document visible). */
export function renderResumeTick(): number {
  return _state.resumeTick;
}

// Callers parked in `whenRenderable`, flushed the moment rendering resumes.
const _renderableWaiters: (() => void)[] = [];

/**
 * Resolves the moment pixels can reach a lit display — immediately when they
 * already can. `untrack`ed so awaiting it inside an `$effect` (the preview
 * blob-swap effects do) does not make that effect re-run on every sleep/wake.
 */
export function whenRenderable(): Promise<void> {
  if (!untrack(renderIdle)) return Promise.resolve();
  return new Promise((resolve) => {
    _renderableWaiters.push(resolve);
  });
}

function flushRenderableWaiters(): void {
  if (_renderableWaiters.length === 0 || renderIdle()) return;
  for (const resolve of _renderableWaiters.splice(0)) resolve();
}

let _initialized = false;

export function initRenderIdle(): void {
  if (_initialized || typeof window === "undefined") return;
  _initialized = true;

  // The frame-preview holders park their blob swaps behind this gate: a URL
  // painted while nothing can render strands a non-purgeable IOSurface. Armed
  // here (not statically in frame-preview) so plain unit tests and SSR never
  // park. `null` while renderable — the swap sync-prefix contract requires no
  // added await on the open path.
  setFramePreviewRenderGate(() => (untrack(renderIdle) ? whenRenderable() : null));

  const wake = () => {
    _state.screensAsleep = false;
    _state.resumeTick += 1;
    flushRenderableWaiters();
  };
  void listen("screens_did_sleep", () => {
    _state.screensAsleep = true;
  });
  void listen("screens_did_wake", wake);
  // Backstop: `screens_did_wake` is a single NSWorkspace notification, and a
  // lost one would freeze every gated updater for the rest of the process
  // (nothing else clears `screensAsleep` — `visibilitychange` only bumps the
  // resume tick). The backend already emits `system_did_wake` from every wake
  // path it has (NSWorkspaceDidWake, the display-reconfiguration recovery, and
  // the missed-wake resync poll), and `+page.svelte` treats it as the primary
  // wake trigger — so it un-gates here too. Redundant on a normal wake.
  void listen("system_did_wake", wake);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      _state.resumeTick += 1;
      flushRenderableWaiters();
    }
  });
}
