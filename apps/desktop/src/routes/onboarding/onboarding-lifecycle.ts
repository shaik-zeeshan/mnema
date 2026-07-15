// Onboarding load / save / finish lifecycle. Lifted 1:1 out of
// `OnboardingController` (only `this.x` → `target.x`) to keep that file under the
// size budget; the controller methods now delegate here. Behavior is identical:
// `loadOnboarding` hydrates settings + permissions and forces optional features
// off, `finishOnboarding` commits the whole config in one shot and (optionally)
// starts capture, and the internal `saveSettings` round-trips the atomic
// full-settings command.
import { goto } from "$app/navigation";
import { invoke } from "@tauri-apps/api/core";
import type {
  GetPermissionsResponse,
  RecordingSettings,
} from "$lib/types";
import { serializeError } from "./onboarding-mapping";
import type { OnboardingAiStore } from "./onboarding-ai.svelte";
import type { PermissionKey, PermissionValue } from "./onboarding-attention";

type OnboardingState = {
  schemaVersion: number;
  completedAtUnixMs: number | null;
  // True once the user has explicitly saved recording settings at least once
  // (the recording-settings.json file exists). Distinguishes a GENUINE first run
  // from a returning user re-opening onboarding. Hand-mirrored from the Rust
  // `OnboardingStateView` (serde camelCase) — keep the field name in sync.
  recordingSettingsEverSaved: boolean;
};

// Just enough of the privacy controller for `loadOnboarding` to kick off its
// candidate/recommendation loads. Mirrors the controller's `appPrivacyExclusion`.
interface PrivacyExclusionLoaders {
  loadPrivacyAppCandidates(): unknown;
  loadSensitiveCaptureRecommendations(): unknown;
}

// The slice of `OnboardingController` the lifecycle drives. The controller
// satisfies this structurally (it owns every field/method below), so passing
// `this` keeps load/save/finish operating on the live state.
export interface OnboardingLifecycleTarget {
  loading: boolean;
  saving: boolean;
  completing: boolean;
  starting: boolean;
  errorMessage: string | null;
  settings: RecordingSettings | null;
  permissions: Record<PermissionKey, PermissionValue> | null;
  draftCaptureScreen: boolean;
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  readonly canComplete: boolean;
  readonly canSkipToDashboard: boolean;
  readonly ai: OnboardingAiStore;
  readonly appPrivacyExclusion: PrivacyExclusionLoaders;
  syncDrafts(next: RecordingSettings): void;
  buildSettingsRequest(): RecordingSettings;
  resetOptionalFeaturesOff(): void;
  loadGeckoUrlAccess(): Promise<void>;
}

export async function loadOnboarding(target: OnboardingLifecycleTarget): Promise<void> {
  target.loading = true;
  target.errorMessage = null;
  try {
    const state = await invoke<OnboardingState>("get_onboarding_state");
    if (state.completedAtUnixMs !== null) {
      await goto("/", { replaceState: true });
      return;
    }
    const [loadedSettings, permissionResponse] = await Promise.all([
      invoke<RecordingSettings>("get_recording_settings"),
      invoke<GetPermissionsResponse>("get_capture_permissions"),
    ]);
    target.settings = loadedSettings;
    target.permissions = permissionResponse.permissions as Record<PermissionKey, PermissionValue>;
    target.syncDrafts(loadedSettings);
    // Force every OPTIONAL feature OFF for a GENUINE first run only. `syncDrafts`
    // is a verbatim settings round-trip (and the default RecordingSettings ships
    // OCR/transcription enabled), so on a true first run we force the optional
    // toggles off so the user opts in per-row. A RETURNING user (one who has
    // explicitly saved settings before — `recordingSettingsEverSaved`) keeps the
    // real persisted enable toggles that syncDrafts already seeded, so re-opening
    // onboarding reflects/preserves their saved configuration rather than
    // silently disabling enabled features. Required features (screen, storage,
    // permissions) are untouched either way.
    if (!state.recordingSettingsEverSaved) {
      target.resetOptionalFeaturesOff();
    }
    target.ai.init();
    void target.appPrivacyExclusion.loadPrivacyAppCandidates();
    void target.appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    // Optional browser-URL access probe: non-fatal and self-contained (it swallows
    // its own errors), so fire-and-forget like the privacy loaders above — a
    // failure just leaves the optional Gecko row hidden.
    void target.loadGeckoUrlAccess();
  } catch (err) {
    target.errorMessage = serializeError(err);
  } finally {
    target.loading = false;
  }
}

async function saveSettings(target: OnboardingLifecycleTarget): Promise<void> {
  target.saving = true;
  target.errorMessage = null;
  try {
    // Onboarding commits the whole recording config in one shot. The
    // domain-scoped commands exist for the Settings page's per-domain
    // debounced autosave; here we deliberately use the atomic full-settings
    // command so a late validation failure can't leave a partially-persisted
    // configuration behind.
    const updated = await invoke<RecordingSettings>("update_recording_settings", {
      request: target.buildSettingsRequest(),
    });
    target.settings = updated;
    target.syncDrafts(updated);
  } catch (err) {
    target.errorMessage = serializeError(err);
    throw err;
  } finally {
    target.saving = false;
  }
}

export async function finishOnboarding(
  target: OnboardingLifecycleTarget,
  startRecording: boolean,
): Promise<void> {
  // "Start recording" requires full model readiness (`canComplete`); the
  // "Just open the dashboard" escape hatch only requires a serializable config
  // (`canSkipToDashboard`) so a mid-download / un-ready model never traps the
  // user in onboarding. The skip still commits the settings (with that feature
  // enabled — its download simply continues in the background) and marks
  // onboarding complete, it just doesn't start capture.
  const ready = startRecording ? target.canComplete : target.canSkipToDashboard;
  if (target.settings === null || !ready) return;
  target.completing = true;
  target.starting = startRecording;
  target.errorMessage = null;
  try {
    await saveSettings(target);
    // Persist the completion flag BEFORE the side-effecting capture start.
    // `start_native_capture` and `complete_onboarding` are independent, so if
    // we started capture first and `complete_onboarding` (or the goto) then
    // threw, capture would be live while onboarding stayed incomplete —
    // re-showing onboarding next launch with capture already running.
    await invoke("complete_onboarding");
    if (startRecording) {
      // Defense-in-depth: never request a source whose OS permission isn't
      // granted, independent of the attention gate. Capture must not outrun
      // authorization even if the gating logic ever changes. System audio is
      // exempt and takes its draft flag straight through: its grant is
      // unreadable (ADR 0052), the prompt fires when the tap is first read, and
      // it has no screen dependency — gating it here would never let it start.
      await invoke("start_native_capture", {
        request: {
          captureScreen: target.draftCaptureScreen,
          captureMicrophone: target.draftCaptureMicrophone && target.permissions?.microphone === "granted",
          captureSystemAudio: target.draftCaptureSystemAudio,
        },
      });
    }
    await goto("/");
  } catch (err) {
    target.errorMessage = serializeError(err);
    target.completing = false;
    target.starting = false;
  }
}
