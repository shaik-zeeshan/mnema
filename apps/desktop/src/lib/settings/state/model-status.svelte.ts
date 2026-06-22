// Processing-panel model status state: the OCR / transcription / speaker /
// semantic-search status snapshots, their download-progress objects, and the
// loaders, cancels, progress handlers, and delete-unused flows that operate on
// them. Owns its own non-draft reactive state.
//
// The draft-coupled bits stay in the page: the `$derived` selectors that read
// `draft*ModelId` (e.g. `selectedOcrModel`) and the `start*Selected*` /
// `choose*` flows that mutate drafts or read those selectors. Those call into
// this store for the status objects and to start downloads.

import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import type {
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionModelStatusResponse,
  DeleteUnusedAudioTranscriptionModelsResponse,
  DeleteUnusedOcrModelsResponse,
  OcrModelDownloadProgress,
  OcrModelStatusResponse,
  PersonProfileDto,
  SemanticSearchModelDownloadProgress,
  SemanticSearchModelStatus,
  SemanticSearchModelStatusResponse,
  SemanticSearchSupportedModel,
  SpeakerAnalysisModelDownloadProgress,
  SpeakerAnalysisModelStatusResponse,
} from "$lib/types";
import { errorText } from "./format";

export function createModelStatusStore() {
  // ── OCR ───────────────────────────────────────────────────────────────────
  let ocrModelStatus = $state<OcrModelStatusResponse | null>(null);
  let loadingOcrModelStatus = $state(false);
  let ocrModelError = $state<string | null>(null);
  let ocrDownloadProgress = $state<OcrModelDownloadProgress | null>(null);
  let startingOcrDownload = $state(false);
  let cancellingOcrDownload = $state(false);
  let ocrDownloadError = $state<string | null>(null);
  let deletingUnusedOcrModels = $state(false);
  let confirmingDeleteUnusedOcrModels = $state(false);
  let deleteUnusedOcrModelsMessage = $state<string | null>(null);
  let deletedUnusedOcrModelLabels = $state<string[]>([]);
  let skippedUnusedOcrModelLabels = $state<string[]>([]);
  let skippedOcrProcessingJobModelLabels = $state<string[]>([]);
  let deleteUnusedOcrModelsError = $state<string | null>(null);

  // ── Transcription ───────────────────────────────────────────────────────
  let transcriptionModelStatus = $state<AudioTranscriptionModelStatusResponse | null>(null);
  let loadingTranscriptionModelStatus = $state(false);
  let transcriptionModelError = $state<string | null>(null);
  let transcriptionDownloadProgress = $state<AudioTranscriptionModelDownloadProgress | null>(null);
  let startingTranscriptionDownload = $state(false);
  let cancellingTranscriptionDownload = $state(false);
  let transcriptionDownloadError = $state<string | null>(null);
  let deletingUnusedTranscriptionModels = $state(false);
  let confirmingDeleteUnusedTranscriptionModels = $state(false);
  let deleteUnusedTranscriptionModelsMessage = $state<string | null>(null);
  let deletedUnusedTranscriptionModelLabels = $state<string[]>([]);
  let skippedUnusedTranscriptionModelLabels = $state<string[]>([]);
  let skippedTranscriptionProcessingJobModelLabels = $state<string[]>([]);
  let deleteUnusedTranscriptionModelsError = $state<string | null>(null);
  let requestingAppleSpeechPermission = $state(false);
  let appleSpeechPermissionError = $state<string | null>(null);

  // ── Speakers ──────────────────────────────────────────────────────────────
  let speakerModelStatus = $state<SpeakerAnalysisModelStatusResponse | null>(null);
  let loadingSpeakerModelStatus = $state(false);
  let speakerModelError = $state<string | null>(null);
  let speakerDownloadProgress = $state<SpeakerAnalysisModelDownloadProgress | null>(null);
  let startingSpeakerDownload = $state(false);
  let cancellingSpeakerDownload = $state(false);
  let speakerDownloadError = $state<string | null>(null);
  let deletingSpeakerModel = $state(false);
  let speakerModelDeleteMessage = $state<string | null>(null);
  let switchingSpeakerModel = $state(false);
  // Saved-person count drives the preset-switch warning.
  let personProfileCount = $state(0);

  // ── Semantic search ─────────────────────────────────────────────────────
  let semanticSearchModelStatus = $state<SemanticSearchModelStatusResponse | null>(null);
  let loadingSemanticSearchModelStatus = $state(false);
  let semanticSearchModelError = $state<string | null>(null);
  let semanticSearchDownloadProgress = $state<SemanticSearchModelDownloadProgress | null>(null);
  let semanticSearchDownloadError = $state<string | null>(null);
  let semanticSearchReindexing = $state(false);
  let semanticSearchReindexMessage = $state<string | null>(null);
  let semanticSearchSupportedModels = $state<SemanticSearchSupportedModel[]>([]);
  let loadingSemanticSearchSupportedModels = $state(false);
  let semanticSearchSupportedModelsError = $state<string | null>(null);

  // ── OCR loaders/actions ─────────────────────────────────────────────────
  async function loadOcrModelStatus() {
    loadingOcrModelStatus = true;
    ocrModelError = null;
    try {
      ocrModelStatus = await invoke<OcrModelStatusResponse>("get_ocr_model_status");
    } catch (err) {
      ocrModelError = errorText(err);
    } finally {
      loadingOcrModelStatus = false;
    }
  }

  async function startOcrModelDownload(provider: string, modelId: string) {
    startingOcrDownload = true;
    ocrDownloadError = null;
    try {
      ocrDownloadProgress = await invoke<OcrModelDownloadProgress>("start_ocr_model_download", {
        request: { provider, modelId },
      });
    } catch (err) {
      ocrDownloadError = errorText(err);
    } finally {
      startingOcrDownload = false;
    }
  }

  async function cancelOcrModelDownload() {
    cancellingOcrDownload = true;
    ocrDownloadError = null;
    try {
      await invoke("cancel_ocr_model_download");
    } catch (err) {
      ocrDownloadError = errorText(err);
    } finally {
      cancellingOcrDownload = false;
    }
  }

  async function handleOcrDownloadProgress(progress: OcrModelDownloadProgress) {
    ocrDownloadProgress = progress;
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadOcrModelStatus();
    }
  }

  async function requestDeleteUnusedOcrModels() {
    const ok = await ask("Delete unused OCR model files?", {
      title: "Delete unused OCR models", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel",
    });
    if (!ok) return;
    confirmingDeleteUnusedOcrModels = false;
    deleteUnusedOcrModelsMessage = null;
    deleteUnusedOcrModelsError = null;
    deletedUnusedOcrModelLabels = [];
    skippedUnusedOcrModelLabels = [];
    await deleteUnusedOcrModels();
  }

  async function deleteUnusedOcrModels() {
    deletingUnusedOcrModels = true;
    deleteUnusedOcrModelsMessage = null;
    deletedUnusedOcrModelLabels = [];
    skippedUnusedOcrModelLabels = [];
    skippedOcrProcessingJobModelLabels = [];
    deleteUnusedOcrModelsError = null;
    try {
      const result = await invoke<DeleteUnusedOcrModelsResponse>("delete_unused_ocr_models");
      const skipped = result.skippedActiveDownloads.length + result.skippedProcessingJobs.length;
      deletedUnusedOcrModelLabels = result.deleted.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      skippedUnusedOcrModelLabels = result.skippedActiveDownloads.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      skippedOcrProcessingJobModelLabels = result.skippedProcessingJobs.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      deleteUnusedOcrModelsMessage =
        result.deleted.length === 0
          ? skipped > 0
            ? `No unused OCR models deleted. ${skipped} running model${skipped === 1 ? "" : "s"} skipped.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed OCR job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`
            : "No unused OCR models found."
          : `Deleted ${result.deleted.length} unused OCR model${result.deleted.length === 1 ? "" : "s"}.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed OCR job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`;
      await loadOcrModelStatus();
    } catch (err) {
      deleteUnusedOcrModelsError = errorText(err);
    } finally {
      deletingUnusedOcrModels = false;
      confirmingDeleteUnusedOcrModels = false;
    }
  }

  // ── Transcription loaders/actions ─────────────────────────────────────────
  async function loadTranscriptionModelStatus() {
    loadingTranscriptionModelStatus = true;
    transcriptionModelError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>("get_audio_transcription_model_status");
    } catch (err) {
      transcriptionModelError = errorText(err);
    } finally {
      loadingTranscriptionModelStatus = false;
    }
  }

  async function requestAppleSpeechPermission() {
    requestingAppleSpeechPermission = true;
    appleSpeechPermissionError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>("request_apple_speech_recognition_permission");
    } catch (err) {
      appleSpeechPermissionError = errorText(err);
      await loadTranscriptionModelStatus();
    } finally {
      requestingAppleSpeechPermission = false;
    }
  }

  async function openAppleSpeechPrivacySettings() {
    appleSpeechPermissionError = null;
    try {
      await invoke("open_apple_speech_recognition_privacy_settings");
    } catch (err) {
      appleSpeechPermissionError = errorText(err);
    }
  }

  async function startTranscriptionModelDownload(provider: string, modelId: string) {
    startingTranscriptionDownload = true;
    transcriptionDownloadError = null;
    try {
      transcriptionDownloadProgress = await invoke<AudioTranscriptionModelDownloadProgress>(
        "start_audio_transcription_model_download",
        { request: { provider, modelId } },
      );
    } catch (err) {
      transcriptionDownloadError = errorText(err);
    } finally {
      startingTranscriptionDownload = false;
    }
  }

  async function cancelTranscriptionModelDownload() {
    cancellingTranscriptionDownload = true;
    transcriptionDownloadError = null;
    try {
      await invoke("cancel_audio_transcription_model_download");
    } catch (err) {
      transcriptionDownloadError = errorText(err);
    } finally {
      cancellingTranscriptionDownload = false;
    }
  }

  async function handleTranscriptionDownloadProgress(progress: AudioTranscriptionModelDownloadProgress) {
    transcriptionDownloadProgress = progress;
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadTranscriptionModelStatus();
    }
  }

  async function requestDeleteUnusedTranscriptionModels() {
    const ok = await ask("Delete unused transcription model files?", {
      title: "Delete unused transcription models", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel",
    });
    if (!ok) return;
    confirmingDeleteUnusedTranscriptionModels = false;
    deleteUnusedTranscriptionModelsMessage = null;
    deleteUnusedTranscriptionModelsError = null;
    deletedUnusedTranscriptionModelLabels = [];
    skippedUnusedTranscriptionModelLabels = [];
    await deleteUnusedTranscriptionModels();
  }

  async function deleteUnusedTranscriptionModels() {
    deletingUnusedTranscriptionModels = true;
    deleteUnusedTranscriptionModelsMessage = null;
    deletedUnusedTranscriptionModelLabels = [];
    skippedUnusedTranscriptionModelLabels = [];
    skippedTranscriptionProcessingJobModelLabels = [];
    deleteUnusedTranscriptionModelsError = null;
    try {
      const result = await invoke<DeleteUnusedAudioTranscriptionModelsResponse>("delete_unused_audio_transcription_models");
      const skipped = result.skippedActiveDownloads.length + result.skippedProcessingJobs.length;
      deletedUnusedTranscriptionModelLabels = result.deleted.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      skippedUnusedTranscriptionModelLabels = result.skippedActiveDownloads.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      skippedTranscriptionProcessingJobModelLabels = result.skippedProcessingJobs.map((m) => `${m.displayName} (${m.provider}/${m.modelId})`);
      deleteUnusedTranscriptionModelsMessage =
        result.deleted.length === 0
          ? skipped > 0
            ? `No unused transcription models deleted. ${skipped} running model${skipped === 1 ? "" : "s"} skipped.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed transcription job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`
            : "No unused transcription models found."
          : `Deleted ${result.deleted.length} unused transcription model${result.deleted.length === 1 ? "" : "s"}.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed transcription job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`;
      await loadTranscriptionModelStatus();
    } catch (err) {
      deleteUnusedTranscriptionModelsError = errorText(err);
    } finally {
      deletingUnusedTranscriptionModels = false;
      confirmingDeleteUnusedTranscriptionModels = false;
    }
  }

  // ── Speaker loaders/actions ─────────────────────────────────────────────
  async function loadSpeakerModelStatus() {
    loadingSpeakerModelStatus = true;
    speakerModelError = null;
    try {
      speakerModelStatus = await invoke<SpeakerAnalysisModelStatusResponse>("get_speaker_analysis_model_status");
    } catch (err) {
      speakerModelError = errorText(err);
    } finally {
      loadingSpeakerModelStatus = false;
    }
  }

  // Best-effort saved-person count for the preset-switch warning. A failed load
  // leaves the count at 0 (no warning), never blocking preset selection.
  async function loadPersonProfileCount() {
    try {
      const profiles = await invoke<PersonProfileDto[]>("list_person_profiles");
      personProfileCount = profiles.length;
    } catch {
      personProfileCount = 0;
    }
  }

  async function startSpeakerModelDownload(provider: string, modelId: string) {
    startingSpeakerDownload = true;
    speakerDownloadError = null;
    speakerModelDeleteMessage = null;
    try {
      speakerDownloadProgress = await invoke<SpeakerAnalysisModelDownloadProgress>(
        "start_speaker_analysis_model_download",
        { request: { provider, modelId } },
      );
    } catch (err) {
      speakerDownloadError = errorText(err);
    } finally {
      startingSpeakerDownload = false;
    }
  }

  async function cancelSpeakerModelDownload() {
    cancellingSpeakerDownload = true;
    speakerDownloadError = null;
    try {
      await invoke("cancel_speaker_analysis_model_download");
    } catch (err) {
      speakerDownloadError = errorText(err);
    } finally {
      cancellingSpeakerDownload = false;
    }
  }

  async function deleteSpeakerModel(provider: string, modelId: string, displayName: string) {
    const ok = await ask(`Delete ${displayName}?`, {
      title: "Delete speaker model", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingSpeakerModel = true;
    speakerModelDeleteMessage = null;
    speakerDownloadError = null;
    try {
      await invoke("delete_speaker_analysis_model", { request: { provider, modelId } });
      speakerModelDeleteMessage = `Deleted ${displayName}.`;
      await loadSpeakerModelStatus();
    } catch (err) {
      speakerDownloadError = errorText(err);
    } finally {
      deletingSpeakerModel = false;
    }
  }

  async function handleSpeakerDownloadProgress(progress: SpeakerAnalysisModelDownloadProgress) {
    speakerDownloadProgress = progress;
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadSpeakerModelStatus();
    }
  }

  // ── Semantic search loaders/actions ───────────────────────────────────────
  async function loadSemanticSearchModelStatus() {
    loadingSemanticSearchModelStatus = true;
    semanticSearchModelError = null;
    try {
      semanticSearchModelStatus = await invoke<SemanticSearchModelStatusResponse>("get_semantic_search_model_status");
    } catch (err) {
      semanticSearchModelError = errorText(err);
    } finally {
      loadingSemanticSearchModelStatus = false;
    }
  }

  async function loadSemanticSearchSupportedModels() {
    loadingSemanticSearchSupportedModels = true;
    semanticSearchSupportedModelsError = null;
    try {
      semanticSearchSupportedModels = await invoke<SemanticSearchSupportedModel[]>("list_semantic_search_supported_models");
    } catch (err) {
      semanticSearchSupportedModelsError = errorText(err);
    } finally {
      loadingSemanticSearchSupportedModels = false;
    }
  }

  async function startSemanticSearchModelDownload(model: SemanticSearchModelStatus) {
    semanticSearchDownloadError = null;
    try {
      semanticSearchDownloadProgress = await invoke<SemanticSearchModelDownloadProgress>(
        "start_semantic_search_model_download",
        { request: { provider: model.provider, modelId: model.modelId } },
      );
    } catch (err) {
      semanticSearchDownloadError = errorText(err);
    }
  }

  async function cancelSemanticSearchModelDownload() {
    semanticSearchDownloadError = null;
    try {
      await invoke("cancel_semantic_search_model_download");
    } catch (err) {
      semanticSearchDownloadError = errorText(err);
    }
  }

  async function handleSemanticSearchDownloadProgress(progress: SemanticSearchModelDownloadProgress) {
    semanticSearchDownloadProgress = progress;
    if (progress.status === "failed") {
      semanticSearchDownloadError = progress.message ?? "Download failed.";
    }
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadSemanticSearchModelStatus();
    }
  }

  return {
    // OCR
    get ocrModelStatus() { return ocrModelStatus; },
    get loadingOcrModelStatus() { return loadingOcrModelStatus; },
    get ocrModelError() { return ocrModelError; },
    get ocrDownloadProgress() { return ocrDownloadProgress; },
    get startingOcrDownload() { return startingOcrDownload; },
    get cancellingOcrDownload() { return cancellingOcrDownload; },
    get ocrDownloadError() { return ocrDownloadError; },
    get deletingUnusedOcrModels() { return deletingUnusedOcrModels; },
    get confirmingDeleteUnusedOcrModels() { return confirmingDeleteUnusedOcrModels; },
    set confirmingDeleteUnusedOcrModels(v: boolean) { confirmingDeleteUnusedOcrModels = v; },
    get deleteUnusedOcrModelsMessage() { return deleteUnusedOcrModelsMessage; },
    get deletedUnusedOcrModelLabels() { return deletedUnusedOcrModelLabels; },
    get skippedUnusedOcrModelLabels() { return skippedUnusedOcrModelLabels; },
    get skippedOcrProcessingJobModelLabels() { return skippedOcrProcessingJobModelLabels; },
    get deleteUnusedOcrModelsError() { return deleteUnusedOcrModelsError; },
    loadOcrModelStatus,
    startOcrModelDownload,
    cancelOcrModelDownload,
    handleOcrDownloadProgress,
    requestDeleteUnusedOcrModels,

    // Transcription
    get transcriptionModelStatus() { return transcriptionModelStatus; },
    get loadingTranscriptionModelStatus() { return loadingTranscriptionModelStatus; },
    get transcriptionModelError() { return transcriptionModelError; },
    get transcriptionDownloadProgress() { return transcriptionDownloadProgress; },
    get startingTranscriptionDownload() { return startingTranscriptionDownload; },
    get cancellingTranscriptionDownload() { return cancellingTranscriptionDownload; },
    get transcriptionDownloadError() { return transcriptionDownloadError; },
    get deletingUnusedTranscriptionModels() { return deletingUnusedTranscriptionModels; },
    get confirmingDeleteUnusedTranscriptionModels() { return confirmingDeleteUnusedTranscriptionModels; },
    set confirmingDeleteUnusedTranscriptionModels(v: boolean) { confirmingDeleteUnusedTranscriptionModels = v; },
    get deleteUnusedTranscriptionModelsMessage() { return deleteUnusedTranscriptionModelsMessage; },
    get deletedUnusedTranscriptionModelLabels() { return deletedUnusedTranscriptionModelLabels; },
    get skippedUnusedTranscriptionModelLabels() { return skippedUnusedTranscriptionModelLabels; },
    get skippedTranscriptionProcessingJobModelLabels() { return skippedTranscriptionProcessingJobModelLabels; },
    get deleteUnusedTranscriptionModelsError() { return deleteUnusedTranscriptionModelsError; },
    get requestingAppleSpeechPermission() { return requestingAppleSpeechPermission; },
    get appleSpeechPermissionError() { return appleSpeechPermissionError; },
    loadTranscriptionModelStatus,
    requestAppleSpeechPermission,
    openAppleSpeechPrivacySettings,
    startTranscriptionModelDownload,
    cancelTranscriptionModelDownload,
    handleTranscriptionDownloadProgress,
    requestDeleteUnusedTranscriptionModels,

    // Speakers
    get speakerModelStatus() { return speakerModelStatus; },
    get loadingSpeakerModelStatus() { return loadingSpeakerModelStatus; },
    get speakerModelError() { return speakerModelError; },
    get speakerDownloadProgress() { return speakerDownloadProgress; },
    get startingSpeakerDownload() { return startingSpeakerDownload; },
    get cancellingSpeakerDownload() { return cancellingSpeakerDownload; },
    get speakerDownloadError() { return speakerDownloadError; },
    get deletingSpeakerModel() { return deletingSpeakerModel; },
    get speakerModelDeleteMessage() { return speakerModelDeleteMessage; },
    get switchingSpeakerModel() { return switchingSpeakerModel; },
    set switchingSpeakerModel(v: boolean) { switchingSpeakerModel = v; },
    get personProfileCount() { return personProfileCount; },
    loadSpeakerModelStatus,
    loadPersonProfileCount,
    startSpeakerModelDownload,
    cancelSpeakerModelDownload,
    deleteSpeakerModel,
    handleSpeakerDownloadProgress,

    // Semantic search
    get semanticSearchModelStatus() { return semanticSearchModelStatus; },
    get loadingSemanticSearchModelStatus() { return loadingSemanticSearchModelStatus; },
    get semanticSearchModelError() { return semanticSearchModelError; },
    set semanticSearchModelError(v: string | null) { semanticSearchModelError = v; },
    get semanticSearchDownloadProgress() { return semanticSearchDownloadProgress; },
    get semanticSearchDownloadError() { return semanticSearchDownloadError; },
    get semanticSearchReindexing() { return semanticSearchReindexing; },
    set semanticSearchReindexing(v: boolean) { semanticSearchReindexing = v; },
    get semanticSearchReindexMessage() { return semanticSearchReindexMessage; },
    set semanticSearchReindexMessage(v: string | null) { semanticSearchReindexMessage = v; },
    get semanticSearchSupportedModels() { return semanticSearchSupportedModels; },
    get loadingSemanticSearchSupportedModels() { return loadingSemanticSearchSupportedModels; },
    get semanticSearchSupportedModelsError() { return semanticSearchSupportedModelsError; },
    loadSemanticSearchModelStatus,
    loadSemanticSearchSupportedModels,
    startSemanticSearchModelDownload,
    cancelSemanticSearchModelDownload,
    handleSemanticSearchDownloadProgress,
  };
}

export type ModelStatusStore = ReturnType<typeof createModelStatusStore>;
