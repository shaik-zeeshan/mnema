// Onboarding draft/model/permission state.
//
// Owns the draft settings, the four model subsystems, the permission store and
// the AI store. `OnboardingFlow` (`onboarding-flow.svelte.ts`) owns navigation,
// gates and the atomic commit, and the eight screens under `screens/` read
// `controller.<field>` / call `controller.<method>()` exclusively.
//
// The accordion, the per-feature toggle switch and the whole "attention item"
// concept were removed in issue #195 slice 17 — model readiness is classified by
// `$lib/onboarding/model-readiness` and never gates anything. To keep every file
// under the size budget the pure/cohesive chunks are factored into siblings and
// delegated below so this stays one flat public surface: the model subsystems
// (`onboarding-models`), the settings round-trip (`onboarding-settings-sync`),
// the permission-display/validation helpers (`onboarding-attention`), and the
// download-progress event wiring (`onboarding-listeners`).
import { invoke } from "@tauri-apps/api/core";
import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
import type {
  ActivityMode,
  AudioTranscriptionMemoryMode,
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionProvider,
  ExcludedAppEntry,
  MicrophoneVadAdapter,
  OcrModelDownloadProgress,
  OcrProvider,
  OcrRecognitionMode,
  OcrTesseractPageSegmentationMode,
  OcrTesseractPreprocessMode,
  RecordingSettings,
  ResolutionMode,
  ResolutionPreset,
  RetentionPolicy,
  SemanticSearchModelDownloadProgress,
  SpeakerAnalysisModelDownloadProgress,
  VideoBitrateMode,
  VideoBitratePreset,
} from "$lib/types";
import {
  createOcrModelStore,
  createSemanticSearchModelStore,
  createSpeakerModelStore,
  createTranscriptionModelStore,
  OS_MANAGED_OPTION_VALUE,
} from "./onboarding-models.svelte";
import { createOnboardingAiStore } from "./onboarding-ai.svelte";
import { createOnboardingPermissionsStore } from "./onboarding-permissions.svelte";
import {
  DEFAULT_SPEAKER_MODEL_ID,
  DEFAULT_SPEAKER_PROVIDER,
  defaultOcrLanguageForProvider,
  defaultOcrModelIdForProvider,
  defaultTranscriptionModelIdForProvider,
  isSelectableOcrProvider,
  parsePositiveInteger,
} from "./onboarding-mapping";
import {
  buildSettingsRequestFrom,
  syncDraftsInto,
} from "./onboarding-settings-sync";
import { syncPrivacyDraftInto } from "./onboarding-privacy-sync";
import { startOnboardingListeners } from "./onboarding-listeners";
import {
  customBitrateErrors as buildCustomBitrateErrors,
  customResolutionErrors as buildCustomResolutionErrors,
} from "./onboarding-attention";
import type { PermissionKey, PermissionValue } from "./onboarding-attention";
import {
  finishOnboarding,
  loadOnboarding,
} from "./onboarding-lifecycle";

// Permission types live in `onboarding-attention` (shared by the lifecycle +
// listener helpers); re-exported here so body components keep their import site.
export type { PermissionKey, PermissionValue } from "./onboarding-attention";

