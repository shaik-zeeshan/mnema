import { invoke } from "@tauri-apps/api/core";
import { captureSession, setSession } from "$lib/session.svelte";
import type {
  CaptureSession,
  GetPermissionsResponse,
  RecordingSettings,
} from "$lib/types";
import type {
  IdleDebugInfo,
  RuntimeSourcesStatus,
} from "$lib/types/inactivity";

const _state = $state<{
  recordingSettings: RecordingSettings | null;
  loadingStart: boolean;
  loadingStop: boolean;
  loadingSettings: boolean;
  bootstrapped: boolean;
  error: string | null;
  sessionGeneration: number;
  runtimeSources: RuntimeSourcesStatus | null;
}>({
  recordingSettings: null,
  loadingStart: false,
  loadingStop: false,
  loadingSettings: false,
  bootstrapped: false,
  error: null,
  sessionGeneration: 0,
  runtimeSources: null,
});

function serializeError(err: unknown): string {
  return typeof err === "string" ? err : (JSON.stringify(err) ?? "Unknown error");
}

export const captureControls = {
  get recordingSettings(): RecordingSettings | null {
    return _state.recordingSettings;
  },
  get loadingStart(): boolean {
    return _state.loadingStart;
  },
  get loadingStop(): boolean {
    return _state.loadingStop;
  },
  get loadingSettings(): boolean {
    return _state.loadingSettings;
  },
  get bootstrapped(): boolean {
    return _state.bootstrapped;
  },
  get error(): string | null {
    return _state.error;
  },
  get sessionGeneration(): number {
    return _state.sessionGeneration;
  },
  get isCapturing(): boolean {
    return captureSession.value?.isRunning === true;
  },
  get running(): boolean {
    return captureSession.value?.isRunning === true;
  },
  get paused(): boolean {
    return captureSession.value?.isInactivityPaused === true;
  },
  get isRunning(): boolean {
    return captureSession.value?.isRunning === true;
  },
  get isInactivityPaused(): boolean {
    return captureSession.value?.isInactivityPaused === true;
  },
  get statusLabel(): string {
    if (captureControls.isRunning) {
      return captureControls.isInactivityPaused ? "Paused" : "Recording";
    }
    return captureSession.value?.isRunning === false ? "Stopped" : "Idle";
  },
  get statusModifier(): "idle" | "running" | "paused" {
    if (captureControls.isRunning) {
      return captureControls.isInactivityPaused ? "paused" : "running";
    }
    return "idle";
  },
  get followTimelineLive(): boolean {
    return _state.recordingSettings?.followTimelineLive === true;
  },
  get runtimeSources(): RuntimeSourcesStatus | null {
    return _state.runtimeSources;
  },
};

export async function bootstrapCaptureControls(): Promise<void> {
  _state.loadingSettings = true;
  const gen = _state.sessionGeneration;
  try {
    const [perm, settings] = await Promise.all([
      invoke<GetPermissionsResponse>("get_capture_permissions"),
      invoke<RecordingSettings>("get_recording_settings"),
    ]);
    if (perm.session && _state.sessionGeneration === gen) {
      setSession(perm.session);
    }
    _state.recordingSettings = settings;
    _state.error = null;
  } catch (err) {
    _state.error = serializeError(err);
  } finally {
    _state.loadingSettings = false;
    _state.bootstrapped = true;
  }
}

export async function startCapture(): Promise<void> {
  if (_state.loadingStart || captureControls.isRunning) return;
  _state.loadingStart = true;
  _state.error = null;
  try {
    const result = await invoke<{ session: CaptureSession }>(
      "start_native_capture",
      {
        request: {
          captureScreen: _state.recordingSettings?.captureScreen ?? true,
          captureMicrophone: _state.recordingSettings?.captureMicrophone ?? false,
          captureSystemAudio: _state.recordingSettings?.captureSystemAudio ?? false,
        },
      },
    );
    _state.sessionGeneration += 1;
    setSession(result.session);
    void refreshRuntimeSources();
  } catch (err) {
    _state.error = serializeError(err);
  } finally {
    _state.loadingStart = false;
  }
}

export async function stopCapture(): Promise<void> {
  if (_state.loadingStop || !captureControls.isRunning) return;
  _state.loadingStop = true;
  _state.error = null;
  try {
    const result = await invoke<{ session: CaptureSession }>("stop_native_capture");
    _state.sessionGeneration += 1;
    setSession(result.session);
    _state.runtimeSources = null;
  } catch (err) {
    _state.error = serializeError(err);
  } finally {
    _state.loadingStop = false;
  }
}

export async function resyncCaptureSession(): Promise<void> {
  const gen = _state.sessionGeneration;
  try {
    const result = await invoke<GetPermissionsResponse>("get_capture_permissions");
    if (_state.sessionGeneration !== gen) return;
    if (result.session) setSession(result.session);
  } catch {
    // Best-effort refresh only.
  }
}

// ── Per-source runtime indicator polling ──────────────────────────────
// The title-bar per-source recording indicator (screen / microphone /
// system audio) reads `runtimeSources` from `get_idle_debug`. We poll
// only while a session is running; when stopped we clear the snapshot
// so the indicator doesn't render stale state on next start.
const RUNTIME_POLL_INTERVAL_MS = 2000;
let _runtimePollHandle: ReturnType<typeof setInterval> | null = null;
let _runtimeRefCount = 0;

async function refreshRuntimeSources(): Promise<void> {
  if (!captureControls.isRunning) {
    _state.runtimeSources = null;
    return;
  }
  try {
    const info = await invoke<IdleDebugInfo>("get_idle_debug");
    if (!captureControls.isRunning) {
      _state.runtimeSources = null;
      return;
    }
    _state.runtimeSources = info.runtimeSources;
  } catch {
    // Best-effort; keep last snapshot.
  }
}

