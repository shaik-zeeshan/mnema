// Microphone (Audio) settings store — Slice-5 shell-ification.
//
// The microphone controller is its OWN autosave domain (the `microphone`
// engine unit, command `update_microphone_controller`), separate from the
// recording-settings domains. This module owns its draft state, the live
// controller snapshot, load/save/sync/build machinery, and the autosave
// baseline — so the Audio panel is self-contained. Behavior is a 1:1 port of
// the page-local microphone code it replaces.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { MIC_AUTOSAVE_DEBOUNCE_MS } from "./autosave-core";
import type { AutosaveEngine } from "./autosave.svelte";
import type {
  MicrophoneControllerState,
  MicrophonePreferenceMode,
  MicrophoneDisconnectPolicy,
  MicrophoneAutoDisconnectTransitionFailedEvent,
} from "$lib/types";

export class AudioStore {
  // Live controller state from the backend (null while loading/failed).
  micState = $state<MicrophoneControllerState | null>(null);

  // Microphone drafts (the SEPARATE microphone autosave domain).
  draftPreferenceMode = $state<MicrophonePreferenceMode>("default");
  draftDeviceId = $state<string | null>(null);
  draftDisconnectPolicy = $state<MicrophoneDisconnectPolicy>("fallback_to_default");

  // Load/save/error/flash state.
  loadingMicState = $state(false);
  savingMicSettings = $state(false);
  micError = $state<string | null>(null);
  micSaved = $state(false);

  // Autosave baseline: the last successfully-persisted snapshot string.
  lastSavedMicSnapshot = $state<string | null>(null);

  // ─── Derived ──────────────────────────────────────────────────────────────
  // Specific-device mode requires a chosen device before it can be saved.
  micApplyBlocked = $derived(
    this.draftPreferenceMode === "specific_device" && !this.draftDeviceId,
  );

  micDeviceOptions = $derived(
    (this.micState?.devices ?? []).map((d) => ({
      value: d.id,
      label: d.name + (d.isDefault ? " (default)" : ""),
    })),
  );

  // ─── Build / snapshot / sync ────────────────────────────────────────────────
  buildMicRequest() {
    return {
      preference: {
        mode: this.draftPreferenceMode,
        deviceId: this.draftPreferenceMode === "specific_device" ? this.draftDeviceId : null,
      },
      disconnectPolicy: this.draftDisconnectPolicy,
    };
  }

  buildMicSnapshot(): string {
    return JSON.stringify(this.buildMicRequest());
  }

  syncMicDrafts(s: MicrophoneControllerState) {
    this.draftPreferenceMode = s.preference.mode;
    this.draftDeviceId = s.preference.deviceId ?? null;
    this.draftDisconnectPolicy = s.disconnectPolicy;
    this.lastSavedMicSnapshot = this.buildMicSnapshot();
  }

  // Resync drafts + baseline from a REALTIME `microphone_controller_changed`
  // event (device hotplug / auto-disconnect). Mirrors the recording store's
  // dirty-guard (`resyncRecordingDraftsFromCanonical`): if the user has an
  // unsaved pending mic edit (snapshot ≠ baseline), keep their in-flight edit
  // and only refresh the baseline; otherwise adopt the new canonical drafts.
  // The baseline ALWAYS refreshes so the next autosave diffs against the new
  // canonical truth (and a clean domain doesn't fire a redundant save).
  resyncMicDraftsFromCanonical(s: MicrophoneControllerState) {
    const dirty =
      this.lastSavedMicSnapshot !== null &&
      this.buildMicSnapshot() !== this.lastSavedMicSnapshot;
    if (!dirty) {
      this.draftPreferenceMode = s.preference.mode;
      this.draftDeviceId = s.preference.deviceId ?? null;
      this.draftDisconnectPolicy = s.disconnectPolicy;
    }
    // Refresh the baseline to the new canonical value regardless of dirtiness.
    this.lastSavedMicSnapshot = JSON.stringify({
      preference: {
        mode: s.preference.mode,
        deviceId: s.preference.mode === "specific_device" ? (s.preference.deviceId ?? null) : null,
      },
      disconnectPolicy: s.disconnectPolicy,
    });
  }

  // ─── Load / save ────────────────────────────────────────────────────────────
  async loadMicState() {
    this.loadingMicState = true;
    this.micError = null;
    try {
      const s = await invoke<MicrophoneControllerState>("get_microphone_controller_state");
      this.micState = s;
      this.syncMicDrafts(s);
    } catch (err) {
      this.micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      this.loadingMicState = false;
    }
  }

  async saveMicSettings() {
    this.savingMicSettings = true;
    this.micError = null;
    this.micSaved = false;
    try {
      const updated = await invoke<MicrophoneControllerState>("update_microphone_controller", {
        request: this.buildMicRequest(),
      });
      this.micState = updated;
      this.syncMicDrafts(updated);
      this.micSaved = true;
      setTimeout(() => { this.micSaved = false; }, 2200);
    } catch (err) {
      this.micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      this.savingMicSettings = false;
    }
  }

  // ─── Autosave registration ──────────────────────────────────────────────────
  registerAutosave(engine: AutosaveEngine) {
    engine.register({
      key: "microphone",
      debounceMs: MIC_AUTOSAVE_DEBOUNCE_MS,
      snapshot: () => this.buildMicSnapshot(),
      baseline: () => this.lastSavedMicSnapshot,
      blocked: () => this.micApplyBlocked,
      saving: () => this.savingMicSettings,
      save: () => this.saveMicSettings(),
    });
  }

  // ─── Realtime listeners ─────────────────────────────────────────────────────
  // Returns an unlisten function. Mirrors the page's two mic listeners.
  startListeners(): () => void {
    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let destroyed = false;

    listen<MicrophoneControllerState>("microphone_controller_changed", (event) => {
      this.micState = event.payload;
      // Dirty-guard: a hotplug/auto-disconnect echo must NOT clobber an unsaved
      // pending mic edit (nor cancel its in-flight autosave by resetting the
      // baseline to match). Initial load keeps its unconditional `syncMicDrafts`.
      this.resyncMicDraftsFromCanonical(event.payload);
      this.micError = null;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenControllerChanged = fn;
    });

    listen<MicrophoneAutoDisconnectTransitionFailedEvent>(
      "microphone_auto_disconnect_transition_failed",
      (event) => {
        const { context, code, message } = event.payload;
        this.micError = `[${context}] [${code}] ${message}`;
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenAutoDisconnectFailure = fn;
    });

    return () => {
      destroyed = true;
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
    };
  }
}

export function createAudioStore(): AudioStore {
  return new AudioStore();
}
