// Settings page controller — Slice-5 shell-ification.
//
// One controller instance owns every settings store + the page-local
// derivations/helpers that did not already live in a domain store. The shell
// (`routes/settings/+page.svelte`) builds it once, runs the mount/autosave/
// realtime effects against it, and shares it via Svelte context
// (`setSettingsController`). Each panel reads it via `getSettingsController()`
// and destructures the exact names its (verbatim) markup references, so panel
// markup is a byte-for-byte move of the legacy `{#if activeTab === ...}` blocks.
//
// This is a behavior-preserving consolidation: the helpers/derivations are a
// 1:1 port of the page-local code, only re-homed onto a class so multiple
// panels can share the single live instance.

import { getContext, setContext } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import { ask, confirm } from "@tauri-apps/plugin-dialog";
import { humanizeError } from "$lib/format-error";
import { retentionToDays } from "$lib/components/retention";
import ModelPickerMenu from "$lib/insights/ModelPickerMenu.svelte";
import { ModelPoolLoader } from "$lib/insights/modelPool.svelte";
import { setAppearance } from "$lib/theme.svelte";
import { setDeveloperOptionsEnabled } from "$lib/developer-options.svelte";
import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
import {
  RECORDING_DRAFT_DOMAINS,
  RECORDING_DOMAIN_COMMANDS,
  type AutosaveRecordingDomain,
} from "./autosave-core";
import { createAutosaveEngine } from "./autosave.svelte";
import { createRecordingStore } from "./recording.svelte";
import {
  recValidationErrors as recValidationErrorsFn,
  recSaveBlocked as recSaveBlockedFn,
  customResolutionErrors as customResolutionErrorsFn,
  customResolutionBlocked as customResolutionBlockedFn,
  customBitrateErrors as customBitrateErrorsFn,
  customBitrateBlocked as customBitrateBlockedFn,
  parseCustomDimension,
} from "./recording-validation";
import { createCliAccessStore } from "./cli-access.svelte";
import { createGeckoUrlAccessStore } from "./gecko-url-access.svelte";
import { createSystemAudioAccessStore } from "./system-audio-access.svelte";
import { createLogsStore } from "./logs.svelte";
import { createAskAiStore, askAiReasonLabel as askAiReasonLabelCore } from "./ask-ai.svelte";
import { createAboutStore } from "./about.svelte";
import { createUserContextStore } from "./user-context.svelte";
import { createAiRuntimeStore } from "./ai-runtime.svelte";
import { createModelStatusStore } from "./model-status.svelte";
import { createAudioStore } from "./audio.svelte";
import { createKeyboardStore } from "./keyboard.svelte";
import { createProcessingModelsView } from "./controller-processing.svelte";
import { createSemanticSearchView } from "./controller-semantic-search.svelte";
import {
  AI_PROVIDER_KINDS,
  CLOUD_AI_PROVIDER_KINDS,
  AI_LOCAL_DEFAULT_ENDPOINTS,
  isCloudAiProviderKind,
  aiProviderKindLabel,
  aiProviderKindDescription,
  baseUrlHost,
  aiProviderInstanceLabel,
  newAiProviderId,
} from "./ai-providers";
import { createMcpConnectorActions } from "./controller-mcp";
import type { McpPreset } from "./mcp-presets";
import type {
  CaptureSupport,
  OcrProvider,
  RecordingSettings,
  RecordingSettingsDomainUpdateResponse,
  AiProviderKind,
  AiProviderConfig,
  AiEngineRef,
  McpServerConfig,
  BrowserUrlMode,
  SemanticSearchModelStatus,
} from "$lib/types";

export type RetentionCleanupSummary = {
  policy: string;
  cutoffEndedBefore: string | null;
  eligibleCaptureSegments: number;
  deletedCaptureSegments: number;
  deletedFrames: number;
  deletedAudioSegments: number;
  deletedProcessingJobs: number;
  deletedProcessingResults: number;
  skippedRunningJobs: number;
  skippedActiveSegments: number;
  pendingFileTombstones: number;
};

export class SettingsController {
  // Re-exported so markup/components can reference these constants verbatim.
  readonly ModelPickerMenu = ModelPickerMenu;
  readonly AI_PROVIDER_KINDS = AI_PROVIDER_KINDS;
  readonly CLOUD_AI_PROVIDER_KINDS = CLOUD_AI_PROVIDER_KINDS;
  readonly AI_LOCAL_DEFAULT_ENDPOINTS = AI_LOCAL_DEFAULT_ENDPOINTS;

