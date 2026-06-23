// Onboarding flow controller (Slice 3).
//
// Owns ALL onboarding state + logic, relocated from the legacy 2,674-line
// `routes/onboarding/+page.svelte`. The accordion shell (`+page.svelte`) is a
// thin wiring layer over this; the per-feature body components (Slice 4) read
// `controller.<field>` / call `controller.<method>()` exclusively.
//
// Behavior parity is mandatory: `syncDrafts()` and `buildSettingsRequest()` are
// VERBATIM copies of the legacy page (only `let x` → `this.x`), so a fresh
// onboarding produces the same `RecordingSettings` the old flow would. The OCR
// and transcription model subsystems are factored into `onboarding-models`
// (delegated below, so this stays one flat public surface) to keep every file
// under the size budget.
import { goto } from "$app/navigation";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
import { theme } from "$lib/theme.svelte";
import type {
  ActivityMode,
  AudioTranscriptionMemoryMode,
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionProvider,
  ExcludedAppEntry,
  GetPermissionsResponse,
  OcrModelDownloadProgress,
  OcrProvider,
  OcrRecognitionMode,
  OcrTesseractPageSegmentationMode,
  OcrTesseractPreprocessMode,
  PermissionStatus,
  RecordingSettings,
  ResolutionMode,
  ResolutionPreset,
  RetentionPolicy,
  SpeakerAnalysisModelDownloadProgress,
  VideoBitrateMode,
  VideoBitratePreset,
} from "$lib/types";
import type { FeatureId } from "./feature-model";
import { FEATURES } from "./feature-model";
import {
  createOcrModelStore,
  createSpeakerModelStore,
  createTranscriptionModelStore,
} from "./onboarding-models.svelte";
import {
  AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
  DEFAULT_SPEAKER_MODEL_ID,
  DEFAULT_SPEAKER_PROVIDER,
  OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
  RECORDING_SETTINGS_CHANGED_EVENT,
  SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
  defaultOcrLanguageForProvider,
  defaultOcrModelIdForProvider,
  defaultTranscriptionModelIdForProvider,
  isSelectableOcrProvider,
  parsePositiveInteger,
  serializeError,
} from "./onboarding-mapping";

type OnboardingState = {
  schemaVersion: number;
  completedAtUnixMs: number | null;
};
// PermissionValue mirrors the legacy page: the backend may return statuses the
// `PermissionStatus` union doesn't model, plus the synthetic "unsupported".
export type PermissionValue = PermissionStatus | "unsupported" | "unknown";
export type PermissionKey = "screen" | "microphone" | "systemAudio";

export class OnboardingController {
  // ── Draft fields (same names/types/defaults as the legacy page) ───────────
  draftCaptureScreen = $state(true);
  draftCaptureMicrophone = $state(false);
  draftCaptureSystemAudio = $state(false);
  draftFrameRate = $state(1);
  draftSegmentDuration = $state(60);
  draftResolutionMode = $state<ResolutionMode>("original");
  draftResolutionPreset = $state<ResolutionPreset>("1080p");
  draftCustomWidth = $state<number | null>(null);
  draftCustomHeight = $state<number | null>(null);
  customWidthRaw = $state("");
  customHeightRaw = $state("");
  draftBitrateMode = $state<VideoBitrateMode>("preset");
  draftBitratePreset = $state<VideoBitratePreset>("medium");
  draftCustomMbpsRaw = $state("");
  draftCustomMbps = $state<number | null>(null);
  draftSaveDirectory = $state("");
  draftPreviewCacheTtlSeconds = $state(3600);
  draftRetentionPolicy = $state<RetentionPolicy>("never");
  draftAutoStart = $state(false);
  draftPauseCaptureOnInactivity = $state(false);
  draftIdleTimeoutSeconds = $state(30);
  draftActivityMode = $state<ActivityMode>("system_input_only");
  draftMicrophoneActivitySensitivity = $state(50);
  draftSystemAudioActivitySensitivity = $state(50);
  draftOcrEnabled = $state(true);
  draftOcrProvider = $state<OcrProvider>("apple_vision");
  draftOcrModelId = $state<string | null>(null);
  draftOcrLanguage = $state("");
  draftOcrRecognitionMode = $state<OcrRecognitionMode>("fast");
  draftOcrLanguageCorrection = $state(false);
  draftOcrTesseractPageSegmentationMode = $state<OcrTesseractPageSegmentationMode>("single_block");
  draftOcrTesseractPreprocessMode = $state<OcrTesseractPreprocessMode>("grayscale");
  draftOcrTesseractUpscaleFactor = $state(1);
  draftTranscriptionEnabled = $state(true);
  draftTranscriptionProvider = $state<AudioTranscriptionProvider>("local_whisper");
  draftTranscriptionModelId = $state<string | null>("base");
  draftTranscriptionLanguage = $state("auto");
  draftTranscriptionMemoryMode = $state<AudioTranscriptionMemoryMode>("balanced");
  draftTranscriptionIdleUnloadSeconds = $state(300);
  draftTranscriptionChunkSeconds = $state(30);
  draftTranscriptionMicrophoneEnabled = $state(true);
  draftTranscriptionSystemAudioEnabled = $state(false);
  draftSpeakerSeparateSpeakers = $state(false);
  draftSpeakerRecognizeSavedPeople = $state(false);
  draftSpeakerProvider = $state(DEFAULT_SPEAKER_PROVIDER);
  draftSpeakerModelId = $state<string | null>(DEFAULT_SPEAKER_MODEL_ID);
  draftSpeakerTimeoutMinutes = $state(10);
  draftExcludedApps = $state<ExcludedAppEntry[]>([]);
  draftAskAiEnabled = $state(false);