export class OnboardingController {
  // ── Draft fields (same names/types/defaults as the legacy page) ───────────
  draftCaptureScreen = $state(true);
  draftCaptureMicrophone = $state(false);
  draftCaptureSystemAudio = $state(false);
  draftFrameRate = $state(0.5);
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
  draftActivityMode = $state<ActivityMode>("system_input_or_screen_or_audio");
  draftMicrophoneActivitySensitivity = $state(50);
  // Voice Activity Detection adapter for the mic — mirrors real settings. "off"
  // falls back to the legacy peak-level sensitivity slider (the only mode where
  // draftMicrophoneActivitySensitivity is meaningful).
  draftMicrophoneVadAdapter = $state<MicrophoneVadAdapter>("silero");
  draftSystemAudioActivitySensitivity = $state(50);
  // Optional feature — starts OFF; the user opts in via its accordion toggle.
  draftOcrEnabled = $state(false);
  draftOcrProvider = $state<OcrProvider>("apple_vision");
  draftOcrModelId = $state<string | null>(null);
  draftOcrLanguage = $state("");
  draftOcrRecognitionMode = $state<OcrRecognitionMode>("fast");
  draftOcrLanguageCorrection = $state(false);
  draftOcrTesseractPageSegmentationMode = $state<OcrTesseractPageSegmentationMode>("single_block");
  draftOcrTesseractPreprocessMode = $state<OcrTesseractPreprocessMode>("grayscale");
  draftOcrTesseractUpscaleFactor = $state(1);
  // Optional feature — starts OFF; the user opts in via its accordion toggle.
  draftTranscriptionEnabled = $state(false);
  draftTranscriptionProvider = $state<AudioTranscriptionProvider>("local_whisper");
  draftTranscriptionModelId = $state<string | null>("base");
  draftTranscriptionLanguage = $state("auto");
  draftTranscriptionMemoryMode = $state<AudioTranscriptionMemoryMode>("balanced");
  draftTranscriptionIdleUnloadSeconds = $state(300);
  draftTranscriptionChunkSeconds = $state(30);
  // Per-source transcribe flags default OFF: enabling a capture source alone
  // (e.g. "record mic, don't transcribe") must NOT silently request a transcript
  // while the Audio-transcription master is off (which would trip the transcribe
  // attention rule). The master toggle (`toggleFeature("transcribe")`) turns
  // these on for the currently-enabled audio sources when the feature is enabled.
  draftTranscriptionMicrophoneEnabled = $state(false);
  draftTranscriptionSystemAudioEnabled = $state(false);
  draftSpeakerSeparateSpeakers = $state(false);
  draftSpeakerRecognizeSavedPeople = $state(false);
  // Default on: inert until a voiceprint exists, and enrolling is what makes it real.
  draftSpeakerAutoLabelOwner = $state(true);
  draftSpeakerProvider = $state(DEFAULT_SPEAKER_PROVIDER);
  draftSpeakerModelId = $state<string | null>(DEFAULT_SPEAKER_MODEL_ID);
  draftSpeakerTimeoutMinutes = $state(10);
  draftExcludedApps = $state<ExcludedAppEntry[]>([]);
  draftAskAiEnabled = $state(false);
  // Optional feature — starts OFF; the user opts in via its accordion toggle.
  // Semantic search self-gates on model presence (surfaced via attention), so it
  // has no hard dependency. Selection is draft-only (committed at finish); only
  // the model DOWNLOAD runs live, like OCR/transcription.
  draftSemanticSearchEnabled = $state(false);
  draftSemanticSearchModelId = $state<string | null>(null);

  // Onboarding-only UI flag — NOT backend-mapped. There is no `privacy.enabled`
  // field in RecordingSettings; excluded apps are ALWAYS persisted from
  // `draftExcludedApps`. This flag only drives the privacy row's toggle, the
  // dim-when-off of the privacy body, and the footer "features on" count.
  // Optional feature — starts OFF; the user opts in via its accordion toggle.
  privacyEnabled = $state(false);

  // ── Backing settings ──────────────────────────────────────────────────────
  settings = $state<RecordingSettings | null>(null);

  // Permissions + optional Gecko (Firefox/Zen) browser-URL access — split into
  // OnboardingPermissionsStore to keep this file under the 800-line cap; members
  // re-exposed flat below so body components stay verbatim.
  private readonly permsStore = createOnboardingPermissionsStore({
    setError: (message) => {
      this.errorMessage = message;
    },
  });

  get permissions() { return this.permsStore.permissions; }
  set permissions(value: Record<PermissionKey, PermissionValue> | null) { this.permsStore.permissions = value; }
  get requestingPerm() { return this.permsStore.requestingPerm; }
  get refreshingPerms() { return this.permsStore.refreshingPerms; }
  get sysAudioPromptRaised() { return this.permsStore.sysAudioPromptRaised; }
  get geckoUrlAccess() { return this.permsStore.geckoUrlAccess; }
  get requestingGeckoAccess() { return this.permsStore.requestingGeckoAccess; }
  get recheckingGeckoAccess() { return this.permsStore.recheckingGeckoAccess; }
  get grantedCount() { return this.permsStore.grantedCount; }
  get geckoInstalled() { return this.permsStore.geckoInstalled; }
  get geckoTrusted() { return this.permsStore.geckoTrusted; }
  get geckoInstalledNames() { return this.permsStore.geckoInstalledNames; }
  get refreshPermissions() { return this.permsStore.refreshPermissions; }
  get permissionAction() { return this.permsStore.permissionAction; }
  get requestPermission() { return this.permsStore.requestPermission; }
  get permissionLabel() { return this.permsStore.permissionLabel; }
  get permissionTone() { return this.permsStore.permissionTone; }
  get loadGeckoUrlAccess() { return this.permsStore.loadGeckoUrlAccess; }
  get requestGeckoAccess() { return this.permsStore.requestGeckoAccess; }
  get openGeckoAccessSettings() { return this.permsStore.openGeckoAccessSettings; }
  get recheckGeckoAccess() { return this.permsStore.recheckGeckoAccess; }

