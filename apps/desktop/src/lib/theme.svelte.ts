// Shared theme runtime. Resolves the active light/dark theme from a persisted
// `appearance` setting (`system` | `light` | `dark`) plus the OS color-scheme
// preference, and applies it globally as `data-theme` on the document root so
// any consumer can react via CSS or attribute selectors.
//
// The module is intentionally independent from `capture-controls` so it can be
// initialized as early as possible by the app shell. It does a best-effort
// fetch of the persisted recording settings; if that fails, it falls back to
// `system` and continues to track the OS preference. Later slices (settings
// UI, dashboard restyle) can call `setAppearance` to push updates without a
// round-trip.
//
// Svelte 5 runes module — depends on the `.svelte.ts` extension to enable
// `$state`/`$effect` outside components. Only the document side-effects run in
// the browser; SSR is disabled at the layout level so this module is always
// invoked client-side.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppearanceSetting, RecordingSettings } from "$lib/types";

export type ResolvedTheme = "light" | "dark";

const DEFAULT_APPEARANCE: AppearanceSetting = "system";
const DEFAULT_RESOLVED: ResolvedTheme = "dark";
const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";

const _state = $state<{
  appearance: AppearanceSetting;
  resolved: ResolvedTheme;
  loaded: boolean;
}>({
  appearance: DEFAULT_APPEARANCE,
  resolved: DEFAULT_RESOLVED,
  loaded: false,
});

let _appearanceRevision = 0;

export const theme: {
  readonly appearance: AppearanceSetting;
  readonly resolved: ResolvedTheme;
  readonly loaded: boolean;
} = {
  get appearance() { return _state.appearance; },
  get resolved() { return _state.resolved; },
  get loaded() { return _state.loaded; },
};

// ── Internal helpers ─────────────────────────────────────────────

function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return true; // safe default — keep dark chrome if we can't detect.
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function resolve(appearance: AppearanceSetting): ResolvedTheme {
  if (appearance === "light") return "light";
  if (appearance === "dark") return "dark";
  return systemPrefersDark() ? "dark" : "light";
}

function applyToDocument(resolved: ResolvedTheme): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.setAttribute("data-theme", resolved);
  // `color-scheme` lets the browser style native form controls and
  // scrollbars to match the active theme without extra CSS.
  root.style.colorScheme = resolved;
}

function recompute(): void {
  const next = resolve(_state.appearance);
  if (next !== _state.resolved) {
    _state.resolved = next;
  }
  applyToDocument(_state.resolved);
}

// ── Public API ───────────────────────────────────────────────────

/**
 * Update the persisted-mirror appearance value and re-resolve the active
 * theme. Does not write back to the settings store; callers that persist
 * should invoke the appropriate Tauri command and then call this to keep the
 * UI in sync immediately.
 */
export function setAppearance(appearance: AppearanceSetting): void {
  _appearanceRevision += 1;
  _state.appearance = appearance;
  _state.loaded = true;
  recompute();
}

export async function persistAppearance(appearance: AppearanceSetting): Promise<RecordingSettings> {
  const current = await invoke<RecordingSettings>("get_recording_settings");
  const updated = await invoke<RecordingSettings>("update_recording_settings", {
    request: {
      ...current,
      appearance,
    },
  });
  setAppearance(updated.appearance ?? DEFAULT_APPEARANCE);
  return updated;
}

/**
 * Best-effort load of the persisted appearance from recording settings.
 * Failures are swallowed; the UI keeps its current (or default `system`)
 * value so that an offline/missing backend never blocks the shell.
 */
export async function loadTheme(): Promise<void> {
  const appearanceRevisionAtLoadStart = _appearanceRevision;

  try {
    const s = await invoke<RecordingSettings>("get_recording_settings");
    if (_appearanceRevision === appearanceRevisionAtLoadStart) {
      _appearanceRevision += 1;
      _state.appearance = s.appearance ?? DEFAULT_APPEARANCE;
    }
  } catch {
    // keep current value.
  } finally {
    _state.loaded = true;
    recompute();
  }
}

let _initialized = false;
let _mediaQuery: MediaQueryList | null = null;
let _mediaListener: ((e: MediaQueryListEvent) => void) | null = null;
let _settingsListenerInitialized = false;

/**
 * Initialize the theme runtime: applies the current (default) resolution
 * immediately so the very first paint already has correct chrome, subscribes
 * to OS color-scheme changes for `system` mode, and kicks off a best-effort
 * load of the persisted appearance. Safe to call multiple times.
 */
export function initTheme(): void {
  if (_initialized) return;
  _initialized = true;

  // Apply default resolution immediately so the first paint isn't
  // mis-themed while we wait for the settings round-trip.
  recompute();

  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    _mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    _mediaListener = () => {
      // Only `system` mode needs to react — explicit light/dark are pinned.
      if (_state.appearance === "system") recompute();
    };
    // `addEventListener` is the modern API; older Safari fell back to
    // `addListener` but the Tauri webview is current enough not to need it.
    _mediaQuery.addEventListener("change", _mediaListener);
  }

  if (typeof window !== "undefined" && !_settingsListenerInitialized) {
    _settingsListenerInitialized = true;
    // Each Tauri window owns its own in-memory theme runtime, so every window
    // must subscribe to the shared settings-change event to stay in sync when
    // another window persists a new appearance value.
    void listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      setAppearance(event.payload.appearance ?? DEFAULT_APPEARANCE);
    });
  }

  void loadTheme();
}
