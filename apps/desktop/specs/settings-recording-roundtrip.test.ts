// Round-trip regression for the recording autosave dirty-diff.
//
// The autosave engine decides a domain is dirty by comparing two JSON strings:
//
//   live      = JSON.stringify(buildRecDomainRequest(domain, drafts))
//   baseline  = JSON.stringify(buildRecDomainRequestFromSettings(domain, settings))
//
// The live build runs through the draft store, which CLAMPS / TRIMS values; the
// baseline build reads raw canonical `RecordingSettings`. For settings that are
// ALREADY CANONICAL (clamping/trimming is a no-op), the two builds must be
// byte-identical — otherwise the domain reads as perpetually dirty and the
// engine autosaves on every mount forever (no fixed point).
//
// This test pins that contract per autosave domain, emphasizing `processing`
// and `video` (the only builders that reorder fields, trim strings, and clamp
// numbers). The drafts here are derived from the SAME canonical fixture using
// the no-op slice of the store's settings -> draft mapping
// (`recording.svelte.ts` `sync*Drafts`), so this also guards against the live
// builder, the settings builder, and the draft-sync mapping drifting apart.

import { describe, expect, test } from "bun:test";
import {
  buildRecDomainRequest,
  buildRecDomainRequestFromSettings,
  ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
  DEFAULT_USER_CONTEXT_BUDGET_TIER,
  DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS,
  type RecordingDraftState,
} from "../src/lib/settings/state/recording-build";
import { RECORDING_AUTOSAVE_DOMAINS } from "../src/lib/settings/state/autosave-core";
import type { RecordingSettings } from "../src/lib/types";

// A representative ALREADY-CANONICAL settings object. Every value is chosen so
// the live builder's clamp/trim/coerce steps are no-ops:
//   - OCR provider is selectable (apple_vision), language is a non-empty trimmed
//     string, char-whitelist is null (live coerces "" -> null), upscale in [1,4].
//   - transcription language is non-empty; idle/chunk are clean non-negative ints.
//   - speaker provider is non-legacy (speakrs) with a model id, and
//     timeoutSeconds is a clean multiple of 60 in [60, 3600] so
//     round(sec/60)*60 === sec.
//   - resolution + bitrate are presets (the live builder reconstructs preset
//     objects in the type's field order).
const CANONICAL_SETTINGS: RecordingSettings = {
  captureScreen: true,
  captureMicrophone: true,
  captureSystemAudio: false,
  segmentDurationSeconds: 120,
  screenFrameRate: 2,
  saveDirectory: "/Users/test/Mnema",
  autoStart: true,
  screenResolution: { mode: "preset", preset: "1080p" },
  videoBitrate: { mode: "preset", preset: "medium", customMbps: null },
  nativeCaptureDebugLoggingEnabled: false,
  pauseCaptureOnInactivity: true,
  idleTimeoutSeconds: 300,
  activityMode: "system_input_or_screen_or_audio",
  microphoneActivitySensitivity: 50,
  systemAudioActivitySensitivity: 50,
  audioSpeechDetection: { detector: "silero" },
  metadata: { enabled: true, browserUrlMode: "sanitized" },
  privacy: { excludedApps: [] },
  access: {
    askAiEnabled: true,
    askAiMaxToolCalls: ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
    askAiModel: "claude-haiku-4-5",
  },
  aiRuntime: {
    enabled: true,
    providers: [{ id: "anthropic", kind: "anthropic", label: "", baseUrl: "" }],
    defaultModel: { provider: "anthropic", model: "claude-haiku-4-5" },
  },
  userContext: {
    enabled: false,
    derivationBudgetTier: DEFAULT_USER_CONTEXT_BUDGET_TIER,
    backfillWindowDays: DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS,
    backfillGoDeeper: false,
  },
  // Not consulted by any autosave domain builder, but required by the type.
  semanticSearch: { enabled: true, modelId: null } as RecordingSettings["semanticSearch"],
  previewCacheTtlSeconds: 3600,
  followTimelineLive: false,
  retentionPolicy: "never",
  appearance: "system",
  ocr: {
    enabled: true,
    provider: "apple_vision",
    modelId: null,
    language: "en-US",
    recognitionMode: "accurate",
    languageCorrection: true,
    tesseractPageSegmentationMode: "single_block",
    tesseractPreprocessMode: "grayscale",
    tesseractUpscaleFactor: 2,
    tesseractCharWhitelist: null,
  },
  transcription: {
    enabled: true,
    microphoneEnabled: true,
    systemAudioEnabled: false,
    provider: "local_whisper",
    modelId: "base",
    language: "auto",
    memoryMode: "balanced",
    idleUnloadSeconds: 300,
    chunkSeconds: 30,
  },
  speakerAnalysis: {
    separateSpeakers: true,
    recognizeSavedPeople: false,
    provider: "speakrs",
    modelId: "pyannote-community-1-wespeaker",
    timeoutSeconds: 600,
  },
  developerOptionsEnabled: false,
};

