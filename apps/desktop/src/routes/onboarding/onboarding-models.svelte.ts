// OCR + audio-transcription model status/download subsystems for onboarding.
//
// These two subsystems are structurally identical (status fetch, download
// start/cancel, progress handling, plus selected-model deriveds), so they live
// together here, factored out of the controller to keep `onboarding.svelte.ts`
// under the file-size budget. Each subsystem reads the controller's draft
// provider/model selection through getter accessors so its deriveds track the
// live draft state; the controller exposes a SINGLE FLAT surface by delegating
// to these instances.
import { invoke } from "@tauri-apps/api/core";
import type {
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionModelStatus,
  AudioTranscriptionModelStatusResponse,
  AudioTranscriptionProvider,
  OcrModelDownloadProgress,
  OcrModelStatus,
  OcrModelStatusResponse,
  OcrProvider,
  SemanticSearchModelDownloadProgress,
  SemanticSearchModelStatus,
  SemanticSearchModelStatusResponse,
  SemanticSearchSupportedModel,
  SpeakerAnalysisModelDownloadProgress,
  SpeakerAnalysisModelStatus,
  SpeakerAnalysisModelStatusResponse,
} from "$lib/types";
import { semanticSearchTierLabel } from "$lib/settings/state/models-format";
import {
  formatBytes,
  isSelectableOcrProvider,
  ocrStatusLabel,
  serializeError,
  speakerPresetKey,
  speakerStatusLabel,
  transcriptionStatusLabel,
} from "./onboarding-mapping";

const OS_MANAGED_OPTION_VALUE = "__os_managed__";
const RUNNING_DOWNLOAD_STATUSES = ["starting", "downloading", "installing"];
const TERMINAL_DOWNLOAD_STATUSES = ["completed", "failed", "cancelled"];

// ── OCR model subsystem ────────────────────────────────────────────────────
export interface OcrModelStoreAccess {
  ocrProvider: () => OcrProvider;
  ocrModelId: () => string | null;
}