  // ── Lifecycle flags ──────────────────────────────────────────────────────
  loading = $state(true);
  saving = $state(false);
  completing = $state(false);
  starting = $state(false);
  errorMessage = $state<string | null>(null);


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
  private readonly semanticSearchStore = createSemanticSearchModelStore({
    semanticSearchModelId: () => this.draftSemanticSearchModelId,
  });

  // Reasoning-Engine (Ask AI) provider setup. Public so AskAiBody can render the
  // inline provider list / key fields / default-model picker. Its drafts are
  // committed as the `aiRuntime` domain in buildSettingsRequest().
  readonly ai = createOnboardingAiStore();

  // The privacy controller updates settings via `onSettingsUpdated` on every
  // add/remove/recommend command. We sync ONLY the privacy slice — a full
  // `syncDrafts` would re-derive EVERY draft (OCR/transcription/sysaudio/...)
  // from server settings and clobber unsaved in-progress toggles (onboarding
  // doesn't save until finish). `this.settings` is still updated as the base for
  // buildSettingsRequest.
  readonly appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => this.draftExcludedApps,
    onSettingsUpdated: (updated) => {
      this.settings = updated.settings;
      syncPrivacyDraftInto(this, updated.settings);
    },
    setError: (message) => {
      this.errorMessage = message;
    },
  });

  // ── Validation effects (parse raw custom inputs → clamped numbers) ────────
  // Exposed so the +page can run them as `$effect`s. The clamp ranges match the
  // Settings page's `recording-validation` (width/height 16-8192, mbps 1-40) so
  // the two surfaces agree on what a valid custom resolution/bitrate is.
  syncCustomWidth(): void {
    const parsed = parsePositiveInteger(this.customWidthRaw);
    this.draftCustomWidth = parsed !== null && parsed >= 16 && parsed <= 8192 ? parsed : null;
  }
  syncCustomHeight(): void {
    const parsed = parsePositiveInteger(this.customHeightRaw);
    this.draftCustomHeight = parsed !== null && parsed >= 16 && parsed <= 8192 ? parsed : null;
  }
  syncCustomMbps(): void {
    const parsed = parsePositiveInteger(this.draftCustomMbpsRaw);
    this.draftCustomMbps = parsed !== null && parsed >= 1 && parsed <= 40 ? parsed : null;
  }

  customResolutionErrors = $derived(
    buildCustomResolutionErrors(this.draftResolutionMode, this.draftCustomWidth, this.draftCustomHeight),
  );
  customBitrateErrors = $derived(
    buildCustomBitrateErrors(this.draftBitrateMode, this.draftCustomMbps),
  );

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

  // ── Semantic search model subsystem (flat delegation) ────────────────────
  get semanticSearchModelStatus() { return this.semanticSearchStore.semanticSearchModelStatus; }
  get loadingSemanticSearchModelStatus() { return this.semanticSearchStore.loadingSemanticSearchModelStatus; }
  get semanticSearchModelError() { return this.semanticSearchStore.semanticSearchModelError; }
  get semanticSearchSupportedModels() { return this.semanticSearchStore.semanticSearchSupportedModels; }
  get loadingSemanticSearchSupportedModels() { return this.semanticSearchStore.loadingSemanticSearchSupportedModels; }
  get semanticSearchSupportedModelsError() { return this.semanticSearchStore.semanticSearchSupportedModelsError; }
  get semanticSearchDownloadError() { return this.semanticSearchStore.semanticSearchDownloadError; }
  get startingSemanticSearchDownload() { return this.semanticSearchStore.startingSemanticSearchDownload; }
  get cancellingSemanticSearchDownload() { return this.semanticSearchStore.cancellingSemanticSearchDownload; }
  get semanticSearchModelOptions() { return this.semanticSearchStore.semanticSearchModelOptions; }
  get selectedSemanticSearchModel() { return this.semanticSearchStore.selectedSemanticSearchModel; }
  get selectedSemanticSearchDownloadProgress() { return this.semanticSearchStore.selectedSemanticSearchDownloadProgress; }
  get selectedSemanticSearchDownloadRunning() { return this.semanticSearchStore.selectedSemanticSearchDownloadRunning; }
  get selectedSemanticSearchDownloadPercent() { return this.semanticSearchStore.selectedSemanticSearchDownloadPercent; }
  loadSemanticSearchModelStatus = () => this.semanticSearchStore.loadSemanticSearchModelStatus();
  loadSemanticSearchSupportedModels = () => this.semanticSearchStore.loadSemanticSearchSupportedModels();
  startSelectedSemanticSearchModelDownload = () =>
    this.semanticSearchStore.startSelectedSemanticSearchModelDownload();
  cancelSelectedSemanticSearchModelDownload = () =>
    this.semanticSearchStore.cancelSelectedSemanticSearchModelDownload();
  handleSemanticSearchDownloadProgress = (payload: SemanticSearchModelDownloadProgress) =>
    this.semanticSearchStore.handleSemanticSearchDownloadProgress(payload);

  // Draft-only selection: picking a model just sets the draft id (persisted at
  // finish via buildSettingsRequest). Onboarding never calls the live
  // `select_semantic_search_model` command (that triggers a reindex).
  chooseSemanticSearchModel(value: string): void {
    this.draftSemanticSearchModelId = value || null;
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
    this.draftOcrModelId = value === OS_MANAGED_OPTION_VALUE ? null : value;
  }

  chooseTranscriptionProvider(value: string): void {
    // ADR 0047: cloud transcription is Settings-only, behind a consent gate — never in onboarding.
    // Defensive guard in case the provider ever leaks back into the onboarding picker.
    if (value === "deepgram") return;
    this.draftTranscriptionProvider = value as AudioTranscriptionProvider;
    this.draftTranscriptionModelId = this.transcriptionStore.preferredTranscriptionModelIdForProvider(
      this.draftTranscriptionProvider,
      defaultTranscriptionModelIdForProvider(this.draftTranscriptionProvider),
    );
  }

  chooseTranscriptionModel(value: string): void {
    this.draftTranscriptionModelId = value === OS_MANAGED_OPTION_VALUE ? null : value;
  }

  // Force every OPTIONAL feature OFF — applied ONLY for a GENUINE first run (no
  // persisted recording-settings.json; see `loadOnboarding`). Called after the
  // initial `syncDrafts` (a verbatim settings round-trip that would otherwise
  // inherit the default RecordingSettings' OCR/transcription = on) so a fresh
  // onboarding is opt-in. A RETURNING user skips this, so re-opening onboarding
  // reflects/preserves their saved enables. Required features
  // (permissions/screen/storage) have no toggle and are left alone. Cascades that
  // hang off these toggles are reset here too.
  resetOptionalFeaturesOff(): void {
    this.draftCaptureMicrophone = false;
    this.draftCaptureSystemAudio = false;
    this.draftOcrEnabled = false;
    this.draftTranscriptionEnabled = false;
    this.draftSpeakerSeparateSpeakers = false;
    this.draftSpeakerRecognizeSavedPeople = false;
    this.privacyEnabled = false;
    this.draftAskAiEnabled = false;
    this.draftSemanticSearchEnabled = false;
  }

  // The finale escape hatch ("Just open the dashboard") must NOT share the
  // model-readiness gate that "Start recording" uses — opening the dashboard
  // while a model still downloads (or with an attention item outstanding) is
  // harmless, so blocking the skip would be a dead-end. It only needs the
  // settings to serialize cleanly, so it stays gated solely on the custom
  // resolution/bitrate validity that would break the backend save (those
  // serialize as null) plus the in-flight save/complete guard.
  canSkipToDashboard = $derived(
    this.customResolutionErrors.length === 0 && this.customBitrateErrors.length === 0,
  );
  // ── Settings round-trip (VERBATIM from the legacy page) ──────────────────
  // The two transforms are factored into `onboarding-settings-sync` (operating
  // on this controller's draft fields) to keep this file under the size budget;
  // these stay as thin delegators so the public surface + behavior are identical.
  syncDrafts(next: RecordingSettings): void {
    syncDraftsInto(this, next);
  }

  buildSettingsRequest(): RecordingSettings {
    return buildSettingsRequestFrom(this);
  }

  // ── Lifecycle (load/save/finish factored into `onboarding-lifecycle`) ─────
  async load(): Promise<void> {
    await loadOnboarding(this);
  }

  async loadModelStatuses(): Promise<void> {
    await Promise.all([
      this.loadOcrModelStatus(),
      this.loadTranscriptionModelStatus(),
      this.loadSpeakerModelStatus(),
      this.loadSemanticSearchModelStatus(),
      this.loadSemanticSearchSupportedModels(),
    ]);
  }

  // Subscribes to the model-download-progress + settings-changed events and
  // returns a single combined unlisten for the +page's `$effect` cleanup. The
  // wiring lives in `onboarding-listeners` to keep this file under the size
  // budget; it guards against an async resolve landing after teardown.
  async startListeners(): Promise<() => void> {
    return startOnboardingListeners(this);
  }

  async finish(startRecording: boolean): Promise<void> {
    await finishOnboarding(this, startRecording);
  }
}