// Build the draft slice from canonical settings by mirroring the no-op path of
// the store's `sync*Drafts` mapping (recording.svelte.ts). For canonical input
// every `?? default` and coercion below is an identity, so this reproduces
// exactly what the live store would hold after a fresh sync of these settings.
function draftsFromCanonical(s: RecordingSettings): RecordingDraftState {
  const cap = s.access.askAiMaxToolCalls;
  return {
    draftCaptureScreen: s.captureScreen,
    draftCaptureMicrophone: s.captureMicrophone,
    draftCaptureSystemAudio: s.captureSystemAudio,
    draftSegmentDuration: s.segmentDurationSeconds,
    draftFrameRate: s.screenFrameRate,
    draftSaveDirectory: s.saveDirectory,
    draftAutoStart: s.autoStart,

    // canonical fixture uses a non-original preset resolution
    draftResolutionMode: "preset",
    draftResolutionPreset:
      s.screenResolution.mode === "preset" && s.screenResolution.preset !== "original"
        ? s.screenResolution.preset
        : "1080p",
    draftCustomWidth: null,
    draftCustomHeight: null,

    // canonical fixture uses a preset bitrate
    draftBitrateMode: "preset",
    draftBitratePreset: s.videoBitrate.mode === "preset" ? s.videoBitrate.preset : "medium",
    draftCustomMbps: null,

    draftPauseCaptureOnInactivity: s.pauseCaptureOnInactivity,
    draftIdleTimeoutSeconds: s.idleTimeoutSeconds,
    draftActivityMode: "system_input_or_screen_or_audio",
    draftMicrophoneActivitySensitivity: s.microphoneActivitySensitivity,
    draftSystemAudioActivitySensitivity: s.systemAudioActivitySensitivity,
    draftMicrophoneVadAdapter: s.audioSpeechDetection.detector,

    draftNativeCaptureDebugLoggingEnabled: s.nativeCaptureDebugLoggingEnabled,
    draftDeveloperOptionsEnabled: s.developerOptionsEnabled,
    draftPreviewCacheTtlSeconds: s.previewCacheTtlSeconds,

    draftFollowTimelineLive: s.followTimelineLive,
    draftRetentionPolicy: s.retentionPolicy,
    draftMetadataEnabled: s.metadata.enabled,
    draftBrowserUrlMode: s.metadata.browserUrlMode,
    draftExcludedApps: [...s.privacy.excludedApps],

    draftAskAiEnabled: s.access.askAiEnabled,
    draftAskAiModel: s.access.askAiModel ?? "",
    effectiveAskAiMaxToolCalls: cap,

    draftAiEnabled: s.aiRuntime.enabled,
    draftAiProviders: s.aiRuntime.providers.map((p) => ({
      id: (p.id ?? "").trim() || p.kind,
      kind: p.kind,
      label: p.label ?? "",
      baseUrl: p.baseUrl ?? "",
    })),
    draftAiDefaultModel: s.aiRuntime.defaultModel
      ? { provider: s.aiRuntime.defaultModel.provider, model: s.aiRuntime.defaultModel.model }
      : null,

    draftUserContextEnabled: s.userContext.enabled,
    draftUserContextBudgetTier: s.userContext.derivationBudgetTier,
    draftUserContextBackfillWindowDays: s.userContext.backfillWindowDays,
    draftUserContextBackfillGoDeeper: s.userContext.backfillGoDeeper,

    draftAppearance: s.appearance,

    draftOcrEnabled: s.ocr.enabled,
    draftOcrProvider: s.ocr.provider,
    draftOcrModelId: s.ocr.modelId,
    draftOcrLanguage: s.ocr.language ?? "",
    draftOcrRecognitionMode: s.ocr.recognitionMode,
    draftOcrLanguageCorrection: s.ocr.languageCorrection,
    draftOcrTesseractPageSegmentationMode: s.ocr.tesseractPageSegmentationMode,
    draftOcrTesseractPreprocessMode: s.ocr.tesseractPreprocessMode,
    draftOcrTesseractUpscaleFactor: s.ocr.tesseractUpscaleFactor,
    draftOcrTesseractCharWhitelist: s.ocr.tesseractCharWhitelist ?? "",

    draftTranscriptionEnabled: s.transcription.enabled,
    draftTranscriptionMicrophoneEnabled: s.transcription.microphoneEnabled,
    draftTranscriptionSystemAudioEnabled: s.transcription.systemAudioEnabled,
    draftTranscriptionProvider: s.transcription.provider,
    draftTranscriptionModelId: s.transcription.modelId,
    draftTranscriptionLanguage: s.transcription.language,
    draftTranscriptionMemoryMode: s.transcription.memoryMode,
    draftTranscriptionIdleUnloadSeconds: s.transcription.idleUnloadSeconds,
    draftTranscriptionChunkSeconds: s.transcription.chunkSeconds,

    draftSpeakerSeparateSpeakers: s.speakerAnalysis.separateSpeakers,
    draftSpeakerRecognizeSavedPeople: s.speakerAnalysis.recognizeSavedPeople,
    draftSpeakerProvider: s.speakerAnalysis.provider,
    draftSpeakerModelId: s.speakerAnalysis.modelId,
    // store holds minutes; live builder converts back to seconds.
    draftSpeakerTimeoutMinutes: Math.round(s.speakerAnalysis.timeoutSeconds / 60),
  };
}