export function createOcrModelStore(access: OcrModelStoreAccess) {
  let ocrModelStatus = $state<OcrModelStatusResponse | null>(null);
  let loadingOcrModelStatus = $state(false);
  let ocrModelError = $state<string | null>(null);
  let ocrDownloadProgress = $state<OcrModelDownloadProgress | null>(null);
  let startingOcrDownload = $state(false);
  let cancellingOcrDownload = $state(false);
  let ocrDownloadError = $state<string | null>(null);

  const selectedOcrProviderStatus = $derived(
    ocrModelStatus?.providers.find((provider) => provider.provider === access.ocrProvider()) ?? null,
  );
  const selectedOcrModels = $derived(selectedOcrProviderStatus?.models ?? []);
  const ocrModelOptions = $derived(
    selectedOcrModels.map((model) => ({
      value: model.modelId ?? OS_MANAGED_OPTION_VALUE,
      label: `${model.displayName} · ${ocrStatusLabel(model)}`,
    })),
  );
  const selectedOcrModel = $derived(
    selectedOcrModels.find((model) => model.modelId === access.ocrModelId())
      ?? selectedOcrModels[0]
      ?? null,
  );
  const selectedOcrDownloadProgress = $derived(
    ocrDownloadProgress
      && ocrDownloadProgress.provider === access.ocrProvider()
      && ocrDownloadProgress.modelId === access.ocrModelId()
      ? ocrDownloadProgress
      : null,
  );
  const selectedOcrDownloadRunning = $derived(
    selectedOcrDownloadProgress !== null
      && RUNNING_DOWNLOAD_STATUSES.includes(selectedOcrDownloadProgress.status),
  );
  const selectedOcrDownloadPercent = $derived.by(() => {
    const progress = selectedOcrDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  async function loadOcrModelStatus(): Promise<void> {
    loadingOcrModelStatus = true;
    ocrModelError = null;
    try {
      ocrModelStatus = await invoke<OcrModelStatusResponse>("get_ocr_model_status");
    } catch (err) {
      ocrModelError = serializeError(err);
    } finally {
      loadingOcrModelStatus = false;
    }
  }

  async function startSelectedOcrModelDownload(): Promise<void> {
    if (!selectedOcrModel?.modelId) return;
    startingOcrDownload = true;
    ocrDownloadError = null;
    try {
      ocrDownloadProgress = await invoke<OcrModelDownloadProgress>("start_ocr_model_download", {
        request: {
          provider: selectedOcrModel.provider,
          modelId: selectedOcrModel.modelId,
        },
      });
    } catch (err) {
      ocrDownloadError = serializeError(err);
    } finally {
      startingOcrDownload = false;
    }
  }

  async function cancelSelectedOcrModelDownload(): Promise<void> {
    cancellingOcrDownload = true;
    ocrDownloadError = null;
    try {
      await invoke("cancel_ocr_model_download");
    } catch (err) {
      ocrDownloadError = serializeError(err);
    } finally {
      cancellingOcrDownload = false;
    }
  }

  async function handleOcrDownloadProgress(progress: OcrModelDownloadProgress): Promise<void> {
    ocrDownloadProgress = progress;
    // Surface async download failures (mirrors the semantic-search store); the
    // initial `start_*` rejection is caught synchronously, but a failure that
    // arrives later only flows through this progress event.
    if (progress.status === "failed") {
      ocrDownloadError = progress.message ?? "Download failed.";
    }
    if (TERMINAL_DOWNLOAD_STATUSES.includes(progress.status)) {
      await loadOcrModelStatus();
    }
  }

  return {
    get ocrModelStatus() { return ocrModelStatus; },
    get loadingOcrModelStatus() { return loadingOcrModelStatus; },
    get ocrModelError() { return ocrModelError; },
    get ocrDownloadProgress() { return ocrDownloadProgress; },
    get startingOcrDownload() { return startingOcrDownload; },
    get cancellingOcrDownload() { return cancellingOcrDownload; },
    get ocrDownloadError() { return ocrDownloadError; },
    get selectedOcrProviderStatus() { return selectedOcrProviderStatus; },
    get selectedOcrModels() { return selectedOcrModels; },
    get selectedOcrModel() { return selectedOcrModel; },
    get selectedOcrDownloadProgress() { return selectedOcrDownloadProgress; },
    get selectedOcrDownloadRunning() { return selectedOcrDownloadRunning; },
    get selectedOcrDownloadPercent() { return selectedOcrDownloadPercent; },
    get ocrModelOptions() { return ocrModelOptions; },
    ocrStatusLabel: (model: OcrModelStatus) => ocrStatusLabel(model),
    loadOcrModelStatus,
    startSelectedOcrModelDownload,
    cancelSelectedOcrModelDownload,
    handleOcrDownloadProgress,
    // Preferred-model resolution depends on the live status; the controller
    // calls this when the provider changes (matching the legacy page).
    preferredOcrModelIdForProvider(provider: OcrProvider, defaultModelId: string | null): string | null {
      const providerStatus = ocrModelStatus?.providers.find((status) => status.provider === provider);
      if (!providerStatus) return defaultModelId;
      const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
      return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
    },
    isSelectableOcrProvider,
  };
}

export type OcrModelStore = ReturnType<typeof createOcrModelStore>;

// ── Transcription model subsystem ──────────────────────────────────────────
export interface TranscriptionModelStoreAccess {
  transcriptionProvider: () => AudioTranscriptionProvider;
  transcriptionModelId: () => string | null;
}

export function createTranscriptionModelStore(access: TranscriptionModelStoreAccess) {
  let transcriptionModelStatus = $state<AudioTranscriptionModelStatusResponse | null>(null);
  let loadingTranscriptionModelStatus = $state(false);
  let transcriptionModelError = $state<string | null>(null);
  let transcriptionDownloadProgress = $state<AudioTranscriptionModelDownloadProgress | null>(null);
  let startingTranscriptionDownload = $state(false);
  let cancellingTranscriptionDownload = $state(false);
  let transcriptionDownloadError = $state<string | null>(null);

  const selectedTranscriptionProviderStatus = $derived(
    transcriptionModelStatus?.providers.find(
      (provider) => provider.provider === access.transcriptionProvider(),
    ) ?? null,
  );
  const selectedTranscriptionModels = $derived(selectedTranscriptionProviderStatus?.models ?? []);
  const transcriptionModelOptions = $derived(
    selectedTranscriptionModels.map((model) => ({
      value: model.modelId ?? OS_MANAGED_OPTION_VALUE,
      label: `${model.displayName} · ${transcriptionStatusLabel(model)}`,
    })),
  );
  const selectedTranscriptionModel = $derived(
    selectedTranscriptionModels.find((model) => model.modelId === access.transcriptionModelId())
      ?? selectedTranscriptionModels[0]
      ?? null,
  );
  const selectedTranscriptionDownloadProgress = $derived(
    transcriptionDownloadProgress
      && transcriptionDownloadProgress.provider === access.transcriptionProvider()
      && transcriptionDownloadProgress.modelId === access.transcriptionModelId()
      ? transcriptionDownloadProgress
      : null,
  );
  const selectedTranscriptionDownloadRunning = $derived(
    selectedTranscriptionDownloadProgress !== null
      && RUNNING_DOWNLOAD_STATUSES.includes(selectedTranscriptionDownloadProgress.status),
  );
  const selectedTranscriptionDownloadPercent = $derived.by(() => {
    const progress = selectedTranscriptionDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  async function loadTranscriptionModelStatus(): Promise<void> {
    loadingTranscriptionModelStatus = true;
    transcriptionModelError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>(
        "get_audio_transcription_model_status",
      );
    } catch (err) {
      transcriptionModelError = serializeError(err);
    } finally {
      loadingTranscriptionModelStatus = false;
    }
  }

  async function startSelectedTranscriptionModelDownload(): Promise<void> {
    if (!selectedTranscriptionModel?.modelId) return;
    startingTranscriptionDownload = true;
    transcriptionDownloadError = null;
    try {
      transcriptionDownloadProgress = await invoke<AudioTranscriptionModelDownloadProgress>(
        "start_audio_transcription_model_download",
        {
          request: {
            provider: selectedTranscriptionModel.provider,
            modelId: selectedTranscriptionModel.modelId,
          },
        },
      );
    } catch (err) {
      transcriptionDownloadError = serializeError(err);
    } finally {
      startingTranscriptionDownload = false;
    }
  }

  async function cancelSelectedTranscriptionModelDownload(): Promise<void> {
    cancellingTranscriptionDownload = true;
    transcriptionDownloadError = null;
    try {
      await invoke("cancel_audio_transcription_model_download");
    } catch (err) {
      transcriptionDownloadError = serializeError(err);
    } finally {
      cancellingTranscriptionDownload = false;
    }
  }

  async function handleTranscriptionDownloadProgress(
    progress: AudioTranscriptionModelDownloadProgress,
  ): Promise<void> {
    transcriptionDownloadProgress = progress;
    // Surface async download failures (mirrors the semantic-search store); the
    // initial `start_*` rejection is caught synchronously, but a failure that
    // arrives later only flows through this progress event.
    if (progress.status === "failed") {
      transcriptionDownloadError = progress.message ?? "Download failed.";
    }
    if (TERMINAL_DOWNLOAD_STATUSES.includes(progress.status)) {
      await loadTranscriptionModelStatus();
    }
  }

  return {
    get transcriptionModelStatus() { return transcriptionModelStatus; },
    get loadingTranscriptionModelStatus() { return loadingTranscriptionModelStatus; },
    get transcriptionModelError() { return transcriptionModelError; },
    get transcriptionDownloadProgress() { return transcriptionDownloadProgress; },
    get startingTranscriptionDownload() { return startingTranscriptionDownload; },
    get cancellingTranscriptionDownload() { return cancellingTranscriptionDownload; },
    get transcriptionDownloadError() { return transcriptionDownloadError; },
    get selectedTranscriptionProviderStatus() { return selectedTranscriptionProviderStatus; },
    get selectedTranscriptionModels() { return selectedTranscriptionModels; },
    get selectedTranscriptionModel() { return selectedTranscriptionModel; },
    get selectedTranscriptionDownloadProgress() { return selectedTranscriptionDownloadProgress; },
    get selectedTranscriptionDownloadRunning() { return selectedTranscriptionDownloadRunning; },
    get selectedTranscriptionDownloadPercent() { return selectedTranscriptionDownloadPercent; },
    get transcriptionModelOptions() { return transcriptionModelOptions; },
    transcriptionStatusLabel: (model: AudioTranscriptionModelStatus) => transcriptionStatusLabel(model),
    loadTranscriptionModelStatus,
    startSelectedTranscriptionModelDownload,
    cancelSelectedTranscriptionModelDownload,
    handleTranscriptionDownloadProgress,
    preferredTranscriptionModelIdForProvider(
      provider: AudioTranscriptionProvider,
      defaultModelId: string | null,
    ): string | null {
      const providerStatus = transcriptionModelStatus?.providers.find(
        (status) => status.provider === provider,
      );
      if (!providerStatus) return defaultModelId;
      const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
      return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
    },
  };
}

export type TranscriptionModelStore = ReturnType<typeof createTranscriptionModelStore>;

// ── Speaker analysis model subsystem ───────────────────────────────────────
// Same shape as OCR/transcription, but presets are keyed by (provider, modelId)
// rather than a bare model id (matching the Settings panel's preset picker), so
// the option `value` is a `speakerPresetKey` and `chooseSpeakerModel` parses it.
export interface SpeakerModelStoreAccess {
  speakerProvider: () => string;
  speakerModelId: () => string | null;
}

export function createSpeakerModelStore(access: SpeakerModelStoreAccess) {
  let speakerModelStatus = $state<SpeakerAnalysisModelStatusResponse | null>(null);
  let loadingSpeakerModelStatus = $state(false);
  let speakerModelError = $state<string | null>(null);
  let speakerDownloadProgress = $state<SpeakerAnalysisModelDownloadProgress | null>(null);
  let startingSpeakerDownload = $state(false);
  let cancellingSpeakerDownload = $state(false);
  let speakerDownloadError = $state<string | null>(null);

  const allSpeakerModels = $derived(
    (speakerModelStatus?.providers ?? []).flatMap((provider) => provider.models),
  );
  const selectedSpeakerModel = $derived(
    allSpeakerModels.find(
      (model) => model.provider === access.speakerProvider() && model.modelId === access.speakerModelId(),
    )
      ?? allSpeakerModels.find((model) => model.modelId === access.speakerModelId())
      ?? null,
  );
  const speakerModelOptions = $derived(
    allSpeakerModels.map((model) => ({
      value: speakerPresetKey(model.provider, model.modelId),
      label: model.download
        ? `${model.displayName} · ${formatBytes(model.download.byteSize)}`
        : model.displayName,
    })),
  );
  const selectedSpeakerPresetKey = $derived(
    selectedSpeakerModel
      ? speakerPresetKey(selectedSpeakerModel.provider, selectedSpeakerModel.modelId)
      : speakerPresetKey(access.speakerProvider(), access.speakerModelId()),
  );
  const selectedSpeakerDownloadProgress = $derived(
    speakerDownloadProgress
      && speakerDownloadProgress.provider === selectedSpeakerModel?.provider
      && speakerDownloadProgress.modelId === selectedSpeakerModel?.modelId
      ? speakerDownloadProgress
      : null,
  );
  const selectedSpeakerDownloadRunning = $derived(
    selectedSpeakerDownloadProgress !== null
      && RUNNING_DOWNLOAD_STATUSES.includes(selectedSpeakerDownloadProgress.status),
  );
  const selectedSpeakerDownloadPercent = $derived.by(() => {
    const progress = selectedSpeakerDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  async function loadSpeakerModelStatus(): Promise<void> {
    loadingSpeakerModelStatus = true;
    speakerModelError = null;
    try {
      speakerModelStatus = await invoke<SpeakerAnalysisModelStatusResponse>(
        "get_speaker_analysis_model_status",
      );
    } catch (err) {
      speakerModelError = serializeError(err);
    } finally {
      loadingSpeakerModelStatus = false;
    }
  }

  async function startSelectedSpeakerModelDownload(): Promise<void> {
    if (!selectedSpeakerModel?.modelId) return;
    startingSpeakerDownload = true;
    speakerDownloadError = null;
    try {
      speakerDownloadProgress = await invoke<SpeakerAnalysisModelDownloadProgress>(
        "start_speaker_analysis_model_download",
        {
          request: {
            provider: selectedSpeakerModel.provider,
            modelId: selectedSpeakerModel.modelId,
          },
        },
      );
    } catch (err) {
      speakerDownloadError = serializeError(err);
    } finally {
      startingSpeakerDownload = false;
    }
  }

  async function cancelSelectedSpeakerModelDownload(): Promise<void> {
    cancellingSpeakerDownload = true;
    speakerDownloadError = null;
    try {
      await invoke("cancel_speaker_analysis_model_download");
    } catch (err) {
      speakerDownloadError = serializeError(err);
    } finally {
      cancellingSpeakerDownload = false;
    }
  }

  async function handleSpeakerDownloadProgress(
    progress: SpeakerAnalysisModelDownloadProgress,
  ): Promise<void> {
    speakerDownloadProgress = progress;
    // Surface async download failures (mirrors the semantic-search store); the
    // initial `start_*` rejection is caught synchronously, but a failure that
    // arrives later only flows through this progress event.
    if (progress.status === "failed") {
      speakerDownloadError = progress.message ?? "Download failed.";
    }
    if (TERMINAL_DOWNLOAD_STATUSES.includes(progress.status)) {
      await loadSpeakerModelStatus();
    }
  }

  // Preset picker writes `provider::modelId`; onboarding has no saved people to
  // warn about (first run), so the parse just splits and applies — no dialog.
  function parseSpeakerPresetKey(value: string): { provider: string; modelId: string | null } {
    const [provider, rawModelId] = value.split("::");
    const modelId = !rawModelId || rawModelId === "__os_managed__" ? null : rawModelId;
    return { provider, modelId };
  }

  return {
    get speakerModelStatus() { return speakerModelStatus; },
    get loadingSpeakerModelStatus() { return loadingSpeakerModelStatus; },
    get speakerModelError() { return speakerModelError; },
    get speakerDownloadProgress() { return speakerDownloadProgress; },
    get startingSpeakerDownload() { return startingSpeakerDownload; },
    get cancellingSpeakerDownload() { return cancellingSpeakerDownload; },
    get speakerDownloadError() { return speakerDownloadError; },
    get selectedSpeakerModel() { return selectedSpeakerModel; },
    get speakerModelOptions() { return speakerModelOptions; },
    get selectedSpeakerPresetKey() { return selectedSpeakerPresetKey; },
    get selectedSpeakerDownloadProgress() { return selectedSpeakerDownloadProgress; },
    get selectedSpeakerDownloadRunning() { return selectedSpeakerDownloadRunning; },
    get selectedSpeakerDownloadPercent() { return selectedSpeakerDownloadPercent; },
    speakerStatusLabel: (model: SpeakerAnalysisModelStatus) => speakerStatusLabel(model),
    loadSpeakerModelStatus,
    startSelectedSpeakerModelDownload,
    cancelSelectedSpeakerModelDownload,
    handleSpeakerDownloadProgress,
    parseSpeakerPresetKey,
  };
}

export type SpeakerModelStore = ReturnType<typeof createSpeakerModelStore>;

// ── Semantic search model subsystem ────────────────────────────────────────
// Mirrors the OCR store, but the picker mixes a guided/recommended tier list
// (from `get_semantic_search_model_status`) with a custom catalog (from
// `list_semantic_search_supported_models`), so the picked-model view resolves
// live status first and falls back to the catalog (mirrors the Settings
// controller's `semanticSearchPickedModel`). Onboarding only DOWNLOADS live
// (matching OCR/transcription); model SELECTION is a draft committed at finish,
// so this store never calls `select_semantic_search_model`.
export interface SemanticSearchModelStoreAccess {
  semanticSearchModelId: () => string | null;
}

// The render-ready view the body reads for the picked-model card. `available`
// is true only when the live status reports the model installed; catalog-only
// (custom) models are never installed yet.
export interface SemanticSearchPickedModel {
  modelId: string;
  provider: string | null;
  displayName: string;
  description: string;
  metaLine: string;
  available: boolean;
  approxDownloadBytes: number | null;
}

export function createSemanticSearchModelStore(access: SemanticSearchModelStoreAccess) {
  let semanticSearchModelStatus = $state<SemanticSearchModelStatusResponse | null>(null);
  let loadingSemanticSearchModelStatus = $state(false);
  let semanticSearchModelError = $state<string | null>(null);
  let semanticSearchSupportedModels = $state<SemanticSearchSupportedModel[]>([]);
  let loadingSemanticSearchSupportedModels = $state(false);
  let semanticSearchSupportedModelsError = $state<string | null>(null);
  let semanticSearchDownloadProgress = $state<SemanticSearchModelDownloadProgress | null>(null);
  let startingSemanticSearchDownload = $state(false);
  let cancellingSemanticSearchDownload = $state(false);
  let semanticSearchDownloadError = $state<string | null>(null);

  // Provider is uniform across the live status list (one on-device provider);
  // catalog-only picks inherit it so a download can name the provider.
  const semanticSearchProvider = $derived(
    (semanticSearchModelStatus?.models ?? [])[0]?.provider ?? null,
  );
  const semanticSearchGuidedModels = $derived(
    (semanticSearchModelStatus?.models ?? []).filter((m) => m.tier !== "custom"),
  );
  const semanticSearchGuidedModelIds = $derived(
    new Set(semanticSearchGuidedModels.map((m) => m.modelId)),
  );
  const semanticSearchCustomOptions = $derived(
    semanticSearchSupportedModels.filter((m) => !semanticSearchGuidedModelIds.has(m.modelId)),
  );
  // Guided/recommended tiers first, then custom catalog models (mirrors the
  // Settings controller's `semanticSearchModelOptions`).
  const semanticSearchModelOptions = $derived([
    ...semanticSearchGuidedModels.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} · ${m.dimension}d${m.tier === "multilingual" ? " · multilingual" : ""} · recommended`,
    })),
    ...semanticSearchCustomOptions.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} — ${m.dimension}d${m.multilingual ? " · multilingual" : ""}`,
    })),
  ]);

  const selectedSemanticSearchModel = $derived.by((): SemanticSearchPickedModel | null => {
    const id = access.semanticSearchModelId();
    if (!id) return null;
    const live = (semanticSearchModelStatus?.models ?? []).find((m) => m.modelId === id);
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
    const catalog = semanticSearchSupportedModels.find((m) => m.modelId === id);
    if (catalog) {
      const size =
        catalog.approxDownloadBytes != null ? `${formatBytes(catalog.approxDownloadBytes)} on disk · ` : "";
      return {
        modelId: catalog.modelId,
        provider: semanticSearchProvider,
        displayName: catalog.displayName,
        description: catalog.description,
        metaLine: `${semanticSearchTierLabel("custom")} · ${size}${catalog.dimension}-dim · runs on-device${catalog.multilingual ? " · multilingual" : ""}`,
        available: false,
        approxDownloadBytes: catalog.approxDownloadBytes,
      };
    }
    return null;
  });

  const selectedSemanticSearchDownloadProgress = $derived(
    semanticSearchDownloadProgress
      && semanticSearchDownloadProgress.modelId === access.semanticSearchModelId()
      ? semanticSearchDownloadProgress
      : null,
  );
  const selectedSemanticSearchDownloadRunning = $derived(
    selectedSemanticSearchDownloadProgress !== null
      && RUNNING_DOWNLOAD_STATUSES.includes(selectedSemanticSearchDownloadProgress.status),
  );
  const selectedSemanticSearchDownloadPercent = $derived.by(() => {
    const progress = selectedSemanticSearchDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  async function loadSemanticSearchModelStatus(): Promise<void> {
    loadingSemanticSearchModelStatus = true;
    semanticSearchModelError = null;
    try {
      semanticSearchModelStatus = await invoke<SemanticSearchModelStatusResponse>(
        "get_semantic_search_model_status",
      );
    } catch (err) {
      semanticSearchModelError = serializeError(err);
    } finally {
      loadingSemanticSearchModelStatus = false;
    }
  }

  async function loadSemanticSearchSupportedModels(): Promise<void> {
    loadingSemanticSearchSupportedModels = true;
    semanticSearchSupportedModelsError = null;
    try {
      semanticSearchSupportedModels = await invoke<SemanticSearchSupportedModel[]>(
        "list_semantic_search_supported_models",
      );
    } catch (err) {
      semanticSearchSupportedModelsError = serializeError(err);
    } finally {
      loadingSemanticSearchSupportedModels = false;
    }
  }

  async function startSelectedSemanticSearchModelDownload(): Promise<void> {
    const model = selectedSemanticSearchModel;
    if (!model?.provider) return;
    startingSemanticSearchDownload = true;
    semanticSearchDownloadError = null;
    try {
      semanticSearchDownloadProgress = await invoke<SemanticSearchModelDownloadProgress>(
        "start_semantic_search_model_download",
        {
          request: {
            provider: model.provider,
            modelId: model.modelId,
          },
        },
      );
    } catch (err) {
      semanticSearchDownloadError = serializeError(err);
    } finally {
      startingSemanticSearchDownload = false;
    }
  }

  async function cancelSelectedSemanticSearchModelDownload(): Promise<void> {
    cancellingSemanticSearchDownload = true;
    semanticSearchDownloadError = null;
    try {
      await invoke("cancel_semantic_search_model_download");
    } catch (err) {
      semanticSearchDownloadError = serializeError(err);
    } finally {
      cancellingSemanticSearchDownload = false;
    }
  }

  async function handleSemanticSearchDownloadProgress(
    progress: SemanticSearchModelDownloadProgress,
  ): Promise<void> {
    semanticSearchDownloadProgress = progress;
    if (progress.status === "failed") {
      semanticSearchDownloadError = progress.message ?? "Download failed.";
    }
    if (TERMINAL_DOWNLOAD_STATUSES.includes(progress.status)) {
      await loadSemanticSearchModelStatus();
    }
  }

  return {
    get semanticSearchModelStatus() { return semanticSearchModelStatus; },
    get loadingSemanticSearchModelStatus() { return loadingSemanticSearchModelStatus; },
    get semanticSearchModelError() { return semanticSearchModelError; },
    get semanticSearchSupportedModels() { return semanticSearchSupportedModels; },
    get loadingSemanticSearchSupportedModels() { return loadingSemanticSearchSupportedModels; },
    get semanticSearchSupportedModelsError() { return semanticSearchSupportedModelsError; },
    get semanticSearchDownloadProgress() { return semanticSearchDownloadProgress; },
    get startingSemanticSearchDownload() { return startingSemanticSearchDownload; },
    get cancellingSemanticSearchDownload() { return cancellingSemanticSearchDownload; },
    get semanticSearchDownloadError() { return semanticSearchDownloadError; },
    get semanticSearchModelOptions() { return semanticSearchModelOptions; },
    get selectedSemanticSearchModel() { return selectedSemanticSearchModel; },
    get selectedSemanticSearchDownloadProgress() { return selectedSemanticSearchDownloadProgress; },
    get selectedSemanticSearchDownloadRunning() { return selectedSemanticSearchDownloadRunning; },
    get selectedSemanticSearchDownloadPercent() { return selectedSemanticSearchDownloadPercent; },
    semanticSearchTierLabel: (model: SemanticSearchModelStatus) => semanticSearchTierLabel(model.tier),
    loadSemanticSearchModelStatus,
    loadSemanticSearchSupportedModels,
    startSelectedSemanticSearchModelDownload,
    cancelSelectedSemanticSearchModelDownload,
    handleSemanticSearchDownloadProgress,
  };
}

export type SemanticSearchModelStore = ReturnType<typeof createSemanticSearchModelStore>;