  // Onboarding-only UI flag — NOT backend-mapped. There is no `privacy.enabled`
  // field in RecordingSettings; excluded apps are ALWAYS persisted from
  // `draftExcludedApps`. This flag only drives the privacy row's toggle, the
  // dim-when-off of the privacy body, and the footer "features on" count.
  privacyEnabled = $state(true);

  // ── Backing settings + permissions ───────────────────────────────────────
  settings = $state<RecordingSettings | null>(null);
  permissions = $state<Record<PermissionKey, PermissionValue> | null>(null);
  requestingPerm = $state<PermissionKey | null>(null);
  refreshingPerms = $state(false);

  // ── Lifecycle flags ──────────────────────────────────────────────────────
  loading = $state(true);
  saving = $state(false);
  completing = $state(false);
  starting = $state(false);
  errorMessage = $state<string | null>(null);

  // ── Accordion ────────────────────────────────────────────────────────────
  openId = $state<FeatureId>("screen");

  // ── Subsystems (delegated; surfaced flat below) ──────────────────────────
  private readonly ocrStore = createOcrModelStore({
    ocrProvider: () => this.draftOcrProvider,
    ocrModelId: () => this.draftOcrModelId,
  });
  private readonly transcriptionStore = createTranscriptionModelStore({
    transcriptionProvider: () => this.draftTranscriptionProvider,
    transcriptionModelId: () => this.draftTranscriptionModelId,
  });
  private readonly speakerStore = createSpeakerModelStore({
    speakerProvider: () => this.draftSpeakerProvider,
    speakerModelId: () => this.draftSpeakerModelId,
  });

