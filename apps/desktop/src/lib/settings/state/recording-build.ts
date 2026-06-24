// Pure per-domain request/snapshot builders for the recording-settings store.
//
// Everything here is a pure function of either the live draft store (`rec`) or a
// canonical `RecordingSettings` object — no module-level state, no `$state`, no
// IPC. The reactive store (`recording.svelte.ts`) owns the `draft*` fields and
// calls these to build the autosave request payloads, the diff snapshots, and
// the persisted baseline snapshots. Keeping the switches here (instead of inline
// in the store) keeps the store file small and the build logic unit-friendly.
//
// `RecordingDraftState` is the structural slice of the store these builders read.
// It is intentionally a plain interface (not the store class) so the builders
// stay decoupled from the store's reactivity.

import type {
  RecordingSettings,
  UpdateAccessSettingsRequest,
  UpdateCaptureSourceSettingsRequest,
  UpdateCaptureTimingSettingsRequest,
  UpdateDeveloperSettingsRequest,
  UpdateDisplaySettingsRequest,
  UpdateInactivitySettingsRequest,
  UpdateMetadataSettingsRequest,
  UpdateProcessingSettingsRequest,
  UpdateStorageSettingsRequest,
  UpdateVideoSettingsRequest,
  UpdateAiRuntimeSettingsRequest,
  UpdateUserContextSettingsRequest,
  AiProviderConfig,
  AiEngineRef,
  AppearanceSetting,
  AudioTranscriptionMemoryMode,
  AudioTranscriptionProvider,
  BrowserUrlMode,
  DerivationBudgetTier,
  ExcludedAppEntry,
  MicrophoneVadAdapter,
  OcrProvider,
  OcrRecognitionMode,
  OcrTesseractPageSegmentationMode,
  OcrTesseractPreprocessMode,
  ResolutionMode,
  ResolutionPreset,
  RetentionPolicy,
  VideoBitrateMode,
  VideoBitratePreset,
} from "$lib/types";
import type {
  AutosaveRecordingDomain,
  RecordingSettingsDraftDomain,
} from "./autosave-core";

// Tool-call cap default. Persisted as a single number where 0 = no cap; the UI
// splits that into a "limit on/off" toggle plus the numeric value.
export const ASK_AI_DEFAULT_TOOL_CALL_LIMIT = 12;

// Tool-call cap ceiling = the runtime MULTI_TURN_CEILING. A value above this is
// not honored by the engine, so clamp the effective cap (and the UI Stepper) to
// it rather than persisting a number that silently has no effect.
export const ASK_AI_MAX_TOOL_CALL_LIMIT = 64;

export const DEFAULT_USER_CONTEXT_BUDGET_TIER: DerivationBudgetTier = "balanced";
export const DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS = 30;

// Effective tool-call cap normalization, shared by the live access request
// (via the store's `effectiveAskAiMaxToolCalls` derived) and the canonical
// baseline builder so both land on the same fixed point: 0 (= no cap) passes
// through, any positive value is clamped to [1, ASK_AI_MAX_TOOL_CALL_LIMIT].
export function clampAskAiMaxToolCalls(cap: number): number {
  const n = Math.floor(Number(cap) || 0);
  if (n <= 0) return 0;
  return Math.min(ASK_AI_MAX_TOOL_CALL_LIMIT, Math.max(1, n));
}

export type RecordingDomainRequest =
  | UpdateCaptureSourceSettingsRequest
  | UpdateCaptureTimingSettingsRequest
  | UpdateVideoSettingsRequest
  | UpdateStorageSettingsRequest
  | UpdateDisplaySettingsRequest
  | UpdateMetadataSettingsRequest
  | UpdateInactivitySettingsRequest
  | UpdateProcessingSettingsRequest
  | UpdateDeveloperSettingsRequest
  | UpdateAccessSettingsRequest
  | UpdateAiRuntimeSettingsRequest
  | UpdateUserContextSettingsRequest;

// The structural draft slice the builders read. Mirrors the store's `draft*`
// fields (plus the entangled raw text drafts + the effective tool-call cap).
export interface RecordingDraftState {
  draftCaptureScreen: boolean;
  draftCaptureMicrophone: boolean;
  draftCaptureSystemAudio: boolean;
  draftSegmentDuration: number;
  draftFrameRate: number;
  draftSaveDirectory: string;
  draftAutoStart: boolean;

  draftResolutionMode: ResolutionMode;
  draftResolutionPreset: ResolutionPreset;
  draftCustomWidth: number | null;
  draftCustomHeight: number | null;