/**
 * Begin polling per-source runtime status while a capture session is active.
 * Reference-counted so multiple consumers (layout, dashboard) can subscribe
 * without stomping each other's lifecycle. Returns a stop fn.
 */
export function subscribeRuntimeSources(): () => void {
  _runtimeRefCount += 1;
  if (_runtimePollHandle === null) {
    void refreshRuntimeSources();
    _runtimePollHandle = setInterval(refreshRuntimeSources, RUNTIME_POLL_INTERVAL_MS);
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    _runtimeRefCount = Math.max(0, _runtimeRefCount - 1);
    if (_runtimeRefCount === 0 && _runtimePollHandle !== null) {
      clearInterval(_runtimePollHandle);
      _runtimePollHandle = null;
      _state.runtimeSources = null;
    }
  };
}

/**
 * Force an immediate runtime-sources refresh. Useful right after start/stop
 * so the indicator updates without waiting a poll tick.
 */
export async function refreshRuntimeSourcesNow(): Promise<void> {
  await refreshRuntimeSources();
}

// ── Per-source selection (used by the title-bar source pills) ─────────
// While not recording, each source pill acts as a toggle that flips the
// corresponding `captureScreen / captureMicrophone / captureSystemAudio`
// field in `RecordingSettings`, exactly mirroring the settings page. The
// flag is persisted via `update_recording_settings` so the next session
// (and the settings UI itself) picks up the same choice.
export type SourceKey = "screen" | "microphone" | "systemAudio";

const _selectionState = $state<{
  saving: Record<SourceKey, boolean>;
}>({
  saving: { screen: false, microphone: false, systemAudio: false },
});

export const sourceSelection = {
  get screen(): boolean {
    return _state.recordingSettings?.captureScreen ?? true;
  },
  get microphone(): boolean {
    return _state.recordingSettings?.captureMicrophone ?? false;
  },
  get systemAudio(): boolean {
    return _state.recordingSettings?.captureSystemAudio ?? false;
  },
  isSaving(key: SourceKey): boolean {
    return _selectionState.saving[key];
  },
  isSelected(key: SourceKey): boolean {
    if (key === "screen") return sourceSelection.screen;
    if (key === "microphone") return sourceSelection.microphone;
    return sourceSelection.systemAudio;
  },
};

/**
 * Build an `UpdateRecordingSettingsRequest` payload from the cached
 * `RecordingSettings` snapshot. Both shapes share the same camelCase field
 * names by construction, so this is a faithful pass-through that only the
 * caller-supplied overrides differ from.
 */
function buildUpdatePayload(
  base: RecordingSettings,
  overrides: Partial<Pick<RecordingSettings, "captureScreen" | "captureMicrophone" | "captureSystemAudio">>,
): Record<string, unknown> {
  return {
    captureScreen: overrides.captureScreen ?? base.captureScreen,
    captureMicrophone: overrides.captureMicrophone ?? base.captureMicrophone,
    captureSystemAudio: overrides.captureSystemAudio ?? base.captureSystemAudio,
    segmentDurationSeconds: base.segmentDurationSeconds,
    screenFrameRate: base.screenFrameRate,
    saveDirectory: base.saveDirectory,
    autoStart: base.autoStart,
    pauseCaptureOnInactivity: base.pauseCaptureOnInactivity,
    idleTimeoutSeconds: base.idleTimeoutSeconds,
    activityMode: base.activityMode,
    microphoneActivitySensitivity: base.microphoneActivitySensitivity,
    systemAudioActivitySensitivity: base.systemAudioActivitySensitivity,
    nativeCaptureDebugLoggingEnabled: base.nativeCaptureDebugLoggingEnabled,
    previewCacheTtlSeconds: base.previewCacheTtlSeconds,
    followTimelineLive: base.followTimelineLive,
    appearance: base.appearance,
    developerOptionsEnabled: base.developerOptionsEnabled,
    ocr: base.ocr,
    screenResolution: base.screenResolution,
    videoBitrate: base.videoBitrate,
  };
}

/**
 * Toggle (or set) whether a given source will be captured the next time
 * `startCapture()` runs. Persists the choice through the same
 * `update_recording_settings` Tauri command the settings page uses, so the
 * title-bar pill and the settings page stay perfectly in sync.
 *
 * No-op while a session is currently running — the live `runtimeSources`
 * snapshot determines the indicator state in that case, and source
 * selection only affects the next session.
 */
export async function setSourceSelected(
  key: SourceKey,
  selected: boolean,
): Promise<void> {
  if (captureControls.isRunning) return;
  const base = _state.recordingSettings;
  if (!base) return;
  if (_selectionState.saving[key]) return;

  const overrides:
    Partial<Pick<RecordingSettings, "captureScreen" | "captureMicrophone" | "captureSystemAudio">> =
    key === "screen"
      ? { captureScreen: selected }
      : key === "microphone"
        ? { captureMicrophone: selected }
        : { captureSystemAudio: selected };

  _selectionState.saving[key] = true;
  try {
    const updated = await invoke<RecordingSettings>("update_recording_settings", {
      request: buildUpdatePayload(base, overrides),
    });
    _state.recordingSettings = updated;
    _state.error = null;
  } catch (err) {
    _state.error = serializeError(err);
  } finally {
    _selectionState.saving[key] = false;
  }
}

export async function toggleSourceSelected(key: SourceKey): Promise<void> {
  await setSourceSelected(key, !sourceSelection.isSelected(key));
}
