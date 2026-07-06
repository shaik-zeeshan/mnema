// The recording-settings draft store (Slice 3 core cutover).
//
// Owns the ~50 `draft*` fields that derive from the single canonical
// `RecordingSettings` object, the canonical itself, the per-domain persisted
// baselines, and the build/snapshot/sync/load/realtime-resync machinery that
// the page used to hold inline. The `draft*` are `$state` CLASS FIELDS so the
// page markup can two-way bind to `rec.draftFoo` (Svelte 5 generates accessors
// for class `$state` fields, so `bind:value={rec.draftFoo}` works).
//
// ── Injected dependencies (mirrors the autosave injected-closure contract) ───
// The store needs a few things that live OUTSIDE recording draft state:
//   • side effects on sync (theme, developer-options, AI-runtime refresh, debug
//     log reload) — these belong to other stores / app-runtime modules;
//   • the capture-support GATES the page derives from `captureSupport` (page
//     state, not a recording draft) for save-block validation;
//   • the autosave engine (passed to `registerAutosave`).
// All are injected so the store stays decoupled from the page and the other
// slice-2 stores.

import { invoke } from "@tauri-apps/api/core";
import { humanizeError } from "$lib/format-error";
import type {
  RecordingSettings,
  RecordingSettingsDomainUpdateResponse,
  SettingsOwnershipDomain,
  AiProviderConfig,
  AiEngineRef,
  McpServerConfig,
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
import {
  RECORDING_AUTOSAVE_DOMAINS,
  RECORDING_DRAFT_DOMAINS,
  RECORDING_DOMAIN_COMMANDS,
  RECORDING_AUTOSAVE_DEBOUNCE_MS,
  makeRecordingDomainState,
  type AutosaveRecordingDomain,
  type RecordingSettingsDraftDomain,
} from "./autosave-core";
import type { AutosaveEngine } from "./autosave.svelte";
import {
  ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
  ASK_AI_MAX_TOOL_CALL_LIMIT,
  DEFAULT_USER_CONTEXT_BUDGET_TIER,
  DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS,
  buildRecDomainRequest,
  buildRecDomainSnapshot,
  buildRecDomainSnapshotFromSettings,
  clampAskAiMaxToolCalls,
  clampTranscriptionIdleUnloadSeconds,
  clampTranscriptionChunkSeconds,
  clampOcrTesseractUpscaleFactor,
  computeApplyDrafts,
  type RecordingDomainRequest,
} from "./recording-build";
import {
  recDomainSaveBlocked,
  type RecordingValidationGates,
} from "./recording-validation";
import {
  defaultOcrModelIdForProvider,
  defaultOcrLanguageForProvider,
} from "./models-format";

export { ASK_AI_DEFAULT_TOOL_CALL_LIMIT, ASK_AI_MAX_TOOL_CALL_LIMIT, DEFAULT_USER_CONTEXT_BUDGET_TIER, DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS };
export type { RecordingDomainRequest };

const SELECTABLE_OCR_PROVIDERS: readonly OcrProvider[] = ["apple_vision", "tesseract"];
function isSelectableOcrProvider(value: string | null | undefined): value is OcrProvider {
  return SELECTABLE_OCR_PROVIDERS.includes(value as OcrProvider);
}

// Deep-copy an MCP connector so edits to the draft never mutate the loaded
// settings snapshot (nested `args`/`env` are fresh arrays/objects).
export function cloneMcpServer(server: McpServerConfig): McpServerConfig {
  return {
    id: server.id,
    label: server.label ?? "",
    enabled: server.enabled ?? false,
    transport: server.transport,
    // Carry the http auth mode (ADR 0051) through the load clone so an OAuth
    // connector stays http+oauth in the draft (drives its lifecycle row).
    authMode: server.authMode,
    command: server.command ?? null,
    args: [...(server.args ?? [])],
    env: (server.env ?? []).map((e) => ({ name: e.name, value: e.value })),
    url: server.url ?? null,
    secretEnvName: server.secretEnvName ?? null,
    enabledTools: server.enabledTools ? [...server.enabledTools] : null,
  };
}

// Side-effect + gate dependencies injected from the page / sibling stores.
export interface RecordingStoreDeps {
  // App-wide theme runtime (lib/theme.svelte): apply the loaded appearance.
  setAppearance: (value: AppearanceSetting) => void;
  // Developer-options runtime (lib/developer-options.svelte): gate the Debug page.
  setDeveloperOptionsEnabled: (value: boolean) => void;
  // Reload the debug-log status (page-owned loader) after a developer-domain sync.
  loadDebugLogStatus: () => void;
  // AI-runtime store refreshers, re-run after an ai_runtime-domain sync so the
  // key-presence badges + availability reflect the synced provider list.
  refreshAiProviderKeyPresence: () => void;
  // Re-check which MCP connectors have a keychain secret after an ai_runtime sync.
  refreshMcpServerSecretPresence: () => void;
  loadAiRuntimeStatus: () => void;
  // Re-check Ask AI availability after an ai_runtime-domain sync so its readiness
  // pill reflects the synced provider list / default model (sibling store).
  loadAskAiAvailability: () => void;
  // The capture-support-derived save-block gates (page state).
  gates: () => RecordingValidationGates;
  // Run once the canonical recording settings (incl. the persisted
  // semantic-search selection) have just landed from a full load. The page-owned
  // semantic-search picker re-seeds its selection here, closing the init race
  // where the picker status resolved before settings (so the picker would read a
  // still-null `semanticSearchSelectedModelId` and never re-seed).
  onRecordingSettingsLoaded?: () => void;
}

export class RecordingStore {
  // ─── Canonical + load/error state ─────────────────────────────────────────
  recordingSettings = $state<RecordingSettings | null>(null);
  loadingRecSettings = $state(false);
  recError = $state<string | null>(null);
  // True only once loadRecordingSettings() has resolved at least once, so the
  // persisted semantic-search selection is known (the page's chooser gates on it).
  recordingSettingsLoaded = $state(false);
  // Transient "saved!" flash, mirrored from the page's prior `recSaved`.
  recSaved = $state(false);

  // Per-domain in-flight save flags + the last successfully-persisted snapshots.
  savingRecDomains = $state<Record<RecordingSettingsDraftDomain, boolean>>(
    makeRecordingDomainState(false),
  );
  lastSavedRecSnapshots = $state<Record<RecordingSettingsDraftDomain, string | null>>(
    makeRecordingDomainState<string | null>(null),
  );

  // ─── Recording-settings drafts ────────────────────────────────────────────
  draftCaptureScreen = $state(true);
  draftCaptureMicrophone = $state(false);
  draftCaptureSystemAudio = $state(false);
  draftSegmentDuration = $state(60);
  draftFrameRate = $state(0.5);
  draftSaveDirectory = $state("");
  draftAutoStart = $state(false);

  // Resolution drafts
  draftResolutionMode = $state<ResolutionMode>("original");
  draftResolutionPreset = $state<ResolutionPreset>("1080p");
  draftCustomWidth = $state<number | null>(null);
  draftCustomHeight = $state<number | null>(null);
  customWidthRaw = $state("");
  customHeightRaw = $state("");

  // Video bitrate drafts
  draftBitrateMode = $state<VideoBitrateMode>("preset");
  draftBitratePreset = $state<VideoBitratePreset>("medium");
  draftCustomMbpsRaw = $state("");
  draftCustomMbps = $state<number | null>(null);

  // Inactivity drafts
  draftPauseCaptureOnInactivity = $state(false);
  draftIdleTimeoutSeconds = $state(30);
  draftMicrophoneActivitySensitivity = $state(50);
  draftSystemAudioActivitySensitivity = $state(50);
  draftMicrophoneVadAdapter = $state<MicrophoneVadAdapter>("silero");

  // Developer drafts
  draftNativeCaptureDebugLoggingEnabled = $state(false);
  draftDeveloperOptionsEnabled = $state(false);

  // Processing: preview cache TTL (seconds; 0 disables)
  draftPreviewCacheTtlSeconds = $state(3600);

  // Display / storage / metadata drafts
  draftFollowTimelineLive = $state(false);
  draftRetentionPolicy = $state<RetentionPolicy>("never");
  draftMetadataEnabled = $state(true);
  draftBrowserUrlMode = $state<BrowserUrlMode>("sanitized");
  draftAppearance = $state<AppearanceSetting>("system");

  // Privacy-exclusion draft (committed through the dedicated controller).
  draftExcludedApps = $state<ExcludedAppEntry[]>([]);

  // Access drafts (Ask AI). Tool-call cap: persisted as a single number where
  // 0 = no cap; the UI splits it into a "limit on/off" toggle + numeric value.
  draftAskAiEnabled = $state(false);
  draftAskAiWebFetchEnabled = $state(false);
  draftAskAiLimitToolCalls = $state(true);
  draftAskAiMaxToolCalls = $state(ASK_AI_DEFAULT_TOOL_CALL_LIMIT);
  draftAskAiModel = $state("");

  // AI runtime drafts (ADR 0034). The per-provider key is keychain-only and
  // never travels through this draft state.
  draftAiEnabled = $state(false);
  draftAiProviders = $state<AiProviderConfig[]>([]);
  draftAiDefaultModel = $state<AiEngineRef | null>(null);
  // MCP tool connectors (Workstream C). The per-server secret is keychain-only
  // and never travels through this draft state.
  draftMcpServers = $state<McpServerConfig[]>([]);

  // User Context (derivation) drafts.
  draftUserContextBudgetTier = $state<DerivationBudgetTier>(DEFAULT_USER_CONTEXT_BUDGET_TIER);
  draftUserContextBackfillWindowDays = $state(DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS);
  draftUserContextBackfillGoDeeper = $state(false);
  draftUserContextEnabled = $state(false);

  // OCR drafts
  draftOcrEnabled = $state(true);
  draftOcrProvider = $state<OcrProvider>("apple_vision");
  draftOcrModelId = $state<string | null>(null);
  draftOcrLanguage = $state("");
  draftOcrRecognitionMode = $state<OcrRecognitionMode>("fast");
  draftOcrLanguageCorrection = $state(false);
  draftOcrTesseractPageSegmentationMode = $state<OcrTesseractPageSegmentationMode>("single_block");
  draftOcrTesseractPreprocessMode = $state<OcrTesseractPreprocessMode>("grayscale");
  draftOcrTesseractUpscaleFactor = $state(1);
  draftOcrTesseractCharWhitelist = $state("");

  // Transcription + speaker drafts
  draftTranscriptionEnabled = $state(true);
  draftTranscriptionMicrophoneEnabled = $state(true);
  draftTranscriptionSystemAudioEnabled = $state(false);
  draftTranscriptionProvider = $state<AudioTranscriptionProvider>("local_whisper");
  draftTranscriptionModelId = $state<string | null>("base");
  draftTranscriptionLanguage = $state("auto");
  draftTranscriptionMemoryMode = $state<AudioTranscriptionMemoryMode>("balanced");
  draftTranscriptionIdleUnloadSeconds = $state(300);
  draftTranscriptionChunkSeconds = $state(30);
  draftSpeakerSeparateSpeakers = $state(false);
  draftSpeakerRecognizeSavedPeople = $state(false);
  draftSpeakerProvider = $state("speakrs");
  draftSpeakerModelId = $state<string | null>("pyannote-community-1-wespeaker");
  draftSpeakerTimeoutMinutes = $state(10);

  // Semantic search: the persisted (sticky) selected model id. Switching it is a
  // confirmed re-index action the page drives; the draft only moves after confirm.
  draftSemanticSearchEnabled = $state(true);
  semanticSearchSelectedModelId = $state<string | null>(null);

  // Effective persisted tool-call cap: 0 when the cap is off, else the chosen
  // number clamped to [1, 64] (the runtime ceiling, MULTI_TURN_CEILING). Floored
  // to 1 so an empty/invalid input never silently becomes unlimited, and capped
  // so a typed 9999 can't persist past what the engine honors. Shares the
  // `clampAskAiMaxToolCalls` fixed point with the canonical baseline builder.
  effectiveAskAiMaxToolCalls = $derived(
    this.draftAskAiLimitToolCalls
      ? clampAskAiMaxToolCalls(this.draftAskAiMaxToolCalls || ASK_AI_DEFAULT_TOOL_CALL_LIMIT)
      : 0,
  );

  readonly #deps: RecordingStoreDeps;

  constructor(deps: RecordingStoreDeps) {
    this.#deps = deps;
  }

  // ─── Snapshot / baseline / validation (delegating to pure modules) ────────
  buildRecDomainRequest(domain: AutosaveRecordingDomain): RecordingDomainRequest {
    return buildRecDomainRequest(domain, this);
  }

  buildRecDomainSnapshot(domain: RecordingSettingsDraftDomain): string {
    return buildRecDomainSnapshot(domain, this);
  }

  setRecDomainBaseline(domain: RecordingSettingsDraftDomain, s: RecordingSettings): void {
    this.lastSavedRecSnapshots = {
      ...this.lastSavedRecSnapshots,
      [domain]: buildRecDomainSnapshotFromSettings(domain, s),
    };
  }

  recDomainSaveBlocked(domain: AutosaveRecordingDomain): boolean {
    return recDomainSaveBlocked(domain, this, this.#deps.gates());
  }

  // ─── Per-domain draft sync (from canonical settings) ──────────────────────
  syncCaptureSourceDrafts(s: RecordingSettings): void {
    this.draftCaptureScreen = s.captureScreen;
    this.draftCaptureMicrophone = s.captureMicrophone;
    this.draftCaptureSystemAudio = s.captureSystemAudio;
  }

  syncCaptureTimingDrafts(s: RecordingSettings): void {
    this.draftSegmentDuration = s.segmentDurationSeconds;
    this.draftAutoStart = s.autoStart;
  }

  syncVideoDrafts(s: RecordingSettings): void {
    this.draftFrameRate = s.screenFrameRate;
    if (s.screenResolution.mode === "custom") {
      this.draftResolutionMode = "custom";
      this.draftCustomWidth = s.screenResolution.width;
      this.draftCustomHeight = s.screenResolution.height;
      this.customWidthRaw = String(s.screenResolution.width);
      this.customHeightRaw = String(s.screenResolution.height);
    } else if (s.screenResolution.preset === "original") {
      this.draftResolutionMode = "original";
      this.draftResolutionPreset = "1080p";
      this.draftCustomWidth = null;
      this.draftCustomHeight = null;
      this.customWidthRaw = "";
      this.customHeightRaw = "";
    } else {
      this.draftResolutionMode = "preset";
      this.draftResolutionPreset = s.screenResolution.preset;
      this.draftCustomWidth = null;
      this.draftCustomHeight = null;
      this.customWidthRaw = "";
      this.customHeightRaw = "";
    }
    if (s.videoBitrate.mode === "custom") {
      this.draftBitrateMode = "custom";
      this.draftBitratePreset = "medium";
      this.draftCustomMbps = s.videoBitrate.customMbps;
      this.draftCustomMbpsRaw = String(s.videoBitrate.customMbps);
    } else {
      this.draftBitrateMode = "preset";
      this.draftBitratePreset = s.videoBitrate.preset;
      this.draftCustomMbps = null;
      this.draftCustomMbpsRaw = "";
    }
  }

  syncStorageDrafts(s: RecordingSettings): void {
    this.draftSaveDirectory = s.saveDirectory;
    this.draftRetentionPolicy = s.retentionPolicy ?? "never";
  }

  syncDisplayDrafts(s: RecordingSettings): void {
    this.draftFollowTimelineLive = s.followTimelineLive ?? false;
    this.draftAppearance = s.appearance ?? "system";
  }

  syncMetadataDrafts(s: RecordingSettings): void {
    this.draftMetadataEnabled = s.metadata?.enabled ?? true;
    this.draftBrowserUrlMode = s.metadata?.browserUrlMode ?? "sanitized";
  }

  syncPrivacyDrafts(s: RecordingSettings): void {
    this.draftExcludedApps = [...(s.privacy?.excludedApps ?? [])];
  }

  syncAccessDrafts(s: RecordingSettings): void {
    this.draftAskAiEnabled = s.access?.askAiEnabled ?? false;
    this.draftAskAiWebFetchEnabled = s.access?.askAiWebFetchEnabled ?? false;
    const cap = s.access?.askAiMaxToolCalls ?? ASK_AI_DEFAULT_TOOL_CALL_LIMIT;
    this.draftAskAiLimitToolCalls = cap > 0;
    this.draftAskAiMaxToolCalls = cap > 0 ? cap : ASK_AI_DEFAULT_TOOL_CALL_LIMIT;
    this.draftAskAiModel = s.access?.askAiModel ?? "";
  }

  syncAiRuntimeDrafts(s: RecordingSettings): void {
    this.draftAiEnabled = s.aiRuntime?.enabled ?? false;
    this.draftAiProviders = (s.aiRuntime?.providers ?? []).map((p) => ({
      // Backfill a legacy provider (saved before instance ids) to id === kind.
      id: (p.id ?? "").trim() || p.kind,
      kind: p.kind,
      label: p.label ?? "",
      baseUrl: p.baseUrl ?? "",
    }));
    this.draftAiDefaultModel = s.aiRuntime?.defaultModel
      ? { provider: s.aiRuntime.defaultModel.provider, model: s.aiRuntime.defaultModel.model }
      : null;
    this.draftMcpServers = (s.aiRuntime?.mcpServers ?? []).map(cloneMcpServer);
  }

  syncUserContextDrafts(s: RecordingSettings): void {
    this.draftUserContextEnabled = s.userContext?.enabled ?? false;
    this.draftUserContextBudgetTier =
      s.userContext?.derivationBudgetTier ?? DEFAULT_USER_CONTEXT_BUDGET_TIER;
    this.draftUserContextBackfillWindowDays =
      s.userContext?.backfillWindowDays ?? DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS;
    this.draftUserContextBackfillGoDeeper = s.userContext?.backfillGoDeeper ?? false;
  }

  syncInactivityDrafts(s: RecordingSettings): void {
    this.draftPauseCaptureOnInactivity = s.pauseCaptureOnInactivity;
    this.draftIdleTimeoutSeconds = s.idleTimeoutSeconds;
    this.draftMicrophoneActivitySensitivity = s.microphoneActivitySensitivity ?? 50;
    this.draftSystemAudioActivitySensitivity = s.systemAudioActivitySensitivity ?? 50;
    this.draftMicrophoneVadAdapter =
      s.audioSpeechDetection?.detector ?? s.microphoneVadAdapter ?? "silero";
  }

  syncDeveloperDrafts(s: RecordingSettings): void {
    this.draftNativeCaptureDebugLoggingEnabled = s.nativeCaptureDebugLoggingEnabled ?? false;
    this.draftDeveloperOptionsEnabled = s.developerOptionsEnabled ?? false;
  }

  syncProcessingDrafts(s: RecordingSettings): void {
    this.draftPreviewCacheTtlSeconds = s.previewCacheTtlSeconds ?? 3600;
    this.draftOcrEnabled = s.ocr?.enabled ?? true;
    const loadedOcrProvider = s.ocr?.provider;
    const loadedOcrProviderSelectable = isSelectableOcrProvider(loadedOcrProvider);
    this.draftOcrProvider = loadedOcrProviderSelectable ? loadedOcrProvider : "apple_vision";
    this.draftOcrModelId = loadedOcrProviderSelectable
      ? (s.ocr?.modelId ?? defaultOcrModelIdForProvider(this.draftOcrProvider))
      : defaultOcrModelIdForProvider(this.draftOcrProvider);
    this.draftOcrLanguage = loadedOcrProviderSelectable
      ? (s.ocr?.language ?? defaultOcrLanguageForProvider(this.draftOcrProvider) ?? "")
      : (defaultOcrLanguageForProvider(this.draftOcrProvider) ?? "");
    this.draftOcrRecognitionMode = s.ocr?.recognitionMode ?? "fast";
    this.draftOcrLanguageCorrection = s.ocr?.languageCorrection ?? false;
    this.draftOcrTesseractPageSegmentationMode =
      s.ocr?.tesseractPageSegmentationMode ?? "single_block";
    this.draftOcrTesseractPreprocessMode = s.ocr?.tesseractPreprocessMode ?? "grayscale";
    // Clamp on load to the SAME [1,4] ceiling `buildProcessingRequest` uses, so
    // an out-of-band-persisted value (older build / CLI) renders in the Stepper
    // as the effective value, not the unclamped raw one.
    this.draftOcrTesseractUpscaleFactor = clampOcrTesseractUpscaleFactor(
      s.ocr?.tesseractUpscaleFactor ?? 1,
    );
    this.draftOcrTesseractCharWhitelist = s.ocr?.tesseractCharWhitelist ?? "";
    this.draftTranscriptionEnabled = s.transcription?.enabled ?? true;
    this.draftTranscriptionMicrophoneEnabled = s.transcription?.microphoneEnabled ?? true;
    this.draftTranscriptionSystemAudioEnabled = s.transcription?.systemAudioEnabled ?? false;
    this.draftTranscriptionProvider = s.transcription?.provider ?? "local_whisper";
    this.draftTranscriptionModelId =
      s.transcription?.modelId ??
      (this.draftTranscriptionProvider === "apple_speech_on_device"
        ? null
        : this.draftTranscriptionProvider === "deepgram"
          ? "nova-3"
          : "base");
    this.draftTranscriptionLanguage = s.transcription?.language ?? "auto";
    this.draftTranscriptionMemoryMode = s.transcription?.memoryMode ?? "balanced";
    // Clamp idle/chunk on load to the SAME ceilings `buildProcessingRequest`
    // applies ([0,1800] / [0,300]), so the Stepper shows the effective value an
    // out-of-band-persisted setting actually resolves to (not the raw value).
    this.draftTranscriptionIdleUnloadSeconds = clampTranscriptionIdleUnloadSeconds(
      s.transcription?.idleUnloadSeconds ?? 300,
    );
    this.draftTranscriptionChunkSeconds = clampTranscriptionChunkSeconds(
      s.transcription?.chunkSeconds ?? 30,
    );
    this.draftSpeakerSeparateSpeakers = s.speakerAnalysis?.separateSpeakers ?? false;
    this.draftSpeakerRecognizeSavedPeople = s.speakerAnalysis?.recognizeSavedPeople ?? false;
    // Coerce legacy saved values: the sherpa_onnx provider (and its model ids)
    // no longer exist, so old users' saved settings must resolve to the speakrs
    // default — otherwise the preset picker would select a provider/model the
    // backend manifest never returns. When the saved provider is legacy (or
    // absent) we drop its stale model id too and fall back to the speakrs default.
    const savedSpeakerProvider = s.speakerAnalysis?.provider;
    const isLegacySpeakerProvider =
      !savedSpeakerProvider || savedSpeakerProvider === "sherpa_onnx";
    this.draftSpeakerProvider = isLegacySpeakerProvider ? "speakrs" : savedSpeakerProvider;
    this.draftSpeakerModelId = isLegacySpeakerProvider
      ? "pyannote-community-1-wespeaker"
      : (s.speakerAnalysis?.modelId ?? "pyannote-community-1-wespeaker");
    this.draftSpeakerTimeoutMinutes = Math.round((s.speakerAnalysis?.timeoutSeconds ?? 600) / 60);
    this.draftSemanticSearchEnabled = s.semanticSearch?.enabled ?? true;
    this.semanticSearchSelectedModelId = s.semanticSearch?.modelId ?? null;
  }

  // Semantic-search enable + selected-model drafts. These live in the
  // `processing` sync above but ALSO arrive on their own `semantic_search`
  // ownership domain (committed through `update_semantic_search_settings` /
  // `select_semantic_search_model`, not the generic autosave engine), which is
  // NOT a recording draft domain — so the generic per-domain sync skips it. This
  // helper is the seam a cross-window `semantic_search` echo refreshes through.
  syncSemanticSearchDrafts(s: RecordingSettings): void {
    this.draftSemanticSearchEnabled = s.semanticSearch?.enabled ?? true;
    this.semanticSearchSelectedModelId = s.semanticSearch?.modelId ?? null;
  }

  syncRecDomainDrafts(domain: RecordingSettingsDraftDomain, s: RecordingSettings): void {
    switch (domain) {
      case "capture_sources":
        this.syncCaptureSourceDrafts(s);
        break;
      case "capture_timing":
        this.syncCaptureTimingDrafts(s);
        break;
      case "video":
        this.syncVideoDrafts(s);
        break;
      case "storage":
        this.syncStorageDrafts(s);
        break;
      case "display":
        this.syncDisplayDrafts(s);
        break;
      case "metadata":
        this.syncMetadataDrafts(s);
        break;
      case "app_privacy_exclusion":
        this.syncPrivacyDrafts(s);
        break;
      case "inactivity":
        this.syncInactivityDrafts(s);
        break;
      case "processing":
        this.syncProcessingDrafts(s);
        break;
      case "developer":
        this.syncDeveloperDrafts(s);
        break;
      case "access":
        this.syncAccessDrafts(s);
        break;
      case "ai_runtime":
        this.syncAiRuntimeDrafts(s);
        break;
      case "user_context":
        this.syncUserContextDrafts(s);
        break;
    }
  }

  // Sync every draft domain + establish its baseline (called after a full load).
  syncRecDrafts(s: RecordingSettings): void {
    for (const domain of RECORDING_DRAFT_DOMAINS) {
      this.syncRecDomainDrafts(domain, s);
      this.setRecDomainBaseline(domain, s);
    }
  }

  // Re-apply one domain's drafts + baseline from the canonical settings. Dirty
  // domains keep their in-flight edits (only the baseline refreshes) unless
  // `force` is set (a save/privacy echo that should adopt the persisted truth).
  // Mirrors the prior page behavior including the per-domain side effects.
  //
  // `opts.dispatchedSnapshot` is the save-echo seam: a domain save serializes the
  // drafts it shipped to `invoke` and passes that string here on success. The
  // baseline ALWAYS refreshes (the canonical truth is now persisted), but the
  // drafts are only clobbered back to canonical when the live drafts STILL equal
  // what was dispatched — i.e. the user did not edit during the save's flight. If
  // they diverged (edit C arrived mid-save), we leave the newer drafts alone so
  // the reactive driver schedules a follow-up save instead of losing the edit.
  // `force` (no dispatched snapshot) keeps the old unconditional-clobber meaning
  // for the privacy echo, which has no in-flight-edit window to protect.
  syncRecordingDomainFromCanonical(
    domain: SettingsOwnershipDomain,
    s: RecordingSettings,
    opts: boolean | { force?: boolean; dispatchedSnapshot?: string } = false,
  ): void {
    if (!RECORDING_DRAFT_DOMAINS.includes(domain as RecordingSettingsDraftDomain)) {
      // `semantic_search` is a settings ownership domain but NOT a generic
      // recording draft domain (it commits through its own command), so it falls
      // through here. Still refresh its local drafts so a cross-window enable/
      // model change isn't left stale on the toggle + picker.
      if (domain === "semantic_search") this.syncSemanticSearchDrafts(s);
      return;
    }
    const draftDomain = domain as RecordingSettingsDraftDomain;
    const force = typeof opts === "boolean" ? opts : (opts.force ?? false);
    const dispatchedSnapshot = typeof opts === "boolean" ? undefined : opts.dispatchedSnapshot;

    // Whether to overwrite the live drafts with canonical. The pure decision —
    // unit-tested in `recording-build.computeApplyDrafts` — is: on a save echo,
    // adopt only when the live drafts still equal what was dispatched (no edit C
    // mid-flight); otherwise adopt on force or when the domain is clean.
    const applyDrafts = computeApplyDrafts({
      liveSnapshot: this.buildRecDomainSnapshot(draftDomain),
      baseline: this.lastSavedRecSnapshots[draftDomain],
      force,
      dispatchedSnapshot,
    });

    if (applyDrafts) {
      this.syncRecDomainDrafts(draftDomain, s);
    }
    this.setRecDomainBaseline(draftDomain, s);

    if (draftDomain === "display" && applyDrafts) {
      this.#deps.setAppearance(s.appearance ?? "system");
    }
    if (draftDomain === "developer" && applyDrafts) {
      this.#deps.setDeveloperOptionsEnabled(s.developerOptionsEnabled ?? false);
      this.#deps.loadDebugLogStatus();
    }
    if (draftDomain === "ai_runtime" && applyDrafts) {
      this.#deps.refreshAiProviderKeyPresence();
      this.#deps.refreshMcpServerSecretPresence();
      this.#deps.loadAiRuntimeStatus();
      this.#deps.loadAskAiAvailability();
    }
  }

  // A domain-less `recording_settings_changed` carries no per-domain payload, so
  // resync every domain's drafts + baselines. Dirty domains keep their in-flight
  // edits (baseline refreshes only) so a later same-domain save doesn't ship
  // stale companion fields back over the external change. App-wide appearance /
  // developer-mode side effects are handled by the dedicated stores listening on
  // the same event, so we skip them (and their per-domain IPC) here.
  resyncRecordingDraftsFromCanonical(s: RecordingSettings): void {
    for (const domain of RECORDING_DRAFT_DOMAINS) {
      const baseline = this.lastSavedRecSnapshots[domain];
      const dirty = baseline !== null && this.buildRecDomainSnapshot(domain) !== baseline;
      if (!dirty) {
        this.syncRecDomainDrafts(domain, s);
      }
      this.setRecDomainBaseline(domain, s);
    }
  }

  // ─── Load ─────────────────────────────────────────────────────────────────
  async loadRecordingSettings(): Promise<void> {
    this.loadingRecSettings = true;
    this.recError = null;
    try {
      const s = await invoke<RecordingSettings>("get_recording_settings");
      this.recordingSettings = s;
      this.syncRecDrafts(s);
      // Settings (incl. the persisted semantic-search selection) are now known.
      this.recordingSettingsLoaded = true;
      this.#deps.onRecordingSettingsLoaded?.();
    } catch (err) {
      this.recError = humanizeError(err);
    } finally {
      this.loadingRecSettings = false;
    }
  }

  // ─── Realtime listener handlers (registered from the page mount effect) ────
  // The page owns the `listen(...)` registration (entangled with mic/about/
  // privacy listeners); these are the recording-specific handler bodies.
  onRecordingSettingsChanged(payload: RecordingSettings): void {
    this.recordingSettings = payload;
    this.resyncRecordingDraftsFromCanonical(payload);
    this.recError = null;
  }

  onRecordingSettingsDomainChanged(payload: RecordingSettingsDomainUpdateResponse): void {
    this.recordingSettings = payload.settings;
    this.syncRecordingDomainFromCanonical(payload.domain, payload.settings);
    this.recError = null;
  }

  // ─── Autosave registration ────────────────────────────────────────────────
  // Register one engine unit per autosave domain. Each unit hands the engine
  // ONLY closures reading this store, so the engine never reads draft state.
  // `save` is injected because the per-domain save (retention confirm, etc.)
  // lives in the page; we pass it in to keep that page-coupled flow intact.
  registerAutosave(
    engine: AutosaveEngine,
    save: (domain: AutosaveRecordingDomain) => void | Promise<void>,
  ): void {
    for (const domain of RECORDING_AUTOSAVE_DOMAINS) {
      engine.register({
        key: domain,
        debounceMs: RECORDING_AUTOSAVE_DEBOUNCE_MS,
        snapshot: () => this.buildRecDomainSnapshot(domain),
        baseline: () => this.lastSavedRecSnapshots[domain],
        blocked: () => this.recDomainSaveBlocked(domain),
        saving: () => this.savingRecDomains[domain],
        save: () => save(domain),
      });
    }
  }
}

export function createRecordingStore(deps: RecordingStoreDeps): RecordingStore {
  return new RecordingStore(deps);
}
