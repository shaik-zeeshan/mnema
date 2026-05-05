// Shared developer-options state — drives whether the Debug page and its
// nav entry are exposed in the UI. Backed by the persisted recording-settings
// `developerOptionsEnabled` field; the layout loads it once on mount and the
// settings page updates the in-memory value after a successful save so nav
// visibility and redirects react immediately without a round-trip.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { RecordingSettings } from "$lib/types";

const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
let _settingsSyncInitialized = false;

const _state = $state<{ value: boolean; loaded: boolean }>({
  value: false,
  loaded: false,
});

export const developerOptions: {
  readonly value: boolean;
  readonly loaded: boolean;
} = {
  get value() { return _state.value; },
  get loaded() { return _state.loaded; },
};

export function setDeveloperOptionsEnabled(enabled: boolean): void {
  _state.value = enabled;
  _state.loaded = true;
}

function initDeveloperOptionsSync(): void {
  if (_settingsSyncInitialized || typeof window === "undefined") return;
  _settingsSyncInitialized = true;

  void listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
    setDeveloperOptionsEnabled(event.payload.developerOptionsEnabled ?? false);
  });
}

/**
 * Best-effort load of the persisted developer-options flag. Failures are
 * swallowed and the flag stays at its current (or default `false`) value —
 * the gating is fail-safe: hidden Debug surfaces are the secure default.
 */
export async function loadDeveloperOptions(): Promise<void> {
  initDeveloperOptionsSync();
  try {
    const s = await invoke<RecordingSettings>("get_recording_settings");
    _state.value = s.developerOptionsEnabled ?? false;
  } catch {
    // keep current value; mark loaded so the UI doesn't wait forever.
  } finally {
    _state.loaded = true;
  }
}
