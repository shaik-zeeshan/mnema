import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { captureSession, setSession } from "$lib/session.svelte";
import { toast } from "$lib/toast.svelte";
import { humanizeError } from "$lib/format-error";
import type {
  CaptureSession,
  GetPermissionsResponse,
  PermissionsMap,
  RecordingSettings,
  RecordingSettingsDomainUpdateResponse,
  SourceSessions,
} from "$lib/types";
import type {
  IdleDebugInfo,
  RuntimeSourcesStatus,
} from "$lib/types/inactivity";

const _state = $state<{
  recordingSettings: RecordingSettings | null;
  loadingStart: boolean;
  loadingStop: boolean;
  loadingPause: boolean;
  loadingSettings: boolean;
  bootstrapped: boolean;
  sessionGeneration: number;
  runtimeSources: RuntimeSourcesStatus | null;
  idleMs: number;
  permissions: PermissionsMap | null;
}>({
  recordingSettings: null,
  loadingStart: false,
  loadingStop: false,
  loadingPause: false,
  loadingSettings: false,
  bootstrapped: false,
  sessionGeneration: 0,
  runtimeSources: null,
  idleMs: 0,
  permissions: null,
});

const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
const NATIVE_CAPTURE_SESSION_CHANGED_EVENT = "native_capture_session_changed";
let _settingsSyncInitialized = false;

// Lifecycle failures (start/stop/pause/resume, source-toggle, bootstrap) raise
// the app-wide error toast instead of a modal alert: the user needs to see the
// failure, not be blocked by it. One id, so a retry loop replaces its own row.
function reportCaptureError(err: unknown): void {
  toast({
    id: "capture-error",
    tone: "error",
    title: "Recording error",
    message: humanizeError(err),
  });
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
  get loadingPause(): boolean {
    return _state.loadingPause;
  },
  get loadingSettings(): boolean {
    return _state.loadingSettings;
  },
  get bootstrapped(): boolean {
    return _state.bootstrapped;
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
    return (
      captureSession.value?.isInactivityPaused === true ||
      captureSession.value?.isUserPaused === true ||
      captureSession.value?.isLowDiskSuspended === true
    );
  },
  get isRunning(): boolean {
    return captureSession.value?.isRunning === true;
  },
  get isInactivityPaused(): boolean {
    return captureSession.value?.isInactivityPaused === true;
  },
  get isUserPaused(): boolean {
    return captureSession.value?.isUserPaused === true;
  },
  get isLowDiskSuspended(): boolean {
    return captureSession.value?.isLowDiskSuspended === true;
  },
  get followTimelineLive(): boolean {
    return _state.recordingSettings?.followTimelineLive === true;
  },
  get runtimeSources(): RuntimeSourcesStatus | null {
    return _state.runtimeSources;
  },
  /** Per-source start stamps — the state pill's elapsed clock reads the earliest. */
  get sourceSessions(): SourceSessions | null {
    return captureSession.value?.sourceSessions ?? null;
  },
  /** How long the activity detector has seen nothing; drives "Idle 12m". */
  get idleMs(): number {
    return _state.idleMs;
  },
  /**
   * Last-read TCC answers. Only hard `denied`/`restricted` are facts — system
   * audio's `possibly_blocked` is an inference with no API behind it (ADR 0052).
   */
  get permissions(): PermissionsMap | null {
    return _state.permissions;
  },
};

export async function bootstrapCaptureControls(): Promise<void> {
  initRecordingSettingsSync();
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
    _state.permissions = perm.permissions;
    _state.recordingSettings = settings;
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _state.loadingSettings = false;
    _state.bootstrapped = true;
  }
}

function initRecordingSettingsSync(): void {
  if (_settingsSyncInitialized || typeof window === "undefined") return;
  _settingsSyncInitialized = true;

  void listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
    _state.recordingSettings = event.payload;
  });

  void listen<CaptureSession>(NATIVE_CAPTURE_SESSION_CHANGED_EVENT, (event) => {
    applyCaptureSession(event.payload);
  });
}

function applyCaptureSession(session: CaptureSession): void {
  const wasRunning = captureControls.isRunning;
  setSession(session);
  if (wasRunning !== session.isRunning) {
    _state.sessionGeneration += 1;
  }
  _state.loadingStart = false;
  _state.loadingStop = false;
  _state.loadingPause = false;
  if (session.isRunning) {
    void refreshRuntimeSources();
  } else {
    _state.runtimeSources = null;
    _state.idleMs = 0;
  }
}

export async function pauseCapture(): Promise<void> {
  if (
    _state.loadingStart ||
    _state.loadingStop ||
    _state.loadingPause ||
    !captureControls.isRunning ||
    captureControls.isUserPaused
  ) return;
  _state.loadingPause = true;
  try {
    const result = await invoke<{ session: CaptureSession }>("pause_native_capture");
    applyCaptureSession(result.session);
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _state.loadingPause = false;
  }
}

export async function resumeCapture(): Promise<void> {
  if (
    _state.loadingStart ||
    _state.loadingStop ||
    _state.loadingPause ||
    !captureControls.isRunning ||
    !captureControls.isUserPaused
  ) return;
  _state.loadingPause = true;
  try {
    const result = await invoke<{ session: CaptureSession }>("resume_native_capture");
    applyCaptureSession(result.session);
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _state.loadingPause = false;
  }
}

export async function startCapture(): Promise<void> {
  if (_state.loadingStart || captureControls.isRunning) return;
  _state.loadingStart = true;
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
    applyCaptureSession(result.session);
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _state.loadingStart = false;
  }
}