  draftBitrateMode: VideoBitrateMode;
  draftBitratePreset: VideoBitratePreset;
  draftCustomMbps: number | null;

  draftPauseCaptureOnInactivity: boolean;
  draftIdleTimeoutSeconds: number;
  draftMicrophoneActivitySensitivity: number;
  draftSystemAudioActivitySensitivity: number;
  draftMicrophoneVadAdapter: MicrophoneVadAdapter;

  draftNativeCaptureDebugLoggingEnabled: boolean;
  draftDeveloperOptionsEnabled: boolean;
  draftPreviewCacheTtlSeconds: number;

  draftFollowTimelineLive: boolean;
  draftRetentionPolicy: RetentionPolicy;
  draftMetadataEnabled: boolean;
  draftBrowserUrlMode: BrowserUrlMode;
  draftExcludedApps: ExcludedAppEntry[];

  draftAskAiEnabled: boolean;
  draftAskAiModel: string;
  // The effective persisted tool-call cap (0 = no cap), derived from the
  // limit toggle + numeric value.
  effectiveAskAiMaxToolCalls: number;

  draftAiEnabled: boolean;
  draftAiProviders: AiProviderConfig[];
  draftAiDefaultModel: AiEngineRef | null;

  draftUserContextEnabled: boolean;
  draftUserContextBudgetTier: DerivationBudgetTier;
  draftUserContextBackfillWindowDays: number;
  draftUserContextBackfillGoDeeper: boolean;

  draftAppearance: AppearanceSetting;

  draftOcrEnabled: boolean;
  draftOcrProvider: OcrProvider;
  draftOcrModelId: string | null;
  draftOcrLanguage: string;
  draftOcrRecognitionMode: OcrRecognitionMode;
  draftOcrLanguageCorrection: boolean;
  draftOcrTesseractPageSegmentationMode: OcrTesseractPageSegmentationMode;
  draftOcrTesseractPreprocessMode: OcrTesseractPreprocessMode;
  draftOcrTesseractUpscaleFactor: number;
  draftOcrTesseractCharWhitelist: string;

  draftTranscriptionEnabled: boolean;
  draftTranscriptionMicrophoneEnabled: boolean;
  draftTranscriptionSystemAudioEnabled: boolean;
  draftTranscriptionProvider: AudioTranscriptionProvider;
  draftTranscriptionModelId: string | null;
  draftTranscriptionLanguage: string;
  draftTranscriptionMemoryMode: AudioTranscriptionMemoryMode;
  draftTranscriptionIdleUnloadSeconds: number;
  draftTranscriptionChunkSeconds: number;

  draftSpeakerSeparateSpeakers: boolean;
  draftSpeakerRecognizeSavedPeople: boolean;
  draftSpeakerProvider: string;
  draftSpeakerModelId: string | null;
  draftSpeakerTimeoutMinutes: number;
}

function buildScreenResolutionRequest(
  rec: RecordingDraftState,
): UpdateVideoSettingsRequest["screenResolution"] {
  if (rec.draftResolutionMode === "custom") {
    // The save path is gated on `customResolutionBlocked` (both raw fields must
    // parse), so these are non-null when we get here. Fail fast rather than
    // serialize a `null` width/height the backend would reject — and without a
    // `!` type-lie on the `number | null` drafts.
    if (rec.draftCustomWidth === null || rec.draftCustomHeight === null) {
      throw new Error("Custom resolution requires both width and height.");
    }
    return {
      mode: "custom" as const,
      width: rec.draftCustomWidth,
      height: rec.draftCustomHeight,
    };
  }
  return {
    mode: "preset" as const,
    preset:
      rec.draftResolutionMode === "original"
        ? ("original" as const)
        : rec.draftResolutionPreset,
  };
}

function buildVideoBitrateRequest(
  rec: RecordingDraftState,
): UpdateVideoSettingsRequest["videoBitrate"] {
  if (rec.draftBitrateMode === "custom") {
    // Gated on `customBitrateBlocked` (the raw field must parse), so this is
    // non-null here. Fail fast rather than serialize a `null` the backend would
    // reject — and without a `!` type-lie on the `number | null` draft.
    if (rec.draftCustomMbps === null) {
      throw new Error("Custom bitrate requires a value.");
    }
    return { mode: "custom" as const, preset: null, customMbps: rec.draftCustomMbps };
  }
  return { mode: "preset" as const, preset: rec.draftBitratePreset, customMbps: null };
}

