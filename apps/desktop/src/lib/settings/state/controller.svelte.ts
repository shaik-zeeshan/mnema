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
import { errorText, formatBytes } from "./format";
import { createCliAccessStore } from "./cli-access.svelte";
import { createLogsStore } from "./logs.svelte";
import { createAskAiStore, askAiReasonLabel as askAiReasonLabelCore } from "./ask-ai.svelte";
import { createAboutStore } from "./about.svelte";
import { createUserContextStore } from "./user-context.svelte";
import { createAiRuntimeStore } from "./ai-runtime.svelte";
import { createModelStatusStore } from "./model-status.svelte";
import { createAudioStore } from "./audio.svelte";
import { createKeyboardStore } from "./keyboard.svelte";
import { createProcessingModelsView } from "./controller-processing.svelte";
import { semanticSearchTierLabel } from "./models-format";
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
import type {
  CaptureSupport,
  OcrProvider,
  RecordingSettings,
  RecordingSettingsDomainUpdateResponse,
  AiProviderKind,
  AiProviderConfig,
  AiEngineRef,
  BrowserUrlMode,
  SemanticSearchModelStatus,
  SemanticSearchModelDownloadProgress,
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

interface SemanticSearchPickedView {
  modelId: string;
  provider: string | null;
  displayName: string;
  description: string;
  metaLine: string;
  available: boolean;
  approxDownloadBytes: number | null;
}

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
    loadAiRuntimeStatus: () => void this.aiRuntime.loadAiRuntimeStatus(),
    gates: () => ({ resolutionSupportPendingForNonOriginal: this.resolutionSupportPendingForNonOriginal }),
    // Re-seed the semantic-search picker once settings land — closes the init
    // race where the picker status resolved before recording settings, leaving
    // the picker blank while a model is actually persisted. Dirty-guarded.
    onRecordingSettingsLoaded: () => this.reseedSemanticSearchPickedModel(),
  });
  cliAccess = createCliAccessStore();
  logs = createLogsStore();
  askAi = createAskAiStore();
  about = createAboutStore();
  aiRuntime = createAiRuntimeStore({
    getProviders: () => this.rec.draftAiProviders,
    isCloudProviderKind: (kind) => this.isCloudAiProviderKind(kind),
    labelForProvider: (id) => this.aiProviderLabelById(id),
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

  // Ask AI / AI model picker open state.
  askAiModelOpen = $state(false);
  aiModelOpen = $state(false);

  // True while a provider removal (incl. its awaited keychain clear) is in
  // flight. The add-provider control reads this and stays disabled so a new
  // provider can't be added mid-clear and race a same-kind id re-add (ADR 0035)
  // into a false "key in keychain" probe.
  aiProviderRemoving = $state(false);

  // Semantic-search picked model (page picker draft).
  semanticSearchPickedModelId = $state<string | null>(null);

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

  // Seed the page picker from the persisted (sticky) selection, but ONLY while
  // the picker has not been touched (`semanticSearchPickedModelId === null`), so
  // a live user edit is never clobbered. Idempotent — safe to call from every
  // path that might learn the persisted selection (status load, download
  // progress, or a post-settings-load re-seed that fixes the init race where the
  // picker status resolved before recording settings).
  reseedSemanticSearchPickedModel() {
    if (this.semanticSearchPickedModelId === null && this.rec.semanticSearchSelectedModelId !== null) {
      this.semanticSearchPickedModelId = this.rec.semanticSearchSelectedModelId;
    }
  }

  async loadSemanticSearchModelStatus() {
    await this.models.loadSemanticSearchModelStatus();
    this.reseedSemanticSearchPickedModel();
  }

  async handleSemanticSearchDownloadProgress(progress: SemanticSearchModelDownloadProgress) {
    await this.models.handleSemanticSearchDownloadProgress(progress);
    this.reseedSemanticSearchPickedModel();
  }

  async chooseSemanticSearchModel(model: SemanticSearchModelStatus) {
    // In-flight re-entry guard (mirrors `saveRecordingDomain`'s `savingRecDomains`
    // gate). The confirm() dialog below awaits, so without this a second invocation
    // while a `select_semantic_search_model` invoke is in flight would stack a
    // second clear/reindex. Correctness must not depend solely on the UI `disabled`.
    if (this.models.semanticSearchReindexing) return;
    if (!this.rec.recordingSettingsLoaded) await this.rec.loadRecordingSettings();
    if (this.rec.semanticSearchSelectedModelId === model.modelId) return;
    const isFirstSelection = this.rec.semanticSearchSelectedModelId === null;
    if (!isFirstSelection) {
      const confirmed = await confirm(
        `Switching to “${model.displayName}” re-indexes every recording: all existing meaning vectors are cleared and re-derived under the new model in the background. Your captures are not changed.`,
        {
          title: "Re-index for new search model?",
          kind: "warning",
          okLabel: "Switch & Re-index",
          cancelLabel: "Keep Current Model",
        },
      );
      if (!confirmed) return;
    }

    this.models.semanticSearchModelError = null;
    this.models.semanticSearchReindexing = true;
    this.models.semanticSearchReindexMessage = null;
    try {
      const cleared = await invoke<number>("select_semantic_search_model", {
        modelId: model.modelId,
      });
      this.rec.semanticSearchSelectedModelId = model.modelId;
      if (!isFirstSelection) {
        this.models.semanticSearchReindexMessage =
          cleared > 0
            ? `Cleared ${cleared} vector${cleared === 1 ? "" : "s"}; re-indexing in the background.`
            : "Re-index started in the background.";
      }
      await this.loadSemanticSearchModelStatus();
    } catch (err) {
      this.models.semanticSearchModelError = errorText(err);
    } finally {
      this.models.semanticSearchReindexing = false;
    }
  }

  async setSemanticSearchEnabled(enabled: boolean) {
    this.models.semanticSearchModelError = null;
    try {
      await invoke<RecordingSettingsDomainUpdateResponse>("update_semantic_search_settings", {
        request: { enabled },
      });
      this.rec.draftSemanticSearchEnabled = enabled;
    } catch (err) {
      this.models.semanticSearchModelError = errorText(err);
      this.rec.draftSemanticSearchEnabled = !enabled;
    }
  }

  // ─── Semantic-search picker derivations ─────────────────────────────────────
  semanticSearchGuidedModels = $derived(
    (this.models.semanticSearchModelStatus?.models ?? []).filter((m) => m.tier !== "custom"),
  );
  semanticSearchProvider = $derived(
    (this.models.semanticSearchModelStatus?.models ?? [])[0]?.provider ?? null,
  );
  semanticSearchGuidedModelIds = $derived(
    new Set(this.semanticSearchGuidedModels.map((m) => m.modelId)),
  );
  semanticSearchCustomOptions = $derived(
    this.models.semanticSearchSupportedModels.filter(
      (m) => !this.semanticSearchGuidedModelIds.has(m.modelId),
    ),
  );
  semanticSearchModelOptions = $derived([
    ...this.semanticSearchGuidedModels.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} · ${m.dimension}d${m.tier === "multilingual" ? " · multilingual" : ""} · recommended`,
    })),
    ...this.semanticSearchCustomOptions.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} — ${m.dimension}d${m.multilingual ? " · multilingual" : ""}`,
    })),
  ]);

  semanticSearchPickedModel = $derived.by((): SemanticSearchPickedView | null => {
    const id = this.semanticSearchPickedModelId;
    if (!id) return null;
    const live = (this.models.semanticSearchModelStatus?.models ?? []).find((m) => m.modelId === id);
    if (live) {
      return {
        modelId: live.modelId,
        provider: live.provider,
        displayName: live.displayName,
        description: live.description,
        metaLine: `${semanticSearchTierLabel(live.tier)} · ${formatBytes(live.approxDownloadBytes)} on disk · ${live.dimension}-dim · runs on-device${live.licenseLabel ? ` · ${live.licenseLabel}` : ""}`,
        available: live.available,
        approxDownloadBytes: live.approxDownloadBytes,
      };
    }
    const catalog = this.models.semanticSearchSupportedModels.find((m) => m.modelId === id);
    if (catalog) {
      const size =
        catalog.approxDownloadBytes != null
          ? `${formatBytes(catalog.approxDownloadBytes)} on disk · `
          : "";
      return {
        modelId: catalog.modelId,
        provider: this.semanticSearchProvider,
        displayName: catalog.displayName,
        description: catalog.description,
        metaLine: `${semanticSearchTierLabel("custom")} · ${size}${catalog.dimension}-dim · runs on-device${catalog.multilingual ? " · multilingual" : ""}`,
        available: false,
        approxDownloadBytes: catalog.approxDownloadBytes,
      };
    }
    return null;
  });

  semanticSearchPickedProgress = $derived.by(() => {
    const id = this.semanticSearchPickedModelId;
    const p = this.models.semanticSearchDownloadProgress;
    return id && p && p.modelId === id ? p : null;
  });

  async startSemanticSearchPickedDownload(model: SemanticSearchPickedView) {
    if (!model.provider) return;
    await this.startSemanticSearchModelDownload({
      provider: model.provider,
      modelId: model.modelId,
    } as SemanticSearchModelStatus);
  }

  async chooseSemanticSearchPickedModel(model: SemanticSearchPickedView) {
    await this.chooseSemanticSearchModel({
      modelId: model.modelId,
      displayName: model.displayName,
    } as SemanticSearchModelStatus);
  }

  // ─── Recording-domain save + retention ──────────────────────────────────────
  async saveRecordingDomain(domain: AutosaveRecordingDomain) {
    if (this.appPrivacyExclusion.commandInFlight) return;
    if (this.rec.recDomainSaveBlocked(domain)) {
      if (domain === "video" && this.resolutionSupportPendingForNonOriginal) {
        this.rec.recError = "Wait for capture support to load before saving preset/custom resolution.";
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
          this.rec.recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
        setTimeout(() => { this.rec.recSaved = false; }, 2200);

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
            this.retentionCleanupError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
          } finally {
            this.retentionCleanupRunning = false;
          }
        }
      } catch (err) {
        this.rec.recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
      }
    } finally {
      this.rec.savingRecDomains = { ...this.rec.savingRecDomains, [domain]: false };
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
      this.retentionCleanupError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
