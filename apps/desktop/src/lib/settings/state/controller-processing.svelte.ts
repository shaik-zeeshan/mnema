// Processing-model derivations for the settings controller — Slice-5.
//
// The OCR / Transcription / Speaker option lists + selected-model + download-
// progress derivations and their chooser helpers live here so the main
// `controller.svelte.ts` stays under the 800-line file cap. This is a pure
// re-home of the page-local derivations: it reads the recording-draft store
// (`rec`) and the model-status store (`models`). It is a FACTORY (not a class)
// so the `rec`/`models` references are closure variables defined before any
// `$derived` — a class would trip "used before initialization" because class
// `$derived` field initializers run before the constructor body assigns the
// store refs. The controller composes one instance and re-exposes its members
// so panel markup references stay flat (`c.selectedOcrModel`, etc.) and verbatim.

import { ask } from "@tauri-apps/plugin-dialog";
import { formatBytes } from "./format";
import {
  ocrStatusLabel,
  transcriptionStatusLabel,
  speakerPresetKey,
  defaultOcrModelIdForProvider,
  defaultOcrLanguageForProvider,
  defaultTranscriptionModelIdForProvider,
  isSelectableOcrProvider as ocrProviderInBackendStatus,
  isSelectableAudioTranscriptionProvider as transcriptionProviderInBackendStatus,
  shouldConfirmDeepgramSwitch,
} from "./models-format";
import type { RecordingStore } from "./recording.svelte";
import type { createModelStatusStore } from "./model-status.svelte";
import type {
  OcrModelDownloadProgress,
  OcrProvider,
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionProvider,
  SpeakerAnalysisModelDownloadProgress,
} from "$lib/types";

type ModelStatusStore = ReturnType<typeof createModelStatusStore>;