function buildProcessingRequest(rec: RecordingDraftState): UpdateProcessingSettingsRequest {
  return {
    previewCacheTtlSeconds: rec.draftPreviewCacheTtlSeconds,
    ocr: {
      enabled: rec.draftOcrEnabled,
      provider: rec.draftOcrProvider,
      modelId: rec.draftOcrModelId,
      language: rec.draftOcrLanguage.trim() || null,
      recognitionMode: rec.draftOcrRecognitionMode,
      languageCorrection: rec.draftOcrLanguageCorrection,
      tesseractPageSegmentationMode: rec.draftOcrTesseractPageSegmentationMode,
      tesseractPreprocessMode: rec.draftOcrTesseractPreprocessMode,
      tesseractUpscaleFactor: Math.max(
        1,
        Math.min(4, Math.trunc(Number(rec.draftOcrTesseractUpscaleFactor) || 1)),
      ),
      tesseractCharWhitelist: rec.draftOcrTesseractCharWhitelist.trim() || null,
    },
    transcription: {
      enabled: rec.draftTranscriptionEnabled,
      microphoneEnabled: rec.draftTranscriptionMicrophoneEnabled,
      systemAudioEnabled: rec.draftTranscriptionSystemAudioEnabled,
      provider: rec.draftTranscriptionProvider,
      modelId: rec.draftTranscriptionModelId,
      language: rec.draftTranscriptionLanguage.trim() || "auto",
      memoryMode: rec.draftTranscriptionMemoryMode,
      // Clamp to the product UI ceilings so an out-of-range typed value can
      // never reach `invoke`: the backend REJECTS (does not clamp) idle > 86400
      // or chunk > 3600, and a rejected save retries forever. 1800/300 are the
      // maxes the UI advertises (the onboarding Slider enforces them); chunk's
      // 300s also respects the 5-minute capture-segment cap.
      idleUnloadSeconds: Math.max(
        0,
        Math.min(1800, Math.trunc(Number(rec.draftTranscriptionIdleUnloadSeconds) || 0)),
      ),
      chunkSeconds: Math.max(
        0,
        Math.min(300, Math.trunc(Number(rec.draftTranscriptionChunkSeconds) || 0)),
      ),
    },
    speakerAnalysis: {
      separateSpeakers: rec.draftSpeakerSeparateSpeakers,
      recognizeSavedPeople: rec.draftSpeakerRecognizeSavedPeople,
      provider: rec.draftSpeakerProvider,
      modelId: rec.draftSpeakerModelId,
      timeoutSeconds: Math.max(
        60,
        Math.min(3600, Math.trunc(Number(rec.draftSpeakerTimeoutMinutes) || 10) * 60),
      ),
    },
  };
}

// Build the autosave request payload for one autosave domain from the live
// draft store.
export function buildRecDomainRequest(
  domain: AutosaveRecordingDomain,
  rec: RecordingDraftState,
): RecordingDomainRequest {
  switch (domain) {
    case "capture_sources":
      return {
        captureScreen: rec.draftCaptureScreen,
        captureMicrophone: rec.draftCaptureMicrophone,
        captureSystemAudio: rec.draftCaptureSystemAudio,
      };
    case "capture_timing":
      return {
        segmentDurationSeconds: rec.draftSegmentDuration,
        autoStart: rec.draftAutoStart,
      };
    case "video":
      return {
        screenFrameRate: rec.draftFrameRate,
        screenResolution: buildScreenResolutionRequest(rec),
        videoBitrate: buildVideoBitrateRequest(rec),
      };
    case "storage":
      return {
        saveDirectory: rec.draftSaveDirectory,
        retentionPolicy: rec.draftRetentionPolicy,
      };
    case "display":
      return {
        appearance: rec.draftAppearance,
        followTimelineLive: rec.draftFollowTimelineLive,
      };
    case "metadata":
      return {
        enabled: rec.draftMetadataEnabled,
        browserUrlMode: rec.draftBrowserUrlMode,
      };
    case "inactivity":
      return {
        pauseCaptureOnInactivity: rec.draftPauseCaptureOnInactivity,
        idleTimeoutSeconds: rec.draftIdleTimeoutSeconds,
        microphoneActivitySensitivity: rec.draftMicrophoneActivitySensitivity,
        systemAudioActivitySensitivity: rec.draftSystemAudioActivitySensitivity,
        audioSpeechDetection: {
          detector: rec.draftMicrophoneVadAdapter,
        },
      };
    case "processing":
      return buildProcessingRequest(rec);
    case "developer":
      return {
        developerOptionsEnabled: rec.draftDeveloperOptionsEnabled,
        nativeCaptureDebugLoggingEnabled: rec.draftNativeCaptureDebugLoggingEnabled,
      };
    case "access":
      return {
        askAiEnabled: rec.draftAskAiEnabled,
        askAiMaxToolCalls: rec.effectiveAskAiMaxToolCalls,
        askAiModel: rec.draftAskAiModel,
      };
    case "ai_runtime":
      // `defaultModel` is tri-state on the wire; the card always sends the
      // full intent (object = set, null = clear), never "leave unchanged".
      return {
        enabled: rec.draftAiEnabled,
        providers: rec.draftAiProviders.map((p) => ({
          id: p.id,
          kind: p.kind,
          label: p.label,
          baseUrl: p.baseUrl,
        })),
        defaultModel: rec.draftAiDefaultModel
          ? {
              provider: rec.draftAiDefaultModel.provider,
              model: rec.draftAiDefaultModel.model,
            }
          : null,
      };
    case "user_context":
      return {
        enabled: rec.draftUserContextEnabled,
        derivationBudgetTier: rec.draftUserContextBudgetTier,
        backfillWindowDays: rec.draftUserContextBackfillWindowDays,
        backfillGoDeeper: rec.draftUserContextBackfillGoDeeper,
      };
  }
}

