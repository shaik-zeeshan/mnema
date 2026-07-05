// Onboarding ⇄ RecordingSettings round-trip (VERBATIM from the legacy page).
//
// `syncDraftsInto()` and `buildSettingsRequestFrom()` are the two pure mapping
// transforms that keep a fresh onboarding producing the same `RecordingSettings`
// the old 2,674-line `routes/onboarding/+page.svelte` would. They were lifted
// 1:1 out of `OnboardingController` (only `this.x` → `draft.x`) to keep that file
// under the size budget; the controller's `syncDrafts`/`buildSettingsRequest`
// methods now delegate here so the public surface and behavior are unchanged.
//
// No reactive state lives here — these operate on the controller's draft fields
// through the `OnboardingDraftTarget` view, which the controller satisfies by
// construction (it owns every field/getter referenced below).
import { theme } from "$lib/theme.svelte";
import type {
  ActivityMode,
  AudioTranscriptionMemoryMode,
  AudioTranscriptionProvider,
  ExcludedAppEntry,
  MicrophoneVadAdapter,
  OcrProvider,
  OcrRecognitionMode,
  OcrTesseractPageSegmentationMode,
  OcrTesseractPreprocessMode,
  RecordingSettings,
  ResolutionMode,
  ResolutionPreset,
  RetentionPolicy,
  VideoBitrateMode,
  VideoBitratePreset,
} from "$lib/types";
import type { OnboardingAiStore } from "./onboarding-ai.svelte";
import type { SemanticSearchPickedModel } from "./onboarding-models.svelte";
import {
  DEFAULT_SPEAKER_MODEL_ID,
  DEFAULT_SPEAKER_PROVIDER,
  defaultOcrLanguageForProvider,
  defaultOcrModelIdForProvider,
  defaultTranscriptionModelIdForProvider,
  isSelectableOcrProvider,
} from "./onboarding-mapping";
import { syncPrivacyDraftInto } from "./onboarding-privacy-sync";

// The exact slice of `OnboardingController` these transforms read/write. The
// controller satisfies this structurally, so passing `this` here keeps the
// settings round-trip operating on the live draft state.
export interface OnboardingDraftTarget {
  draftCaptureScreen: boolean;
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  draftFrameRate: number;
  draftSegmentDuration: number;
  draftResolutionMode: ResolutionMode;
  draftResolutionPreset: ResolutionPreset;
  draftCustomWidth: number | null;
  draftCustomHeight: number | null;
  customWidthRaw: string;
  customHeightRaw: string;
  draftBitrateMode: VideoBitrateMode;
  draftBitratePreset: VideoBitratePreset;
  draftCustomMbpsRaw: string;
  draftCustomMbps: number | null;
  draftSaveDirectory: string;
  draftPreviewCacheTtlSeconds: number;
  draftRetentionPolicy: RetentionPolicy;
  draftAutoStart: boolean;
  draftPauseCaptureOnInactivity: boolean;
  draftIdleTimeoutSeconds: number;
  draftActivityMode: ActivityMode;
  draftMicrophoneActivitySensitivity: number;
  draftMicrophoneVadAdapter: MicrophoneVadAdapter;
  draftSystemAudioActivitySensitivity: number;
  draftOcrEnabled: boolean;
  draftOcrProvider: OcrProvider;
  draftOcrModelId: string | null;
  draftOcrLanguage: string;
  draftOcrRecognitionMode: OcrRecognitionMode;
  draftOcrLanguageCorrection: boolean;
  draftOcrTesseractPageSegmentationMode: OcrTesseractPageSegmentationMode;
  draftOcrTesseractPreprocessMode: OcrTesseractPreprocessMode;
  draftOcrTesseractUpscaleFactor: number;
  draftTranscriptionEnabled: boolean;
  draftTranscriptionProvider: AudioTranscriptionProvider;
  draftTranscriptionModelId: string | null;
  draftTranscriptionLanguage: string;
  draftTranscriptionMemoryMode: AudioTranscriptionMemoryMode;
  draftTranscriptionIdleUnloadSeconds: number;
  draftTranscriptionChunkSeconds: number;
  draftTranscriptionMicrophoneEnabled: boolean;
  draftTranscriptionSystemAudioEnabled: boolean;
  draftSpeakerSeparateSpeakers: boolean;
  draftSpeakerRecognizeSavedPeople: boolean;
  draftSpeakerProvider: string;
  draftSpeakerModelId: string | null;
  draftSpeakerTimeoutMinutes: number;
  draftExcludedApps: ExcludedAppEntry[];
  privacyEnabled: boolean;
  draftAskAiEnabled: boolean;
  draftSemanticSearchEnabled: boolean;
  draftSemanticSearchModelId: string | null;
  // Backing settings + delegated subsystems read by buildSettingsRequest.
  settings: RecordingSettings | null;
  readonly selectedSemanticSearchModel: SemanticSearchPickedModel | null;
  readonly ai: OnboardingAiStore;
}

