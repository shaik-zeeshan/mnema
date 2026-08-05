// "Open Mnema on" — which main surface a cold-started Main window lands on.
//
// This is a shell preference: nothing in Rust reads it, and it is needed
// synchronously during the first render (an async settings round-trip would
// flash Timeline first). So it lives in webview storage next to the other shell
// prefs (the Insights rail's collapsed/width keys) rather than paying a
// recording-settings schema change across capture-types + the Tauri layer for a
// value the backend never uses.
// ponytail: localStorage; move it into RecordingSettings only if it ever has to
// be readable outside the Main webview (tray menu, CLI, another window).

export type StartupSurface = "timeline" | "overview";

const STORAGE_KEY = "mnema.startupSurface";

/** The persisted preference, defaulting to Timeline. */
export function getStartupSurface(): StartupSurface {
  try {
    return localStorage.getItem(STORAGE_KEY) === "overview" ? "overview" : "timeline";
  } catch {
    // Storage can be unavailable (private mode / blocked); Timeline is the default.
    return "timeline";
  }
}

export function setStartupSurface(value: StartupSurface): void {
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    // Best-effort: a blocked write just means the app keeps opening on Timeline.
  }
}