export function createProcessingModelsView(rec: RecordingStore, models: ModelStatusStore) {
  // The selectable set is whatever the backend status response returns — it omits
  // platform-locked providers server-side, so there is no hardcoded list (and no
  // `if windows`) here. Reading `models.*ModelStatus` inside these closures keeps
  // the option derivations reactive to the live status.
  function isSelectableOcrProvider(value: string | null | undefined): value is OcrProvider {
    return ocrProviderInBackendStatus(value, models.ocrModelStatus);
  }
  function isSelectableTranscriptionProvider(
    value: string | null | undefined,
  ): value is AudioTranscriptionProvider {
    return transcriptionProviderInBackendStatus(value, models.transcriptionModelStatus);
  }

  // ─── OCR option derivations ────────────────────────────────────────────────
  const ocrProviderOptions = $derived(
    (models.ocrModelStatus?.providers ?? [])
      .filter((provider) => isSelectableOcrProvider(provider.provider))
      .map((provider) => ({
        value: provider.provider,
        label: provider.displayName,
        description: provider.models.some((model) => model.available)
          ? "Available now"
          : "Unavailable or missing",
      })),
  );
  const selectedOcrProviderStatus = $derived(
    models.ocrModelStatus?.providers.find((provider) => provider.provider === rec.draftOcrProvider) ?? null,
  );
  const selectedOcrModels = $derived(selectedOcrProviderStatus?.models ?? []);
  const ocrModelOptions = $derived(
    selectedOcrModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: `${model.displayName} · ${ocrStatusLabel(model)}`,
    })),
  );
  const selectedOcrModel = $derived(
    selectedOcrModels.find((model) => model.modelId === rec.draftOcrModelId) ?? selectedOcrModels[0] ?? null,
  );
  const selectedOcrDownloadProgress = $derived(
    models.ocrDownloadProgress
      && models.ocrDownloadProgress.provider === rec.draftOcrProvider
      && models.ocrDownloadProgress.modelId === rec.draftOcrModelId
      ? models.ocrDownloadProgress
      : null,
  );
  const selectedOcrDownloadRunning = $derived(
    selectedOcrDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedOcrDownloadProgress.status),
  );
  const selectedOcrDownloadPercent = $derived.by(() => {
    const progress = selectedOcrDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  function preferredOcrModelIdForProvider(provider: OcrProvider): string | null {
    const providerStatus = models.ocrModelStatus?.providers.find((status) => status.provider === provider);
    const defaultModelId = defaultOcrModelIdForProvider(provider);
    if (!providerStatus) return defaultModelId;
    const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
    return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
  }
  function chooseOcrProvider(provider: string) {
    if (!isSelectableOcrProvider(provider)) return;
    rec.draftOcrProvider = provider;
    rec.draftOcrModelId = preferredOcrModelIdForProvider(rec.draftOcrProvider);
    rec.draftOcrLanguage = defaultOcrLanguageForProvider(rec.draftOcrProvider) ?? "";
  }
  function chooseOcrModel(value: string) {
    rec.draftOcrModelId = value === "__os_managed__" ? null : value;
  }

  // ─── Transcription option derivations ──────────────────────────────────────
  const transcriptionProviderOptions = $derived(
    (models.transcriptionModelStatus?.providers ?? [])
      .filter((provider) => isSelectableTranscriptionProvider(provider.provider))
      .map((provider) => ({
        value: provider.provider,
        label: provider.displayName,
        description: provider.models.some((model) => model.available)
          ? "At least one model is available"
          : "No available model detected",
      })),
  );
  const selectedTranscriptionProviderStatus = $derived(
    models.transcriptionModelStatus?.providers.find((provider) => provider.provider === rec.draftTranscriptionProvider) ?? null,
  );
  const selectedTranscriptionModels = $derived(selectedTranscriptionProviderStatus?.models ?? []);
  const transcriptionModelOptions = $derived(
    selectedTranscriptionModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: `${model.displayName} · ${transcriptionStatusLabel(model)}`,
    })),
  );
  const selectedTranscriptionModel = $derived(
    selectedTranscriptionModels.find((model) => model.modelId === rec.draftTranscriptionModelId) ?? selectedTranscriptionModels[0] ?? null,
  );
  const selectedAppleSpeechPermissionStatus = $derived(
    selectedTranscriptionModel?.provider === "apple_speech_on_device"
      ? selectedTranscriptionModel.availabilityStatus
      : null,
  );
  const selectedAppleSpeechNeedsPermission = $derived(
    selectedAppleSpeechPermissionStatus === "permission_not_determined"
      || selectedAppleSpeechPermissionStatus === "permission_denied"
      || selectedAppleSpeechPermissionStatus === "permission_restricted",
  );
  const selectedTranscriptionDownloadProgress = $derived(
    models.transcriptionDownloadProgress
      && models.transcriptionDownloadProgress.provider === rec.draftTranscriptionProvider
      && models.transcriptionDownloadProgress.modelId === rec.draftTranscriptionModelId
      ? models.transcriptionDownloadProgress
      : null,
  );
  const selectedTranscriptionDownloadRunning = $derived(
    selectedTranscriptionDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedTranscriptionDownloadProgress.status),
  );
  const selectedTranscriptionDownloadPercent = $derived.by(() => {
    const progress = selectedTranscriptionDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  function preferredTranscriptionModelIdForProvider(provider: AudioTranscriptionProvider): string | null {
    const providerStatus = models.transcriptionModelStatus?.providers.find((status) => status.provider === provider);
    const defaultModelId = defaultTranscriptionModelIdForProvider(provider);
    if (!providerStatus) return defaultModelId;
    const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
    return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
  }
  async function chooseTranscriptionProvider(provider: string) {
    if (!isSelectableTranscriptionProvider(provider)) return;
    const previousProvider = rec.draftTranscriptionProvider;
    // Apply first so the controlled Provider RadioGroup reflects the click while the blocking
    // consent dialog is open, then revert on cancel. The RadioGroup writes its selection into its
    // one-way `value={draftTranscriptionProvider}` prop locally; if we returned without changing the
    // draft, that unchanged expression would never re-sync and the control would stay stuck on
    // "Deepgram" while the draft (and every `provider === "deepgram"` block) said otherwise.
    rec.draftTranscriptionProvider = provider;
    rec.draftTranscriptionModelId = preferredTranscriptionModelIdForProvider(provider);
    if (shouldConfirmDeepgramSwitch(provider, previousProvider)) {
      const ok = await ask(
        "Switching to Deepgram uploads your microphone and system-audio recordings to Deepgram's "
          + "cloud service, under your own Deepgram account and data policies. Only audio recorded "
          + "from now on is affected — existing transcripts stay on your device. Continue?",
        { title: "Send audio to Deepgram?", kind: "warning", okLabel: "Use Deepgram", cancelLabel: "Cancel" },
      );
      if (!ok) {
        rec.draftTranscriptionProvider = previousProvider;
        rec.draftTranscriptionModelId = preferredTranscriptionModelIdForProvider(previousProvider);
      }
    }
  }
  function chooseTranscriptionModel(value: string) {
    rec.draftTranscriptionModelId = value === "__os_managed__" ? null : value;
  }

  // ─── Speaker option derivations ────────────────────────────────────────────
  const allSpeakerModels = $derived(
    (models.speakerModelStatus?.providers ?? []).flatMap((provider) => provider.models),
  );
  const selectedSpeakerModel = $derived(
    allSpeakerModels.find(
      (model) => model.provider === rec.draftSpeakerProvider && model.modelId === rec.draftSpeakerModelId,
    )
      ?? allSpeakerModels.find((model) => model.modelId === rec.draftSpeakerModelId)
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
      : speakerPresetKey(rec.draftSpeakerProvider, rec.draftSpeakerModelId),
  );
  const selectedSpeakerDownloadProgress = $derived(
    models.speakerDownloadProgress
      && models.speakerDownloadProgress.provider === selectedSpeakerModel?.provider
      && models.speakerDownloadProgress.modelId === selectedSpeakerModel?.modelId
      ? models.speakerDownloadProgress
      : null,
  );
  const selectedSpeakerDownloadRunning = $derived(
    selectedSpeakerDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedSpeakerDownloadProgress.status),
  );
  const selectedSpeakerDownloadPercent = $derived.by(() => {
    const progress = selectedSpeakerDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  });

  async function chooseSpeakerModel(value: string) {
    const [nextProvider, rawModelId] = value.split("::");
    const nextModelId = !rawModelId || rawModelId === "__os_managed__" ? null : rawModelId;
    if (nextProvider === rec.draftSpeakerProvider && nextModelId === rec.draftSpeakerModelId) return;

    const savedProvider = rec.recordingSettings?.speakerAnalysis?.provider ?? null;
    const savedModelId = rec.recordingSettings?.speakerAnalysis?.modelId ?? null;
    const switchingAwayFromSaved =
      nextProvider !== savedProvider || nextModelId !== savedModelId;
    const needsWarning =
      switchingAwayFromSaved && rec.draftSpeakerRecognizeSavedPeople && models.personProfileCount > 0;

    if (needsWarning) {
      models.switchingSpeakerModel = true;
      try {
        const ok = await ask(
          "Switching the speaker model is safe and reversible — your saved people are not deleted. "
            + "But saved voices won't be recognized under the new model until you re-tag each person once. "
            + "Switching back to the previous model restores them. Switch anyway?",
          { title: "Switch speaker model?", kind: "warning", okLabel: "Switch", cancelLabel: "Keep current" },
        );
        if (!ok) return;
      } finally {
        models.switchingSpeakerModel = false;
      }
    }

    rec.draftSpeakerProvider = nextProvider;
    rec.draftSpeakerModelId = nextModelId;
  }

  // ─── Model loaders / download wrappers (draft-derived) ──────────────────────
  return {
    isSelectableOcrProvider,
    isSelectableTranscriptionProvider,
    chooseOcrProvider,
    chooseOcrModel,
    preferredOcrModelIdForProvider,
    chooseTranscriptionProvider,
    chooseTranscriptionModel,
    preferredTranscriptionModelIdForProvider,
    chooseSpeakerModel,

    // OCR loaders
    loadOcrModelStatus: () => models.loadOcrModelStatus(),
    startSelectedOcrModelDownload: () => {
      if (!selectedOcrModel?.modelId) return;
      return models.startOcrModelDownload(selectedOcrModel.provider, selectedOcrModel.modelId);
    },
    cancelSelectedOcrModelDownload: () => models.cancelOcrModelDownload(),
    handleOcrDownloadProgress: (progress: OcrModelDownloadProgress) =>
      models.handleOcrDownloadProgress(progress),
    requestDeleteUnusedOcrModels: () => models.requestDeleteUnusedOcrModels(),

    // Transcription loaders
    loadTranscriptionModelStatus: () => models.loadTranscriptionModelStatus(),
    requestAppleSpeechPermission: () => models.requestAppleSpeechPermission(),
    openAppleSpeechPrivacySettings: () => models.openAppleSpeechPrivacySettings(),
    startSelectedTranscriptionModelDownload: () => {
      if (!selectedTranscriptionModel?.modelId) return;
      return models.startTranscriptionModelDownload(
        selectedTranscriptionModel.provider,
        selectedTranscriptionModel.modelId,
      );
    },
    cancelSelectedTranscriptionModelDownload: () => models.cancelTranscriptionModelDownload(),
    handleTranscriptionDownloadProgress: (progress: AudioTranscriptionModelDownloadProgress) =>
      models.handleTranscriptionDownloadProgress(progress),
    requestDeleteUnusedTranscriptionModels: () => models.requestDeleteUnusedTranscriptionModels(),

    // Speaker loaders
    loadSpeakerModelStatus: () => models.loadSpeakerModelStatus(),
    loadPersonProfileCount: () => models.loadPersonProfileCount(),
    startSelectedSpeakerModelDownload: () => {
      if (!selectedSpeakerModel?.modelId) return;
      return models.startSpeakerModelDownload(selectedSpeakerModel.provider, selectedSpeakerModel.modelId);
    },
    cancelSelectedSpeakerModelDownload: () => models.cancelSpeakerModelDownload(),
    deleteSelectedSpeakerModel: () => {
      if (!selectedSpeakerModel?.modelId) return;
      return models.deleteSpeakerModel(
        selectedSpeakerModel.provider,
        selectedSpeakerModel.modelId,
        selectedSpeakerModel.displayName,
      );
    },
    handleSpeakerDownloadProgress: (progress: SpeakerAnalysisModelDownloadProgress) =>
      models.handleSpeakerDownloadProgress(progress),

    // OCR derivations
    get ocrProviderOptions() { return ocrProviderOptions; },
    get selectedOcrProviderStatus() { return selectedOcrProviderStatus; },
    get selectedOcrModels() { return selectedOcrModels; },
    get ocrModelOptions() { return ocrModelOptions; },
    get selectedOcrModel() { return selectedOcrModel; },
    get selectedOcrDownloadProgress() { return selectedOcrDownloadProgress; },
    get selectedOcrDownloadRunning() { return selectedOcrDownloadRunning; },
    get selectedOcrDownloadPercent() { return selectedOcrDownloadPercent; },

    // Transcription derivations
    get transcriptionProviderOptions() { return transcriptionProviderOptions; },
    get selectedTranscriptionProviderStatus() { return selectedTranscriptionProviderStatus; },
    get selectedTranscriptionModels() { return selectedTranscriptionModels; },
    get transcriptionModelOptions() { return transcriptionModelOptions; },
    get selectedTranscriptionModel() { return selectedTranscriptionModel; },
    get selectedAppleSpeechPermissionStatus() { return selectedAppleSpeechPermissionStatus; },
    get selectedAppleSpeechNeedsPermission() { return selectedAppleSpeechNeedsPermission; },
    get selectedTranscriptionDownloadProgress() { return selectedTranscriptionDownloadProgress; },
    get selectedTranscriptionDownloadRunning() { return selectedTranscriptionDownloadRunning; },
    get selectedTranscriptionDownloadPercent() { return selectedTranscriptionDownloadPercent; },

    // Speaker derivations
    get allSpeakerModels() { return allSpeakerModels; },
    get selectedSpeakerModel() { return selectedSpeakerModel; },
    get speakerModelOptions() { return speakerModelOptions; },
    get selectedSpeakerPresetKey() { return selectedSpeakerPresetKey; },
    get selectedSpeakerDownloadProgress() { return selectedSpeakerDownloadProgress; },
    get selectedSpeakerDownloadRunning() { return selectedSpeakerDownloadRunning; },
    get selectedSpeakerDownloadPercent() { return selectedSpeakerDownloadPercent; },
  };
}

export type ProcessingModelsView = ReturnType<typeof createProcessingModelsView>;
