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
import { buildStartCaptureRequest } from "./onboarding-start-request";

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

/**
 * Enrolling a voiceprint on the Voice screen turns `recognize_saved_people` ON
 * backend-side (`enroll_account_owner_voice`) — without it the voiceprint is
 * loaded by nothing. This save is authoritative and is rebuilt from drafts that
 * were seeded BEFORE the enrollment, so it would write the stale `false` straight
 * back over the flip.
 *
 * Read here rather than mirrored into a draft on the Voice screen because
 * `finish()` re-derives the speaker drafts from `flow.features` anyway, and this
 * is the one funnel every onboarding save goes through — the Settings page's
 * enrollment door mirrors into its own draft for the same reason.
 */
async function withEnrolledVoiceRecognition(
  request: RecordingSettings,
): Promise<RecordingSettings> {
  if (request.speakerAnalysis?.recognizeSavedPeople !== false) return request;
  const enrolled = await invoke<number | null>("get_account_owner_person_id").catch(() => null);
  if (enrolled === null || enrolled === undefined) return request;
  return {
    ...request,
    speakerAnalysis: { ...request.speakerAnalysis, recognizeSavedPeople: true },
  };
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
      request: await withEnrolledVoiceRecognition(target.buildSettingsRequest()),
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
  // MODEL READINESS NEVER GATES FINISHING (issue #195). A download in flight is
  // progress, not a problem: the settings commit, onboarding completes, capture
  // starts, and the download continues in the background. The only remaining
  // guard is that the config actually serializes — an invalid custom
  // resolution/bitrate becomes `null` and breaks the backend save, which is
  // exactly what `canSkipToDashboard` covers (and what the Capture & Storage
  // gate refuses to leave in place in the first place).
  if (target.settings === null || !target.canSkipToDashboard) return;
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
      // Defense-in-depth mic/screen permission gating (and system audio's ADR
      // 0052 exemption) lives in `buildStartCaptureRequest` — see its doc.
      await invoke("start_native_capture", {
        request: buildStartCaptureRequest(target),
      });
    }
    // NAVIGATION IS THE CALLER'S. Capture starts on ARRIVAL at the Finale, and
    // the Finale then proves it with the real first frame and the first OCR hit
    // (issue #195, slice 10) — a goto here would unmount that evidence before
    // it could exist. `FinaleScreen` navigates on "Open Mnema".
  } catch (err) {
    target.errorMessage = serializeError(err);
  } finally {
    // Because navigation is the caller's, NOTHING unmounts this state on the
    // success path any more — so the in-flight flags have to be released here,
    // on every path. `OnboardingFlow.busy` derives from `completing`, and a
    // stuck `true` leaves Welcome's "Begin setup" disabled for a user who walks
    // back from the Finale.
    target.completing = false;
    target.starting = false;
  }
}
