// System-audio access state (ADR 0052).
//
// A Core Audio process tap has its own TCC category and no authorization query,
// so unlike every other permission in Settings there is nothing to read: the
// backend infers `possibly_blocked` when system audio is on and no tap has ever
// delivered a sound, and that inference is all this store surfaces. Sibling of
// `gecko-url-access.svelte.ts` in shape — probe on load, deep-link to the pane,
// non-fatal on failure.
//
// The hint is a suspicion, not a verdict (a quiet Mac looks identical to a
// denied one), so it is dismissible — and the dismissal is recorded through the
// existing one-time-prompt store so it survives a restart, exactly as the
// sensitive-app recommendation prompt does.

import { invoke } from "@tauri-apps/api/core";
import { errorText } from "./format";

/// Mirrors `SystemAudioAccessHint` in `native_capture.rs`. Whether to show is
/// the backend's call — same shape as the sensitive-app recommendation prompt,
/// which also folds "is it warranted" and "was it dismissed" server-side.
interface SystemAudioAccessHint {
  promptId: string;
  shouldShow: boolean;
}

export function createSystemAudioAccessStore() {
  let hint = $state<SystemAudioAccessHint | null>(null);
  let dismissed = $state(false);
  let error = $state<string | null>(null);

  // Non-fatal: a failed probe leaves the hint hidden rather than accusing macOS
  // of a denial we could not observe.
  async function load() {
    try {
      hint = await invoke<SystemAudioAccessHint>("get_system_audio_access_hint");
      dismissed = false;
    } catch {
      hint = null;
    }
  }

  async function openSettings() {
    error = null;
    try {
      await invoke("open_capture_privacy_settings", { kind: "systemAudio" });
    } catch (err) {
      error = errorText(err);
    }
  }

  async function dismiss() {
    const promptId = hint?.promptId;
    if (!promptId) return;
    // Hidden immediately; the record is what keeps it hidden across restarts.
    dismissed = true;
    error = null;
    try {
      await invoke("dismiss_one_time_prompt", { promptId });
    } catch (err) {
      error = errorText(err);
    }
  }

  return {
    get mayBeBlocked() {
      return (hint?.shouldShow ?? false) && !dismissed;
    },
    get error() {
      return error;
    },
    load,
    openSettings,
    dismiss,
  };
}

export type SystemAudioAccessStore = ReturnType<typeof createSystemAudioAccessStore>;
