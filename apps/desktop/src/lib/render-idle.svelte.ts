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

  void listen("screens_did_sleep", () => {
    _state.screensAsleep = true;
  });
  void listen("screens_did_wake", () => {
    _state.screensAsleep = false;
    _state.resumeTick += 1;
  });
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

// Physical pixels; window and monitor rects share the same global space.
const MIN_VISIBLE_PX = 100;

async function clampWindowOntoScreen(): Promise<void> {
  try {
    const win = getCurrentWindow();
    const [monitors, pos, size] = await Promise.all([
      availableMonitors(),
      win.outerPosition(),
      win.outerSize(),
    ]);
    if (monitors.length === 0) return;
    const meaningfullyVisible = monitors.some((m) => {
      const overlapW =
        Math.min(pos.x + size.width, m.position.x + m.size.width) -
        Math.max(pos.x, m.position.x);
      const overlapH =
        Math.min(pos.y + size.height, m.position.y + m.size.height) -
        Math.max(pos.y, m.position.y);
      return overlapW >= MIN_VISIBLE_PX && overlapH >= MIN_VISIBLE_PX;
    });
    if (meaningfullyVisible) return;
    const m = monitors[0];
    await win.setPosition(
      new PhysicalPosition(
        m.position.x + Math.max(0, Math.round((m.size.width - size.width) / 2)),
        m.position.y + Math.max(0, Math.round((m.size.height - size.height) / 2)),
      ),
    );
  } catch {
    // Best-effort: a failed clamp just leaves the window where it was.
  }
}
