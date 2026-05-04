import { invoke } from "@tauri-apps/api/core";
import { captureSession, setSession } from "$lib/session.svelte";
import type {
  CaptureSession,
  GetPermissionsResponse,
  RecordingSettings,
} from "$lib/types";

const _state = $state<{
  recordingSettings: RecordingSettings | null;
  loadingStart: boolean;
  loadingStop: boolean;
  loadingSettings: boolean;
  bootstrapped: boolean;
  error: string | null;
  sessionGeneration: number;
}>({
  recordingSettings: null,
  loadingStart: false,
  loadingStop: false,
  loadingSettings: false,
  bootstrapped: false,
  error: null,
  sessionGeneration: 0,
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