// Build the comparable request payload for one draft domain from a CANONICAL
// settings object (the persisted-baseline source of truth).
export function buildRecDomainRequestFromSettings(
  domain: RecordingSettingsDraftDomain,
  s: RecordingSettings,
): unknown {
  switch (domain) {
    case "capture_sources":
      return {
        captureScreen: s.captureScreen,
        captureMicrophone: s.captureMicrophone,
        captureSystemAudio: s.captureSystemAudio,
      };
    case "capture_timing":
      return {
        segmentDurationSeconds: s.segmentDurationSeconds,
        autoStart: s.autoStart,
      };
    case "video":
      return {
        screenFrameRate: s.screenFrameRate,
        screenResolution: s.screenResolution,
        videoBitrate: s.videoBitrate,
      };
    case "storage":
      return {
        saveDirectory: s.saveDirectory,
        retentionPolicy: s.retentionPolicy ?? "never",
      };
    case "display":
      return {
        appearance: s.appearance ?? "system",
        followTimelineLive: s.followTimelineLive ?? false,
      };
    case "metadata":
      return {
        enabled: s.metadata?.enabled ?? true,
        browserUrlMode: s.metadata?.browserUrlMode ?? "sanitized",
      };
    case "app_privacy_exclusion":
      return {
        excludedApps: s.privacy?.excludedApps ?? [],
      };
    case "inactivity":
      return {
        pauseCaptureOnInactivity: s.pauseCaptureOnInactivity,
        idleTimeoutSeconds: s.idleTimeoutSeconds,
        microphoneActivitySensitivity: s.microphoneActivitySensitivity ?? 50,
        systemAudioActivitySensitivity: s.systemAudioActivitySensitivity ?? 50,
        audioSpeechDetection: {
          detector: s.audioSpeechDetection?.detector ?? s.microphoneVadAdapter ?? "silero",
        },
      };
    case "processing":
      // Symmetrize the empty-string/null/"auto" normalization with the live
      // request builder (`buildProcessingRequest`). The live builder coerces
      // `ocr.language`/`ocr.tesseractCharWhitelist` ""->null and
      // `transcription.language` ""->"auto"; passing the raw canonical values
      // here would make those fields read perpetually dirty if the backend ever
      // persisted an empty string. Defensive: unreachable today (backend
      // validates), but keeps the dirty-diff fixed point stable either way.
      return {
        previewCacheTtlSeconds: s.previewCacheTtlSeconds ?? 3600,
        ocr: {
          ...s.ocr,
          language: (s.ocr.language ?? "").trim() || null,
          tesseractCharWhitelist: (s.ocr.tesseractCharWhitelist ?? "").trim() || null,
        },
        transcription: {
          ...s.transcription,
          language: (s.transcription.language ?? "").trim() || "auto",
          // Mirror the live builder's idle/chunk clamp (0..1800 / 0..300). The
          // backend only rejects idle > 86400 / chunk > 3600, so a persisted
          // value above the UI ceiling (e.g. saved by an older build or the CLI)
          // would otherwise read perpetually dirty against the clamped request.
          idleUnloadSeconds: Math.max(
            0,
            Math.min(1800, Math.trunc(Number(s.transcription.idleUnloadSeconds) || 0)),
          ),
          chunkSeconds: Math.max(
            0,
            Math.min(300, Math.trunc(Number(s.transcription.chunkSeconds) || 0)),
          ),
        },
        speakerAnalysis: {
          ...s.speakerAnalysis,
          // Mirror the live builder's clamp+round-to-minute fixed point exactly.
          // The draft minutes are `round(canonicalSeconds / 60)`, and the live
          // request is `max(60, min(3600, (truncMinutes || 10) * 60))`. A
          // canonical value that is not a 60s multiple or is out of range
          // (e.g. 90, or a sub-30s value that rounds to 0 minutes) would
          // otherwise read perpetually dirty against that normalized request.
          timeoutSeconds: Math.max(
            60,
            Math.min(
              3600,
              (Math.round((s.speakerAnalysis?.timeoutSeconds ?? 600) / 60) || 10) * 60,
            ),
          ),
        },
      };
    case "developer":
      return {
        developerOptionsEnabled: s.developerOptionsEnabled ?? false,
        nativeCaptureDebugLoggingEnabled: s.nativeCaptureDebugLoggingEnabled ?? false,
      };
    case "access":
      return {
        askAiEnabled: s.access?.askAiEnabled ?? false,
        // Mirror the live builder's effective cap: a positive cap is clamped to
        // [1, 64]; 0 (= no cap) passes through. A persisted value above the
        // ceiling (the backend stores the cap verbatim) would otherwise read
        // perpetually dirty against the clamped request.
        askAiMaxToolCalls: clampAskAiMaxToolCalls(
          s.access?.askAiMaxToolCalls ?? ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
        ),
        askAiModel: s.access?.askAiModel ?? "",
      };
    case "ai_runtime":
      return {
        enabled: s.aiRuntime?.enabled ?? false,
        providers: (s.aiRuntime?.providers ?? []).map((p) => ({
          id: (p.id ?? "").trim() || p.kind,
          kind: p.kind,
          label: p.label ?? "",
          baseUrl: p.baseUrl ?? "",
        })),
        defaultModel: s.aiRuntime?.defaultModel
          ? {
              provider: s.aiRuntime.defaultModel.provider,
              model: s.aiRuntime.defaultModel.model,
            }
          : null,
      };
    case "user_context":
      return {
        enabled: s.userContext?.enabled ?? false,
        derivationBudgetTier:
          s.userContext?.derivationBudgetTier ?? DEFAULT_USER_CONTEXT_BUDGET_TIER,
        backfillWindowDays:
          s.userContext?.backfillWindowDays ?? DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS,
        backfillGoDeeper: s.userContext?.backfillGoDeeper ?? false,
      };
  }
}