export async function stopCapture(): Promise<void> {
  if (_state.loadingStop || !captureControls.isRunning) return;
  _state.loadingStop = true;
  try {
    const result = await invoke<{ session: CaptureSession }>("stop_native_capture");
    applyCaptureSession(result.session);
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _state.loadingStop = false;
  }
}

export async function resyncCaptureSession(): Promise<void> {
  const gen = _state.sessionGeneration;
  try {
    const result = await invoke<GetPermissionsResponse>("get_capture_permissions");
    if (_state.sessionGeneration !== gen) return;
    _state.permissions = result.permissions;
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
    _state.idleMs = 0;
    return;
  }
  try {
    const info = await invoke<IdleDebugInfo>("get_idle_debug");
    if (!captureControls.isRunning) {
      _state.runtimeSources = null;
      _state.idleMs = 0;
      return;
    }
    _state.runtimeSources = info.runtimeSources;
    _state.idleMs = info.effectiveIdleMs;
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
      _state.idleMs = 0;
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

// ── Per-source selection ──────────────────────────────────────────────
// One toggle, two meanings, decided by whether a session is running:
//
// - **idle**: flips `captureScreen / captureMicrophone / captureSystemAudio`
//   in the capture-sources settings domain — the *next* session's sources.
// - **recording**: flips the live user mask through the lifecycle's paused-flag
//   seam — this session only, and only within the sources it was started with.
//   Nothing but the user clears a mask; liveness recovery leaves it alone.
//
// All three doors (this store, the tray, the 1/2/3 shortcuts) go through here
// or through the same command, so they cannot disagree.
export type SourceKey = "screen" | "microphone" | "systemAudio";

const _selectionState = $state<{
  saving: Record<SourceKey, boolean>;
}>({
  saving: { screen: false, microphone: false, systemAudio: false },
});

function settingsSelected(key: SourceKey): boolean {
  if (key === "screen") return _state.recordingSettings?.captureScreen ?? true;
  if (key === "microphone") return _state.recordingSettings?.captureMicrophone ?? false;
  return _state.recordingSettings?.captureSystemAudio ?? false;
}

export const sourceSelection = {
  get screen(): boolean {
    return sourceSelection.isSelected("screen");
  },
  get microphone(): boolean {
    return sourceSelection.isSelected("microphone");
  },
  get systemAudio(): boolean {
    return sourceSelection.isSelected("systemAudio");
  },
  isSaving(key: SourceKey): boolean {
    return _selectionState.saving[key];
  },
  /**
   * Whether this session started with the source at all. A source it never
   * requested cannot be added mid-session — the mask only works inside
   * `requestedSources` — so its toggle stays disabled until the next start.
   */
  isInSession(key: SourceKey): boolean {
    return captureSession.value?.requestedSources?.[key] === true;
  },
  /** Turned off by the user for this session (not idle, not suspended). */
  isMasked(key: SourceKey): boolean {
    if (!captureControls.isRunning) return false;
    return captureSession.value?.maskedSources?.[key] === true;
  },
  /** What the toggle reads: live sources while recording, settings while idle. */
  isSelected(key: SourceKey): boolean {
    if (!captureControls.isRunning) return settingsSelected(key);
    return sourceSelection.isInSession(key) && !sourceSelection.isMasked(key);
  },
};

/**
 * Turn a source on or off.
 *
 * While recording this is the live user mask: the desired live set is sent to
 * the lifecycle, which pauses or resumes that one source (finalizing its
 * in-flight segment the way an idle pause does). The backend refuses a change
 * that would leave nothing recording — stop the session for that.
 *
 * While idle it persists the choice through the capture-sources domain command,
 * so unrelated settings drafts cannot be overwritten by a title-bar change.
 */
export async function setSourceSelected(
  key: SourceKey,
  selected: boolean,
): Promise<void> {
  if (_selectionState.saving[key]) return;

  if (captureControls.isRunning) {
    if (!sourceSelection.isInSession(key)) return;
    const sources = {
      screen: key === "screen" ? selected : sourceSelection.isSelected("screen"),
      microphone: key === "microphone" ? selected : sourceSelection.isSelected("microphone"),
      systemAudio: key === "systemAudio" ? selected : sourceSelection.isSelected("systemAudio"),
    };
    _selectionState.saving[key] = true;
    try {
      const result = await invoke<{ session: CaptureSession }>(
        "set_native_capture_live_sources",
        { sources },
      );
      applyCaptureSession(result.session);
      await refreshRuntimeSourcesNow();
    } catch (err) {
      reportCaptureError(err);
    } finally {
      _selectionState.saving[key] = false;
    }
    return;
  }

  const base = _state.recordingSettings;
  if (!base) return;

  const overrides:
    Partial<Pick<RecordingSettings, "captureScreen" | "captureMicrophone" | "captureSystemAudio">> =
    key === "screen"
      ? { captureScreen: selected }
      : key === "microphone"
        ? { captureMicrophone: selected }
        : { captureSystemAudio: selected };

  _selectionState.saving[key] = true;
  try {
    const updated = await invoke<RecordingSettingsDomainUpdateResponse>("update_capture_source_settings", {
      request: overrides,
    });
    _state.recordingSettings = updated.settings;
  } catch (err) {
    reportCaptureError(err);
  } finally {
    _selectionState.saving[key] = false;
  }
}

export async function toggleSourceSelected(key: SourceKey): Promise<void> {
  await setSourceSelected(key, !sourceSelection.isSelected(key));
}