  // ─── Stores ────────────────────────────────────────────────────────────────
  rec = createRecordingStore({
    setAppearance: (value) => setAppearance(value),
    setDeveloperOptionsEnabled: (value) => setDeveloperOptionsEnabled(value),
    loadDebugLogStatus: () => this.logs.loadDebugLogStatus(),
    refreshAiProviderKeyPresence: () => void this.aiRuntime.refreshAiProviderKeyPresence(),
    refreshMcpServerSecretPresence: () => void this.aiRuntime.refreshMcpServerSecretPresence(),
    loadAiRuntimeStatus: () => void this.aiRuntime.loadAiRuntimeStatus(),
    loadAskAiAvailability: () => void this.askAi.loadAskAiAvailability(),
    gates: () => ({ resolutionSupportPendingForNonOriginal: this.resolutionSupportPendingForNonOriginal }),
    // Re-seed the semantic-search picker once settings land — closes the init
    // race where the picker status resolved before recording settings, leaving
    // the picker blank while a model is actually persisted. Dirty-guarded.
    onRecordingSettingsLoaded: () => this.reseedSemanticSearchPickedModel(),
  });
  cliAccess = createCliAccessStore();
  // Optional Gecko (Firefox/Zen) browser-URL access — surfaced in the capture
  // Privacy panel, loaded on mount and re-polled on window focus.
  geckoUrlAccess = createGeckoUrlAccessStore();
  // The inferred "system audio may be blocked" hint (ADR 0052) — surfaced in the
  // capture panel, loaded on mount and re-polled on window focus (the evidence
  // only moves while a recording is running).
  systemAudioAccess = createSystemAudioAccessStore();
  logs = createLogsStore();
  askAi = createAskAiStore();
  about = createAboutStore();
  aiRuntime = createAiRuntimeStore({
    getProviders: () => this.rec.draftAiProviders,
    getMcpServers: () => this.rec.draftMcpServers,
    isCloudProviderKind: (kind) => this.isCloudAiProviderKind(kind),
    labelForProvider: (id) => this.aiProviderLabelById(id),
    loadAskAiAvailability: () => void this.askAi.loadAskAiAvailability(),
  });
  userContext = createUserContextStore({
    onWiped: () => {
      void this.aiRuntime.loadAiRuntimeStatus();
    },
  });
  models = createModelStatusStore();
  audio = createAudioStore();
  keyboard = createKeyboardStore();

  // The shared debounced autosave engine.
  autosaveEngine = createAutosaveEngine({
    privacyCommandInFlight: () => this.appPrivacyExclusion.commandInFlight,
  });

  appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => this.rec.draftExcludedApps,
    onSettingsUpdated: (response) => {
      this.rec.recordingSettings = response.settings;
      this.rec.syncRecordingDomainFromCanonical(response.domain, response.settings, true);
    },
    setError: (message) => {
      this.rec.recError = message;
    },
    beforePrivacyCommand: () => {
      this.autosaveEngine.cancelAll();
    },
    enableExistingUserPrompt: true,
  });

  // ─── Capture support ─────────────────────────────────────────────────────
  captureSupport = $state<CaptureSupport | null>(null);
  captureSupportLoading = $state(false);
  captureSupportFailed = $state(false);

  retentionCleanupSummary = $state<RetentionCleanupSummary | null>(null);
  retentionCleanupRunning = $state(false);
  retentionCleanupError = $state<string | null>(null);

  // ─── Autosave failure surfacing + exponential backoff ──────────────────────
  // A failed recording-domain save leaves the domain dirty, so the autosave
  // engine's re-tick (on the `savingRecDomains` flag flipping back to false)
  // would re-fire the save every ~450ms forever — a silent hammer. These track
  // the failed domain (so the rail footer can offer a targeted Retry/Dismiss),
  // the consecutive-failure count, and the per-domain "don't retry before"
  // timestamp that throttles the loop with capped exponential backoff.
  lastFailedSaveDomain = $state<AutosaveRecordingDomain | null>(null);
  recSaveFailureCount = $state<Record<string, number>>({});
  recSaveBackoffUntil = $state<Record<string, number>>({});
  // One-shot backoff re-tick timers, tracked per domain so teardown can cancel
  // them — an orphaned timer would re-arm saves (and tick the engine) on a
  // detached controller after Settings closes.
  private recSaveRetryTimers = new Map<
    AutosaveRecordingDomain,
    ReturnType<typeof setTimeout>
  >();
  // The domain whose save most recently succeeded — drives a near-the-control
  // "Saved" micro-affordance (the rail footer status is remote from the edit).
  recSavedDomain = $state<AutosaveRecordingDomain | null>(null);

  // Ask AI / AI model picker open state.
  askAiModelOpen = $state(false);
  aiModelOpen = $state(false);

  // True while a provider removal (incl. its awaited keychain clear) is in
  // flight. The add-provider control reads this and stays disabled so a new
  // provider can't be added mid-clear and race a same-kind id re-add (ADR 0035)
  // into a false "key in keychain" probe.
  aiProviderRemoving = $state(false);

  // Access prompt + section ref (focus deeplink target).
  brokerAuthorizationPromptVisible = $state(false);
  agentAccessSection = $state<HTMLElement | null>(null);

  // ─── Shared model-pool loader ──────────────────────────────────────────────
  settingsModelLoader = new ModelPoolLoader();

  // ─── Constructor: wire ai_runtime label helpers that the store needs at
  //     construction can't read `this` yet, so bind lazily via arrow closures
  //     above. Nothing to do here. ──────────────────────────────────────────
  constructor() {}

  // ─── Backend capability derivations ────────────────────────────────────────
  nativeCaptureUnsupported = $derived(
    this.captureSupport !== null && !this.captureSupport.nativeCaptureSupported,
  );
  onlyOriginalResolutionSupported = $derived(
    this.captureSupport !== null
      && this.captureSupport.nativeCaptureSupported
      && !this.captureSupport.supportedSources.systemAudio,
  );
  nonOriginalResolutionSupported = $derived(
    this.captureSupport !== null
      && this.captureSupport.nativeCaptureSupported
      && this.captureSupport.supportedSources.systemAudio,
  );
  resolutionSupportPending = $derived(this.captureSupportLoading);
  nonOriginalResolutionDisabled = $derived(
    this.rec.draftCaptureScreen
      && (this.resolutionSupportPending || this.nativeCaptureUnsupported || this.onlyOriginalResolutionSupported),
  );
  resolutionSupportPendingForNonOriginal = $derived(
    this.rec.draftCaptureScreen && this.resolutionSupportPending && this.rec.draftResolutionMode !== "original",
  );

  // ─── AI provider helpers (delegate to the shared pure module) ───────────────
  isCloudAiProviderKind(kind: string): boolean {
    return isCloudAiProviderKind(kind);
  }

  aiProviderKindLabel(kind: string): string {
    return aiProviderKindLabel(kind);
  }

  aiProviderKindDescription(kind: AiProviderKind): string {
    return aiProviderKindDescription(kind);
  }

  connectedAiProviderIds = $derived(this.rec.draftAiProviders.map((p) => p.id));
  anyCloudAiProviderConnected = $derived(
    this.rec.draftAiProviders.some((p) => this.isCloudAiProviderKind(p.kind)),
  );

  aiProviderById(id: string): AiProviderConfig | undefined {
    return this.rec.draftAiProviders.find((p) => p.id === id);
  }
  aiProviderKindById(id: string): AiProviderKind | undefined {
    return this.aiProviderById(id)?.kind;
  }
  isCloudAiProviderInstance(id: string): boolean {
    const kind = this.aiProviderKindById(id);
    return kind !== undefined && this.isCloudAiProviderKind(kind);
  }
  baseUrlHost(baseUrl: string): string {
    return baseUrlHost(baseUrl);
  }
  aiProviderInstanceLabel(provider: AiProviderConfig): string {
    return aiProviderInstanceLabel(provider);
  }
  aiProviderLabelById(id: string): string {
    const provider = this.aiProviderById(id);
    return provider ? this.aiProviderInstanceLabel(provider) : this.aiProviderKindLabel(id);
  }

  newAiProviderId(kind: AiProviderKind): string {
    return newAiProviderId(kind, this.connectedAiProviderIds);
  }

  addAiProvider(kind: AiProviderKind): void {
    // Guarded by aiProviderRemoving at the call site (control is disabled while a
    // clear is in flight); this guard is the defensive backstop.
    if (this.aiProviderRemoving) return;
    this.rec.draftAiProviders = [
      ...this.rec.draftAiProviders,
      { id: this.newAiProviderId(kind), kind, label: "", baseUrl: "" },
    ];
    void this.aiRuntime.refreshAiProviderKeyPresence();
  }

  async removeAiProvider(id: string): Promise<void> {
    const removed = this.rec.draftAiProviders.find((p) => p.id === id);
    // Removing a cloud provider tears down its keychain secret immediately
    // (clearKeyForRemovedProvider below), with no undo — gate that on an
    // explicit confirm so a single mis-click can't wipe a saved API key.
    if (removed && this.isCloudAiProviderKind(removed.kind)) {
      const label = this.aiProviderInstanceLabel(removed);
      const confirmed = await confirm(
        `Removing “${label}” deletes its API key from the macOS keychain right away. Any AI feature using this provider will stop working until you reconnect it.`,
        {
          title: "Remove this provider?",
          kind: "warning",
          okLabel: "Remove & Delete Key",
          cancelLabel: "Keep Provider",
        },
      );
      if (!confirmed) return;
    }
    this.rec.draftAiProviders = this.rec.draftAiProviders.filter((p) => p.id !== id);
    if (this.rec.draftAiDefaultModel?.provider === id) {
      this.rec.draftAiDefaultModel = null;
    }
    // The last test-connection banner names the tested provider/model; once it is
    // removed the banner would misrepresent the live config, so clear it.
    this.aiRuntime.resetTestResult();
    if (removed && this.isCloudAiProviderKind(removed.kind)) {
      // AWAIT the keychain clear so a same-kind re-add (which reuses the bare
      // kind id, ADR 0035) re-probes only after the clear has resolved — never
      // racing an in-flight clear into a false "key in keychain". The
      // aiProviderRemoving flag disables the add-provider control for the
      // duration so a new provider can't be added mid-clear.
      this.aiProviderRemoving = true;
      try {
        await this.aiRuntime.clearKeyForRemovedProvider(id);
      } finally {
        this.aiProviderRemoving = false;
      }
    }
  }

  // ─── MCP connectors ─────────────────────────────────────────────────────────
  // Actions live in controller-mcp.ts (800-line cap); thin delegates here keep
  // call sites on the controller. The blank-draft addMcpServer died with the
  // inline form — the picker (preset / Custom) is the only add path now.
  mcp = createMcpConnectorActions({
    rec: this.rec,
    aiRuntime: this.aiRuntime,
    saveAiRuntime: () => this.saveRecordingDomain("ai_runtime"),
  });

  addMcpServerDraft(draft: McpServerConfig): string {
    return this.mcp.addMcpServerDraft(draft);
  }
  addMcpServerFromPreset(preset: McpPreset, overrides?: Partial<McpServerConfig>): string {
    return this.mcp.addMcpServerFromPreset(preset, overrides);
  }
  removeMcpServer(id: string, opts?: { confirm?: boolean }): Promise<void> {
    return this.mcp.removeMcpServer(id, opts);
  }
  flushAiRuntimeSave(): Promise<void> {
    return this.mcp.flushAiRuntimeSave();
  }

  // ─── Model-pool picker ──────────────────────────────────────────────────────
  // Only surface failures for providers STILL in the connected (draft) set. The
  // shared loader prunes its slice for providers still in a load target, but a
  // removed provider's failure lingers until the next load/route re-entry — and
  // removal has no reachable path that re-runs a load. Filtering here clears the
  // stale banner immediately on removal (the derivation re-runs when
  // `connectedAiProviderIds` changes) without disturbing legitimate failures for
  // providers that are still connected. `connectedAiProviderIds` already keys off
  // the instance id, matching `failures[].provider`.
  settingsModelLoaderFailures = $derived(
    this.settingsModelLoader.failures.filter((f) => this.connectedAiProviderIds.includes(f.provider)),
  );
  settingsModelFailureRows = $derived(
    this.settingsModelLoaderFailures.map((f) => ({
      provider: f.provider,
      label: this.aiProviderLabelById(f.provider),
      reason: f.reason,
    })),
  );
  settingsModelRetryTargets = $derived(
    this.rec.draftAiProviders.filter((p) =>
      this.settingsModelLoaderFailures.some((f) => f.provider === p.id),
    ),
  );
  settingsModelsError = $derived(
    this.settingsModelLoaderFailures.length > 0
      ? this.settingsModelLoaderFailures
          .map((f) => `${this.aiProviderLabelById(f.provider)}: ${f.reason}`)
          .join("; ")
      : null,
  );

  async loadSettingsModels() {
    await this.settingsModelLoader.load(this.rec.draftAiProviders);
  }

  aiProviderSignature = $derived(
    this.rec.draftAiProviders.map((p) => `${p.id}:${p.baseUrl ?? ""}`).join("|"),
  );

  aiDefaultModelLabel(ref: AiEngineRef | null): string {
    if (!ref || ref.model.trim().length === 0) return "";
    return `${this.aiProviderLabelById(ref.provider)} · ${ref.model}`;
  }
  aiModelValue = $derived(this.aiDefaultModelLabel(this.rec.draftAiDefaultModel));

  askAiModelLabel(value: string): string {
    return value ? value : "Global default model";
  }

  userContextCloudDefault = $derived(
    this.rec.draftAiDefaultModel !== null && this.isCloudAiProviderInstance(this.rec.draftAiDefaultModel.provider),
  );
  userContextLocalDefault = $derived(
    this.rec.draftAiDefaultModel !== null && !this.isCloudAiProviderInstance(this.rec.draftAiDefaultModel.provider),
  );

  // Friendly Ask AI reason copy.
  askAiReasonLabel = (reason: string | null | undefined) =>
    askAiReasonLabelCore(reason, (r) => this.aiRuntime.aiRuntimeReasonLabel(r));

  askAiStatusDetail = $derived.by(() => {
    if (this.askAi.askAiAvailabilityLoading) return "Checking Ask AI availability…";
    if (this.askAi.askAiAvailable) {
      return "Ask AI can answer over your redacted capture history.";
    }
    return this.askAiReasonLabel(this.askAi.askAiAvailability?.reason);
  });

  async setBrowserUrlMode(mode: string) {
    if (mode === this.rec.draftBrowserUrlMode) return;
    if (mode === "full") {
      const ok = await ask("Full URL metadata stores query strings and fragments. Continue?", {
        title: "Enable full URL metadata",
        kind: "warning",
        okLabel: "Enable",
        cancelLabel: "Cancel",
      });
      if (!ok) return;
    }
    this.rec.draftBrowserUrlMode = mode as BrowserUrlMode;
  }

  // ─── Capture support load ────────────────────────────────────────────────────
  async loadCaptureSupport() {
    this.captureSupportLoading = true;
    this.captureSupportFailed = false;
    this.captureSupport = null;
    try {
      this.captureSupport = await invoke<CaptureSupport>("get_capture_support");
    } catch {
      this.captureSupportFailed = true;
    } finally {
      this.captureSupportLoading = false;
    }
  }

  // ─── Processing-model derivations + loaders (OCR / transcription / speaker) ──
  // Split into ProcessingModelsView to keep this file under the 800-line cap.
  // Members are re-exposed below so panel markup stays flat + verbatim.
  processing = createProcessingModelsView(this.rec, this.models);

  // OCR
  get loadOcrModelStatus() { return this.processing.loadOcrModelStatus; }
  get startSelectedOcrModelDownload() { return this.processing.startSelectedOcrModelDownload; }
  get cancelSelectedOcrModelDownload() { return this.processing.cancelSelectedOcrModelDownload; }
  get handleOcrDownloadProgress() { return this.processing.handleOcrDownloadProgress; }
  get requestDeleteUnusedOcrModels() { return this.processing.requestDeleteUnusedOcrModels; }
  get ocrProviderOptions() { return this.processing.ocrProviderOptions; }
  get selectedOcrModels() { return this.processing.selectedOcrModels; }
  get ocrModelOptions() { return this.processing.ocrModelOptions; }
  get selectedOcrModel() { return this.processing.selectedOcrModel; }
  get selectedOcrDownloadProgress() { return this.processing.selectedOcrDownloadProgress; }
  get selectedOcrDownloadRunning() { return this.processing.selectedOcrDownloadRunning; }
  get selectedOcrDownloadPercent() { return this.processing.selectedOcrDownloadPercent; }
  isSelectableOcrProvider(value: string | null | undefined): value is OcrProvider {
    return this.processing.isSelectableOcrProvider(value);
  }
  chooseOcrProvider(provider: string) { this.processing.chooseOcrProvider(provider); }
  chooseOcrModel(value: string) { this.processing.chooseOcrModel(value); }

  // Transcription
  get loadTranscriptionModelStatus() { return this.processing.loadTranscriptionModelStatus; }
  get requestAppleSpeechPermission() { return this.processing.requestAppleSpeechPermission; }
  get openAppleSpeechPrivacySettings() { return this.processing.openAppleSpeechPrivacySettings; }
  get startSelectedTranscriptionModelDownload() { return this.processing.startSelectedTranscriptionModelDownload; }
  get cancelSelectedTranscriptionModelDownload() { return this.processing.cancelSelectedTranscriptionModelDownload; }
  get handleTranscriptionDownloadProgress() { return this.processing.handleTranscriptionDownloadProgress; }
  get requestDeleteUnusedTranscriptionModels() { return this.processing.requestDeleteUnusedTranscriptionModels; }
  get transcriptionProviderOptions() { return this.processing.transcriptionProviderOptions; }
  get selectedTranscriptionModels() { return this.processing.selectedTranscriptionModels; }
  get transcriptionModelOptions() { return this.processing.transcriptionModelOptions; }
  get selectedTranscriptionModel() { return this.processing.selectedTranscriptionModel; }
  get selectedAppleSpeechPermissionStatus() { return this.processing.selectedAppleSpeechPermissionStatus; }
  get selectedAppleSpeechNeedsPermission() { return this.processing.selectedAppleSpeechNeedsPermission; }
  get selectedTranscriptionDownloadProgress() { return this.processing.selectedTranscriptionDownloadProgress; }
  get selectedTranscriptionDownloadRunning() { return this.processing.selectedTranscriptionDownloadRunning; }
  get selectedTranscriptionDownloadPercent() { return this.processing.selectedTranscriptionDownloadPercent; }
  chooseTranscriptionProvider(provider: string) { this.processing.chooseTranscriptionProvider(provider); }
  chooseTranscriptionModel(value: string) { this.processing.chooseTranscriptionModel(value); }

  // Speaker
  get loadSpeakerModelStatus() { return this.processing.loadSpeakerModelStatus; }
  get loadPersonProfileCount() { return this.processing.loadPersonProfileCount; }
  get startSelectedSpeakerModelDownload() { return this.processing.startSelectedSpeakerModelDownload; }
  get cancelSelectedSpeakerModelDownload() { return this.processing.cancelSelectedSpeakerModelDownload; }
  get deleteSelectedSpeakerModel() { return this.processing.deleteSelectedSpeakerModel; }
  get handleSpeakerDownloadProgress() { return this.processing.handleSpeakerDownloadProgress; }
  get allSpeakerModels() { return this.processing.allSpeakerModels; }
  get selectedSpeakerModel() { return this.processing.selectedSpeakerModel; }
  get speakerModelOptions() { return this.processing.speakerModelOptions; }
  get selectedSpeakerPresetKey() { return this.processing.selectedSpeakerPresetKey; }
  get selectedSpeakerDownloadProgress() { return this.processing.selectedSpeakerDownloadProgress; }
  get selectedSpeakerDownloadRunning() { return this.processing.selectedSpeakerDownloadRunning; }
  get selectedSpeakerDownloadPercent() { return this.processing.selectedSpeakerDownloadPercent; }
  chooseSpeakerModel(value: string) { return this.processing.chooseSpeakerModel(value); }

  loadSemanticSearchSupportedModels = () => this.models.loadSemanticSearchSupportedModels();
  startSemanticSearchModelDownload = (model: SemanticSearchModelStatus) =>
    this.models.startSemanticSearchModelDownload(model);
  cancelSemanticSearchModelDownload = () => this.models.cancelSemanticSearchModelDownload();

  // ─── Semantic-search picker ─────────────────────────────────────────────────
  // Split into SemanticSearchView to keep this file under the 800-line cap.
  // Members are re-exposed below so panel markup stays flat + verbatim.
  semanticSearch = createSemanticSearchView(this.rec, this.models);

  get semanticSearchPickedModelId() { return this.semanticSearch.semanticSearchPickedModelId; }
  set semanticSearchPickedModelId(value: string | null) { this.semanticSearch.semanticSearchPickedModelId = value; }
  get reseedSemanticSearchPickedModel() { return this.semanticSearch.reseedSemanticSearchPickedModel; }
  get loadSemanticSearchModelStatus() { return this.semanticSearch.loadSemanticSearchModelStatus; }
  get handleSemanticSearchDownloadProgress() { return this.semanticSearch.handleSemanticSearchDownloadProgress; }
  get setSemanticSearchEnabled() { return this.semanticSearch.setSemanticSearchEnabled; }
  get startSemanticSearchPickedDownload() { return this.semanticSearch.startSemanticSearchPickedDownload; }
  get chooseSemanticSearchPickedModel() { return this.semanticSearch.chooseSemanticSearchPickedModel; }
  get deleteSemanticSearchPickedModel() { return this.semanticSearch.deleteSemanticSearchPickedModel; }
  get semanticSearchModelOptions() { return this.semanticSearch.semanticSearchModelOptions; }
  get semanticSearchPickedModel() { return this.semanticSearch.semanticSearchPickedModel; }
  get semanticSearchPickedProgress() { return this.semanticSearch.semanticSearchPickedProgress; }

  // ─── Recording-domain save + retention ──────────────────────────────────────
  async saveRecordingDomain(domain: AutosaveRecordingDomain) {
    if (this.appPrivacyExclusion.commandInFlight) return;
    if (this.rec.recDomainSaveBlocked(domain)) {
      if (domain === "video" && this.resolutionSupportPendingForNonOriginal) {
        this.rec.recError = "Wait for capture support to load before saving preset/custom resolution.";
        // This is a validation-block, not a failed dispatch — don't offer Retry
        // against a stale failed domain alongside this message.
        this.lastFailedSaveDomain = null;
      }
      return;
    }

    // Arm the per-domain in-flight guard BEFORE the (awaited) retention preview +
    // confirm dialog. The autosave engine's only re-entry gate for this domain is
    // `savingRecDomains[domain]`; without setting it here, a concurrent draft edit
    // made while the confirm dialog is open re-arms the debounce and stacks a
    // SECOND preview + confirm. Setting it now closes that window; the single
    // `finally` below always clears it.
    if (this.rec.savingRecDomains[domain]) return;
    // Exponential-backoff gate: a failed save leaves the domain dirty, so the
    // autosave engine would otherwise re-fire every ~450ms forever. While a
    // backoff window is open, skip the attempt — the engine quiesces because we
    // return before toggling `savingRecDomains` (no flag flip = no re-tick), and
    // `noteSaveFailure` has scheduled a single one-shot re-tick at expiry. The
    // manual Retry clears the window so it is never swallowed here.
    if (Date.now() < (this.recSaveBackoffUntil[domain] ?? 0)) return;
    this.rec.savingRecDomains = { ...this.rec.savingRecDomains, [domain]: true };

    try {
      const previousRetentionPolicy = this.rec.recordingSettings?.retentionPolicy ?? "never";

      // Confirm whenever the NEW retention window is SHORTER than the previous
      // one — i.e. tightening to a bounded policy that can delete newly-eligible
      // data. `retentionToDays` returns null for the unbounded "Forever" policy;
      // going from unbounded (prev === null) to any bounded window shortens, as
      // does shrinking one bounded window to a smaller one (newDays < prevDays).
      const prevDays = retentionToDays(previousRetentionPolicy);
      const newDays = retentionToDays(this.rec.draftRetentionPolicy);
      const retentionShortened =
        prevDays === null ? newDays !== null : newDays !== null && newDays < prevDays;

      if (domain === "storage" && retentionShortened) {
        try {
          const preview = await invoke<RetentionCleanupSummary>("preview_retention_cleanup", {
            request: { policy: this.rec.draftRetentionPolicy },
          });
          this.retentionCleanupSummary = preview;
          const ok = await ask(
            `Retention will delete ${preview.deletedFrames} frame row(s), ${preview.deletedAudioSegments} audio segment row(s), and ${preview.eligibleCaptureSegments} capture segment(s) before ${preview.cutoffEndedBefore ?? "the cutoff"}. Continue?`,
            {
              title: "Confirm retention cleanup",
              kind: "warning",
              okLabel: "Continue",
              cancelLabel: "Cancel",
            },
          );
          if (!ok) {
            this.rec.draftRetentionPolicy = this.rec.recordingSettings?.retentionPolicy ?? "never";
            return;
          }
        } catch (err) {
          this.noteSaveFailure(domain, humanizeError(err));
          return;
        }
      }

      this.rec.recError = null;
      this.rec.recSaved = false;
      // Snapshot the drafts EXACTLY as dispatched to `invoke`, so the post-save
      // sync can tell whether the user edited during the flight (edit C). If the
      // live drafts still equal this on success, adopt canonical; if they diverged,
      // the newer edit is kept and the reactive driver schedules a follow-up save.
      const dispatchedSnapshot = this.rec.buildRecDomainSnapshot(domain);
      try {
        const response = await invoke<RecordingSettingsDomainUpdateResponse>(RECORDING_DOMAIN_COMMANDS[domain], {
          request: this.rec.buildRecDomainRequest(domain),
        });
        const updated = response.settings;
        this.rec.recordingSettings = updated;
        this.rec.syncRecordingDomainFromCanonical(response.domain, updated, { dispatchedSnapshot });
        this.rec.recSaved = true;
        this.recSavedDomain = domain;
        this.clearSaveFailure(domain);
        setTimeout(() => {
          this.rec.recSaved = false;
          if (this.recSavedDomain === domain) this.recSavedDomain = null;
        }, 2200);

        // Only run cleanup when retention was TIGHTENED (same predicate that
        // gates the confirm dialog above). Loosening the policy (longer window or
        // "Forever") can never make data newly-eligible for deletion, so running
        // cleanup there would be an unconfirmed, pointless pass.
        if (domain === "storage" && retentionShortened && previousRetentionPolicy !== updated.retentionPolicy) {
          this.retentionCleanupRunning = true;
          this.retentionCleanupError = null;
          try {
            this.retentionCleanupSummary = await invoke<RetentionCleanupSummary>("run_retention_cleanup_now");
          } catch (err) {
            this.retentionCleanupError = humanizeError(err);
          } finally {
            this.retentionCleanupRunning = false;
          }
        }
      } catch (err) {
        this.noteSaveFailure(domain, humanizeError(err));
      }
    } finally {
      this.rec.savingRecDomains = { ...this.rec.savingRecDomains, [domain]: false };
    }
  }

  // Record a failed recording-domain save: surface the message, remember the
  // domain (so the rail footer can target Retry/Dismiss), and arm capped
  // exponential backoff with a single one-shot re-tick so the retry resumes
  // once — not as a tight ~450ms hammer — when the window elapses.
  private noteSaveFailure(domain: AutosaveRecordingDomain, message: string) {
    this.rec.recError = message;
    this.lastFailedSaveDomain = domain;
    const failures = (this.recSaveFailureCount[domain] ?? 0) + 1;
    this.recSaveFailureCount = { ...this.recSaveFailureCount, [domain]: failures };
    const delay = Math.min(30_000, 500 * 2 ** (failures - 1));
    this.recSaveBackoffUntil = { ...this.recSaveBackoffUntil, [domain]: Date.now() + delay };
    const existing = this.recSaveRetryTimers.get(domain);
    if (existing) clearTimeout(existing);
    this.recSaveRetryTimers.set(
      domain,
      setTimeout(() => {
        this.recSaveRetryTimers.delete(domain);
        this.autosaveEngine.tick();
      }, delay + 50),
    );
  }

  // Clear the failure bookkeeping for a domain (on a successful save, a manual
  // retry, or a dismiss-and-reconcile).
  private clearSaveFailure(domain: AutosaveRecordingDomain) {
    const timer = this.recSaveRetryTimers.get(domain);
    if (timer) {
      clearTimeout(timer);
      this.recSaveRetryTimers.delete(domain);
    }
    if (this.lastFailedSaveDomain === domain) this.lastFailedSaveDomain = null;
    if (this.recSaveFailureCount[domain]) {
      this.recSaveFailureCount = { ...this.recSaveFailureCount, [domain]: 0 };
    }
    if (this.recSaveBackoffUntil[domain]) {
      this.recSaveBackoffUntil = { ...this.recSaveBackoffUntil, [domain]: 0 };
    }
  }

  // Cancel any pending backoff re-tick timers. Called on teardown so a failed
  // save's backoff cannot fire on a detached controller (re-arming saves /
  // ticking the engine) after Settings closes.
  cancelPendingSaveRetries(): void {
    for (const timer of this.recSaveRetryTimers.values()) clearTimeout(timer);
    this.recSaveRetryTimers.clear();
  }

  // Re-run the last failed domain save immediately (the user pressed Retry).
  // Clears the backoff window first so the attempt is not swallowed by the gate.
  retryFailedSave(): void {
    const domain = this.lastFailedSaveDomain;
    if (!domain) return;
    this.recSaveBackoffUntil = { ...this.recSaveBackoffUntil, [domain]: 0 };
    this.recSaveFailureCount = { ...this.recSaveFailureCount, [domain]: 0 };
    void this.saveRecordingDomain(domain);
  }

  // Dismiss the autosave error banner. When a domain save is the source, reconcile
  // its control back to the last-saved canonical value — this both shows the user
  // what is actually persisted and clears the dirtiness that was driving the retry
  // loop. Non-domain errors (e.g. a privacy-echo failure) just clear the message.
  dismissRecError(): void {
    const domain = this.lastFailedSaveDomain;
    this.rec.recError = null;
    if (domain) {
      if (this.rec.recordingSettings) {
        this.rec.syncRecordingDomainFromCanonical(domain, this.rec.recordingSettings, true);
      }
      this.clearSaveFailure(domain);
    }
  }

  async runRetentionCleanupNow() {
    const ok = await ask("Run retention cleanup now? This can delete captured data that matches the current retention policy.", {
      title: "Run cleanup now",
      kind: "warning",
      okLabel: "Run cleanup",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    this.retentionCleanupRunning = true;
    this.retentionCleanupError = null;
    try {
      this.retentionCleanupSummary = await invoke<RetentionCleanupSummary>("run_retention_cleanup_now");
    } catch (err) {
      this.retentionCleanupError = humanizeError(err);
    } finally {
      this.retentionCleanupRunning = false;
    }
  }

  // ─── Validation derivations ──────────────────────────────────────────────────
  customResolutionErrors = $derived(customResolutionErrorsFn(this.rec));
  customResolutionBlocked = $derived(customResolutionBlockedFn(this.rec));
  customBitrateErrors = $derived(customBitrateErrorsFn(this.rec));
  customBitrateBlocked = $derived(customBitrateBlockedFn(this.rec));
  recValidationErrors = $derived(
    recValidationErrorsFn(this.rec, { resolutionSupportPendingForNonOriginal: this.resolutionSupportPendingForNonOriginal }),
  );
  recSaveBlocked = $derived(
    recSaveBlockedFn(this.rec, { resolutionSupportPendingForNonOriginal: this.resolutionSupportPendingForNonOriginal }),
  );

  savingRecSettings = $derived(
    RECORDING_DRAFT_DOMAINS.some((domain) => this.rec.savingRecDomains[domain]),
  );
}

// ─── Svelte context plumbing ──────────────────────────────────────────────────
const SETTINGS_CONTROLLER_KEY = Symbol("settings-controller");

export function setSettingsController(controller: SettingsController): SettingsController {
  return setContext(SETTINGS_CONTROLLER_KEY, controller);
}

export function getSettingsController(): SettingsController {
  const controller = getContext<SettingsController>(SETTINGS_CONTROLLER_KEY);
  if (!controller) {
    throw new Error("SettingsController not found in context — render inside the settings shell.");
  }
  return controller;
}