// The diff snapshot for one draft domain (the live draft serialized). The
// privacy-exclusion domain snapshots only its excluded-apps draft.
export function buildRecDomainSnapshot(
  domain: RecordingSettingsDraftDomain,
  rec: RecordingDraftState,
): string {
  if (domain === "app_privacy_exclusion") {
    return JSON.stringify({ excludedApps: rec.draftExcludedApps });
  }
  return JSON.stringify(buildRecDomainRequest(domain, rec));
}

// The persisted-baseline snapshot for one draft domain (from canonical settings).
export function buildRecDomainSnapshotFromSettings(
  domain: RecordingSettingsDraftDomain,
  s: RecordingSettings,
): string {
  return JSON.stringify(buildRecDomainRequestFromSettings(domain, s));
}

// The "adopt canonical drafts?" decision for `syncRecordingDomainFromCanonical`.
//
// Extracted here (a pure `.ts`) so the load-bearing in-flight-save rule is
// testable in isolation — the store that calls it (`recording.svelte.ts`) is a
// runes module and cannot be instantiated under bun:test. The store passes the
// CURRENT live snapshot, the established baseline, and either:
//   • a `dispatchedSnapshot` (a save echo): adopt canonical only when the live
//     drafts STILL equal what was dispatched to `invoke` — i.e. no edit landed
//     during the flight. If they diverged, leave the newer drafts alone so the
//     reactive driver schedules a follow-up save (the edit is never dropped).
//   • no dispatched snapshot (force / external echo): the classic rule — adopt
//     on `force`, or when the domain is clean (not dirty vs its baseline).
export function computeApplyDrafts(args: {
  liveSnapshot: string;
  baseline: string | null;
  force: boolean;
  dispatchedSnapshot?: string;
}): boolean {
  if (args.dispatchedSnapshot !== undefined) {
    return args.liveSnapshot === args.dispatchedSnapshot;
  }
  const dirty = args.baseline !== null && args.liveSnapshot !== args.baseline;
  return args.force || !dirty;
}