export function syncDraftsInto(draft: OnboardingDraftTarget, next: RecordingSettings): void {
  draft.draftCaptureScreen = next.captureScreen;
  draft.draftCaptureMicrophone = next.captureMicrophone;
  draft.draftCaptureSystemAudio = next.captureSystemAudio;
  draft.draftFrameRate = next.screenFrameRate;
  draft.draftSegmentDuration = next.segmentDurationSeconds;
  if (next.screenResolution.mode === "custom") {
    draft.draftResolutionMode = "custom";
    draft.draftCustomWidth = next.screenResolution.width;
    draft.draftCustomHeight = next.screenResolution.height;
    draft.customWidthRaw = String(next.screenResolution.width);
    draft.customHeightRaw = String(next.screenResolution.height);
  } else if (next.screenResolution.preset === "original") {
    draft.draftResolutionMode = "original";
    draft.draftResolutionPreset = "1080p";
    draft.draftCustomWidth = null;
    draft.draftCustomHeight = null;
    draft.customWidthRaw = "";
    draft.customHeightRaw = "";
  } else {
    draft.draftResolutionMode = "preset";
    draft.draftResolutionPreset = next.screenResolution.preset;
    draft.draftCustomWidth = null;
    draft.draftCustomHeight = null;
    draft.customWidthRaw = "";
    draft.customHeightRaw = "";
  }
  if (next.videoBitrate.mode === "custom") {
    draft.draftBitrateMode = "custom";
    draft.draftBitratePreset = "medium";
    draft.draftCustomMbps = next.videoBitrate.customMbps;
    draft.draftCustomMbpsRaw = String(next.videoBitrate.customMbps);
  } else {
    draft.draftBitrateMode = "preset";
    draft.draftBitratePreset = next.videoBitrate.preset;
    draft.draftCustomMbps = null;
    draft.draftCustomMbpsRaw = "";
  }
  draft.draftSaveDirectory = next.saveDirectory;
  draft.draftPreviewCacheTtlSeconds = next.previewCacheTtlSeconds ?? 3600;
  draft.draftRetentionPolicy = next.retentionPolicy ?? "never";
  draft.draftAutoStart = next.autoStart;
  draft.draftPauseCaptureOnInactivity = next.pauseCaptureOnInactivity;
  draft.draftIdleTimeoutSeconds = next.idleTimeoutSeconds;
  draft.draftActivityMode = "system_input_or_screen_or_audio";
  draft.draftMicrophoneActivitySensitivity = next.microphoneActivitySensitivity ?? 50;
  // Mirror real settings' fallback chain so a returning user's saved VAD
  // ("webrtc"/"off") round-trips instead of being clobbered to "silero".
  draft.draftMicrophoneVadAdapter =
    next.audioSpeechDetection?.detector ?? next.microphoneVadAdapter ?? "silero";
  draft.draftSystemAudioActivitySensitivity = next.systemAudioActivitySensitivity ?? 50;
  draft.draftOcrEnabled = next.ocr?.enabled ?? true;
  const loadedOcrProvider = next.ocr?.provider;
  const loadedOcrProviderSelectable = isSelectableOcrProvider(loadedOcrProvider);
  draft.draftOcrProvider = loadedOcrProviderSelectable ? loadedOcrProvider : "apple_vision";
  draft.draftOcrModelId = loadedOcrProviderSelectable
    ? (next.ocr?.modelId ?? defaultOcrModelIdForProvider(draft.draftOcrProvider))
    : defaultOcrModelIdForProvider(draft.draftOcrProvider);
  draft.draftOcrLanguage = loadedOcrProviderSelectable
    ? (next.ocr?.language ?? defaultOcrLanguageForProvider(draft.draftOcrProvider) ?? "")
    : defaultOcrLanguageForProvider(draft.draftOcrProvider) ?? "";
  draft.draftOcrRecognitionMode = next.ocr?.recognitionMode ?? "fast";
  draft.draftOcrLanguageCorrection = next.ocr?.languageCorrection ?? false;
  draft.draftOcrTesseractPageSegmentationMode = next.ocr?.tesseractPageSegmentationMode ?? "single_block";
  draft.draftOcrTesseractPreprocessMode = next.ocr?.tesseractPreprocessMode ?? "grayscale";
  draft.draftOcrTesseractUpscaleFactor = next.ocr?.tesseractUpscaleFactor ?? 1;
  draft.draftTranscriptionEnabled = next.transcription?.enabled ?? true;
  // Reconcile the per-source transcribe flags to the master on rehydrate, mirroring
  // `toggleFeature("transcribe")`'s off-branch (onboarding.svelte.ts): when the master
  // is off, zero the per-source requests so a returning user (saved enabled=false,
  // microphoneEnabled=true, captureMicrophone=true) doesn't get phantom attention
  // (`transcriptionRequestedWhileOff`) that deadlocks the finale CTAs.
  draft.draftTranscriptionMicrophoneEnabled = next.transcription?.enabled
    ? (next.transcription?.microphoneEnabled ?? true)
    : false;
  draft.draftTranscriptionSystemAudioEnabled = next.transcription?.enabled
    ? (next.transcription?.systemAudioEnabled ?? false)
    : false;
  draft.draftTranscriptionProvider = next.transcription?.provider ?? "local_whisper";
  draft.draftTranscriptionModelId = next.transcription?.modelId ?? defaultTranscriptionModelIdForProvider(draft.draftTranscriptionProvider);
  draft.draftTranscriptionLanguage = next.transcription?.language ?? "auto";
  draft.draftTranscriptionMemoryMode = next.transcription?.memoryMode ?? "balanced";
  draft.draftTranscriptionIdleUnloadSeconds = next.transcription?.idleUnloadSeconds ?? 300;
  draft.draftTranscriptionChunkSeconds = next.transcription?.chunkSeconds ?? 30;
  draft.draftSpeakerSeparateSpeakers = next.speakerAnalysis?.separateSpeakers ?? false;
  draft.draftSpeakerRecognizeSavedPeople = next.speakerAnalysis?.recognizeSavedPeople ?? false;
  // Coerce legacy saved values: the sherpa_onnx provider (and its model ids)
  // no longer exist, so old settings resolve to the speakrs default — else the
  // preset picker would select a provider/model the backend manifest never
  // returns. Mirrors recording.svelte.ts.
  const savedSpeakerProvider = next.speakerAnalysis?.provider;
  const isLegacySpeakerProvider = !savedSpeakerProvider || savedSpeakerProvider === "sherpa_onnx";
  draft.draftSpeakerProvider = isLegacySpeakerProvider ? DEFAULT_SPEAKER_PROVIDER : savedSpeakerProvider;
  draft.draftSpeakerModelId = isLegacySpeakerProvider
    ? DEFAULT_SPEAKER_MODEL_ID
    : (next.speakerAnalysis?.modelId ?? DEFAULT_SPEAKER_MODEL_ID);
  draft.draftSpeakerTimeoutMinutes = Math.round((next.speakerAnalysis?.timeoutSeconds ?? 600) / 60);
  syncPrivacyDraftInto(draft, next);
  draft.draftAskAiEnabled = next.access?.askAiEnabled ?? false;
  draft.draftSemanticSearchEnabled = next.semanticSearch?.enabled ?? false;
  draft.draftSemanticSearchModelId = next.semanticSearch?.modelId ?? null;
  // Re-seed the inline Reasoning-Engine setup from the canonical aiRuntime
  // domain (the whole-settings round-trip flows back through here after save).
  draft.ai.syncFromSettings(next.aiRuntime?.providers ?? [], next.aiRuntime?.defaultModel ?? null);
}

