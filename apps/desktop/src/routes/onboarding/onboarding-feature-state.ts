// Per-feature accordion state: the enable/attention/collapsed-download readouts
// keyed off a `FeatureId`. Lifted 1:1 out of `OnboardingController` (only
// `this.x` → `target.x`) to keep that file under the size budget; the controller
// methods now delegate here. Behavior is byte-identical — these are pure reads
// over the controller's live draft state (passed as `target`), no reactive state
// lives here.
import type { FeatureId } from "./feature-model";
import {
  ocrModelNeedsAttention as ocrModelNeedsAttentionFor,
  semanticSearchModelNeedsAttention as semanticSearchModelNeedsAttentionFor,
  speakerModelNeedsAttention as speakerModelNeedsAttentionFor,
  transcriptionModelNeedsAttention as transcriptionModelNeedsAttentionFor,
  type PermissionKey,
  type PermissionValue,
} from "./onboarding-attention";

// Each model predicate only reads `available`, so a minimal `{ available }` view
// keeps this slice decoupled from the four distinct model-status shapes (matching
// `onboarding-attention`'s `ModelAvailability`).
type ModelAvailability = { available: boolean } | null | undefined;

// The exact slice of `OnboardingController` these readouts drive. The controller
// satisfies it structurally (it owns every field/getter below — the `selected*`
// getters resolve to the four model-store deriveds, `ai.aiConfigReady` to the
// onboarding-ai store), so passing `this` keeps the readouts on live state.
export interface OnboardingFeatureTarget {
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  draftOcrEnabled: boolean;
  draftTranscriptionEnabled: boolean;
  draftSpeakerSeparateSpeakers: boolean;
  privacyEnabled: boolean;
  draftAskAiEnabled: boolean;
  draftSemanticSearchEnabled: boolean;
  permissions: Partial<Record<PermissionKey, PermissionValue>> | null;
  transcriptionRequestedWhileOff: boolean;
  selectedOcrModel: ModelAvailability;
  selectedTranscriptionModel: ModelAvailability;
  selectedSpeakerModel: ModelAvailability;
  selectedSemanticSearchModel: ModelAvailability;
  selectedOcrDownloadRunning: boolean;
  selectedOcrDownloadPercent: number | null;
  selectedTranscriptionDownloadRunning: boolean;
  selectedTranscriptionDownloadPercent: number | null;
  selectedSpeakerDownloadRunning: boolean;
  selectedSpeakerDownloadPercent: number | null;
  selectedSemanticSearchDownloadRunning: boolean;
  selectedSemanticSearchDownloadPercent: number | null;
  ai: { aiConfigReady: boolean };
}

export function isFeatureEnabled(target: OnboardingFeatureTarget, id: FeatureId): boolean {
  switch (id) {
    case "permissions":
    case "screen":
    case "storage":
      return true; // required — always on
    case "mic":
      return target.draftCaptureMicrophone;
    case "sysaudio":
      return target.draftCaptureSystemAudio;
    case "ocr":
      return target.draftOcrEnabled;
    case "transcribe":
      return target.draftTranscriptionEnabled;
    case "speakers":
      return target.draftSpeakerSeparateSpeakers;
    case "privacy":
      return target.privacyEnabled;
    case "askai":
      return target.draftAskAiEnabled;
    case "semanticSearch":
      return target.draftSemanticSearchEnabled;
  }
}

// Single-owner attention so the footer count never double-counts an issue.
export function featureAttentionFor(target: OnboardingFeatureTarget, id: FeatureId): boolean {
  switch (id) {
    case "permissions":
      // "unsupported" needs no action (mirrors `permissionAction`, which
      // returns no button for granted/unsupported) — treating it as blocking
      // would be an unrecoverable dead-end (no fix button to clear it).
      return target.permissions?.screen !== "granted" && target.permissions?.screen !== "unsupported";
    case "mic":
      return target.draftCaptureMicrophone && target.permissions?.microphone !== "granted";
    case "sysaudio":
      // Only a positive suspicion counts (ADR 0052). System audio's grant
      // cannot be read, so "not yet requested" is the *normal* state during
      // onboarding — the prompt fires at the first recording. Demanding proof
      // here would make the attention count unclearable and the finish button
      // unreachable, since nothing short of recording can produce that proof.
      return target.draftCaptureSystemAudio && target.permissions?.systemAudio === "possibly_blocked";
    case "ocr":
      return ocrModelNeedsAttentionFor(target.draftOcrEnabled, target.selectedOcrModel);
    case "transcribe":
      return (
        transcriptionModelNeedsAttentionFor(target.draftTranscriptionEnabled, target.selectedTranscriptionModel)
        || target.transcriptionRequestedWhileOff
      );
    case "speakers":
      return speakerModelNeedsAttentionFor(target.draftSpeakerSeparateSpeakers, target.selectedSpeakerModel);
    // Ask AI on but no usable reasoning engine (no provider, no default model,
    // or the default model's provider isn't configured). Readiness lives in the
    // onboarding-ai store; reading the derived here keeps `attentionCount`
    // tracking the provider/model/key state it depends on.
    case "askai":
      return target.draftAskAiEnabled && !target.ai.aiConfigReady;
    // Semantic search on but no installed model selected — inert until one is
    // downloaded, surfaced as attention (it self-gates, no hard dependency).
    case "semanticSearch":
      return semanticSearchModelNeedsAttentionFor(
        target.draftSemanticSearchEnabled,
        target.selectedSemanticSearchModel,
      );
    case "screen":
    case "storage":
    case "privacy":
      return false;
  }
}

// Live model-download status for a feature's COLLAPSED row, so a download
// started on one feature stays visible after navigating to another (the
// progress bar only renders inside the OPEN body). Reuses the existing
// selected*DownloadRunning/Percent getters (which already exclude terminal
// statuses, so the badge auto-clears). Returns null for features without a
// model download and when no download is running. Percent may be null when
// totalBytes is unknown — callers render `{percent ?? 0}%`.
export function featureDownloadFor(
  target: OnboardingFeatureTarget,
  id: FeatureId,
): { running: boolean; percent: number | null } | null {
  switch (id) {
    case "ocr":
      return target.selectedOcrDownloadRunning
        ? { running: true, percent: target.selectedOcrDownloadPercent }
        : null;
    case "transcribe":
      return target.selectedTranscriptionDownloadRunning
        ? { running: true, percent: target.selectedTranscriptionDownloadPercent }
        : null;
    case "speakers":
      return target.selectedSpeakerDownloadRunning
        ? { running: true, percent: target.selectedSpeakerDownloadPercent }
        : null;
    case "semanticSearch":
      return target.selectedSemanticSearchDownloadRunning
        ? { running: true, percent: target.selectedSemanticSearchDownloadPercent }
        : null;
    default:
      return null;
  }
}
