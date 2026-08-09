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
import { listen } from "@tauri-apps/api/event";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { availableMonitors, getCurrentWindow } from "@tauri-apps/api/window";

import { clampTarget } from "$lib/render-idle-clamp";

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

let _initialized = false;

export function initRenderIdle(options?: { clampWindow?: boolean }): void {
  if (_initialized || typeof window === "undefined") return;
  _initialized = true;

  const wake = () => {
    _state.screensAsleep = false;
    _state.resumeTick += 1;
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
    if (document.visibilityState === "visible") _state.resumeTick += 1;
  });

  // A display disconnect can park the window (almost) entirely outside every
  // remaining screen — macOS keeps reporting it visible with a sliver
  // on-screen, so it never occludes, never goes `hidden`, and every repaint
  // strands. Clamp it back onto a real monitor. Debounced: one reconfiguration
  // fires the CG callback several times.
  if (options?.clampWindow) {
    let clampTimer: ReturnType<typeof setTimeout> | null = null;
    void listen("display_configuration_changed", () => {
      if (clampTimer != null) clearTimeout(clampTimer);
      clampTimer = setTimeout(() => {
        clampTimer = null;
        void clampWindowOntoScreen();
      }, 500);
    });
  }
}

async function clampWindowOntoScreen(): Promise<void> {
  try {
    const win = getCurrentWindow();
    const [monitors, pos, size] = await Promise.all([
      availableMonitors(),
      win.outerPosition(),
      win.outerSize(),
    ]);
    // The geometry lives in `$lib/render-idle-clamp` so it can be tested without a
    // window: signed monitor origins, oversized windows, and the overlap threshold
    // are all easy to get subtly wrong and impossible to assert from here.
    const target = clampTarget(monitors, pos, size);
    if (target === null) return;
    await win.setPosition(new PhysicalPosition(target.x, target.y));
  } catch {
    // Best-effort: a failed clamp just leaves the window where it was.
  }
}