export function buildSettingsRequestFrom(draft: OnboardingDraftTarget): RecordingSettings {
  const base = draft.settings;
  if (base === null) throw new Error("Recording settings are not loaded.");
  return {
    ...base,
    captureScreen: draft.draftCaptureScreen,
    captureMicrophone: draft.draftCaptureMicrophone,
    captureSystemAudio: draft.draftCaptureScreen && draft.draftCaptureSystemAudio,
    screenFrameRate: draft.draftFrameRate,
    screenResolution: draft.draftResolutionMode === "custom"
      ? { mode: "custom", width: draft.draftCustomWidth!, height: draft.draftCustomHeight! }
      : { mode: "preset", preset: draft.draftResolutionMode === "original" ? "original" : draft.draftResolutionPreset },
    videoBitrate: draft.draftBitrateMode === "custom"
      ? { mode: "custom", preset: null, customMbps: draft.draftCustomMbps! }
      : { mode: "preset", preset: draft.draftBitratePreset, customMbps: null },
    segmentDurationSeconds: draft.draftSegmentDuration,
    saveDirectory: draft.draftSaveDirectory.trim(),
    previewCacheTtlSeconds: draft.draftPreviewCacheTtlSeconds,
    retentionPolicy: draft.draftRetentionPolicy,
    appearance: theme.loaded ? theme.appearance : base.appearance,
    autoStart: draft.draftAutoStart,
    pauseCaptureOnInactivity: draft.draftPauseCaptureOnInactivity,
    idleTimeoutSeconds: draft.draftIdleTimeoutSeconds,
    activityMode: "system_input_or_screen_or_audio",
    microphoneActivitySensitivity: draft.draftMicrophoneActivitySensitivity,
    // Persist the mic VAD adapter alongside the sync-read above — writing this
    // WITHOUT the read would clobber a returning user's saved "webrtc"/"off".
    audioSpeechDetection: { detector: draft.draftMicrophoneVadAdapter },
    systemAudioActivitySensitivity: draft.draftSystemAudioActivitySensitivity,
    ocr: {
      enabled: draft.draftOcrEnabled,
      provider: draft.draftOcrProvider,
      modelId: draft.draftOcrModelId,
      language: draft.draftOcrLanguage.trim() || null,
      recognitionMode: draft.draftOcrRecognitionMode,
      languageCorrection: draft.draftOcrLanguageCorrection,
      tesseractPageSegmentationMode: draft.draftOcrTesseractPageSegmentationMode,
      tesseractPreprocessMode: draft.draftOcrTesseractPreprocessMode,
      tesseractUpscaleFactor: Math.max(1, Math.min(4, Math.trunc(Number(draft.draftOcrTesseractUpscaleFactor) || 1))),
      tesseractCharWhitelist: null,
    },
    transcription: {
      enabled: draft.draftTranscriptionEnabled,
      // `syncDraftsInto` deliberately zeroes the per-source draft flags while the
      // master is off (the phantom-attention fix). Persisting those zeroes would
      // wipe a returning user's saved per-source preference (saved enabled=false,
      // microphoneEnabled=true). When the master is off, round-trip the SAVED
      // per-source flags instead so re-enabling transcription later restores them.
      microphoneEnabled: draft.draftTranscriptionEnabled
        ? draft.draftTranscriptionMicrophoneEnabled
        : (base.transcription?.microphoneEnabled ?? draft.draftTranscriptionMicrophoneEnabled),
      systemAudioEnabled: draft.draftTranscriptionEnabled
        ? draft.draftTranscriptionSystemAudioEnabled
        : (base.transcription?.systemAudioEnabled ?? draft.draftTranscriptionSystemAudioEnabled),
      provider: draft.draftTranscriptionProvider,
      modelId: draft.draftTranscriptionModelId,
      language: draft.draftTranscriptionLanguage.trim() || "auto",
      memoryMode: draft.draftTranscriptionMemoryMode,
      idleUnloadSeconds: Math.max(0, Math.trunc(Number(draft.draftTranscriptionIdleUnloadSeconds) || 0)),
      chunkSeconds: Math.max(0, Math.trunc(Number(draft.draftTranscriptionChunkSeconds) || 0)),
    },
    speakerAnalysis: {
      separateSpeakers: draft.draftSpeakerSeparateSpeakers,
      recognizeSavedPeople: draft.draftSpeakerRecognizeSavedPeople,
      provider: draft.draftSpeakerProvider,
      modelId: draft.draftSpeakerModelId,
      timeoutSeconds: Math.max(
        60,
        Math.min(3600, Math.trunc(Number(draft.draftSpeakerTimeoutMinutes) || 10) * 60),
      ),
    },
    // Semantic search: draft enable + draft model selection committed here
    // (the live `select_semantic_search_model`/`update_semantic_search_settings`
    // commands are never called from onboarding). Prefer the picked model's
    // provider when known, else the base provider, else the on-device default.
    semanticSearch: {
      enabled: draft.draftSemanticSearchEnabled,
      provider:
        draft.selectedSemanticSearchModel?.provider ?? base.semanticSearch?.provider ?? "local",
      modelId: draft.draftSemanticSearchModelId ?? base.semanticSearch?.modelId ?? null,
    },
    access: {
      askAiEnabled: draft.draftAskAiEnabled,
      // Round-trip the opt-in web-fetch toggle (set on the Settings page); this
      // full save is authoritative, so omitting it would reset it to off.
      askAiWebFetchEnabled: base.access?.askAiWebFetchEnabled ?? false,
      askAiMaxToolCalls: base.access?.askAiMaxToolCalls ?? 12,
      // `access` is sent whole and is authoritative, so we must round-trip the
      // Ask AI model selection (chosen on the Settings page); omitting it would
      // reset the selection back to the PI runtime default on every full save.
      // Left null so Ask AI inherits the global default model chosen below.
      askAiModel: base.access?.askAiModel ?? null,
    },
    // Reasoning Engine config connected inline during onboarding (AskAiBody).
    // The master AI switch is MONOTONIC w.r.t. the base: onboarding only surfaces
    // Ask AI, so enabling Ask AI can turn the engine ON, but it must NOT turn the
    // engine OFF for a returning user who enabled it elsewhere (e.g. for User
    // Context / digests with Ask AI off — `aiRuntime.enabled=true`,
    // `askAiEnabled=false`). The per-provider key is keychain-only (saved
    // eagerly) and never travels in this payload.
    aiRuntime: {
      enabled: draft.draftAskAiEnabled || (base.aiRuntime?.enabled ?? false),
      providers: draft.ai.draftAiProviders.map((p) => ({
        id: p.id,
        kind: p.kind,
        label: p.label,
        baseUrl: p.baseUrl,
      })),
      defaultModel: draft.ai.draftAiDefaultModel
        ? { provider: draft.ai.draftAiDefaultModel.provider, model: draft.ai.draftAiDefaultModel.model }
        : null,
      // Onboarding doesn't configure MCP connectors — preserve any the returning
      // user already has (the Settings page owns this list).
      mcpServers: base.aiRuntime?.mcpServers ?? [],
    },
  };
}

// Concise reason the finale CTAs are disabled, or null when nothing to surface.
// Lives here (not in the controller) only to keep `onboarding.svelte.ts` under
// the file-size budget. `active` is the controller's "on the finale and not
// busy" gate; `names` are the regressed FEATURE rows. Surfaces ONLY for an
// attention regression (gate active AND ≥1 named row) — never for an in-flight
// load/save/complete (those CTAs render their own busy labels) and never when
// nothing regressed (empty names → null, so the finale stays clean).
export function finaleBlockReasonFor(active: boolean, names: string[]): string | null {
  if (!active || names.length === 0) return null;
  // Only "Start recording" is gated by these — the "Just open the dashboard"
  // escape hatch stays enabled, so the copy points at both the recover path
  // (back to setup) and the still-available skip rather than implying a dead end.
  return `Start recording is waiting on: ${names.join(", ")}. Open the dashboard now, or return to setup to fix it.`;
}