describe("recording autosave dirty-diff round-trip", () => {
  const drafts = draftsFromCanonical(CANONICAL_SETTINGS);

  // The contract: a freshly-synced, untouched domain has a live snapshot equal
  // to its baseline snapshot — so the autosave engine sees it as clean.
  for (const domain of RECORDING_AUTOSAVE_DOMAINS) {
    test(`${domain}: live build == baseline build for canonical settings`, () => {
      const live = JSON.stringify(buildRecDomainRequest(domain, drafts));
      const baseline = JSON.stringify(
        buildRecDomainRequestFromSettings(domain, CANONICAL_SETTINGS),
      );
      expect(live).toBe(baseline);
    });
  }

  // Spell out the two highest-risk domains so a failure points at the field that
  // drifted, not just "some domain is dirty".
  test("processing: builders agree on field order, trims, and clamps", () => {
    const live = JSON.stringify(buildRecDomainRequest("processing", drafts));
    const baseline = JSON.stringify(
      buildRecDomainRequestFromSettings("processing", CANONICAL_SETTINGS),
    );
    expect(live).toBe(baseline);
  });

  test("video: builders agree on preset resolution + bitrate shape", () => {
    const live = JSON.stringify(buildRecDomainRequest("video", drafts));
    const baseline = JSON.stringify(
      buildRecDomainRequestFromSettings("video", CANONICAL_SETTINGS),
    );
    expect(live).toBe(baseline);
  });

  // Custom resolution + bitrate is the highest serde-drift risk: the live
  // builder reconstructs `{ mode: "custom", width, height }` and
  // `{ mode: "custom", preset: null, customMbps }` field-by-field, while the
  // baseline passes the raw canonical objects. Pin that the two agree so a
  // user with custom video settings isn't perpetually dirty on every mount.
  test("video: builders agree on custom resolution + bitrate shape", () => {
    const customSettings: RecordingSettings = {
      ...CANONICAL_SETTINGS,
      screenResolution: { mode: "custom", width: 2560, height: 1440 },
      videoBitrate: { mode: "custom", preset: null, customMbps: 24 },
    };
    const customDrafts: RecordingDraftState = {
      ...draftsFromCanonical(customSettings),
      draftResolutionMode: "custom",
      draftCustomWidth: 2560,
      draftCustomHeight: 1440,
      draftBitrateMode: "custom",
      draftCustomMbps: 24,
    };
    const live = JSON.stringify(buildRecDomainRequest("video", customDrafts));
    const baseline = JSON.stringify(
      buildRecDomainRequestFromSettings("video", customSettings),
    );
    expect(live).toBe(baseline);
  });
});