  // The privacy controller updates settings via `onSettingsUpdated`, which
  // re-syncs drafts (so `draftExcludedApps` re-derives from the server). Mirrors
  // the legacy page's wiring.
  readonly appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => this.draftExcludedApps,
    onSettingsUpdated: (updated) => {
      this.settings = updated.settings;
      this.syncDrafts(updated.settings);
    },
    setError: (message) => {
      this.errorMessage = message;
    },
  });

  // ── Validation effects (parse raw custom inputs → clamped numbers) ────────
  // Exposed so the +page can run them as `$effect`s with the SAME clamp ranges
  // as the legacy page (width 320-7680, height 240-4320, mbps 1-40).
  syncCustomWidth(): void {
    const parsed = parsePositiveInteger(this.customWidthRaw);
    this.draftCustomWidth = parsed !== null && parsed >= 320 && parsed <= 7680 ? parsed : null;
  }
  syncCustomHeight(): void {
    const parsed = parsePositiveInteger(this.customHeightRaw);
    this.draftCustomHeight = parsed !== null && parsed >= 240 && parsed <= 4320 ? parsed : null;
  }
  syncCustomMbps(): void {
    const parsed = parsePositiveInteger(this.draftCustomMbpsRaw);
    this.draftCustomMbps = parsed !== null && parsed >= 1 && parsed <= 40 ? parsed : null;
  }

  customResolutionErrors = $derived(this.validateCustomResolution());
  customBitrateErrors = $derived(this.validateCustomBitrate());

  private validateCustomResolution(): string[] {
    if (this.draftResolutionMode !== "custom") return [];
    const errors: string[] = [];
    if (this.draftCustomWidth === null) errors.push("Width must be between 320 and 7680 pixels.");
    if (this.draftCustomHeight === null) errors.push("Height must be between 240 and 4320 pixels.");
    return errors;
  }

  private validateCustomBitrate(): string[] {
    if (this.draftBitrateMode !== "custom") return [];
    return this.draftCustomMbps === null
      ? ["Bitrate must be a whole number from 1 to 40 Mbps."]
      : [];
  }

  // ── Permissions ──────────────────────────────────────────────────────────
  grantedCount = $derived(
    this.permissions === null
      ? 0
      : (["screen", "microphone", "systemAudio"] as const).filter(
          (k) => this.permissions?.[k] === "granted",
        ).length,
  );

  async refreshPermissions(): Promise<void> {
    this.errorMessage = null;
    this.refreshingPerms = true;
    try {
      const response = await invoke<GetPermissionsResponse>("get_capture_permissions");
      this.permissions = response.permissions as Record<PermissionKey, PermissionValue>;
    } catch (err) {
      this.errorMessage = serializeError(err);
    } finally {
      this.refreshingPerms = false;
    }
  }

  // Granted/unsupported need no action. macOS won't re-prompt once denied, so
  // those rows deep-link to System Settings instead of re-requesting.
  permissionAction(
    value: PermissionValue | undefined,
  ): { label: string; mode: "request" | "settings" } | null {
    if (value === "granted" || value === "unsupported") return null;
    if (value === "denied" || value === "restricted") return { label: "Open Settings", mode: "settings" };
    return { label: "Grant access", mode: "request" };
  }

  async requestPermission(key: PermissionKey): Promise<void> {
    if (this.requestingPerm) return;
    this.errorMessage = null;
    this.requestingPerm = key;
    try {
      const action = this.permissionAction(this.permissions?.[key]);
      if (action?.mode === "settings") {
        await invoke("open_capture_privacy_settings", { kind: key });
      } else {
        const response = await invoke<GetPermissionsResponse>("request_capture_permission", { kind: key });
        this.permissions = response.permissions as Record<PermissionKey, PermissionValue>;
      }
    } catch (err) {
      this.errorMessage = serializeError(err);
    } finally {
      this.requestingPerm = null;
    }
  }

  permissionLabel(value: PermissionValue | undefined): string {
    switch (value) {
      case "granted": return "Granted";
      case "denied": return "Denied";
      case "not_determined": return "Not requested";
      case "restricted": return "Restricted";
      case "unsupported": return "Unsupported";
      default: return "Unknown";
    }
  }

  permissionTone(value: PermissionValue | undefined): "ok" | "pending" | "blocked" {
    if (value === "granted") return "ok";
    if (value === "not_determined") return "pending";
    return "blocked";
  }

  // ── OCR model subsystem (flat delegation) ────────────────────────────────
  get ocrModelStatus() { return this.ocrStore.ocrModelStatus; }
  get loadingOcrModelStatus() { return this.ocrStore.loadingOcrModelStatus; }
  get ocrModelError() { return this.ocrStore.ocrModelError; }
  get ocrDownloadProgress() { return this.ocrStore.ocrDownloadProgress; }
  get startingOcrDownload() { return this.ocrStore.startingOcrDownload; }
  get cancellingOcrDownload() { return this.ocrStore.cancellingOcrDownload; }
  get ocrDownloadError() { return this.ocrStore.ocrDownloadError; }
  get selectedOcrProviderStatus() { return this.ocrStore.selectedOcrProviderStatus; }
  get selectedOcrModels() { return this.ocrStore.selectedOcrModels; }
  get selectedOcrModel() { return this.ocrStore.selectedOcrModel; }
  get selectedOcrDownloadProgress() { return this.ocrStore.selectedOcrDownloadProgress; }
  get selectedOcrDownloadRunning() { return this.ocrStore.selectedOcrDownloadRunning; }
  get selectedOcrDownloadPercent() { return this.ocrStore.selectedOcrDownloadPercent; }
  get ocrModelOptions() { return this.ocrStore.ocrModelOptions; }
  ocrStatusLabel = this.ocrStore.ocrStatusLabel;
  loadOcrModelStatus = () => this.ocrStore.loadOcrModelStatus();
  startSelectedOcrModelDownload = () => this.ocrStore.startSelectedOcrModelDownload();
  cancelSelectedOcrModelDownload = () => this.ocrStore.cancelSelectedOcrModelDownload();
  handleOcrDownloadProgress = (payload: OcrModelDownloadProgress) =>
    this.ocrStore.handleOcrDownloadProgress(payload);

  // ── Transcription model subsystem (flat delegation) ──────────────────────
  get transcriptionModelStatus() { return this.transcriptionStore.transcriptionModelStatus; }
  get loadingTranscriptionModelStatus() { return this.transcriptionStore.loadingTranscriptionModelStatus; }
  get transcriptionModelError() { return this.transcriptionStore.transcriptionModelError; }
  get transcriptionDownloadProgress() { return this.transcriptionStore.transcriptionDownloadProgress; }
  get startingTranscriptionDownload() { return this.transcriptionStore.startingTranscriptionDownload; }
  get cancellingTranscriptionDownload() { return this.transcriptionStore.cancellingTranscriptionDownload; }
  get transcriptionDownloadError() { return this.transcriptionStore.transcriptionDownloadError; }
  get selectedTranscriptionProviderStatus() { return this.transcriptionStore.selectedTranscriptionProviderStatus; }
  get selectedTranscriptionModels() { return this.transcriptionStore.selectedTranscriptionModels; }
  get selectedTranscriptionModel() { return this.transcriptionStore.selectedTranscriptionModel; }
  get selectedTranscriptionDownloadProgress() { return this.transcriptionStore.selectedTranscriptionDownloadProgress; }
  get selectedTranscriptionDownloadRunning() { return this.transcriptionStore.selectedTranscriptionDownloadRunning; }
  get selectedTranscriptionDownloadPercent() { return this.transcriptionStore.selectedTranscriptionDownloadPercent; }
  get transcriptionModelOptions() { return this.transcriptionStore.transcriptionModelOptions; }
  transcriptionStatusLabel = this.transcriptionStore.transcriptionStatusLabel;
  loadTranscriptionModelStatus = () => this.transcriptionStore.loadTranscriptionModelStatus();
  startSelectedTranscriptionModelDownload = () =>
    this.transcriptionStore.startSelectedTranscriptionModelDownload();
  cancelSelectedTranscriptionModelDownload = () =>
    this.transcriptionStore.cancelSelectedTranscriptionModelDownload();
  handleTranscriptionDownloadProgress = (payload: AudioTranscriptionModelDownloadProgress) =>
    this.transcriptionStore.handleTranscriptionDownloadProgress(payload);

  // ── Speaker analysis model subsystem (flat delegation) ───────────────────
  get speakerModelStatus() { return this.speakerStore.speakerModelStatus; }
  get loadingSpeakerModelStatus() { return this.speakerStore.loadingSpeakerModelStatus; }
  get speakerModelError() { return this.speakerStore.speakerModelError; }
  get speakerDownloadProgress() { return this.speakerStore.speakerDownloadProgress; }
  get startingSpeakerDownload() { return this.speakerStore.startingSpeakerDownload; }
  get cancellingSpeakerDownload() { return this.speakerStore.cancellingSpeakerDownload; }
  get speakerDownloadError() { return this.speakerStore.speakerDownloadError; }
  get selectedSpeakerModel() { return this.speakerStore.selectedSpeakerModel; }
  get speakerModelOptions() { return this.speakerStore.speakerModelOptions; }
  get selectedSpeakerPresetKey() { return this.speakerStore.selectedSpeakerPresetKey; }
  get selectedSpeakerDownloadProgress() { return this.speakerStore.selectedSpeakerDownloadProgress; }
  get selectedSpeakerDownloadRunning() { return this.speakerStore.selectedSpeakerDownloadRunning; }
  get selectedSpeakerDownloadPercent() { return this.speakerStore.selectedSpeakerDownloadPercent; }
  speakerStatusLabel = this.speakerStore.speakerStatusLabel;
  loadSpeakerModelStatus = () => this.speakerStore.loadSpeakerModelStatus();
  startSelectedSpeakerModelDownload = () => this.speakerStore.startSelectedSpeakerModelDownload();
  cancelSelectedSpeakerModelDownload = () => this.speakerStore.cancelSelectedSpeakerModelDownload();
  handleSpeakerDownloadProgress = (payload: SpeakerAnalysisModelDownloadProgress) =>
    this.speakerStore.handleSpeakerDownloadProgress(payload);

  chooseSpeakerModel(value: string): void {
    const { provider, modelId } = this.speakerStore.parseSpeakerPresetKey(value);
    this.draftSpeakerProvider = provider;
    this.draftSpeakerModelId = modelId;
  }

  // ── Provider / model selection helpers (used by Slice 4 bodies) ──────────
  chooseOcrProvider(value: string): void {
    if (!isSelectableOcrProvider(value)) return;
    this.draftOcrProvider = value;
    this.draftOcrModelId = this.ocrStore.preferredOcrModelIdForProvider(
      this.draftOcrProvider,
      defaultOcrModelIdForProvider(this.draftOcrProvider),
    );
    this.draftOcrLanguage = defaultOcrLanguageForProvider(this.draftOcrProvider) ?? "";
  }

  chooseOcrModel(value: string): void {
    this.draftOcrModelId = value === "__os_managed__" ? null : value;
  }

  chooseTranscriptionProvider(value: string): void {
    this.draftTranscriptionProvider = value as AudioTranscriptionProvider;
    this.draftTranscriptionModelId = this.transcriptionStore.preferredTranscriptionModelIdForProvider(
      this.draftTranscriptionProvider,
      defaultTranscriptionModelIdForProvider(this.draftTranscriptionProvider),
    );
  }

  chooseTranscriptionModel(value: string): void {
    this.draftTranscriptionModelId = value === "__os_managed__" ? null : value;
  }

  // ── Accordion + per-feature enable/attention ─────────────────────────────
  setOpen(id: FeatureId): void {
    this.openId = id;
  }

  isEnabled(id: FeatureId): boolean {
    switch (id) {
      case "permissions":
      case "screen":
      case "storage":
        return true; // required — always on
      case "mic":
        return this.draftCaptureMicrophone;
      case "sysaudio":
        return this.draftCaptureSystemAudio;
      case "ocr":
        return this.draftOcrEnabled;
      case "transcribe":
        return this.draftTranscriptionEnabled;
      case "speakers":
        return this.draftSpeakerSeparateSpeakers;
      case "privacy":
        return this.privacyEnabled;
      case "askai":
        return this.draftAskAiEnabled;
    }
  }

  toggleFeature(id: FeatureId): void {
    switch (id) {
      case "permissions":
      case "screen":
      case "storage":
        return; // required — no-op
      case "mic":
        this.draftCaptureMicrophone = !this.draftCaptureMicrophone;
        return;
      case "sysaudio":
        // System audio requires screen capture. Screen is required-on in this
        // flow, so the legacy coupling (screen off → sys audio off) is inert;
        // we still gate enabling on screen for parity.
        if (!this.draftCaptureSystemAudio && !this.draftCaptureScreen) return;
        this.draftCaptureSystemAudio = !this.draftCaptureSystemAudio;
        return;
      case "ocr":
        this.draftOcrEnabled = !this.draftOcrEnabled;
        return;
      case "transcribe":
        this.draftTranscriptionEnabled = !this.draftTranscriptionEnabled;
        return;
      case "speakers":
        this.draftSpeakerSeparateSpeakers = !this.draftSpeakerSeparateSpeakers;
        if (!this.draftSpeakerSeparateSpeakers) this.draftSpeakerRecognizeSavedPeople = false;
        return;
      case "privacy":
        this.privacyEnabled = !this.privacyEnabled;
        return;
      case "askai":
        this.draftAskAiEnabled = !this.draftAskAiEnabled;
        return;
    }
  }

  // A model is "not available" for attention/finish purposes when its feature
  // is on but the selected model isn't ready: app-managed and not currently a
  // completed download. (Completed downloads flip `available` true on reload.)
  private ocrModelNeedsAttention(): boolean {
    if (!this.draftOcrEnabled) return false;
    const model = this.selectedOcrModel;
    if (!model) return true;
    if (model.available) return false;
    if (this.selectedOcrDownloadRunning) return true;
    return true;
  }

  private transcriptionModelNeedsAttention(): boolean {
    if (!this.draftTranscriptionEnabled) return false;
    const model = this.selectedTranscriptionModel;
    if (!model) return true;
    return !model.available;
  }

  private speakerModelNeedsAttention(): boolean {
    if (!this.draftSpeakerSeparateSpeakers) return false;
    const model = this.selectedSpeakerModel;
    if (!model) return true;
    return !model.available;
  }

  // Single-owner attention so the footer count never double-counts an issue.
  featureAttention(id: FeatureId): boolean {
    switch (id) {
      case "permissions":
        return this.permissions?.screen !== "granted";
      case "mic":
        return this.draftCaptureMicrophone && this.permissions?.microphone !== "granted";
      case "sysaudio":
        return this.draftCaptureSystemAudio && this.permissions?.systemAudio !== "granted";
      case "ocr":
        return this.ocrModelNeedsAttention();
      case "transcribe":
        return this.transcriptionModelNeedsAttention();
      case "speakers":
        return this.speakerModelNeedsAttention();
      case "screen":
      case "storage":
      case "privacy":
      case "askai":
        return false;
    }
  }

  // ── Footer / CTA deriveds ────────────────────────────────────────────────
  onCount = $derived(FEATURES.filter((feature) => this.isEnabled(feature.id)).length);
  attentionCount = $derived(FEATURES.filter((feature) => this.featureAttention(feature.id)).length);

  // The legacy completion gate (`processingReady`): finishing is blocked only
  // when a selected, enabled model isn't ready. Permissions never block finish.
  canFinish = $derived(
    (!this.draftOcrEnabled
      || (!!this.selectedOcrModel && this.selectedOcrModel.available && !this.selectedOcrDownloadRunning))
    && (!this.draftTranscriptionEnabled
      || (!!this.selectedTranscriptionModel
        && this.selectedTranscriptionModel.available
        && !this.selectedTranscriptionDownloadRunning))
    && (!this.draftSpeakerSeparateSpeakers
      || (!!this.selectedSpeakerModel
        && this.selectedSpeakerModel.available
        && !this.selectedSpeakerDownloadRunning)),
  );

  ctaLabel = $derived("Start recording");
  ctaDisabled = $derived(this.loading || this.saving || this.completing || !this.canFinish);

  // ── Settings round-trip (VERBATIM from the legacy page) ──────────────────
  syncDrafts(next: RecordingSettings): void {
    this.draftCaptureScreen = next.captureScreen;
    this.draftCaptureMicrophone = next.captureMicrophone;
    this.draftCaptureSystemAudio = next.captureSystemAudio;
    this.draftFrameRate = next.screenFrameRate;
    this.draftSegmentDuration = next.segmentDurationSeconds;
    if (next.screenResolution.mode === "custom") {
      this.draftResolutionMode = "custom";
      this.draftCustomWidth = next.screenResolution.width;
      this.draftCustomHeight = next.screenResolution.height;
      this.customWidthRaw = String(next.screenResolution.width);
      this.customHeightRaw = String(next.screenResolution.height);
    } else if (next.screenResolution.preset === "original") {
      this.draftResolutionMode = "original";
      this.draftResolutionPreset = "1080p";
      this.draftCustomWidth = null;
      this.draftCustomHeight = null;
      this.customWidthRaw = "";
      this.customHeightRaw = "";
    } else {
      this.draftResolutionMode = "preset";
      this.draftResolutionPreset = next.screenResolution.preset;
      this.draftCustomWidth = null;
      this.draftCustomHeight = null;
      this.customWidthRaw = "";
      this.customHeightRaw = "";
    }
    if (next.videoBitrate.mode === "custom") {
      this.draftBitrateMode = "custom";
      this.draftBitratePreset = "medium";
      this.draftCustomMbps = next.videoBitrate.customMbps;
      this.draftCustomMbpsRaw = String(next.videoBitrate.customMbps);
    } else {
      this.draftBitrateMode = "preset";
      this.draftBitratePreset = next.videoBitrate.preset;
      this.draftCustomMbps = null;
      this.draftCustomMbpsRaw = "";
    }
    this.draftSaveDirectory = next.saveDirectory;
    this.draftPreviewCacheTtlSeconds = next.previewCacheTtlSeconds ?? 3600;
    this.draftRetentionPolicy = next.retentionPolicy ?? "never";
    this.draftAutoStart = next.autoStart;
    this.draftPauseCaptureOnInactivity = next.pauseCaptureOnInactivity;
    this.draftIdleTimeoutSeconds = next.idleTimeoutSeconds;
    this.draftActivityMode = "system_input_or_screen_or_audio";
    this.draftMicrophoneActivitySensitivity = next.microphoneActivitySensitivity ?? 50;
    this.draftSystemAudioActivitySensitivity = next.systemAudioActivitySensitivity ?? 50;
    this.draftOcrEnabled = next.ocr?.enabled ?? true;
    const loadedOcrProvider = next.ocr?.provider;
    const loadedOcrProviderSelectable = isSelectableOcrProvider(loadedOcrProvider);
    this.draftOcrProvider = loadedOcrProviderSelectable ? loadedOcrProvider : "apple_vision";
    this.draftOcrModelId = loadedOcrProviderSelectable
      ? (next.ocr?.modelId ?? defaultOcrModelIdForProvider(this.draftOcrProvider))
      : defaultOcrModelIdForProvider(this.draftOcrProvider);
    this.draftOcrLanguage = loadedOcrProviderSelectable
      ? (next.ocr?.language ?? defaultOcrLanguageForProvider(this.draftOcrProvider) ?? "")
      : defaultOcrLanguageForProvider(this.draftOcrProvider) ?? "";
    this.draftOcrRecognitionMode = next.ocr?.recognitionMode ?? "fast";
    this.draftOcrLanguageCorrection = next.ocr?.languageCorrection ?? false;
    this.draftOcrTesseractPageSegmentationMode = next.ocr?.tesseractPageSegmentationMode ?? "single_block";
    this.draftOcrTesseractPreprocessMode = next.ocr?.tesseractPreprocessMode ?? "grayscale";
    this.draftOcrTesseractUpscaleFactor = next.ocr?.tesseractUpscaleFactor ?? 1;
    this.draftTranscriptionEnabled = next.transcription?.enabled ?? true;
    this.draftTranscriptionMicrophoneEnabled = next.transcription?.microphoneEnabled ?? true;
    this.draftTranscriptionSystemAudioEnabled = next.transcription?.systemAudioEnabled ?? false;
    this.draftTranscriptionProvider = next.transcription?.provider ?? "local_whisper";
    this.draftTranscriptionModelId = next.transcription?.modelId ?? defaultTranscriptionModelIdForProvider(this.draftTranscriptionProvider);
    this.draftTranscriptionLanguage = next.transcription?.language ?? "auto";
    this.draftTranscriptionMemoryMode = next.transcription?.memoryMode ?? "balanced";
    this.draftTranscriptionIdleUnloadSeconds = next.transcription?.idleUnloadSeconds ?? 300;
    this.draftTranscriptionChunkSeconds = next.transcription?.chunkSeconds ?? 30;
    this.draftSpeakerSeparateSpeakers = next.speakerAnalysis?.separateSpeakers ?? false;
    this.draftSpeakerRecognizeSavedPeople = next.speakerAnalysis?.recognizeSavedPeople ?? false;
    // Coerce legacy saved values: the sherpa_onnx provider (and its model ids)
    // no longer exist, so old settings resolve to the speakrs default — else the
    // preset picker would select a provider/model the backend manifest never
    // returns. Mirrors recording.svelte.ts.
    const savedSpeakerProvider = next.speakerAnalysis?.provider;
    const isLegacySpeakerProvider = !savedSpeakerProvider || savedSpeakerProvider === "sherpa_onnx";
    this.draftSpeakerProvider = isLegacySpeakerProvider ? DEFAULT_SPEAKER_PROVIDER : savedSpeakerProvider;
    this.draftSpeakerModelId = isLegacySpeakerProvider
      ? DEFAULT_SPEAKER_MODEL_ID
      : (next.speakerAnalysis?.modelId ?? DEFAULT_SPEAKER_MODEL_ID);
    this.draftSpeakerTimeoutMinutes = Math.round((next.speakerAnalysis?.timeoutSeconds ?? 600) / 60);
    this.draftExcludedApps = [...(next.privacy?.excludedApps ?? [])];
    this.draftAskAiEnabled = next.access?.askAiEnabled ?? false;
  }

  buildSettingsRequest(): RecordingSettings {
    const base = this.settings;
    if (base === null) throw new Error("Recording settings are not loaded.");
    return {
      ...base,
      captureScreen: this.draftCaptureScreen,
      captureMicrophone: this.draftCaptureMicrophone,
      captureSystemAudio: this.draftCaptureScreen && this.draftCaptureSystemAudio,
      screenFrameRate: this.draftFrameRate,
      screenResolution: this.draftResolutionMode === "custom"
        ? { mode: "custom", width: this.draftCustomWidth!, height: this.draftCustomHeight! }
        : { mode: "preset", preset: this.draftResolutionMode === "original" ? "original" : this.draftResolutionPreset },
      videoBitrate: this.draftBitrateMode === "custom"
        ? { mode: "custom", preset: null, customMbps: this.draftCustomMbps! }
        : { mode: "preset", preset: this.draftBitratePreset, customMbps: null },
      segmentDurationSeconds: this.draftSegmentDuration,
      saveDirectory: this.draftSaveDirectory.trim(),
      previewCacheTtlSeconds: this.draftPreviewCacheTtlSeconds,
      retentionPolicy: this.draftRetentionPolicy,
      appearance: theme.loaded ? theme.appearance : base.appearance,
      autoStart: this.draftAutoStart,
      pauseCaptureOnInactivity: this.draftPauseCaptureOnInactivity,
      idleTimeoutSeconds: this.draftIdleTimeoutSeconds,
      activityMode: "system_input_or_screen_or_audio",
      microphoneActivitySensitivity: this.draftMicrophoneActivitySensitivity,
      systemAudioActivitySensitivity: this.draftSystemAudioActivitySensitivity,
      ocr: {
        enabled: this.draftOcrEnabled,
        provider: this.draftOcrProvider,
        modelId: this.draftOcrModelId,
        language: this.draftOcrLanguage.trim() || null,
        recognitionMode: this.draftOcrRecognitionMode,
        languageCorrection: this.draftOcrLanguageCorrection,
        tesseractPageSegmentationMode: this.draftOcrTesseractPageSegmentationMode,
        tesseractPreprocessMode: this.draftOcrTesseractPreprocessMode,
        tesseractUpscaleFactor: Math.max(1, Math.min(4, Math.trunc(Number(this.draftOcrTesseractUpscaleFactor) || 1))),
        tesseractCharWhitelist: null,
      },
      transcription: {
        enabled: this.draftTranscriptionEnabled,
        microphoneEnabled: this.draftTranscriptionMicrophoneEnabled,
        systemAudioEnabled: this.draftTranscriptionSystemAudioEnabled,
        provider: this.draftTranscriptionProvider,
        modelId: this.draftTranscriptionModelId,
        language: this.draftTranscriptionLanguage.trim() || "auto",
        memoryMode: this.draftTranscriptionMemoryMode,
        idleUnloadSeconds: Math.max(0, Math.trunc(Number(this.draftTranscriptionIdleUnloadSeconds) || 0)),
        chunkSeconds: Math.max(0, Math.trunc(Number(this.draftTranscriptionChunkSeconds) || 0)),
      },
      speakerAnalysis: {
        separateSpeakers: this.draftSpeakerSeparateSpeakers,
        recognizeSavedPeople: this.draftSpeakerRecognizeSavedPeople,
        provider: this.draftSpeakerProvider,
        modelId: this.draftSpeakerModelId,
        timeoutSeconds: Math.max(
          60,
          Math.min(3600, Math.trunc(Number(this.draftSpeakerTimeoutMinutes) || 10) * 60),
        ),
      },
      access: {
        askAiEnabled: this.draftAskAiEnabled,
        askAiMaxToolCalls: base.access?.askAiMaxToolCalls ?? 12,
        // `access` is sent whole and is authoritative, so we must round-trip the
        // Ask AI model selection (chosen on the Settings page); omitting it would
        // reset the selection back to the PI runtime default on every full save.
        askAiModel: base.access?.askAiModel ?? null,
      },
    };
  }

  private async saveSettings(): Promise<void> {
    this.saving = true;
    this.errorMessage = null;
    try {
      // Onboarding commits the whole recording config in one shot. The
      // domain-scoped commands exist for the Settings page's per-domain
      // debounced autosave; here we deliberately use the atomic full-settings
      // command so a late validation failure can't leave a partially-persisted
      // configuration behind.
      const updated = await invoke<RecordingSettings>("update_recording_settings", {
        request: this.buildSettingsRequest(),
      });
      this.settings = updated;
      this.syncDrafts(updated);
    } catch (err) {
      this.errorMessage = serializeError(err);
      throw err;
    } finally {
      this.saving = false;
    }
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────
  async load(): Promise<void> {
    this.loading = true;
    this.errorMessage = null;
    try {
      const state = await invoke<OnboardingState>("get_onboarding_state");
      if (state.completedAtUnixMs !== null) {
        await goto("/", { replaceState: true });
        return;
      }
      const [loadedSettings, permissionResponse] = await Promise.all([
        invoke<RecordingSettings>("get_recording_settings"),
        invoke<GetPermissionsResponse>("get_capture_permissions"),
      ]);
      this.settings = loadedSettings;
      this.permissions = permissionResponse.permissions as Record<PermissionKey, PermissionValue>;
      this.syncDrafts(loadedSettings);
      void this.appPrivacyExclusion.loadPrivacyAppCandidates();
      void this.appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    } catch (err) {
      this.errorMessage = serializeError(err);
    } finally {
      this.loading = false;
    }
  }

  async loadModelStatuses(): Promise<void> {
    await Promise.all([
      this.loadOcrModelStatus(),
      this.loadTranscriptionModelStatus(),
      this.loadSpeakerModelStatus(),
    ]);
  }

  // Subscribes to the three onboarding events and returns a single combined
  // unlisten for the +page's `$effect` cleanup. Guards against an async resolve
  // landing after the effect/component is torn down.
  async startListeners(): Promise<() => void> {
    let unlistenOcrDownloadProgress: (() => void) | undefined;
    let unlistenTranscriptionDownloadProgress: (() => void) | undefined;
    let unlistenSpeakerDownloadProgress: (() => void) | undefined;
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
    let destroyed = false;

    const unlisten = () => {
      destroyed = true;
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenSpeakerDownloadProgress?.();
      unlistenRecordingSettingsChanged?.();
    };

    await Promise.all([
      listen<OcrModelDownloadProgress>(OCR_MODEL_DOWNLOAD_PROGRESS_EVENT, (event) => {
        void this.handleOcrDownloadProgress(event.payload);
      }).then((fn) => {
        if (destroyed) fn();
        else unlistenOcrDownloadProgress = fn;
      }),
      listen<AudioTranscriptionModelDownloadProgress>(
        AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
        (event) => { void this.handleTranscriptionDownloadProgress(event.payload); },
      ).then((fn) => {
        if (destroyed) fn();
        else unlistenTranscriptionDownloadProgress = fn;
      }),
      listen<SpeakerAnalysisModelDownloadProgress>(
        SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
        (event) => { void this.handleSpeakerDownloadProgress(event.payload); },
      ).then((fn) => {
        if (destroyed) fn();
        else unlistenSpeakerDownloadProgress = fn;
      }),
      listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
        this.settings = event.payload;
      }).then((fn) => {
        if (destroyed) fn();
        else unlistenRecordingSettingsChanged = fn;
      }),
    ]);

    return unlisten;
  }

  async finish(startRecording: boolean): Promise<void> {
    if (this.settings === null || !this.canFinish) return;
    this.completing = true;
    this.starting = startRecording;
    this.errorMessage = null;
    try {
      await this.saveSettings();
      if (startRecording) {
        await invoke("start_native_capture", {
          request: {
            captureScreen: this.draftCaptureScreen,
            captureMicrophone: this.draftCaptureMicrophone,
            captureSystemAudio: this.draftCaptureScreen && this.draftCaptureSystemAudio,
          },
        });
      }
      await invoke("complete_onboarding");
      await goto("/");
    } catch (err) {
      this.errorMessage = serializeError(err);
      this.completing = false;
      this.starting = false;
    }
  }
}
