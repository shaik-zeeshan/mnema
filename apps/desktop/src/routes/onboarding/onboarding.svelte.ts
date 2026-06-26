// Onboarding flow controller (Slice 3).
//
// Owns ALL onboarding state + logic, relocated from the legacy 2,674-line
// `routes/onboarding/+page.svelte`. The accordion shell (`+page.svelte`) is a
// thin wiring layer over this; the per-feature body components (Slice 4) read
// `controller.<field>` / call `controller.<method>()` exclusively.
//
// Behavior parity is mandatory. To keep every file under the size budget the
// pure/cohesive chunks are factored into siblings and delegated below so this
// stays one flat public surface: the model subsystems (`onboarding-models`), the
// settings round-trip (`onboarding-settings-sync`, VERBATIM from the legacy page),
// the attention/validation predicates (`onboarding-attention`), and the
// download-progress event wiring (`onboarding-listeners`).
import { invoke } from "@tauri-apps/api/core";
import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
import type {
  ActivityMode,
  AudioTranscriptionMemoryMode,
  AudioTranscriptionModelDownloadProgress,
  AudioTranscriptionProvider,
  BrowserUrlAccessibilityStatus,
  ExcludedAppEntry,
  GetPermissionsResponse,
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
import type { FeatureId, FeatureLockContext } from "./feature-model";
import { FEATURES, featureLockReason as lockReasonFor } from "./feature-model";
import {
  createOcrModelStore,
  createSemanticSearchModelStore,
  createSpeakerModelStore,
  createTranscriptionModelStore,
  OS_MANAGED_OPTION_VALUE,
} from "./onboarding-models.svelte";
import { createOnboardingAiStore } from "./onboarding-ai.svelte";
import {
  DEFAULT_SPEAKER_MODEL_ID,
  DEFAULT_SPEAKER_PROVIDER,
  defaultOcrLanguageForProvider,
  defaultOcrModelIdForProvider,
  defaultTranscriptionModelIdForProvider,
  isSelectableOcrProvider,
  parsePositiveInteger,
  serializeError,
} from "./onboarding-mapping";
import {
  buildSettingsRequestFrom,
  finaleBlockReasonFor,
  syncDraftsInto,
} from "./onboarding-settings-sync";
import { syncPrivacyDraftInto } from "./onboarding-privacy-sync";
import { startOnboardingListeners } from "./onboarding-listeners";
import {
  customBitrateErrors as buildCustomBitrateErrors,
  customResolutionErrors as buildCustomResolutionErrors,
  permissionActionFor,
  permissionLabelFor,
  permissionToneFor,
} from "./onboarding-attention";
import type { PermissionKey, PermissionValue } from "./onboarding-attention";
import {
  featureAttentionFor,
  featureDownloadFor,
  isFeatureEnabled,
} from "./onboarding-feature-state";
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

  // ── Backing settings + permissions ───────────────────────────────────────
  settings = $state<RecordingSettings | null>(null);
  permissions = $state<Record<PermissionKey, PermissionValue> | null>(null);
  requestingPerm = $state<PermissionKey | null>(null);
  refreshingPerms = $state(false);

  // ── Optional Gecko (Firefox/Zen) browser-URL access ───────────────────────
  // Surfaced via the macOS Accessibility API. Shown only when a Gecko browser is
  // installed; the status is non-fatal (a null probe simply hides the row) and
  // never gates onboarding progression.
  geckoUrlAccess = $state<BrowserUrlAccessibilityStatus | null>(null);
  requestingGeckoAccess = $state(false);
  recheckingGeckoAccess = $state(false);
  geckoInstalled = $derived((this.geckoUrlAccess?.geckoBrowsers ?? []).some((b) => b.installed));
  geckoTrusted = $derived(this.geckoUrlAccess?.trusted ?? false);
  geckoInstalledNames = $derived(
    (this.geckoUrlAccess?.geckoBrowsers ?? []).filter((b) => b.installed).map((b) => b.displayName),
  );

  // ── Lifecycle flags ──────────────────────────────────────────────────────
  loading = $state(true);
  saving = $state(false);
  completing = $state(false);
  starting = $state(false);
  errorMessage = $state<string | null>(null);

  // ── Accordion ────────────────────────────────────────────────────────────
  // `null` = every row collapsed. Nothing is open at start; opening a row sets
  // its id, and clicking the already-open row toggles back to `null`.
  openId = $state<FeatureId | null>(null);

  // ── Phase machine: welcome (first screen) → configure (accordion) → done (finale)
  phase = $state<"welcome" | "configure" | "done">("welcome");
  applyingRecommended = $state(false);

  beginSetup(): void { this.phase = "configure"; }
  backToWelcome(): void { this.phase = "welcome"; }
  reviewAndFinish(): void { this.phase = "done"; }
  backToConfigure(): void { this.phase = "configure"; }

  // One-tap recommended defaults. The capture/processing defaults are DRAFT-ONLY
  // (the redesign's invariant is "save only on finish" — do NOT call any
  // recording-settings save command here). The recommended privacy exclusions
  // are the one exception: like the legacy welcome "Use recommended setup", they
  // commit eagerly through the privacy controller (the existing pattern — privacy
  // is never deferred to the finish-only draft). Applied first; each privacy
  // command's `onSettingsUpdated` now syncs ONLY the privacy slice, so the smart
  // defaults set below are no longer at risk of being clobbered. Safe no-op when
  // nothing is pending.
  async applyRecommendedSetup(): Promise<void> {
    // Always start from a clean banner so a retry isn't shadowed by a stale
    // error — independent of whether the privacy command below actually runs.
    this.errorMessage = null;
    this.applyingRecommended = true;
    try {
      await this.appPrivacyExclusion.applyAllRecommendedPrivacyApps();
    } finally {
      this.applyingRecommended = false;
    }
    this.draftCaptureScreen = true;
    this.draftOcrEnabled = true;
    this.chooseOcrProvider("apple_vision");
    this.draftTranscriptionEnabled = true;
    // Mirror `toggleFeature("transcribe")`'s ON-branch: bind the master to the
    // currently-enabled audio sources, so a source enabled BEFORE this runs
    // (configure → enable mic → "← Back" → "Use recommended setup") is actually
    // transcribed rather than silently captured-but-not-transcribed.
    this.draftTranscriptionMicrophoneEnabled = this.draftCaptureMicrophone;
    this.draftTranscriptionSystemAudioEnabled = this.draftCaptureSystemAudio;
    this.chooseTranscriptionProvider("local_whisper");
    this.draftTranscriptionModelId = "base";
    this.phase = "configure";
    this.openId = "permissions";
  }

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
    return permissionActionFor(value);
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
    return permissionLabelFor(value);
  }

  permissionTone(value: PermissionValue | undefined): "ok" | "pending" | "blocked" {
    return permissionToneFor(value);
  }

  // Probe whether a Gecko browser (Firefox/Zen) is installed and whether Mnema is
  // trusted for the macOS Accessibility API used to read its active-tab URL.
  // Non-fatal: a failure leaves the status null so the optional row simply hides.
  async loadGeckoUrlAccess(): Promise<void> {
    try {
      this.geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch {
      this.geckoUrlAccess = null;
    }
  }

  // Raises the macOS Accessibility prompt (and adds Mnema to the list). The grant
  // is completed by the user in System Settings, so `trusted` usually stays false
  // here until they enable Mnema and we re-poll via recheck.
  async requestGeckoAccess(): Promise<void> {
    if (this.requestingGeckoAccess) return;
    this.errorMessage = null;
    this.requestingGeckoAccess = true;
    try {
      this.geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("request_browser_url_accessibility");
    } catch (err) {
      this.errorMessage = serializeError(err);
    } finally {
      this.requestingGeckoAccess = false;
    }
  }

  async openGeckoAccessSettings(): Promise<void> {
    this.errorMessage = null;
    try {
      await invoke("open_browser_url_accessibility_settings");
    } catch (err) {
      this.errorMessage = serializeError(err);
    }
  }

  async recheckGeckoAccess(): Promise<void> {
    if (this.recheckingGeckoAccess) return;
    this.errorMessage = null;
    this.recheckingGeckoAccess = true;
    try {
      this.geckoUrlAccess = await invoke<BrowserUrlAccessibilityStatus>("get_browser_url_accessibility_status");
    } catch (err) {
      this.errorMessage = serializeError(err);
    } finally {
      this.recheckingGeckoAccess = false;
    }
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
    this.draftTranscriptionProvider = value as AudioTranscriptionProvider;
    this.draftTranscriptionModelId = this.transcriptionStore.preferredTranscriptionModelIdForProvider(
      this.draftTranscriptionProvider,
      defaultTranscriptionModelIdForProvider(this.draftTranscriptionProvider),
    );
  }

  chooseTranscriptionModel(value: string): void {
    this.draftTranscriptionModelId = value === OS_MANAGED_OPTION_VALUE ? null : value;
  }

  // Mic VAD adapter is a closed union (silero/webrtc/off) surfaced as a
  // Segmented in MicBody, so the cast from the control's string value is safe.
  chooseMicrophoneVadAdapter(value: string): void {
    this.draftMicrophoneVadAdapter = value as MicrophoneVadAdapter;
  }

  // ── Accordion + per-feature enable/attention ─────────────────────────────
  // Toggle behavior: clicking a collapsed row opens it (and collapses whatever
  // was open — one-open-at-a-time); clicking the already-open row collapses it.
  setOpen(id: FeatureId): void {
    this.openId = this.openId === id ? null : id;
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

  isEnabled(id: FeatureId): boolean {
    return isFeatureEnabled(this, id);
  }

  toggleFeature(id: FeatureId): void {
    switch (id) {
      case "permissions":
      case "screen":
      case "storage":
        return; // required — no-op
      case "mic":
        // Recording the mic needs Microphone permission — gate the enable only.
        if (!this.draftCaptureMicrophone && this.featureLockReason("mic")) return;
        this.draftCaptureMicrophone = !this.draftCaptureMicrophone;
        // Keep transcription symmetric with the master toggle: if Audio
        // transcription is already on, a newly-enabled source should be
        // transcribed (else it'd be silently captured-but-not-transcribed); a
        // disabled source carries no transcript request.
        if (this.draftTranscriptionEnabled) {
          this.draftTranscriptionMicrophoneEnabled = this.draftCaptureMicrophone;
        }
        return;
      case "sysaudio":
        // Capturing system audio needs System audio permission — gate the
        // enable only. (Screen capture is required-on in this flow.)
        if (!this.draftCaptureSystemAudio && this.featureLockReason("sysaudio")) return;
        this.draftCaptureSystemAudio = !this.draftCaptureSystemAudio;
        if (this.draftTranscriptionEnabled) {
          this.draftTranscriptionSystemAudioEnabled = this.draftCaptureSystemAudio;
        }
        return;
      case "ocr":
        this.draftOcrEnabled = !this.draftOcrEnabled;
        return;
      case "transcribe":
        this.draftTranscriptionEnabled = !this.draftTranscriptionEnabled;
        if (this.draftTranscriptionEnabled) {
          // Turning the master ON: transcribe whatever audio sources are
          // currently enabled, so the feature isn't a no-op. (Sources default to
          // per-source-transcribe OFF; this binds them to the master at enable.)
          this.draftTranscriptionMicrophoneEnabled = this.draftCaptureMicrophone;
          this.draftTranscriptionSystemAudioEnabled = this.draftCaptureSystemAudio;
        } else {
          // Turning the master OFF: clear the per-source transcribe requests too,
          // else a lingering "transcribe this source" flag keeps the transcribe
          // row stuck on attention (`transcriptionRequestedWhileOff`) after the
          // user fully turned transcription off.
          this.draftTranscriptionMicrophoneEnabled = false;
          this.draftTranscriptionSystemAudioEnabled = false;
          // Speaker separation needs a transcript to split — cascade off.
          this.draftSpeakerSeparateSpeakers = false;
          this.draftSpeakerRecognizeSavedPeople = false;
        }
        return;
      case "speakers":
        // Separating speakers needs Audio transcription on — gate the enable.
        if (!this.draftSpeakerSeparateSpeakers && this.featureLockReason("speakers")) return;
        this.draftSpeakerSeparateSpeakers = !this.draftSpeakerSeparateSpeakers;
        if (!this.draftSpeakerSeparateSpeakers) this.draftSpeakerRecognizeSavedPeople = false;
        return;
      case "privacy":
        this.privacyEnabled = !this.privacyEnabled;
        return;
      case "askai":
        this.draftAskAiEnabled = !this.draftAskAiEnabled;
        return;
      case "semanticSearch":
        this.draftSemanticSearchEnabled = !this.draftSemanticSearchEnabled;
        return;
    }
  }

  // Per-feature "model not ready" predicates delegate to the pure helpers in
  // `onboarding-attention` (which read only `available` + an in-flight flag), so
  // the attention/finish gates and the body callouts share one source of truth.
  // (The OCR/transcription/speaker/semantic predicates are composed in
  // `featureAttentionFor` — see `onboarding-feature-state`; whereas
  // `transcriptionRequestedWhileOff` stays a derived here because
  // TranscriptionBody renders it directly.)
  //
  // An audio source is actively set to be transcribed (source on + its per-source
  // "transcribe" toggle on) while the master Audio transcription feature is OFF —
  // the request silently never runs, so the transcribe row needs attention.
  // Public so TranscriptionBody can explain WHY in its callout; this is the single
  // source for both the attention flag and the body copy.
  transcriptionRequestedWhileOff = $derived.by(() => {
    if (this.draftTranscriptionEnabled) return false;
    const micWants = this.draftCaptureMicrophone && this.draftTranscriptionMicrophoneEnabled;
    const sysWants = this.draftCaptureSystemAudio && this.draftTranscriptionSystemAudioEnabled;
    return micWants || sysWants;
  });

  // Single-owner attention so the footer count never double-counts an issue.
  featureAttention(id: FeatureId): boolean {
    return featureAttentionFor(this, id);
  }

  // ── Feature dependency relations ─────────────────────────────────────────
  private lockContext(): FeatureLockContext {
    return {
      micGranted: this.permissions?.microphone === "granted",
      systemAudioGranted: this.permissions?.systemAudio === "granted",
      transcriptionEnabled: this.draftTranscriptionEnabled,
    };
  }
  // Why feature `id` can't be enabled yet, or null. Drives the row lock hint +
  // the disabled toggle + the in-body inline action.
  featureLockReason(id: FeatureId): string | null {
    return lockReasonFor(id, this.lockContext());
  }
  // The toggle is disabled only when the feature is OFF and its prerequisite is
  // unmet — turning a feature OFF is always allowed.
  featureToggleDisabled(id: FeatureId): boolean {
    return !this.isEnabled(id) && this.featureLockReason(id) !== null;
  }

  // Live model-download status for a feature's COLLAPSED row, so a download
  // started on one feature stays visible after navigating to another (the
  // progress bar only renders inside the OPEN body). Reuses the existing
  // selected*DownloadRunning/Percent getters (which already exclude terminal
  // statuses, so the badge auto-clears). Returns null for features without a
  // model download and when no download is running. Percent may be null when
  // totalBytes is unknown — callers render `{percent ?? 0}%`.
  featureDownload(id: FeatureId): { running: boolean; percent: number | null } | null {
    return featureDownloadFor(this, id);
  }

  // ── Footer / CTA deriveds ────────────────────────────────────────────────
  onCount = $derived(FEATURES.filter((feature) => this.isEnabled(feature.id)).length);
  attentionCount = $derived(FEATURES.filter((feature) => this.featureAttention(feature.id)).length);

  // The configure→finale CTA ("Review & finish"): block leaving configure while
  // anything needs attention OR a selected custom resolution/bitrate is invalid
  // (those serialize as null and break the backend save). Mirrors the legacy
  // `canProceedFromActiveStep`/armed-video gate.
  canProceedToFinale = $derived(
    this.attentionCount === 0
      && this.customResolutionErrors.length === 0
      && this.customBitrateErrors.length === 0,
  );

  // The first FEATURE row currently needing attention (in FEATURES order), or
  // null. Drives the footer count chip's "jump to the blocker" affordance so the
  // disabled "Review & finish" CTA points at what to fix instead of leaving the
  // user to hunt for it. Mirrors `attentionCount`'s single-owner predicate.
  firstAttentionFeatureId = $derived(
    FEATURES.find((feature) => this.featureAttention(feature.id))?.id ?? null,
  );

  // Names the rows blocking the configure→finale step, or null when nothing
  // blocks. Mirrors the finale's `finaleBlockReason` copy idiom so the footer can
  // say WHAT is blocking instead of only a terse "N need attention" count. Stays
  // null while a custom resolution/bitrate is the (separately-surfaced) blocker.
  configureBlockReason = $derived.by(() => {
    const names = FEATURES.filter((f) => this.featureAttention(f.id)).map((f) => f.name);
    if (names.length === 0) return null;
    return `Needs attention before you can finish: ${names.join(", ")}.`;
  });

  // Open + scroll to the first attention row so the count chip is an actionable
  // jump target (not just a tally). Opening is the controller's job; the scroll
  // is a best-effort DOM nudge (the row mounts its body on open), guarded for the
  // no-attention case.
  jumpToFirstAttention(): void {
    const id = this.firstAttentionFeatureId;
    if (!id) return;
    this.openId = id;
    // Defer the scroll until the row has re-rendered open.
    requestAnimationFrame(() => {
      const head = document.querySelector<HTMLElement>(
        `[data-feature-row][data-feature-id="${id}"] [data-feature-head]`,
      );
      head?.scrollIntoView({ behavior: "smooth", block: "center" });
      head?.focus();
    });
  }

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
        && !this.selectedSpeakerDownloadRunning))
    && (!this.draftSemanticSearchEnabled
      || (!!this.selectedSemanticSearchModel
        && this.selectedSemanticSearchModel.available
        && !this.selectedSemanticSearchDownloadRunning)),
  );

  // Finishing requires model readiness, zero outstanding attention items
  // (permissions, undownloaded models, etc.), AND a valid custom
  // resolution/bitrate when those modes are selected — an invalid custom value
  // serializes as null and breaks the backend save (ScreenResolution::Custom /
  // the custom bitrate need non-null u32). This is what blocks the finale CTA.
  canComplete = $derived(
    this.canFinish
      && this.attentionCount === 0
      && this.customResolutionErrors.length === 0
      && this.customBitrateErrors.length === 0,
  );

  // ── Finale summary helpers ───────────────────────────────────────────────
  selectedSourceCount = $derived(
    Number(this.draftCaptureScreen) + Number(this.draftCaptureMicrophone) + Number(this.draftCaptureSystemAudio),
  );

  ctaLabel = $derived("Start recording");
  ctaDisabled = $derived(this.loading || this.saving || this.completing || !this.canComplete);

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
  skipDisabled = $derived(this.loading || this.saving || this.completing || !this.canSkipToDashboard);

  // Surfaced reason the finale CTAs are dead for an attention regression (not an in-flight op). Helper owns gate + copy.
  finaleBlockReason = $derived(finaleBlockReasonFor(this.phase === "done" && !this.loading && !this.saving && !this.completing,
    FEATURES.filter((f) => this.featureAttention(f.id)).map((f) => f.name)));

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
