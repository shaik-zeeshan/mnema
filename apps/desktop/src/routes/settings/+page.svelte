<script lang="ts">
  import { page } from "$app/stores";
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { ask } from "@tauri-apps/plugin-dialog";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import AppPrivacyExclusionPrompt from "$lib/components/AppPrivacyExclusionPrompt.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
  import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
  import { setDeveloperOptionsEnabled } from "$lib/developer-options.svelte";
  import { setAppearance } from "$lib/theme.svelte";
  import type {
    ActivityMode,
    AppearanceSetting,
    CaptureSupport,
    GeneralAppLogStatus,
    NativeCaptureDebugLogStatus,
    OcrModelDownloadProgress,
    OcrModelStatus,
    OcrModelStatusResponse,
    OcrProvider,
    OcrRecognitionMode,
    OcrTesseractPageSegmentationMode,
    OcrTesseractPreprocessMode,
    RecordingSettings,
    AudioTranscriptionModelDownloadProgress,
    AudioTranscriptionModelStatus,
    AudioTranscriptionModelStatusResponse,
    DeleteUnusedAudioTranscriptionModelsResponse,
    DeleteUnusedOcrModelsResponse,
    AudioTranscriptionProvider,
    AudioTranscriptionMemoryMode,
    AppleSpeechOnDeviceAvailabilityStatus,
    MicrophoneVadAdapter,
    ResolutionMode,
    ResolutionPreset,
    VideoBitrateMode,
    VideoBitratePreset,
    MicrophoneControllerState,
    MicrophonePreferenceMode,
    MicrophoneDisconnectPolicy,
    MicrophoneAutoDisconnectTransitionFailedEvent,
    RetentionPolicy,
    BrowserUrlMode,
    ExcludedAppEntry,
    SpeakerAnalysisModelDownloadProgress,
    SpeakerAnalysisModelStatus,
    SpeakerAnalysisModelStatusResponse,
    KeyboardBindingsSettings,
  } from "$lib/types";

  type CardIconKind =
    | "capture"
    | "video"
    | "storage"
    | "appearance"
    | "inactivity"
    | "processing"
    | "transcription"
    | "speaker"
    | "developer"
    | "privacy"
    | "audio";

  const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
  const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT = "audio_transcription_model_download_progress";
  const SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT = "speaker_analysis_model_download_progress";
  const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT = "ocr_model_download_progress";
  const SELECTABLE_OCR_PROVIDERS: readonly OcrProvider[] = ["apple_vision", "tesseract"];

  type BrokerGrant = {
    id: string;
    label: string;
    createdAtUnixMs: number;
    expiresAtUnixMs: number;
    revoked: boolean;
    scope: { recent_days: { days: number } } | "all_retained_history" | Record<string, unknown>;
  };

  type BrokerGrantFile = {
    grants: BrokerGrant[];
  };

  type MnemaCliStatus = {
    installPath: string;
    installDir: string;
    bundledCliPath: string;
    bundledCliExists: boolean;
    installed: boolean;
    installDirInPath: boolean;
    existingTarget: string | null;
  };

  type RetentionCleanupSummary = {
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

  // ─── State ────────────────────────────────────────────────────────────────

  let captureSupport = $state<CaptureSupport | null>(null);
  let recordingSettings = $state<RecordingSettings | null>(null);
  let keyboardBindingsSettings = $state<KeyboardBindingsSettings | null>(null);
  let micState = $state<MicrophoneControllerState | null>(null);

  // Recording settings drafts
  let draftCaptureScreen = $state(true);
  let draftCaptureMicrophone = $state(false);
  let draftCaptureSystemAudio = $state(false);
  let draftSegmentDuration = $state(60);
  let draftFrameRate = $state(1);
  let draftSaveDirectory = $state("");
  let draftAutoStart = $state(false);
  let draftGlobalShortcutsEnabled = $state(true);

  // Resolution drafts
  let draftResolutionMode = $state<ResolutionMode>("original");
  let draftResolutionPreset = $state<ResolutionPreset>("1080p");
  let draftCustomWidth = $state<number | null>(null);
  let draftCustomHeight = $state<number | null>(null);
  let customWidthRaw = $state("");
  let customHeightRaw = $state("");

  // Video bitrate drafts
  let draftBitrateMode = $state<VideoBitrateMode>("preset");
  let draftBitratePreset = $state<VideoBitratePreset>("medium");
  let draftCustomMbpsRaw = $state("");
  let draftCustomMbps = $state<number | null>(null);

  // Microphone drafts
  let draftPreferenceMode = $state<MicrophonePreferenceMode>("default");
  let draftDeviceId = $state<string | null>(null);
  let draftDisconnectPolicy = $state<MicrophoneDisconnectPolicy>("fallback_to_default");

  // Inactivity drafts
  let draftPauseCaptureOnInactivity = $state(false);
  let draftIdleTimeoutSeconds = $state(30);
  let draftActivityMode = $state<ActivityMode>("system_input_only");
  let draftMicrophoneActivitySensitivity = $state(50);
  let draftSystemAudioActivitySensitivity = $state(50);
  let draftMicrophoneVadAdapter = $state<MicrophoneVadAdapter>("silero");

  // Debug logging draft
  let draftNativeCaptureDebugLoggingEnabled = $state(false);

  // Developer-options draft (gates the Debug page and its nav entry).
  let draftDeveloperOptionsEnabled = $state(false);

  // Preview cache TTL draft (seconds; 0 disables)
  let draftPreviewCacheTtlSeconds = $state(3600);

  // Timeline behavior draft
  let draftFollowTimelineLive = $state(false);
  let draftRetentionPolicy = $state<RetentionPolicy>("never");
  let draftMetadataEnabled = $state(true);
  let draftBrowserUrlMode = $state<BrowserUrlMode>("sanitized");
  let draftExcludedApps = $state<ExcludedAppEntry[]>([]);
  let retentionCleanupSummary = $state<RetentionCleanupSummary | null>(null);
  let retentionCleanupRunning = $state(false);
  let retentionCleanupError = $state<string | null>(null);
  let brokerGrants = $state<BrokerGrant[]>([]);
  let brokerGrantLoading = $state(false);
  let brokerGrantSaving = $state(false);
  let brokerGrantError = $state<string | null>(null);
  let mnemaCliStatus = $state<MnemaCliStatus | null>(null);
  let mnemaCliLoading = $state(false);
  let mnemaCliInstalling = $state(false);
  let mnemaCliError = $state<string | null>(null);

  // Appearance draft (system | light | dark). Drives the in-memory theme
  // runtime in `$lib/theme.svelte` and is persisted via recording settings.
  let draftAppearance = $state<AppearanceSetting>("system");

  // OCR drafts/status
  let draftOcrEnabled = $state(true);
  let draftOcrProvider = $state<OcrProvider>("apple_vision");
  let draftOcrModelId = $state<string | null>(null);
  let draftOcrLanguage = $state("");
  let draftOcrRecognitionMode = $state<OcrRecognitionMode>("fast");
  let draftOcrLanguageCorrection = $state(false);
  let draftOcrTesseractPageSegmentationMode = $state<OcrTesseractPageSegmentationMode>("single_block");
  let draftOcrTesseractPreprocessMode = $state<OcrTesseractPreprocessMode>("grayscale");
  let draftOcrTesseractUpscaleFactor = $state(1);
  let draftOcrTesseractCharWhitelist = $state("");
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

  // Transcription drafts/status
  let draftTranscriptionEnabled = $state(true);
  let draftTranscriptionMicrophoneEnabled = $state(true);
  let draftTranscriptionSystemAudioEnabled = $state(false);
  let draftTranscriptionProvider = $state<AudioTranscriptionProvider>("local_whisper");
  let draftTranscriptionModelId = $state<string | null>("base");
  let draftTranscriptionLanguage = $state("auto");
  let draftTranscriptionMemoryMode = $state<AudioTranscriptionMemoryMode>("balanced");
  let draftTranscriptionIdleUnloadSeconds = $state(300);
  let draftTranscriptionChunkSeconds = $state(30);
  let draftSpeakerSeparateSpeakers = $state(false);
  let draftSpeakerRecognizeSavedPeople = $state(false);
  let draftSpeakerProvider = $state("sherpa_onnx");
  let draftSpeakerModelId = $state<string | null>("pyannote-3.0-nemo-titanet-small");
  let draftSpeakerTimeoutMinutes = $state(10);
  let speakerModelStatus = $state<SpeakerAnalysisModelStatusResponse | null>(null);
  let loadingSpeakerModelStatus = $state(false);
  let speakerModelError = $state<string | null>(null);
  let speakerDownloadProgress = $state<SpeakerAnalysisModelDownloadProgress | null>(null);
  let startingSpeakerDownload = $state(false);
  let cancellingSpeakerDownload = $state(false);
  let speakerDownloadError = $state<string | null>(null);
  let deletingSpeakerModel = $state(false);
  let speakerModelDeleteMessage = $state<string | null>(null);
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

  // Debug log status
  let debugLogStatus = $state<NativeCaptureDebugLogStatus | null>(null);
  let loadingDebugLogStatus = $state(false);
  let deletingDebugLog = $state(false);
  let debugLogError = $state<string | null>(null);
  let debugLogDeleted = $state(false);

  // General app log status
  let generalLogStatus = $state<GeneralAppLogStatus | null>(null);
  let loadingGeneralLogStatus = $state(false);
  let openingGeneralLog = $state(false);
  let deletingGeneralLog = $state(false);
  let generalLogError = $state<string | null>(null);
  let generalLogDeleted = $state(false);

  // Loading / error state
  let loadingRecSettings = $state(false);
  let savingRecSettings = $state(false);
  let savingKeyboardBindings = $state(false);
  let loadingMicState = $state(false);
  let savingMicSettings = $state(false);
  let recError = $state<string | null>(null);
  let keyboardBindingsError = $state<string | null>(null);
  let micError = $state<string | null>(null);
  let recSaved = $state(false);
  let keyboardBindingsSaved = $state(false);
  let micSaved = $state(false);

  // ─── Tabs ─────────────────────────────────────────────────────────────────
  // The page is split into one-tab-at-a-time categories so the long settings
  // list doesn't overwhelm. Tabs are local UI state only — no persistence.
  type SettingsTab =
    | "capture"
    | "video"
    | "access"
    | "privacy"
    | "audio"
    | "processing"
    | "storage"
    | "appearance"
    | "developer";

  type SettingsFocus = "cliAccess";

  let activeTab = $state<SettingsTab>("capture");
  let brokerAuthorizationPromptVisible = $state(false);
  let agentAccessSection = $state<HTMLElement | null>(null);

  // Scroll-region element. The wrapper persists across tab switches (only
  // the inner `{#if activeTab === ...}` panel re-mounts), so without an
  // explicit reset the previous tab's `scrollTop` would carry over and
  // strand the user mid-page on the next tab. Reset to the top whenever
  // `activeTab` changes — matches the typical tabbed-settings expectation.
  let scrollRegion = $state<HTMLDivElement | null>(null);

  $effect(() => {
    // Track `activeTab` so this fires on every switch.
    activeTab;
    scrollRegion?.scrollTo({ top: 0, behavior: "auto" });
  });

  $effect(() => {
    const requestedTab = $page.url.searchParams.get("tab");
    const normalizedTab = normalizeSettingsTab(requestedTab);
    if (normalizedTab) {
      activeTab = normalizedTab;
    }
    const normalizedFocus = normalizeSettingsFocus($page.url.searchParams.get("focus"));
    if (normalizedFocus) {
      focusSettingsSection(normalizedFocus);
    }
  });

  let initialTabFocusDone = false;
  let initialTabFocusScheduled = false;
  let initialTabFocusAttempts = 0;
  let initialTabFocusTimer: ReturnType<typeof setTimeout> | null = null;

  function isDocumentFocusStillAtEntry(): boolean {
    if (typeof document === "undefined") return false;
    const activeElement = document.activeElement;
    return activeElement === null || activeElement === document.body;
  }

  function tryFocusInitialSettingsTab(): void {
    if (initialTabFocusDone || typeof document === "undefined") return;
    if (!isDocumentFocusStillAtEntry()) {
      initialTabFocusDone = true;
      return;
    }

    const activeTabElement = document.getElementById(`settings-tab-${activeTab}`);
    if (activeTabElement instanceof HTMLElement) {
      activeTabElement.focus({ preventScroll: true });
      if (document.activeElement === activeTabElement) {
        initialTabFocusDone = true;
        return;
      }
    }

    if (initialTabFocusAttempts >= 8) return;
    initialTabFocusAttempts += 1;
    scheduleInitialSettingsTabFocus(50);
  }

  function scheduleInitialSettingsTabFocus(delayMs = 0): void {
    if (initialTabFocusDone || initialTabFocusScheduled || typeof window === "undefined") return;
    initialTabFocusScheduled = true;
    initialTabFocusTimer = setTimeout(() => {
      initialTabFocusScheduled = false;
      initialTabFocusTimer = null;
      void tick().then(tryFocusInitialSettingsTab);
    }, delayMs);
  }

  $effect(() => {
    activeTab;
    scheduleInitialSettingsTabFocus();
  });

  $effect(() => {
    if (typeof window === "undefined") return;
    const onWindowFocus = () => scheduleInitialSettingsTabFocus();
    window.addEventListener("focus", onWindowFocus);
    return () => {
      window.removeEventListener("focus", onWindowFocus);
      if (initialTabFocusTimer !== null) {
        clearTimeout(initialTabFocusTimer);
        initialTabFocusTimer = null;
      }
    };
  });

  const tabs: { id: SettingsTab; label: string; description: string }[] = [
    { id: "capture",    label: "Capture",     description: "Sources, segments, inactivity" },
    { id: "access",     label: "Access",      description: "CLI and local tools" },
    { id: "privacy",    label: "Privacy",     description: "Metadata and exclusions" },
    { id: "video",      label: "Video",       description: "Frame rate, resolution, bitrate" },
    { id: "audio",      label: "Audio",       description: "Microphone devices & disconnects" },
    { id: "processing", label: "Processing",  description: "OCR, transcription, speakers" },
    { id: "storage",    label: "Storage",     description: "Save path, retention, cache" },
    { id: "appearance", label: "Appearance",  description: "Theme and timeline display" },
    { id: "developer",  label: "Developer",   description: "Debug toggles & logs" },
  ];

  function normalizeSettingsTab(value: string | null | undefined): SettingsTab | null {
    if (value === "capture" || value === "behavior") return "capture";
    if (value === "access" || value === "cliAccess" || value === "cli-access") return "access";
    if (value === "privacy" || value === "metadata") return "privacy";
    if (value === "video") return "video";
    if (value === "audio" || value === "microphone") return "audio";
    if (value === "processing" || value === "ocr" || value === "transcription" || value === "speakers") return "processing";
    if (value === "storage") return "storage";
    if (value === "appearance") return "appearance";
    if (value === "developer") return "developer";
    return null;
  }

  function normalizeSettingsFocus(value: string | null | undefined): SettingsFocus | null {
    if (value === "agentAccess" || value === "agent-access" || value === "cliAccess" || value === "cli-access") return "cliAccess";
    return null;
  }

  function isSettingsTab(value: string | null | undefined): value is SettingsTab {
    return normalizeSettingsTab(value) !== null;
  }

  // Keyboard navigation for the tablist follows the WAI-ARIA Authoring
  // Practices "Tabs (Manual Activation)" pattern: ←/→ move focus and
  // activate the next/previous tab, Home/End jump to the first/last tab.
  // We use a roving-tabindex (only the active tab is tabbable) so screen
  // reader users can land on the tablist and step through tabs naturally.
  function handleTabKeydown(event: KeyboardEvent) {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[role="tab"]')
      : null;
    const focusedTabId = focusedTab?.id?.replace(/^settings-tab-/, "") ?? null;
    const focusedIndex = tabs.findIndex((t) => t.id === focusedTabId);
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : tabs.findIndex((t) => t.id === activeTab);
    if (currentIndex === -1) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % tabs.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = tabs.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    event.stopPropagation();
    const nextTab = tabs[nextIndex];
    activeTab = nextTab.id;
    // Move DOM focus to the newly-active tab so the roving tabindex stays
    // visually and assistively accurate.
    const el = document.getElementById(`settings-tab-${nextTab.id}`);
    el?.focus();
  }

  function focusSettingsSection(focus: SettingsFocus): void {
    if (focus !== "cliAccess") return;
    activeTab = "access";
    brokerAuthorizationPromptVisible = true;
    void tick().then(() => {
      agentAccessSection?.scrollIntoView({ block: "start", behavior: "smooth" });
      agentAccessSection?.focus({ preventScroll: true });
    });
  }

  function handleSettingsTabEvent(tab: string, focus?: string | null): void {
    const normalizedTab = normalizeSettingsTab(tab);
    if (normalizedTab) activeTab = normalizedTab;
    const normalizedFocus = normalizeSettingsFocus(focus);
    if (normalizedFocus) focusSettingsSection(normalizedFocus);
  }

  // ─── Auto-save plumbing ──────────────────────────────────────────────────
  // To avoid feedback loops (sync from backend → drafts change → save),
  // we serialize the current draft set to a snapshot string and compare
  // against the last successfully-saved snapshot. After the backend echoes
  // back the persisted values, syncRecDrafts/syncMicDrafts updates that
  // baseline so the effect sees "no change" and stays quiet.
  const RECORDING_AUTOSAVE_DEBOUNCE_MS = 450;
  const MIC_AUTOSAVE_DEBOUNCE_MS = 250;

  let lastSavedRecSnapshot = $state<string | null>(null);
  let lastSavedKeyboardBindingsSnapshot = $state<string | null>(null);
  let lastSavedMicSnapshot = $state<string | null>(null);
  let recAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let keyboardBindingsAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let micAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;

  const appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => draftExcludedApps,
    onSettingsUpdated: (updated) => {
      recordingSettings = updated;
      syncRecDrafts(updated);
    },
    setError: (message) => {
      recError = message;
    },
    beforePrivacyCommand: () => {
      if (recAutoSaveTimer !== null) {
        clearTimeout(recAutoSaveTimer);
        recAutoSaveTimer = null;
      }
    },
    enableExistingUserPrompt: true,
  });

  // Capture-support fetch lifecycle: tracks whether the in-flight request
  // is still running and whether it ended in an unrecoverable failure.
  let captureSupportLoading = $state(false);
  let captureSupportFailed = $state(false);

  // ─── Backend capability ────────────────────────────────────────────────────
  const nativeCaptureUnsupported = $derived(
    captureSupport !== null && !captureSupport.nativeCaptureSupported
  );

  // The AVFoundation fallback backend (pre-macOS 15) only supports "original"
  // resolution. ScreenCaptureKit (macOS 15+) supports all modes.
  // The same macOS version gate controls both system audio and the SCKit
  // backend, so `supportedSources.systemAudio === false` is a precise proxy.
  const onlyOriginalResolutionSupported = $derived(
    captureSupport !== null
    && captureSupport.nativeCaptureSupported
    && !captureSupport.supportedSources.systemAudio
  );

  const nonOriginalResolutionSupported = $derived(
    captureSupport !== null
    && captureSupport.nativeCaptureSupported
    && captureSupport.supportedSources.systemAudio
  );

  // True ONLY while the support request is actively in-flight.
  // A failed lookup is NOT treated as pending — the request has completed,
  // just without useful data. Keeping it "pending" forever would permanently
  // block the user, so we distinguish the two states explicitly.
  const resolutionSupportPending = $derived(captureSupportLoading);

  // Preset and custom are selectable only once support is confirmed available.
  // Three cases: (1) in-flight → disabled, (2) loaded but AVFoundation-only →
  // disabled, (3) loaded with SCKit OR lookup failed → enabled (backend
  // validates at save time if we could not determine support locally).
  const nonOriginalResolutionDisabled = $derived(
    draftCaptureScreen
    && (resolutionSupportPending || nativeCaptureUnsupported || onlyOriginalResolutionSupported)
  );

  // Block saving only while the request is genuinely in-flight for non-original
  // modes. A failed lookup unblocks saving so the backend can validate instead.
  const resolutionSupportPendingForNonOriginal = $derived(
    draftCaptureScreen && resolutionSupportPending && draftResolutionMode !== "original"
  );

  // ─── Helpers ──────────────────────────────────────────────────────────────

  function syncRecDrafts(s: RecordingSettings) {
    draftCaptureScreen = s.captureScreen;
    draftCaptureMicrophone = s.captureMicrophone;
    draftCaptureSystemAudio = s.captureSystemAudio;
    draftSegmentDuration = s.segmentDurationSeconds;
    draftFrameRate = s.screenFrameRate;
    draftSaveDirectory = s.saveDirectory;
    draftAutoStart = s.autoStart;
    draftPauseCaptureOnInactivity = s.pauseCaptureOnInactivity;
    draftIdleTimeoutSeconds = s.idleTimeoutSeconds;
    draftActivityMode = "system_input_or_screen_or_audio";
    draftMicrophoneActivitySensitivity = s.microphoneActivitySensitivity ?? 50;
    draftSystemAudioActivitySensitivity = s.systemAudioActivitySensitivity ?? 50;
    draftMicrophoneVadAdapter = s.audioSpeechDetection?.detector ?? s.microphoneVadAdapter ?? "silero";
    draftNativeCaptureDebugLoggingEnabled = s.nativeCaptureDebugLoggingEnabled ?? false;
    draftPreviewCacheTtlSeconds = s.previewCacheTtlSeconds ?? 3600;
    draftFollowTimelineLive = s.followTimelineLive ?? false;
    draftRetentionPolicy = s.retentionPolicy ?? "never";
    draftMetadataEnabled = s.metadata?.enabled ?? true;
    draftBrowserUrlMode = s.metadata?.browserUrlMode ?? "sanitized";
    draftExcludedApps = [...(s.privacy?.excludedApps ?? [])];
    draftDeveloperOptionsEnabled = s.developerOptionsEnabled ?? false;
    draftAppearance = s.appearance ?? "system";
    draftOcrEnabled = s.ocr?.enabled ?? true;
    const loadedOcrProvider = s.ocr?.provider;
    const loadedOcrProviderSelectable = isSelectableOcrProvider(loadedOcrProvider);
    draftOcrProvider = loadedOcrProviderSelectable ? loadedOcrProvider : "apple_vision";
    draftOcrModelId = loadedOcrProviderSelectable ? (s.ocr?.modelId ?? defaultOcrModelIdForProvider(draftOcrProvider)) : defaultOcrModelIdForProvider(draftOcrProvider);
    draftOcrLanguage = loadedOcrProviderSelectable ? (s.ocr?.language ?? defaultOcrLanguageForProvider(draftOcrProvider) ?? "") : defaultOcrLanguageForProvider(draftOcrProvider) ?? "";
    draftOcrRecognitionMode = s.ocr?.recognitionMode ?? "fast";
    draftOcrLanguageCorrection = s.ocr?.languageCorrection ?? false;
    draftOcrTesseractPageSegmentationMode = s.ocr?.tesseractPageSegmentationMode ?? "single_block";
    draftOcrTesseractPreprocessMode = s.ocr?.tesseractPreprocessMode ?? "grayscale";
    draftOcrTesseractUpscaleFactor = s.ocr?.tesseractUpscaleFactor ?? 1;
    draftOcrTesseractCharWhitelist = s.ocr?.tesseractCharWhitelist ?? "";
    draftTranscriptionEnabled = s.transcription?.enabled ?? true;
    draftTranscriptionMicrophoneEnabled = s.transcription?.microphoneEnabled ?? true;
    draftTranscriptionSystemAudioEnabled = s.transcription?.systemAudioEnabled ?? false;
    draftTranscriptionProvider = s.transcription?.provider ?? "local_whisper";
    draftTranscriptionModelId = s.transcription?.modelId ?? (draftTranscriptionProvider === "apple_speech_on_device" ? null : "base");
    draftTranscriptionLanguage = s.transcription?.language ?? "auto";
    draftTranscriptionMemoryMode = s.transcription?.memoryMode ?? "balanced";
    draftTranscriptionIdleUnloadSeconds = s.transcription?.idleUnloadSeconds ?? 300;
    draftTranscriptionChunkSeconds = s.transcription?.chunkSeconds ?? 30;
    draftSpeakerSeparateSpeakers = s.speakerAnalysis?.separateSpeakers ?? false;
    draftSpeakerRecognizeSavedPeople = s.speakerAnalysis?.recognizeSavedPeople ?? false;
    draftSpeakerProvider = s.speakerAnalysis?.provider ?? "sherpa_onnx";
    draftSpeakerModelId = s.speakerAnalysis?.modelId ?? "pyannote-3.0-nemo-titanet-small";
    draftSpeakerTimeoutMinutes = Math.round((s.speakerAnalysis?.timeoutSeconds ?? 600) / 60);
    if (s.screenResolution.mode === "custom") {
      draftResolutionMode = "custom";
      draftCustomWidth = s.screenResolution.width;
      draftCustomHeight = s.screenResolution.height;
      customWidthRaw = String(s.screenResolution.width);
      customHeightRaw = String(s.screenResolution.height);
    } else if (s.screenResolution.preset === "original") {
      draftResolutionMode = "original";
      draftResolutionPreset = "1080p";
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    } else {
      draftResolutionMode = "preset";
      draftResolutionPreset = s.screenResolution.preset;
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    }
    // Video bitrate
    if (s.videoBitrate.mode === "custom") {
      draftBitrateMode = "custom";
      draftBitratePreset = "medium";
      draftCustomMbps = s.videoBitrate.customMbps;
      draftCustomMbpsRaw = String(s.videoBitrate.customMbps);
    } else {
      draftBitrateMode = "preset";
      draftBitratePreset = s.videoBitrate.preset;
      draftCustomMbps = null;
      draftCustomMbpsRaw = "";
    }
    // Mark this draft set as the "saved baseline" so the auto-save effect
    // does not immediately re-fire after we accept backend-echoed values.
    lastSavedRecSnapshot = buildRecSnapshot();
  }

  function syncKeyboardBindingsDrafts(s: KeyboardBindingsSettings) {
    draftGlobalShortcutsEnabled = s.globalShortcuts.enabled;
    lastSavedKeyboardBindingsSnapshot = buildKeyboardBindingsSnapshot();
  }

  function syncMicDrafts(s: MicrophoneControllerState) {
    draftPreferenceMode = s.preference.mode;
    draftDeviceId = s.preference.deviceId ?? null;
    draftDisconnectPolicy = s.disconnectPolicy;
    lastSavedMicSnapshot = buildMicSnapshot();
  }

  function buildRecRequest() {
    return {
      captureScreen: draftCaptureScreen,
      captureMicrophone: draftCaptureMicrophone,
      captureSystemAudio: draftCaptureSystemAudio,
      segmentDurationSeconds: draftSegmentDuration,
      screenFrameRate: draftFrameRate,
      saveDirectory: draftSaveDirectory,
      autoStart: draftAutoStart,
      pauseCaptureOnInactivity: draftPauseCaptureOnInactivity,
      idleTimeoutSeconds: draftIdleTimeoutSeconds,
      activityMode: "system_input_or_screen_or_audio",
      microphoneActivitySensitivity: draftMicrophoneActivitySensitivity,
      systemAudioActivitySensitivity: draftSystemAudioActivitySensitivity,
      microphoneVadAdapter: draftMicrophoneVadAdapter,
      audioSpeechDetection: {
        detector: draftMicrophoneVadAdapter,
      },
      metadata: {
        enabled: draftMetadataEnabled,
        browserUrlMode: draftBrowserUrlMode,
      },
      privacy: recordingSettings?.privacy ?? {
        excludedApps: draftExcludedApps,
      },
      nativeCaptureDebugLoggingEnabled: draftNativeCaptureDebugLoggingEnabled,
      previewCacheTtlSeconds: draftPreviewCacheTtlSeconds,
      followTimelineLive: draftFollowTimelineLive,
      retentionPolicy: draftRetentionPolicy,
      appearance: draftAppearance,
      developerOptionsEnabled: draftDeveloperOptionsEnabled,
      ocr: {
        enabled: draftOcrEnabled,
        provider: draftOcrProvider,
        modelId: draftOcrModelId,
        language: draftOcrLanguage.trim() || null,
        recognitionMode: draftOcrRecognitionMode,
        languageCorrection: draftOcrLanguageCorrection,
        tesseractPageSegmentationMode: draftOcrTesseractPageSegmentationMode,
        tesseractPreprocessMode: draftOcrTesseractPreprocessMode,
        tesseractUpscaleFactor: Math.max(1, Math.min(4, Math.trunc(Number(draftOcrTesseractUpscaleFactor) || 1))),
        tesseractCharWhitelist: draftOcrTesseractCharWhitelist.trim() || null,
      },
      transcription: {
        enabled: draftTranscriptionEnabled,
        microphoneEnabled: draftTranscriptionMicrophoneEnabled,
        systemAudioEnabled: draftTranscriptionSystemAudioEnabled,
        provider: draftTranscriptionProvider,
        modelId: draftTranscriptionModelId,
        language: draftTranscriptionLanguage.trim() || "auto",
        memoryMode: draftTranscriptionMemoryMode,
        idleUnloadSeconds: Math.max(0, Math.trunc(Number(draftTranscriptionIdleUnloadSeconds) || 0)),
        chunkSeconds: Math.max(0, Math.trunc(Number(draftTranscriptionChunkSeconds) || 0)),
      },
      speakerAnalysis: {
        separateSpeakers: draftSpeakerSeparateSpeakers,
        recognizeSavedPeople: draftSpeakerRecognizeSavedPeople,
        provider: draftSpeakerProvider,
        modelId: draftSpeakerModelId,
        timeoutSeconds: Math.max(60, Math.min(3600, Math.trunc(Number(draftSpeakerTimeoutMinutes) || 10) * 60)),
      },
      screenResolution: draftResolutionMode === "custom"
        ? {
            mode: "custom" as const,
            width: draftCustomWidth!,
            height: draftCustomHeight!,
          }
        : {
            mode: "preset" as const,
            preset: draftResolutionMode === "original" ? "original" as const : draftResolutionPreset,
          },
      videoBitrate: draftBitrateMode === "custom"
        ? { mode: "custom" as const, preset: null, customMbps: draftCustomMbps! }
        : { mode: "preset" as const, preset: draftBitratePreset, customMbps: null },
    };
  }

  function buildKeyboardBindingsRequest(): KeyboardBindingsSettings {
    const current = keyboardBindingsSettings ?? {
      schemaVersion: 1,
      globalShortcuts: {
        enabled: true,
        bindings: {
          toggleRecording: "CommandOrControl+Alt+R",
          toggleMainWindow: "CommandOrControl+Alt+M",
        },
      },
    };
    return {
      ...current,
      globalShortcuts: {
        ...current.globalShortcuts,
        enabled: draftGlobalShortcutsEnabled,
      },
    };
  }

  function buildMicRequest() {
    return {
      preference: {
        mode: draftPreferenceMode,
        deviceId: draftPreferenceMode === "specific_device" ? draftDeviceId : null,
      },
      disconnectPolicy: draftDisconnectPolicy,
    };
  }

  async function loadBrokerGrants() {
    brokerGrantLoading = true;
    brokerGrantError = null;
    try {
      const response = await invoke<BrokerGrantFile>("list_cli_access_grants");
      brokerGrants = response.grants ?? [];
    } catch (err) {
      brokerGrantError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      brokerGrantLoading = false;
    }
  }

  async function loadMnemaCliStatus() {
    mnemaCliLoading = true;
    mnemaCliError = null;
    try {
      mnemaCliStatus = await invoke<MnemaCliStatus>("get_cli_access_status");
    } catch (err) {
      mnemaCliError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      mnemaCliLoading = false;
    }
  }

  async function installMnemaCli() {
    mnemaCliInstalling = true;
    mnemaCliError = null;
    try {
      mnemaCliStatus = await invoke<MnemaCliStatus>(mnemaCliStatus?.installed ? "reinstall_cli" : "install_cli");
    } catch (err) {
      mnemaCliError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      mnemaCliInstalling = false;
    }
  }

  async function revokeAgentBrokerGrant(grantId: string) {
    brokerGrantSaving = true;
    brokerGrantError = null;
    try {
      await invoke<boolean>("revoke_cli_access_grant", { grantId });
      await loadBrokerGrants();
    } catch (err) {
      brokerGrantError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      brokerGrantSaving = false;
    }
  }

  async function setBrowserUrlMode(mode: string) {
    if (mode === draftBrowserUrlMode) return;
    if (mode === "full") {
      const ok = await ask("Full URL metadata stores query strings and fragments. Continue?", {
        title: "Enable full URL metadata",
        kind: "warning",
        okLabel: "Enable",
        cancelLabel: "Cancel",
      });
      if (!ok) return;
    }
    draftBrowserUrlMode = mode as BrowserUrlMode;
  }

  // Snapshots are stable JSON strings derived from the very same payload
  // shape the backend sees. Using the request shape (rather than every raw
  // draft variable) ensures invalid intermediate states — e.g. a custom
  // resolution with `null` width while the user is typing — don't generate
  // spurious snapshot churn that the auto-save guard would have to filter.
  function buildRecSnapshot(): string {
    return JSON.stringify(buildRecRequest());
  }

  function buildKeyboardBindingsSnapshot(): string {
    return JSON.stringify(buildKeyboardBindingsRequest());
  }

  function buildMicSnapshot(): string {
    return JSON.stringify(buildMicRequest());
  }

  // ─── Actions ──────────────────────────────────────────────────────────────

  async function loadCaptureSupport() {
    captureSupportLoading = true;
    captureSupportFailed = false;
    // Clear any stale data immediately so derived state (nativeCaptureUnsupported,
    // onlyOriginalResolutionSupported, nonOriginalResolutionSupported) reflects
    // the in-flight state rather than a previous result while the request runs.
    captureSupport = null;
    try {
      captureSupport = await invoke<CaptureSupport>("get_capture_support");
    } catch {
      // Non-fatal: support info is best-effort. Mark as failed so the UI can
      // distinguish "still loading" from "lookup failed". Preset/custom options
      // are unblocked on failure and the backend validates the selection on save.
      // captureSupport stays null here, which is correct — all capability-gated
      // derived values require captureSupport !== null, so none will fire.
      captureSupportFailed = true;
    } finally {
      captureSupportLoading = false;
    }
  }

  async function loadDebugLogStatus() {
    loadingDebugLogStatus = true;
    debugLogError = null;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>("get_native_capture_debug_log_status");
    } catch (err) {
      debugLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingDebugLogStatus = false;
    }
  }

  async function deleteDebugLog() {
    const ok = await ask("Delete the native capture debug log file?", {
      title: "Delete native capture debug log",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingDebugLog = true;
    debugLogError = null;
    debugLogDeleted = false;
    try {
      debugLogStatus = await invoke<NativeCaptureDebugLogStatus>("delete_native_capture_debug_log");
      debugLogDeleted = true;
      setTimeout(() => { debugLogDeleted = false; }, 2200);
    } catch (err) {
      debugLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      deletingDebugLog = false;
    }
  }

  async function loadGeneralLogStatus() {
    loadingGeneralLogStatus = true;
    generalLogError = null;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("get_general_app_log_status");
    } catch (err) {
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingGeneralLogStatus = false;
    }
  }

  async function openGeneralLog() {
    openingGeneralLog = true;
    generalLogError = null;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("open_general_app_log");
    } catch (err) {
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      openingGeneralLog = false;
    }
  }

  async function deleteGeneralLog() {
    const ok = await ask("Delete the general application log file?", {
      title: "Delete general application log",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingGeneralLog = true;
    generalLogError = null;
    generalLogDeleted = false;
    try {
      generalLogStatus = await invoke<GeneralAppLogStatus>("delete_general_app_log");
      generalLogDeleted = true;
      setTimeout(() => { generalLogDeleted = false; }, 2200);
    } catch (err) {
      generalLogError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      deletingGeneralLog = false;
    }
  }

  async function loadRecordingSettings() {
    loadingRecSettings = true;
    recError = null;
    try {
      const s = await invoke<RecordingSettings>("get_recording_settings");
      recordingSettings = s;
      syncRecDrafts(s);
    } catch (err) {
      recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingRecSettings = false;
    }
  }

  async function loadKeyboardBindingsSettings() {
    keyboardBindingsError = null;
    try {
      const s = await invoke<KeyboardBindingsSettings>("get_keyboard_bindings_settings");
      keyboardBindingsSettings = s;
      syncKeyboardBindingsDrafts(s);
    } catch (err) {
      keyboardBindingsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    }
  }

  async function loadOcrModelStatus() {
    loadingOcrModelStatus = true;
    ocrModelError = null;
    try {
      ocrModelStatus = await invoke<OcrModelStatusResponse>("get_ocr_model_status");
    } catch (err) {
      ocrModelError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingOcrModelStatus = false;
    }
  }

  async function startSelectedOcrModelDownload() {
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
      ocrDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      startingOcrDownload = false;
    }
  }

  async function cancelSelectedOcrModelDownload() {
    cancellingOcrDownload = true;
    ocrDownloadError = null;
    try {
      await invoke("cancel_ocr_model_download");
    } catch (err) {
      ocrDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
      title: "Delete unused OCR models",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
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
      deletedUnusedOcrModelLabels = result.deleted.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      skippedUnusedOcrModelLabels = result.skippedActiveDownloads.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      skippedOcrProcessingJobModelLabels = result.skippedProcessingJobs.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      deleteUnusedOcrModelsMessage =
        result.deleted.length === 0
          ? skipped > 0
            ? `No unused OCR models deleted. ${skipped} running model${skipped === 1 ? "" : "s"} skipped.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed OCR job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`
            : "No unused OCR models found."
          : `Deleted ${result.deleted.length} unused OCR model${result.deleted.length === 1 ? "" : "s"}.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed OCR job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`;
      await loadOcrModelStatus();
    } catch (err) {
      deleteUnusedOcrModelsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      deletingUnusedOcrModels = false;
      confirmingDeleteUnusedOcrModels = false;
    }
  }

  async function loadTranscriptionModelStatus() {
    loadingTranscriptionModelStatus = true;
    transcriptionModelError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>("get_audio_transcription_model_status");
    } catch (err) {
      transcriptionModelError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingTranscriptionModelStatus = false;
    }
  }

  async function requestAppleSpeechPermission() {
    requestingAppleSpeechPermission = true;
    appleSpeechPermissionError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>(
        "request_apple_speech_recognition_permission"
      );
    } catch (err) {
      appleSpeechPermissionError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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
      appleSpeechPermissionError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    }
  }

  async function startSelectedTranscriptionModelDownload() {
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
        }
      );
    } catch (err) {
      transcriptionDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      startingTranscriptionDownload = false;
    }
  }

  async function cancelSelectedTranscriptionModelDownload() {
    cancellingTranscriptionDownload = true;
    transcriptionDownloadError = null;
    try {
      await invoke("cancel_audio_transcription_model_download");
    } catch (err) {
      transcriptionDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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

  async function loadSpeakerModelStatus() {
    loadingSpeakerModelStatus = true;
    speakerModelError = null;
    try {
      speakerModelStatus = await invoke<SpeakerAnalysisModelStatusResponse>("get_speaker_analysis_model_status");
    } catch (err) {
      speakerModelError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingSpeakerModelStatus = false;
    }
  }

  async function startSelectedSpeakerModelDownload() {
    if (!selectedSpeakerModel?.modelId) return;
    startingSpeakerDownload = true;
    speakerDownloadError = null;
    speakerModelDeleteMessage = null;
    try {
      speakerDownloadProgress = await invoke<SpeakerAnalysisModelDownloadProgress>(
        "start_speaker_analysis_model_download",
        {
          request: {
            provider: selectedSpeakerModel.provider,
            modelId: selectedSpeakerModel.modelId,
          },
        }
      );
    } catch (err) {
      speakerDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      startingSpeakerDownload = false;
    }
  }

  async function cancelSelectedSpeakerModelDownload() {
    cancellingSpeakerDownload = true;
    speakerDownloadError = null;
    try {
      await invoke("cancel_speaker_analysis_model_download");
    } catch (err) {
      speakerDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      cancellingSpeakerDownload = false;
    }
  }

  async function deleteSelectedSpeakerModel() {
    if (!selectedSpeakerModel?.modelId) return;
    const ok = await ask(`Delete ${selectedSpeakerModel.displayName}?`, {
      title: "Delete speaker model",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    deletingSpeakerModel = true;
    speakerModelDeleteMessage = null;
    speakerDownloadError = null;
    try {
      await invoke("delete_speaker_analysis_model", {
        request: {
          provider: selectedSpeakerModel.provider,
          modelId: selectedSpeakerModel.modelId,
        },
      });
      speakerModelDeleteMessage = `Deleted ${selectedSpeakerModel.displayName}.`;
      await loadSpeakerModelStatus();
    } catch (err) {
      speakerDownloadError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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

  async function requestDeleteUnusedTranscriptionModels() {
    const ok = await ask("Delete unused transcription model files?", {
      title: "Delete unused transcription models",
      kind: "warning",
      okLabel: "Delete",
      cancelLabel: "Cancel",
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
      const result = await invoke<DeleteUnusedAudioTranscriptionModelsResponse>(
        "delete_unused_audio_transcription_models"
      );
      const skipped = result.skippedActiveDownloads.length + result.skippedProcessingJobs.length;
      deletedUnusedTranscriptionModelLabels = result.deleted.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      skippedUnusedTranscriptionModelLabels = result.skippedActiveDownloads.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      skippedTranscriptionProcessingJobModelLabels = result.skippedProcessingJobs.map((model) => `${model.displayName} (${model.provider}/${model.modelId})`);
      deleteUnusedTranscriptionModelsMessage =
        result.deleted.length === 0
          ? skipped > 0
            ? `No unused transcription models deleted. ${skipped} running model${skipped === 1 ? "" : "s"} skipped.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed transcription job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`
            : "No unused transcription models found."
          : `Deleted ${result.deleted.length} unused transcription model${result.deleted.length === 1 ? "" : "s"}.${result.retargetedProcessingJobs > 0 ? ` Retargeted ${result.retargetedProcessingJobs} queued/failed transcription job${result.retargetedProcessingJobs === 1 ? "" : "s"}.` : ""}`;
      await loadTranscriptionModelStatus();
    } catch (err) {
      deleteUnusedTranscriptionModelsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      deletingUnusedTranscriptionModels = false;
      confirmingDeleteUnusedTranscriptionModels = false;
    }
  }

  async function saveRecordingSettings() {
    if (appPrivacyExclusion.commandInFlight) return;
    if (resolutionSupportPendingForNonOriginal) {
      recError = "Wait for capture support to load before saving preset/custom resolution.";
      return;
    }

    const previousRetentionPolicy = recordingSettings?.retentionPolicy ?? "never";

    if (previousRetentionPolicy === "never" && draftRetentionPolicy !== "never") {
      try {
        const preview = await invoke<RetentionCleanupSummary>("preview_retention_cleanup", {
          request: { policy: draftRetentionPolicy },
        });
        retentionCleanupSummary = preview;
        const ok = await ask(
          `Retention will delete ${preview.deletedFrames} frame row(s), ${preview.deletedAudioSegments} audio segment row(s), and ${preview.eligibleCaptureSegments} capture segment(s) before ${preview.cutoffEndedBefore ?? "the cutoff"}. Continue?`,
          {
            title: "Confirm retention cleanup",
            kind: "warning",
            okLabel: "Continue",
            cancelLabel: "Cancel",
          }
        );
        if (!ok) {
          draftRetentionPolicy = recordingSettings?.retentionPolicy ?? "never";
          return;
        }
      } catch (err) {
        recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
        return;
      }
    }

    savingRecSettings = true;
    recError = null;
    recSaved = false;
    try {
      const updated = await invoke<RecordingSettings>("update_recording_settings", {
        request: buildRecRequest(),
      });
      recordingSettings = updated;
      syncRecDrafts(updated);
      setDeveloperOptionsEnabled(updated.developerOptionsEnabled ?? false);
      // Push the freshly-persisted appearance into the in-memory theme
      // runtime so the entire UI (chrome + dashboard + settings) flips
      // immediately, without waiting for a reload or settings round-trip.
      setAppearance(updated.appearance ?? "system");
      recSaved = true;
      setTimeout(() => { recSaved = false; }, 2200);
      // Refresh debug log status since the enabled flag may have changed.
      loadDebugLogStatus();

      if (previousRetentionPolicy !== updated.retentionPolicy && updated.retentionPolicy !== "never") {
        retentionCleanupRunning = true;
        retentionCleanupError = null;
        try {
          retentionCleanupSummary = await invoke<RetentionCleanupSummary>("run_retention_cleanup_now");
        } catch (err) {
          retentionCleanupError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
        } finally {
          retentionCleanupRunning = false;
        }
      }
    } catch (err) {
      recError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      savingRecSettings = false;
    }
  }

  async function runRetentionCleanupNow() {
    const ok = await ask("Run retention cleanup now? This can delete captured data that matches the current retention policy.", {
      title: "Run cleanup now",
      kind: "warning",
      okLabel: "Run cleanup",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    retentionCleanupRunning = true;
    retentionCleanupError = null;
    try {
      retentionCleanupSummary = await invoke<RetentionCleanupSummary>("run_retention_cleanup_now");
    } catch (err) {
      retentionCleanupError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      retentionCleanupRunning = false;
    }
  }

  async function saveKeyboardBindingsSettings() {
    savingKeyboardBindings = true;
    keyboardBindingsError = null;
    keyboardBindingsSaved = false;
    try {
      const updated = await invoke<KeyboardBindingsSettings>("update_keyboard_bindings_settings", {
        request: buildKeyboardBindingsRequest(),
      });
      keyboardBindingsSettings = updated;
      syncKeyboardBindingsDrafts(updated);
      keyboardBindingsSaved = true;
      setTimeout(() => { keyboardBindingsSaved = false; }, 2200);
    } catch (err) {
      keyboardBindingsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      savingKeyboardBindings = false;
    }
  }

  async function loadMicState() {
    loadingMicState = true;
    micError = null;
    try {
      const s = await invoke<MicrophoneControllerState>("get_microphone_controller_state");
      micState = s;
      syncMicDrafts(s);
    } catch (err) {
      micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      loadingMicState = false;
    }
  }

  async function saveMicSettings() {
    savingMicSettings = true;
    micError = null;
    micSaved = false;
    try {
      const updated = await invoke<MicrophoneControllerState>("update_microphone_controller", {
        request: buildMicRequest(),
      });
      micState = updated;
      syncMicDrafts(updated);
      micSaved = true;
      setTimeout(() => { micSaved = false; }, 2200);
    } catch (err) {
      micError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      savingMicSettings = false;
    }
  }

  // ─── Auto-save effects ────────────────────────────────────────────────────
  // Each effect tracks the relevant draft snapshot and schedules a debounced
  // save when (a) the snapshot diverges from the last persisted value and
  // (b) validation does not block. This replaces the manual Save button while
  // preserving validation semantics — invalid drafts simply don't trigger the
  // backend call, so persisted state stays consistent.
  $effect(() => {
    // Track the current snapshot reactively. Until the initial load completes,
    // the baseline is null and we must not persist.
    if (recordingSettings === null || lastSavedRecSnapshot === null) return;
    const current = buildRecSnapshot();
    if (current === lastSavedRecSnapshot) return;
    if (recSaveBlocked) return;
    if (appPrivacyExclusion.commandInFlight) return;
    if (savingRecSettings) return;

    if (recAutoSaveTimer !== null) clearTimeout(recAutoSaveTimer);
    recAutoSaveTimer = setTimeout(() => {
      recAutoSaveTimer = null;
      // Re-check guards at fire time — drafts may have changed during debounce.
      if (recSaveBlocked || appPrivacyExclusion.commandInFlight || savingRecSettings) return;
      if (buildRecSnapshot() === lastSavedRecSnapshot) return;
      void saveRecordingSettings();
    }, RECORDING_AUTOSAVE_DEBOUNCE_MS);
  });

  $effect(() => {
    if (keyboardBindingsSettings === null || lastSavedKeyboardBindingsSnapshot === null) return;
    const current = buildKeyboardBindingsSnapshot();
    if (current === lastSavedKeyboardBindingsSnapshot) return;
    if (savingKeyboardBindings) return;

    if (keyboardBindingsAutoSaveTimer !== null) clearTimeout(keyboardBindingsAutoSaveTimer);
    keyboardBindingsAutoSaveTimer = setTimeout(() => {
      keyboardBindingsAutoSaveTimer = null;
      if (savingKeyboardBindings) return;
      if (buildKeyboardBindingsSnapshot() === lastSavedKeyboardBindingsSnapshot) return;
      void saveKeyboardBindingsSettings();
    }, RECORDING_AUTOSAVE_DEBOUNCE_MS);
  });

  $effect(() => {
    if (micState === null || lastSavedMicSnapshot === null) return;
    const current = buildMicSnapshot();
    if (current === lastSavedMicSnapshot) return;
    if (micApplyBlocked) return;
    if (savingMicSettings) return;

    if (micAutoSaveTimer !== null) clearTimeout(micAutoSaveTimer);
    micAutoSaveTimer = setTimeout(() => {
      micAutoSaveTimer = null;
      if (micApplyBlocked || savingMicSettings) return;
      if (buildMicSnapshot() === lastSavedMicSnapshot) return;
      void saveMicSettings();
    }, MIC_AUTOSAVE_DEBOUNCE_MS);
  });

  // ─── Recording settings validation ───────────────────────────────────────

  // Invariant: system audio requires screen capture.
  // Reactively coerce the draft: if screen is turned off, force system audio off too.
  $effect(() => {
    if (!draftCaptureScreen && draftCaptureSystemAudio) {
      draftCaptureSystemAudio = false;
    }
  });

  // Invariant: coerce any non-original draft back to "original" only once we
  // have confirmed that non-original is unsupported (AVFoundation / pre-macOS 15).
  // While support is still pending we preserve the loaded draft intact — the
  // UI disables the radio options so the user cannot change them, and saving is
  // blocked by resolutionSupportPendingForNonOriginal, but we must not destroy
  // a valid preset/custom draft that was loaded from persisted settings.
  $effect(() => {
    if (draftCaptureScreen && onlyOriginalResolutionSupported && draftResolutionMode !== "original") {
      draftResolutionMode = "original";
    }
  });

  function parseCustomDimension(raw: string): number | null {
    if (!/^\d+$/.test(raw)) return null;
    const value = Number(raw);
    if (!Number.isInteger(value)) return null;
    return value;
  }

  // Parse custom resolution inputs as integers; keep null if invalid.
  $effect(() => {
    const w = parseCustomDimension(customWidthRaw);
    draftCustomWidth = w ?? null;
  });
  $effect(() => {
    const h = parseCustomDimension(customHeightRaw);
    draftCustomHeight = h ?? null;
  });

  // Parse custom bitrate input as an integer (Mbps); keep null if invalid.
  $effect(() => {
    if (!draftCustomMbpsRaw) { draftCustomMbps = null; return; }
    if (!/^\d+$/.test(draftCustomMbpsRaw.trim())) { draftCustomMbps = null; return; }
    const val = parseInt(draftCustomMbpsRaw.trim(), 10);
    draftCustomMbps = Number.isInteger(val) && val > 0 ? val : null;
  });

  const customResolutionErrors = $derived((() => {
    if (draftResolutionMode !== "custom") return [];
    const errors: string[] = [];
    const w = parseCustomDimension(customWidthRaw);
    const h = parseCustomDimension(customHeightRaw);
    if (customWidthRaw && w === null) errors.push("Width must be an integer.");
    if (customHeightRaw && h === null) errors.push("Height must be an integer.");
    if (w != null && (w < 16 || w > 8192)) errors.push("Width must be between 16 and 8192.");
    if (h != null && (h < 16 || h > 8192)) errors.push("Height must be between 16 and 8192.");
    if (!customWidthRaw || !customHeightRaw) errors.push("Both width and height are required for custom mode.");
    return errors;
  })());

  const customResolutionBlocked = $derived(
    draftResolutionMode === "custom" && customResolutionErrors.length > 0
  );

  const customBitrateErrors = $derived((() => {
    if (draftBitrateMode !== "custom") return [];
    const errors: string[] = [];
    if (!draftCustomMbpsRaw) {
      errors.push("Custom bitrate is required (1–40 Mbps, whole number).");
    } else if (!/^\d+$/.test(draftCustomMbpsRaw.trim())) {
      errors.push("Bitrate must be a whole number of Mbps (e.g. 12).");
    } else {
      const val = parseInt(draftCustomMbpsRaw.trim(), 10);
      if (!Number.isInteger(val) || val <= 0) {
        errors.push("Bitrate must be a positive whole number.");
      } else if (val < 1) {
        errors.push("Bitrate must be at least 1 Mbps.");
      } else if (val > 40) {
        errors.push("Bitrate must not exceed 40 Mbps.");
      }
    }
    return errors;
  })());

  const customBitrateBlocked = $derived(
    draftBitrateMode === "custom" && customBitrateErrors.length > 0
  );

  const recValidationErrors = $derived((() => {
    const errors: string[] = [];
    const anySource = draftCaptureScreen || draftCaptureMicrophone || draftCaptureSystemAudio;
    if (!anySource) {
      errors.push("At least one capture source (Screen, Microphone, or System Audio) must be enabled.");
    }
    if (draftCaptureSystemAudio && !draftCaptureScreen) {
      errors.push("System Audio capture requires Screen capture to be enabled.");
    }
    if (resolutionSupportPendingForNonOriginal) {
      errors.push("Wait for capture support to load before saving preset/custom resolution.");
    }
    return errors;
  })());

  const recSaveBlocked = $derived(
    recValidationErrors.length > 0 || !draftSaveDirectory || customResolutionBlocked || customBitrateBlocked
  );

  const micApplyBlocked = $derived(
    draftPreferenceMode === "specific_device" && !draftDeviceId
  );

  const micDeviceOptions = $derived(
    (micState?.devices ?? []).map((d) => ({
      value: d.id,
      label: d.name + (d.isDefault ? " (default)" : ""),
    }))
  );

  const ocrProviderOptions = $derived(
    (ocrModelStatus?.providers ?? [])
      .filter((provider) => isSelectableOcrProvider(provider.provider))
      .map((provider) => ({
        value: provider.provider,
        label: provider.displayName,
        description: provider.models.some((model) => model.available)
          ? "Available now"
          : "Unavailable or missing",
      }))
  );

  const selectedOcrProviderStatus = $derived(
    ocrModelStatus?.providers.find((provider) => provider.provider === draftOcrProvider) ?? null
  );

  const selectedOcrModels = $derived(selectedOcrProviderStatus?.models ?? []);

  const ocrModelOptions = $derived(
    selectedOcrModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: `${model.displayName} · ${ocrStatusLabel(model)}`,
    }))
  );

  const selectedOcrModel = $derived(
    selectedOcrModels.find((model) => model.modelId === draftOcrModelId) ?? selectedOcrModels[0] ?? null
  );

  const selectedOcrDownloadProgress = $derived(
    ocrDownloadProgress
      && ocrDownloadProgress.provider === draftOcrProvider
      && ocrDownloadProgress.modelId === draftOcrModelId
      ? ocrDownloadProgress
      : null
  );

  const selectedOcrDownloadRunning = $derived(
    selectedOcrDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedOcrDownloadProgress.status)
  );

  const selectedOcrDownloadPercent = $derived((() => {
    const progress = selectedOcrDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  })());

  const transcriptionProviderOptions = $derived(
    (transcriptionModelStatus?.providers ?? []).map((provider) => ({
      value: provider.provider,
      label: provider.displayName,
      description: provider.models.some((model) => model.available)
        ? "At least one model is available"
        : "No available model detected",
    }))
  );

  const selectedTranscriptionProviderStatus = $derived(
    transcriptionModelStatus?.providers.find((provider) => provider.provider === draftTranscriptionProvider) ?? null
  );

  const selectedTranscriptionModels = $derived(
    selectedTranscriptionProviderStatus?.models ?? []
  );

  const transcriptionModelOptions = $derived(
    selectedTranscriptionModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: `${model.displayName} · ${transcriptionStatusLabel(model)}`,
    }))
  );

  const selectedTranscriptionModel = $derived(
    selectedTranscriptionModels.find((model) => model.modelId === draftTranscriptionModelId) ?? selectedTranscriptionModels[0] ?? null
  );

  const selectedAppleSpeechPermissionStatus = $derived(
    selectedTranscriptionModel?.provider === "apple_speech_on_device"
      ? selectedTranscriptionModel.availabilityStatus
      : null
  );

  const selectedAppleSpeechNeedsPermission = $derived(
    selectedAppleSpeechPermissionStatus === "permission_not_determined"
      || selectedAppleSpeechPermissionStatus === "permission_denied"
      || selectedAppleSpeechPermissionStatus === "permission_restricted"
  );

  const selectedTranscriptionDownloadProgress = $derived(
    transcriptionDownloadProgress
      && transcriptionDownloadProgress.provider === draftTranscriptionProvider
      && transcriptionDownloadProgress.modelId === draftTranscriptionModelId
      ? transcriptionDownloadProgress
      : null
  );

  const selectedTranscriptionDownloadRunning = $derived(
    selectedTranscriptionDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedTranscriptionDownloadProgress.status)
  );

  const selectedTranscriptionDownloadPercent = $derived((() => {
    const progress = selectedTranscriptionDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  })());

  const selectedSpeakerProviderStatus = $derived(
    speakerModelStatus?.providers.find((provider) => provider.provider === draftSpeakerProvider) ?? speakerModelStatus?.providers[0] ?? null
  );

  const selectedSpeakerModels = $derived(
    selectedSpeakerProviderStatus?.models ?? []
  );

  const selectedSpeakerModel = $derived(
    selectedSpeakerModels.find((model) => model.modelId === draftSpeakerModelId) ?? selectedSpeakerModels[0] ?? null
  );

  const selectedSpeakerDownloadProgress = $derived(
    speakerDownloadProgress
      && speakerDownloadProgress.provider === selectedSpeakerModel?.provider
      && speakerDownloadProgress.modelId === selectedSpeakerModel?.modelId
      ? speakerDownloadProgress
      : null
  );

  const selectedSpeakerDownloadRunning = $derived(
    selectedSpeakerDownloadProgress !== null
      && ["starting", "downloading", "installing"].includes(selectedSpeakerDownloadProgress.status)
  );

  const selectedSpeakerDownloadPercent = $derived((() => {
    const progress = selectedSpeakerDownloadProgress;
    if (!progress?.totalBytes || progress.totalBytes <= 0) return null;
    return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
  })());

  function speakerStatusLabel(model: SpeakerAnalysisModelStatus): string {
    if (model.status === "installed") return "Installed";
    if (model.status === "downloading") return "Downloading";
    if (model.status === "failed") return "Failed";
    if (model.status === "incomplete") return "Incomplete";
    return "Missing";
  }

  function transcriptionStatusLabel(model: AudioTranscriptionModelStatus): string {
    if (model.provider === "apple_speech_on_device" && model.availabilityStatus) {
      return appleSpeechPermissionLabel(model.availabilityStatus);
    }
    if (model.status === "os_managed") return "OS managed";
    if (model.status === "installed") return "Installed";
    if (model.status === "downloading") return "Downloading";
    if (model.status === "failed") return "Failed";
    return "Missing";
  }

  function appleSpeechPermissionLabel(status: AppleSpeechOnDeviceAvailabilityStatus): string {
    switch (status) {
      case "available":
        return "Permission granted";
      case "permission_not_determined":
        return "Permission not requested";
      case "permission_denied":
        return "Permission denied";
      case "permission_restricted":
        return "Permission restricted";
      case "unsupported_platform":
        return "Unsupported platform";
      case "framework_unavailable":
        return "Speech framework unavailable";
      case "recognizer_unavailable":
        return "Recognizer unavailable";
      case "on_device_recognition_unavailable":
        return "On-device recognition unavailable";
    }
  }

  function appleSpeechPermissionHint(status: AppleSpeechOnDeviceAvailabilityStatus): string {
    switch (status) {
      case "available":
        return "macOS has granted Speech Recognition permission for Mnema.";
      case "permission_not_determined":
        return "Mnema has not asked macOS for Speech Recognition permission yet. Request it before recording with Apple Speech selected.";
      case "permission_denied":
        return "macOS denied Speech Recognition permission. Enable it in System Settings → Privacy & Security → Speech Recognition, then refresh.";
      case "permission_restricted":
        return "macOS reports Speech Recognition permission is restricted by policy or parental controls.";
      default:
        return "Apple Speech cannot be used until this macOS availability check passes.";
    }
  }

  function ocrStatusLabel(model: OcrModelStatus): string {
    if (model.available) return "Available";
    if (model.status === "os_managed") return "OS managed";
    if (model.status === "installed") return "Installed";
    if (model.status === "downloading") return "Downloading";
    if (model.status === "failed") return "Failed";
    return "Missing";
  }

  function isSelectableOcrProvider(value: string | null | undefined): value is OcrProvider {
    return SELECTABLE_OCR_PROVIDERS.includes(value as OcrProvider);
  }

  function defaultOcrModelIdForProvider(provider: OcrProvider): string | null {
    if (provider === "tesseract") return "tesseract-5.5.2";
    return null;
  }

  function defaultOcrLanguageForProvider(provider: OcrProvider): string | null {
    if (provider === "tesseract") return "eng";
    return null;
  }

  function preferredOcrModelIdForProvider(provider: OcrProvider): string | null {
    const providerStatus = ocrModelStatus?.providers.find((status) => status.provider === provider);
    const defaultModelId = defaultOcrModelIdForProvider(provider);
    if (!providerStatus) return defaultModelId;
    const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
    return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
  }

  function chooseOcrProvider(provider: string) {
    if (!isSelectableOcrProvider(provider)) return;
    draftOcrProvider = provider;
    draftOcrModelId = preferredOcrModelIdForProvider(draftOcrProvider);
    draftOcrLanguage = defaultOcrLanguageForProvider(draftOcrProvider) ?? "";
  }

  function chooseOcrModel(value: string) {
    draftOcrModelId = value === "__os_managed__" ? null : value;
  }

  function formatBytes(value: number): string {
    if (!Number.isFinite(value) || value <= 0) return "unknown size";
    const units = ["B", "KB", "MB", "GB"];
    let size = value;
    let unit = 0;
    while (size >= 1024 && unit < units.length - 1) {
      size /= 1024;
      unit += 1;
    }
    return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
  }

  function defaultTranscriptionModelIdForProvider(provider: AudioTranscriptionProvider): string | null {
    if (provider === "local_whisper") return "base";
    if (provider === "parakeet") return "parakeet-tdt-0.6b-v3-onnx-int8";
    return null;
  }

  function preferredTranscriptionModelIdForProvider(provider: AudioTranscriptionProvider): string | null {
    const providerStatus = transcriptionModelStatus?.providers.find((status) => status.provider === provider);
    const defaultModelId = defaultTranscriptionModelIdForProvider(provider);
    if (!providerStatus) return defaultModelId;
    const defaultModel = providerStatus.models.find((model) => model.modelId === defaultModelId);
    return defaultModel?.modelId ?? providerStatus.models[0]?.modelId ?? defaultModelId;
  }

  function chooseTranscriptionProvider(provider: string) {
    draftTranscriptionProvider = provider as AudioTranscriptionProvider;
    draftTranscriptionModelId = preferredTranscriptionModelIdForProvider(draftTranscriptionProvider);
  }

  function chooseTranscriptionModel(value: string) {
    draftTranscriptionModelId = value === "__os_managed__" ? null : value;
  }

  // ─── Init ─────────────────────────────────────────────────────────────────

  $effect(() => {
    loadCaptureSupport();
    loadRecordingSettings();
    loadKeyboardBindingsSettings();
    loadMicState();
    loadOcrModelStatus();
    loadTranscriptionModelStatus();
    loadSpeakerModelStatus();
    loadDebugLogStatus();
    loadGeneralLogStatus();
    void appPrivacyExclusion.loadPrivacyAppCandidates();
    void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    loadBrokerGrants();
    loadMnemaCliStatus();

    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
    let unlistenOpenSettingsTab: (() => void) | undefined;
    let unlistenOcrDownloadProgress: (() => void) | undefined;
    let unlistenTranscriptionDownloadProgress: (() => void) | undefined;
    let unlistenSpeakerDownloadProgress: (() => void) | undefined;
    let destroyed = false;

    listen<MicrophoneControllerState>("microphone_controller_changed", (event) => {
      micState = event.payload;
      syncMicDrafts(event.payload);
      micError = null;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenControllerChanged = fn;
    });

    listen<MicrophoneAutoDisconnectTransitionFailedEvent>(
      "microphone_auto_disconnect_transition_failed",
      (event) => {
        const { context, code, message } = event.payload;
        micError = `[${context}] [${code}] ${message}`;
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenAutoDisconnectFailure = fn;
    });

    listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      recordingSettings = event.payload;
      syncRecDrafts(event.payload);
      recError = null;
      void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    });


    listen<{ tab: string; focus?: string }>("open_settings_tab", (event) => {
      handleSettingsTabEvent(event.payload.tab, event.payload.focus);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenOpenSettingsTab = fn;
    });

    listen<OcrModelDownloadProgress>(
      OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void handleOcrDownloadProgress(event.payload);
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenOcrDownloadProgress = fn;
    });

    listen<AudioTranscriptionModelDownloadProgress>(
      AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void handleTranscriptionDownloadProgress(event.payload);
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenTranscriptionDownloadProgress = fn;
    });

    listen<SpeakerAnalysisModelDownloadProgress>(
      SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void handleSpeakerDownloadProgress(event.payload);
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSpeakerDownloadProgress = fn;
    });

    return () => {
      destroyed = true;
      if (keyboardBindingsAutoSaveTimer !== null) clearTimeout(keyboardBindingsAutoSaveTimer);
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
      unlistenRecordingSettingsChanged?.();
      unlistenOpenSettingsTab?.();
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenSpeakerDownloadProgress?.();
    };
  });
</script>

{#snippet settingsCardIcon(kind: CardIconKind)}
  <span class="card-icon" aria-hidden="true">
    {#if kind === "capture"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8" />
        <path d="M12 17v4" />
      </svg>
    {:else if kind === "video"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="14" rx="2" />
        <path d="m10 9 5 3-5 3Z" />
      </svg>
    {:else if kind === "storage"}
      <svg viewBox="0 0 24 24">
        <ellipse cx="12" cy="6" rx="7" ry="3" />
        <path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6" />
        <path d="M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
      </svg>
    {:else if kind === "appearance"}
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="8" />
        <path d="M12 4a8 8 0 0 0 0 16Z" />
      </svg>
    {:else if kind === "inactivity"}
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="8" />
        <path d="M12 7v5l3 2" />
      </svg>
    {:else if kind === "processing"}
      <svg viewBox="0 0 24 24">
        <path d="M7 4H5a1 1 0 0 0-1 1v2" />
        <path d="M17 4h2a1 1 0 0 1 1 1v2" />
        <path d="M7 20H5a1 1 0 0 1-1-1v-2" />
        <path d="M17 20h2a1 1 0 0 0 1-1v-2" />
        <path d="M8 10h8" />
        <path d="M8 14h5" />
      </svg>
    {:else if kind === "transcription"}
      <svg viewBox="0 0 24 24">
        <path d="M4 12h2l2-5 3 10 2-6 2 3h5" />
        <path d="M5 20h14" />
      </svg>
    {:else if kind === "speaker"}
      <svg viewBox="0 0 24 24">
        <path d="M11 5 7 9H4v6h3l4 4Z" />
        <path d="M15 9a4 4 0 0 1 0 6" />
        <path d="M18 6a8 8 0 0 1 0 12" />
      </svg>
    {:else if kind === "developer"}
      <svg viewBox="0 0 24 24">
        <path d="m8 9-4 3 4 3" />
        <path d="m16 9 4 3-4 3" />
        <path d="m14 5-4 14" />
      </svg>
    {:else if kind === "privacy"}
      <svg viewBox="0 0 24 24">
        <path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6Z" />
        <path d="M9 12h6" />
        <path d="M12 9v6" />
      </svg>
    {:else if kind === "audio"}
      <svg viewBox="0 0 24 24">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0" />
        <path d="M12 18v3" />
        <path d="M9 21h6" />
      </svg>
    {/if}
  </span>
{/snippet}

<!-- ── Page intro ──────────────────────────────────────────────────────── -->
<header class="page-header">
  <div class="page-header__head">
    <div>
      <h1 class="page-header__title">Settings</h1>
    </div>
    <div class="page-header__status" aria-live="polite">
      {#if recError || keyboardBindingsError || micError}
        <span class="page-header__status-text page-header__status-text--error">save failed</span>
      {:else if recSaveBlocked || micApplyBlocked}
        <span class="page-header__status-text page-header__status-text--blocked">resolve issues</span>
      {:else if savingRecSettings || savingKeyboardBindings || savingMicSettings}
        <span class="page-header__status-text page-header__status-text--saving">saving</span>
      {:else if recSaved || keyboardBindingsSaved || micSaved}
        <span class="page-header__status-text page-header__status-text--ok">saved</span>
      {:else}
        <span class="page-header__status-text">auto-save on</span>
      {/if}
    </div>
  </div>
  <p class="page-subtitle">Tune capture, microphone &amp; diagnostics for this workstation.</p>

  {#if recordingSettings}
    <ul class="status-strip" aria-label="Current capture summary">
      <li class="status-pill" class:status-pill--on={draftCaptureScreen}>
        <span class="status-pill__dot"></span>
        <span class="status-pill__label">Screen</span>
      </li>
      <li class="status-pill" class:status-pill--on={draftCaptureMicrophone}>
        <span class="status-pill__dot"></span>
        <span class="status-pill__label">Mic</span>
      </li>
      <li class="status-pill" class:status-pill--on={draftCaptureSystemAudio}>
        <span class="status-pill__dot"></span>
        <span class="status-pill__label">Sys Audio</span>
      </li>
      <li class="status-pill status-pill--info">
        <span class="status-pill__label">{draftFrameRate}fps</span>
      </li>
      <li class="status-pill status-pill--info">
        <span class="status-pill__label">
          {#if draftResolutionMode === "original"}original{:else if draftResolutionMode === "preset"}{draftResolutionPreset}{:else}custom{/if}
        </span>
      </li>
    </ul>
  {/if}

  <AppPrivacyExclusionPrompt
    controller={appPrivacyExclusion}
    onReview={() => { activeTab = "privacy"; }}
  />
</header>

<!-- ── Tab navigation ─────────────────────────────────────────────────────
     Categorized tabs replace the previous long scrolling list. Only one
     section is mounted at a time (see the `{#if activeTab === ...}` guards
     below) so the page stays focused and changes within an unselected tab
     don't trigger reactivity in unrelated UI. -->
<nav class="tab-nav" aria-label="Settings categories">
  <div class="tab-nav__list" role="tablist" tabindex="-1" onkeydown={handleTabKeydown}>
    {#each tabs as tab}
      <button
        class="tab-nav__tab"
        class:tab-nav__tab--active={activeTab === tab.id}
        role="tab"
        aria-selected={activeTab === tab.id}
        aria-controls="settings-panel-{tab.id}"
        id="settings-tab-{tab.id}"
        tabindex={activeTab === tab.id ? 0 : -1}
        onkeydown={handleTabKeydown}
        onclick={() => { activeTab = tab.id; }}
        title={tab.description}
        type="button"
      >
        <span class="tab-nav__label">{tab.label}</span>
      </button>
    {/each}
  </div>
</nav>

<!-- ── Scroll region ──────────────────────────────────────────────────────
     Only the panel area below the tabs scrolls. The page header and the
     tab strip stay pinned at the top of the dedicated Settings window so
     switching tabs never loses the user's place behind the viewport edge.
     The wrapper participates in the flex column established by
     `.app-content`, taking the leftover height (`flex: 1`) and isolating
     overflow with `overflow-y: auto` + `min-height: 0` (the latter lets it
     shrink below its content's intrinsic height inside the flex parent). -->
<div class="settings-scroll" bind:this={scrollRegion}>

<!-- ── Capture & sources ───────────────────────────────────────────────── -->
{#if activeTab === "access"}
  <div role="tabpanel" id="settings-panel-access" aria-labelledby="settings-tab-access" tabindex="0">
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("privacy")}
          <div>
            <h2 class="card__title">Access</h2>
            <p class="card__subtitle">Install the Mnema CLI and manage local tool access to searchable Mnema text.</p>
          </div>
        </div>
      </div>

      <div
        class:settings-group--attention={brokerAuthorizationPromptVisible}
        bind:this={agentAccessSection}
        class="settings-group"
        tabindex="-1"
      >
        <span class="group-label">CLI Access</span>
        <div class="settings-stack">
          {#if brokerAuthorizationPromptVisible}
            <div class="agent-access-callout" role="status">
              <strong>CLI access request</strong>
              <p>Review the request window or native prompt, then rerun the CLI command if needed.</p>
            </div>
          {/if}
          <div class="privacy-disclosure">
            <p>CLI Access lets local tools request time-bounded access to searchable Mnema text, including screen text, audio transcripts, and timeline results.</p>
            <p>CLI output does not include media paths, raw database rows, app/window titles, browser URLs, or deep-link URLs.</p>
            {#if mnemaCliStatus}
              <p>
                CLI: {mnemaCliStatus.installed ? `mnema installed at ${mnemaCliStatus.installPath}` : `mnema not installed at ${mnemaCliStatus.installPath}`}
              </p>
              {#if mnemaCliStatus.installed && !mnemaCliStatus.installDirInPath}
                <p>{mnemaCliStatus.installDir} is not in PATH for this app session.</p>
              {/if}
            {/if}
          </div>
          <div class="row-actions">
            <button class="btn btn--ghost btn--sm" type="button" disabled={mnemaCliInstalling || mnemaCliLoading} onclick={installMnemaCli}>
              {mnemaCliStatus?.installed ? "Reinstall CLI" : "Install CLI"}
            </button>
            <button class="btn btn--ghost btn--sm" type="button" disabled={brokerGrantSaving || brokerGrantLoading || mnemaCliLoading} onclick={() => { void loadBrokerGrants(); void loadMnemaCliStatus(); }}>
              Refresh
            </button>
          </div>
          {#if mnemaCliError}
            <p class="error-text">{mnemaCliError}</p>
          {/if}
          {#if brokerGrantError}
            <p class="error-text">{brokerGrantError}</p>
          {/if}
          {#if brokerGrants.length > 0}
            <div class="excluded-apps-list">
              {#each brokerGrants as grant (grant.id)}
                <div class="excluded-app-row">
                  <div class="excluded-app-row__meta">
                    <span class="excluded-app-row__name">{grant.label}</span>
                    <span class="excluded-app-row__bundle">
                      {grant.revoked ? "Revoked" : `Expires ${new Date(grant.expiresAtUnixMs).toLocaleString()}`}
                    </span>
                  </div>
                  <button class="btn btn--ghost btn--sm" type="button" disabled={brokerGrantSaving || grant.revoked} onclick={() => revokeAgentBrokerGrant(grant.id)}>Revoke</button>
                </div>
              {/each}
            </div>
          {:else}
            <p class="group-hint">No CLI Access grants yet.</p>
          {/if}
        </div>
      </div>
    </section>
  </div>
{/if}

{#if activeTab === "capture"}
<div role="tabpanel" id="settings-panel-capture" aria-labelledby="settings-tab-capture" tabindex="0">
<section class="card">
  <div class="card__header">
    <div class="card__heading">
      {@render settingsCardIcon("capture")}
      <div>
        <h2 id="card-capture" class="card__title">Capture</h2>
        <p class="card__subtitle">What gets recorded and how often segments roll over.</p>
      </div>
    </div>
    <button class="btn btn--ghost btn--sm" onclick={loadRecordingSettings} disabled={loadingRecSettings}>
      {loadingRecSettings ? "…" : "Reload"}
    </button>
  </div>

  {#if loadingRecSettings}
    <p class="loading-text">Loading settings…</p>
  {:else}
    <div class="settings-group">
      <span class="group-label">Capture Sources</span>
      <div class="settings-stack">
        <Switch
          bind:checked={draftCaptureScreen}
          label="Screen"
          description="Capture the display"
        />
        <Switch
          bind:checked={draftCaptureMicrophone}
          label="Microphone"
          description="Capture audio from microphone"
        />
        <Switch
          bind:checked={draftCaptureSystemAudio}
          disabled={!draftCaptureScreen}
          label="System Audio"
          description="Capture Mac system audio (macOS 15+)"
        />
        {#if !draftCaptureScreen}
          <p class="capture-source-hint">System Audio is unavailable — enable Screen first.</p>
        {/if}
      </div>
    </div>

    <div class="settings-group">
      <span class="group-label">Segment Duration</span>
      <Slider
        bind:value={draftSegmentDuration}
        min={10}
        max={300}
        step={10}
        label="Duration"
        unit="s"
        formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60}s` : `${v}s`}
      />
      <p class="group-hint">How long each recording segment is before a new one starts.</p>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Keyboard</span>
      <Switch
        bind:checked={draftGlobalShortcutsEnabled}
        label="Global shortcuts"
        description="Use system-wide shortcuts to show Mnema and start or stop recording while it is in the background"
      />
      <p class="group-hint">
        Show or hide Mnema with <strong>⌥⌘M</strong>. Start or stop recording with <strong>⌥⌘R</strong>.
      </p>
    </div>
  {/if}
</section>
</div>
{/if}

<!-- ── Recording details (split into focused cards) ────────────────────── -->
{#if !loadingRecSettings}
  {#if activeTab === "privacy"}
    <div role="tabpanel" id="settings-panel-privacy" aria-labelledby="settings-tab-privacy" tabindex="0">
      <section class="card" class:card--combobox-open={appPrivacyExclusion.comboboxOpen}>
        <div class="card__header">
          <div class="card__heading">
            {@render settingsCardIcon("privacy")}
            <div>
              <h2 class="card__title">Privacy</h2>
              <p class="card__subtitle">Frame metadata and recording exclusions.</p>
            </div>
          </div>
        </div>

        <div class="settings-group">
          <span class="group-label">Metadata</span>
          <div class="settings-stack">
            <Switch
              bind:checked={draftMetadataEnabled}
              label="Capture frame context"
              description="Store app, window, and supported browser context with frames"
            />
            <SelectMenu
              value={draftBrowserUrlMode}
              onValueChange={setBrowserUrlMode}
              label="Browser URL mode"
              options={[
                { value: "off", label: "Off" },
                { value: "sanitized", label: "Sanitized" },
                { value: "full", label: "Full" },
              ]}
              disabled={!draftMetadataEnabled}
            />
          </div>
          <p class="group-hint">Sanitized URLs keep scheme, host, port, and path while dropping query strings and fragments.</p>
        </div>

        <div class="settings-group">
          <span class="group-label">Excluded Apps</span>
          <AppPrivacyExclusion controller={appPrivacyExclusion} />
        </div>
      </section>
    </div>
  {/if}

  {#if activeTab === "video"}
    <div role="tabpanel" id="settings-panel-video" aria-labelledby="settings-tab-video" tabindex="0">
    <!-- ── Card: Video Output ─────────────────────── -->
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("video")}
          <div>
            <h2 id="card-video" class="card__title">Video Output</h2>
            <p class="card__subtitle">Frame rate, resolution &amp; bitrate.</p>
          </div>
        </div>
      </div>

    <div class="settings-group">
      <span class="group-label">Screen Frame Rate</span>
      <Slider
        bind:value={draftFrameRate}
        min={1}
        max={120}
        step={1}
        label="Frame rate"
        unit=" fps"
      />
      <p class="group-hint">Higher frame rates produce larger files.</p>
    </div>

    <div class="settings-group">
      <span class="group-label">Screen Resolution</span>

      {#if nativeCaptureUnsupported}
        <div class="resolution-unsupported-notice">
          <span class="resolution-unsupported-notice__icon">ℹ</span>
          <span class="resolution-unsupported-notice__text">
            Native screen capture is unsupported on this system. Resolution settings are saved,
            but only apply when native screen capture is available.
          </span>
        </div>
      {:else if onlyOriginalResolutionSupported}
        <div class="resolution-locked-notice">
          <span class="resolution-locked-notice__icon">ℹ</span>
          <span class="resolution-locked-notice__text">
            Preset and custom resolutions require macOS 15 or later (ScreenCaptureKit).
            Only <strong>Original</strong> resolution is available on this system.
          </span>
        </div>
      {:else if resolutionSupportPending}
        <div class="resolution-pending-notice">
          <span class="resolution-pending-notice__icon">⏳</span>
          <span class="resolution-pending-notice__text">
            Checking capture support… Preset and Custom are disabled until support is confirmed.
          </span>
        </div>
      {:else if captureSupportFailed}
        <div class="resolution-warn-notice">
          <span class="resolution-warn-notice__icon">⚠</span>
          <span class="resolution-warn-notice__text">
            Could not determine capture support for this system. You can still edit and save —
            the backend will validate the chosen resolution.
          </span>
        </div>
      {:else if nonOriginalResolutionSupported}
        <div class="resolution-supported-notice">
          <span class="resolution-supported-notice__icon">✓</span>
          <span class="resolution-supported-notice__text">
            Native capture supports Preset and Custom output resolutions.
          </span>
        </div>
      {/if}

      <RadioGroup
        bind:value={draftResolutionMode}
        disabledValues={nonOriginalResolutionDisabled ? ["preset", "custom"] : []}
        options={[
          { value: "original", label: "Original", description: "Capture at the display's native resolution" },
          { value: "preset", label: "Preset", description: "Select a standard output resolution" },
          { value: "custom", label: "Custom", description: "Enter exact width and height in pixels" },
        ]}
      />

      {#if draftResolutionMode === "preset"}
        <div class="resolution-preset-grid">
          {#each (["1080p", "720p", "540p"] as const) as preset}
            {@const presetMeta = { "1080p": { w: 1920, h: 1080 }, "720p": { w: 1280, h: 720 }, "540p": { w: 960, h: 540 } }[preset]}
            <button
              class="preset-chip"
              class:preset-chip--active={draftResolutionPreset === preset}
              onclick={() => { draftResolutionPreset = preset; }}
              type="button"
            >
              <span class="preset-chip__label">{preset}</span>
              <span class="preset-chip__dim">{presetMeta.w}×{presetMeta.h}</span>
            </button>
          {/each}
        </div>
      {/if}

      {#if draftResolutionMode === "custom"}
        <div class="custom-resolution-inputs">
          <div class="custom-res-field">
            <label class="custom-res-label" for="res-width">Width (px)</label>
            <input
              id="res-width"
              type="text"
              inputmode="numeric"
              class="text-input custom-res-input"
              class:text-input--empty={customWidthRaw && draftCustomWidth === null}
              bind:value={customWidthRaw}
              placeholder="e.g. 1920"
              autocomplete="off"
            />
          </div>
          <span class="custom-res-sep" aria-hidden="true">×</span>
          <div class="custom-res-field">
            <label class="custom-res-label" for="res-height">Height (px)</label>
            <input
              id="res-height"
              type="text"
              inputmode="numeric"
              class="text-input custom-res-input"
              class:text-input--empty={customHeightRaw && draftCustomHeight === null}
              bind:value={customHeightRaw}
              placeholder="e.g. 1080"
              autocomplete="off"
            />
          </div>
        </div>

        {#if customResolutionErrors.length > 0}
          <div class="inline-validation">
            {#each customResolutionErrors as err}
              <p class="inline-validation__item">
                <span class="inline-validation__icon">⚠</span>
                {err}
              </p>
            {/each}
          </div>
        {/if}
      {/if}

      <p class="group-hint">
        {#if draftResolutionMode === "original"}
          Output files will match the captured display's native pixel dimensions.
        {:else if draftResolutionMode === "preset"}
          Output will be scaled to the selected preset. Aspect ratio is preserved.
        {:else}
          Output will be scaled to the exact dimensions you specify.
        {/if}
      </p>
    </div>

    <!-- ── Video Bitrate ──────────────────────────────────────── -->
    <div class="settings-group">
      <span class="group-label">Video Bitrate</span>
      <p class="group-hint">
        Bitrate controls the amount of data encoded per second of video.
        Higher bitrate = sharper image and less compression artefact, but
        larger files and higher CPU/GPU load. Lower bitrate reduces file size
        and power use at the cost of some visual quality.
        This setting is applied on <strong>macOS 15+ via ScreenCaptureKit</strong>;
        older systems fall back to the macOS system-default bitrate.
      </p>

      <!-- Mode selector (preset chips + custom) -->
      <div class="bitrate-mode-chips">
        {#each (["low", "medium", "high"] as const) as bp}
          {@const meta = { low: { mbps: "~3", hint: "Lower quality, smallest file" }, medium: { mbps: "~8", hint: "Balanced quality and size" }, high: { mbps: "~20", hint: "High quality, larger file" } }[bp]}
          <button
            type="button"
            class="bitrate-chip"
            class:bitrate-chip--active={draftBitrateMode === "preset" && draftBitratePreset === bp}
            onclick={() => { draftBitrateMode = "preset"; draftBitratePreset = bp; }}
          >
            <span class="bitrate-chip__label">{bp}</span>
            <span class="bitrate-chip__mbps">{meta.mbps} Mbps</span>
          </button>
        {/each}
        <button
          type="button"
          class="bitrate-chip"
          class:bitrate-chip--active={draftBitrateMode === "custom"}
          onclick={() => { draftBitrateMode = "custom"; }}
        >
          <span class="bitrate-chip__label">Custom</span>
          <span class="bitrate-chip__mbps">1–40 Mbps (integer)</span>
        </button>
      </div>

      {#if draftBitrateMode === "preset"}
        <p class="group-hint bitrate-preset-hint">
          {#if draftBitratePreset === "low"}
            <strong>Low</strong> — ~3 Mbps. Good for long sessions, minimal storage. Best for
            low-motion content or when disk space is limited.
          {:else if draftBitratePreset === "medium"}
            <strong>Medium</strong> — ~8 Mbps. Recommended default. Balanced quality and file
            size for most screen recordings.
          {:else}
            <strong>High</strong> — ~20 Mbps. Crisp detail and smooth motion at the cost of
            larger files. Ideal for high-motion content or final delivery.
          {/if}
          {#if draftFrameRate && draftResolutionMode !== "custom"}
            {' '}At {draftFrameRate} fps{draftResolutionMode === "preset" ? ` / ${draftResolutionPreset}` : draftResolutionMode === "original" ? " / original resolution" : ""}.
          {/if}
        </p>
      {/if}

      {#if draftBitrateMode === "custom"}
        <div class="custom-bitrate-row">
          <div class="custom-res-field">
            <label class="custom-res-label" for="bitrate-mbps">Bitrate (Mbps, whole number)</label>
            <div class="custom-bitrate-input-wrap">
              <input
                id="bitrate-mbps"
                type="text"
                inputmode="numeric"
                class="text-input custom-bitrate-input"
                class:text-input--empty={draftCustomMbpsRaw && draftCustomMbps === null}
                bind:value={draftCustomMbpsRaw}
                placeholder="e.g. 12"
                autocomplete="off"
              />
              <span class="custom-bitrate-unit">Mbps</span>
            </div>
          </div>
        </div>

        {#if customBitrateErrors.length > 0}
          <div class="inline-validation">
            {#each customBitrateErrors as err}
              <p class="inline-validation__item">
                <span class="inline-validation__icon">⚠</span>
                {err}
              </p>
            {/each}
          </div>
        {:else if draftCustomMbps !== null}
          <p class="group-hint">
            Custom bitrate: <strong>{draftCustomMbps} Mbps</strong>.
            {#if draftCustomMbps < 3}
              Low quality — may show compression artefacts on fast-moving content.
            {:else if draftCustomMbps <= 12}
              Moderate quality — good for most recordings.
            {:else if draftCustomMbps <= 25}
              High quality — suitable for detail-sensitive content.
            {:else}
              Very high bitrate — expect large output files.
            {/if}
            {#if draftFrameRate && draftResolutionMode !== "custom"}
              At {draftFrameRate} fps{draftResolutionMode === "preset" ? ` / ${draftResolutionPreset}` : draftResolutionMode === "original" ? " / original resolution" : ""}.
            {/if}
          </p>
        {/if}
      {/if}

      <div class="bitrate-compat-notice">
        <span class="bitrate-compat-notice__icon">ℹ</span>
        <span class="bitrate-compat-notice__text">
          Bitrate is applied only on macOS 15+ (ScreenCaptureKit path).
          On older macOS the system default bitrate is used regardless of this setting.
        </span>
      </div>
    </div>
    </section>
    </div>
  {/if}

  {#if activeTab === "storage"}
    <div role="tabpanel" id="settings-panel-storage" aria-labelledby="settings-tab-storage" tabindex="0">
    <!-- ── Card: Storage & Startup ─────────────────────── -->
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("storage")}
          <div>
            <h2 id="card-storage" class="card__title">Storage &amp; Startup</h2>
            <p class="card__subtitle">Where files are saved and when capture begins.</p>
          </div>
        </div>
      </div>

    <div class="settings-group">
      <span class="group-label">Save Directory</span>
      <div class="input-row">
        <input
          type="text"
          class="text-input"
          class:text-input--empty={!draftSaveDirectory}
          bind:value={draftSaveDirectory}
          placeholder="/path/to/recordings"
        />
      </div>
      <p class="group-hint">Where capture files are saved on disk.</p>
    </div>

    <div class="settings-group">
      <span class="group-label">Startup</span>
      <Switch
        bind:checked={draftAutoStart}
        label="Auto-start recording on launch"
        description="Begin capturing immediately when the app opens"
      />
    </div>

    <div class="settings-group">
      <span class="group-label">Retention</span>
      <SelectMenu
        value={draftRetentionPolicy}
        onValueChange={(v) => { draftRetentionPolicy = v as RetentionPolicy; }}
        label="Delete captured data"
        options={[
          { value: "never", label: "Never" },
          { value: "days_7", label: "After 7 days" },
          { value: "days_14", label: "After 14 days" },
          { value: "days_30", label: "After 30 days" },
        ]}
      />
      <div class="row">
        <button type="button" class="btn btn--ghost btn--sm" onclick={runRetentionCleanupNow} disabled={retentionCleanupRunning}>
          {retentionCleanupRunning ? "Running…" : "Run cleanup now"}
        </button>
      </div>
      {#if retentionCleanupSummary}
        <p class="group-hint">
          Latest cleanup: {retentionCleanupSummary.deletedCaptureSegments} segment(s), {retentionCleanupSummary.deletedFrames} frame(s), {retentionCleanupSummary.deletedAudioSegments} audio segment(s).
        </p>
      {/if}
      {#if retentionCleanupError}
        <p class="group-hint group-hint--error">{retentionCleanupError}</p>
      {/if}
    </div>

    </section>
    </div>
  {/if}

  {#if activeTab === "appearance"}
    <div role="tabpanel" id="settings-panel-appearance" aria-labelledby="settings-tab-appearance" tabindex="0">
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("appearance")}
          <div>
            <h2 class="card__title">Appearance</h2>
            <p class="card__subtitle">Theme selection and timeline display behavior.</p>
          </div>
        </div>
      </div>

      <div class="settings-group">
        <span class="group-label">Theme</span>
        <ThemeModeControl bind:value={draftAppearance} />
        <p class="group-hint">Theme switches immediately when saved and is also available from every titlebar.</p>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">Timeline</span>
        <Switch
          bind:checked={draftFollowTimelineLive}
          label="Follow live recording"
          description="Keep the timeline pinned to the latest captured data while recording"
        />
      </div>
    </section>
    </div>
  {/if}

  {#if activeTab === "capture"}
    <div role="tabpanel" id="settings-panel-capture-inactivity" aria-labelledby="settings-tab-capture" tabindex="0">
    <!-- ── Card: Inactivity ─────────────────────── -->
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("inactivity")}
          <div>
            <h2 id="card-inactivity" class="card__title">Inactivity</h2>
            <p class="card__subtitle">Pause &amp; resume rules when you step away.</p>
          </div>
        </div>
      </div>

    <div class="settings-group">
      <span class="group-label">Inactivity Pause</span>
      <Switch
        bind:checked={draftPauseCaptureOnInactivity}
        label="Pause capture when idle"
        description="Automatically pause recording after the system has been idle, and resume when system activity is detected"
      />
      {#if draftPauseCaptureOnInactivity}
        <div class="idle-timeout-row">
          <Slider
            bind:value={draftIdleTimeoutSeconds}
            min={5}
            max={300}
            step={5}
            label="Idle timeout"
            unit="s"
            formatValue={(v) => v >= 60 ? `${Math.floor(v/60)}m ${v%60 > 0 ? ` ${v%60}s` : ""}`.trim() : `${v}s`}
          />
        </div>
        <p class="group-hint">
          Capture pauses after <strong>{draftIdleTimeoutSeconds}s</strong> of system-wide inactivity (no mouse, keyboard,
          or other input anywhere on the Mac). It resumes automatically when system activity is detected again.
        </p>

        <div class="settings-divider"></div>

        <span class="group-label">Activity Mode</span>
        <RadioGroup
          bind:value={draftActivityMode}
          options={[
            {
              value: "system_input_only",
              label: "Input only",
              description: "Only keyboard and mouse/pointer events count as activity. Recording pauses whenever direct input stops, even during video calls or media playback.",
            },
            {
              value: "system_input_or_screen",
              label: "Input or screen change",
              description: "Keyboard/mouse input AND visible on-screen changes (video calls, animations, media) both count as activity. Helps keep recordings running during calls or video playback with no direct input.",
            },
            {
              value: "system_input_or_screen_or_audio",
              label: "Input, screen, or audio",
              description: "All of the above, plus microphone and system audio levels. Sound picked up by the microphone or played through the system keeps capture active — useful for meetings, voice sessions, or any audio-driven workflow.",
            },
          ]}
        />
        <p class="group-hint">
          {#if draftActivityMode === "system_input_or_screen_or_audio"}
            <strong>Audio mode</strong> monitors keyboard/mouse, on-screen changes, <em>and</em>
            source-specific audio activity. Microphone activity is speech-first when voice detection
            is enabled, while system audio still uses the configured level threshold.
          {:else if draftActivityMode === "system_input_or_screen"}
            <strong>Screen change mode</strong> monitors on-screen activity in addition to input events — useful for
            keeping recordings active during video calls, live streams, or media playback where you may not be
            typing or moving the mouse.
          {:else}
            <strong>Input-only mode</strong> triggers the idle timeout strictly on keyboard and mouse inactivity.
            Suitable for general screen recording when you want pauses to match direct interaction gaps exactly.
          {/if}
        </p>

        {#if draftActivityMode === "system_input_or_screen_or_audio"}
          <div class="settings-divider"></div>
          <span class="group-label">Microphone Voice Detection</span>
          <RadioGroup
            bind:value={draftMicrophoneVadAdapter}
            options={[
              {
                value: "silero",
                label: "Silero",
                description: "Default speech detector. Falls back to WebRTC when unavailable.",
              },
              {
                value: "webrtc",
                label: "WebRTC",
                description: "Lightweight local speech detector.",
              },
              {
                value: "off",
                label: "Off",
                description: "Use legacy microphone peak-level activity.",
              },
            ]}
            disabled={!draftCaptureMicrophone}
          />
          {#if !draftCaptureMicrophone}
            <p class="group-hint group-hint--warn">Microphone capture is disabled — voice detection has no effect until enabled.</p>
          {:else if draftMicrophoneVadAdapter === "off"}
            <p class="group-hint">Microphone inactivity uses the legacy peak-level detector.</p>
          {:else}
            <p class="group-hint">Microphone inactivity uses local speech detection. Raw peak levels remain visible in debug output.</p>
          {/if}

          <div class="settings-divider"></div>
          <span class="group-label">Microphone Activity Sensitivity</span>
          <Slider
            bind:value={draftMicrophoneActivitySensitivity}
            min={0}
            max={100}
            step={1}
            label="Mic sensitivity"
            unit="%"
            disabled={!draftCaptureMicrophone}
          />
          {#if !draftCaptureMicrophone}
            <p class="group-hint group-hint--warn">Microphone capture is disabled — this setting has no effect until enabled.</p>
          {:else}
            <p class="group-hint">
              {#if draftMicrophoneVadAdapter !== "off"}
                Tunes the compatibility peak-level fallback used when no speech adapter is available.
              {:else if draftMicrophoneActivitySensitivity >= 80}
                <strong>Very high</strong> — whispers and background noise keep capture active.
              {:else if draftMicrophoneActivitySensitivity >= 60}
                <strong>High</strong> — quiet speech counts as activity.
              {:else if draftMicrophoneActivitySensitivity >= 40}
                <strong>Medium</strong> — normal speech triggers activity. Recommended.
              {:else if draftMicrophoneActivitySensitivity >= 20}
                <strong>Low</strong> — only louder audio keeps capture active.
              {:else}
                <strong>Very low</strong> — only very loud audio triggers activity.
              {/if}
            </p>
          {/if}

          <div class="settings-divider"></div>
          <span class="group-label">System Audio Activity Sensitivity</span>
          <Slider
            bind:value={draftSystemAudioActivitySensitivity}
            min={0}
            max={100}
            step={1}
            label="System audio sensitivity"
            unit="%"
            disabled={!draftCaptureSystemAudio}
          />
          {#if !draftCaptureSystemAudio}
            <p class="group-hint group-hint--warn">System audio capture is disabled — this setting has no effect until enabled.</p>
          {:else}
            <p class="group-hint">
              {#if draftSystemAudioActivitySensitivity >= 80}
                <strong>Very high</strong> — quiet system sounds keep capture active.
              {:else if draftSystemAudioActivitySensitivity >= 60}
                <strong>High</strong> — moderate system audio counts as activity.
              {:else if draftSystemAudioActivitySensitivity >= 40}
                <strong>Medium</strong> — typical media playback triggers activity. Recommended.
              {:else if draftSystemAudioActivitySensitivity >= 20}
                <strong>Low</strong> — only louder system audio keeps capture active.
              {:else}
                <strong>Very low</strong> — only very loud system audio triggers activity.
              {/if}
            </p>
          {/if}

          <div class="audio-activity-notice">
            <span class="audio-activity-notice__icon">♪</span>
            <span class="audio-activity-notice__text">
              {#if !draftCaptureMicrophone && !draftCaptureSystemAudio}
                Neither microphone nor system audio capture is enabled — audio activity detection
                will not function. Enable at least one source in <strong>Capture Sources</strong> above.
              {:else if !draftCaptureMicrophone}
                Microphone capture is disabled — only system audio is monitored for activity.
              {:else if !draftCaptureSystemAudio}
                System audio capture is disabled — only microphone audio is monitored for activity.
              {:else}
                Both microphone and system audio are monitored independently for activity.
              {/if}
            </span>
          </div>
        {/if}
      {/if}
    </div>
    </section>

    </div>
  {/if}

  {#if activeTab === "processing"}
    <div role="tabpanel" id="settings-panel-processing" aria-labelledby="settings-tab-processing" tabindex="0">
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("processing")}
          <div>
            <h2 class="card__title">OCR &amp; Previews</h2>
            <p class="card__subtitle">Choose the OCR engine, inspect model availability, and tune preview caching.</p>
          </div>
        </div>
        <button class="btn btn--ghost btn--sm" onclick={loadOcrModelStatus} disabled={loadingOcrModelStatus}>
          {loadingOcrModelStatus ? "Checking" : "Refresh"}
        </button>
      </div>

    <div class="settings-group">
      <span class="group-label">OCR engine</span>
      <Switch
        bind:checked={draftOcrEnabled}
        label="Enable OCR"
        description="Automatically queue OCR for captured screen frames when the selected engine is available"
      />
      <div class="settings-divider"></div>
      <RadioGroup
        value={draftOcrProvider}
        onValueChange={chooseOcrProvider}
        disabled={!draftOcrEnabled}
        label="Provider"
        options={ocrProviderOptions.length > 0 ? ocrProviderOptions : [
          { value: "apple_vision", label: "Apple Vision", description: "Model status is loading" },
          { value: "tesseract", label: "Tesseract", description: "Model status is loading" },
        ]}
      />
      <div class="settings-divider"></div>
      <SelectMenu
        value={draftOcrModelId ?? "__os_managed__"}
        onValueChange={chooseOcrModel}
        disabled={!draftOcrEnabled}
        label="Model"
        options={ocrModelOptions.length > 0 ? ocrModelOptions : [
          { value: draftOcrModelId ?? "__os_managed__", label: "Loading model options" },
        ]}
      />
      {#if draftOcrProvider === "tesseract"}
        <label class="field-label" for="ocr-language">Language</label>
        <input
          id="ocr-language"
          class="text-input"
          bind:value={draftOcrLanguage}
          disabled={!draftOcrEnabled}
          placeholder="eng"
        />
      {/if}
      {#if draftOcrProvider === "apple_vision"}
        <div class="settings-divider"></div>
        <RadioGroup
          bind:value={draftOcrRecognitionMode}
          disabled={!draftOcrEnabled}
          label="Recognition mode"
          options={[
            { value: "fast", label: "Fast", description: "Lower CPU usage; default for continuous capture." },
            { value: "accurate", label: "Accurate", description: "Higher OCR cost with better Apple Vision accuracy." },
          ]}
        />
        <div class="settings-divider"></div>
        <Switch
          bind:checked={draftOcrLanguageCorrection}
          disabled={!draftOcrEnabled}
          label="Language correction"
          description="Let Apple Vision spend extra work correcting recognized text using language models"
        />
      {:else if draftOcrProvider === "tesseract"}
        <div class="settings-divider"></div>
        <SelectMenu
          value={draftOcrTesseractPageSegmentationMode}
          onValueChange={(value) => { draftOcrTesseractPageSegmentationMode = value as OcrTesseractPageSegmentationMode; }}
          disabled={!draftOcrEnabled}
          label="Page segmentation"
          options={[
            { value: "auto", label: "Auto" },
            { value: "single_block", label: "Single block" },
            { value: "single_line", label: "Single line" },
            { value: "single_word", label: "Single word" },
            { value: "sparse_text", label: "Sparse text" },
          ]}
        />
        <p class="group-hint">Use Auto for mixed layouts, Single block for paragraph regions, Single line for titles/labels, Single word for isolated tokens, and Sparse text for screenshots with scattered text.</p>
        <div class="settings-divider"></div>
        <SelectMenu
          value={draftOcrTesseractPreprocessMode}
          onValueChange={(value) => { draftOcrTesseractPreprocessMode = value as OcrTesseractPreprocessMode; }}
          disabled={!draftOcrEnabled}
          label="Image preprocessing"
          options={[
            { value: "grayscale", label: "Grayscale" },
            { value: "thresholded", label: "Thresholded" },
          ]}
        />
        <p class="group-hint">Grayscale usually works best for clean UI text. Thresholded black/white can help when edges are muddy or contrast is weak.</p>
        <div class="settings-divider"></div>
        <SelectMenu
          value={String(draftOcrTesseractUpscaleFactor)}
          onValueChange={(value) => { draftOcrTesseractUpscaleFactor = parseInt(value, 10) || 1; }}
          disabled={!draftOcrEnabled}
          label="Upscale before OCR"
          options={[
            { value: "1", label: "1×" },
            { value: "2", label: "2×" },
            { value: "3", label: "3×" },
            { value: "4", label: "4×" },
          ]}
        />
        <p class="group-hint">Tesseract works best around 300 DPI. For tiny screenshots, 2× upscaling is a good first step before trying 3× or 4×.</p>
        <div class="settings-divider"></div>
        <label class="field-label" for="ocr-tesseract-whitelist">Character whitelist</label>
        <input
          id="ocr-tesseract-whitelist"
          class="text-input"
          bind:value={draftOcrTesseractCharWhitelist}
          disabled={!draftOcrEnabled}
          placeholder="Optional, e.g. ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"
        />
        <p class="group-hint">
          Tesseract works best with dark, high-contrast text on a light background. For tiny screenshots, try 2× upscaling first; if edges are muddy, try thresholded preprocessing or a narrow whitelist.
        </p>
      {/if}
      <p class="group-hint">
        {draftOcrEnabled
          ? "If screen capture is enabled, recording start is blocked until the selected OCR provider is available."
          : "Screen recording can start without OCR while this is disabled."}
        Existing OCR results remain visible after switching engines.
      </p>
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Selected model status</span>
      {#if ocrModelError}
        <p class="group-hint group-hint--warn">Failed to load OCR model status: {ocrModelError}</p>
      {:else if selectedOcrModel}
        <div class="model-status" class:model-status--available={selectedOcrModel.available}>
          <div>
            <div class="model-status__title">{selectedOcrModel.displayName}</div>
            <div class="model-status__meta">{ocrStatusLabel(selectedOcrModel)}</div>
          </div>
          <span class="model-status__pill">{selectedOcrModel.available ? "available" : "unavailable"}</span>
        </div>
        <p class="group-hint">{selectedOcrModel.description}</p>
        {#if selectedOcrModel.runtimeMessage}
          <p class="group-hint group-hint--warn"><strong>Runtime:</strong> {selectedOcrModel.runtimeMessage}</p>
        {/if}
        {#if selectedOcrModel.installPath}
          <p class="group-hint"><strong>Install path:</strong> {selectedOcrModel.installPath}</p>
        {/if}
        {#if selectedOcrModel.missingFiles.length > 0}
          <p class="group-hint group-hint--warn"><strong>Missing files:</strong> {selectedOcrModel.missingFiles.join(", ")}</p>
        {/if}
        {#if selectedOcrModel.failureMessage}
          <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedOcrModel.failureMessage}</p>
        {/if}
        {#if selectedOcrModel.licenseLabel || selectedOcrModel.sourceUrl}
          <p class="group-hint">
            {#if selectedOcrModel.licenseLabel}<strong>License:</strong> {selectedOcrModel.licenseLabel}. {/if}
            {#if selectedOcrModel.sourceUrl}<strong>Source:</strong> {selectedOcrModel.sourceUrl}{/if}
          </p>
        {/if}
        {#if selectedOcrModel.management === "app_managed"}
          {#if selectedOcrModel.download}
            {#if selectedOcrDownloadRunning}
              <div class="download-progress" aria-live="polite">
                <div class="download-progress__bar">
                  <span style={`width: ${selectedOcrDownloadPercent ?? 8}%`}></span>
                </div>
                <p class="group-hint">
                  {selectedOcrDownloadProgress?.status ?? "downloading"}
                  {#if selectedOcrDownloadPercent !== null} · {selectedOcrDownloadPercent}%{/if}
                  {#if selectedOcrDownloadProgress?.message} · {selectedOcrDownloadProgress.message}{/if}
                </p>
                <button class="btn btn--ghost" onclick={cancelSelectedOcrModelDownload} disabled={cancellingOcrDownload}>
                  {cancellingOcrDownload ? "Cancelling" : "Cancel download"}
                </button>
              </div>
            {:else}
              <button class="btn btn--ghost" onclick={startSelectedOcrModelDownload} disabled={startingOcrDownload || selectedOcrModel.available}>
                {startingOcrDownload ? "Starting" : `Download (${formatBytes(selectedOcrModel.download.byteSize)})`}
              </button>
            {/if}
          {:else if !selectedOcrModel.available}
            <p class="group-hint group-hint--warn">
              {#if selectedOcrModel.provider === "tesseract"}
                This provider still needs a Mnema-published self-contained runtime bundle before in-app download can work.
              {:else}
                This app-managed OCR bundle is missing, and the current manifest does not ship a downloadable artifact yet.
              {/if}
            </p>
          {/if}
          {#if ocrDownloadError}
            <p class="group-hint group-hint--warn">Download failed: {ocrDownloadError}</p>
          {/if}
        {:else}
          <p class="group-hint">This provider is managed by macOS. There is no app-managed model download.</p>
        {/if}
        <div class="debug-log-actions">
          <button class="btn btn--danger" onclick={requestDeleteUnusedOcrModels} disabled={deletingUnusedOcrModels || selectedOcrDownloadRunning}>
            Delete unused OCR models
          </button>
        </div>
        <p class="group-hint">Removes app-managed OCR model files except the model selected above.</p>
        {#if confirmingDeleteUnusedOcrModels}
          <div class="delete-confirmation" role="alert">
            <strong>Delete unused OCR models?</strong>
            <p>This removes app-managed OCR model directories that are not currently selected. The selected model, active downloads, and running OCR jobs are kept. Queued and failed OCR jobs using deleted models are moved to the current OCR selection.</p>
            <div class="debug-log-actions">
              <button class="btn btn--danger" onclick={deleteUnusedOcrModels} disabled={deletingUnusedOcrModels}>
                {deletingUnusedOcrModels ? "Deleting" : "Confirm delete"}
              </button>
              <button class="btn btn--ghost" onclick={() => { confirmingDeleteUnusedOcrModels = false; }} disabled={deletingUnusedOcrModels}>
                Cancel
              </button>
            </div>
          </div>
        {/if}
        {#if deleteUnusedOcrModelsMessage}
          <div class="cleanup-result" aria-live="polite">
            <strong>{deleteUnusedOcrModelsMessage}</strong>
            {#if deletedUnusedOcrModelLabels.length > 0}
              <p>Deleted:</p>
              <ul>
                {#each deletedUnusedOcrModelLabels as model}
                  <li>{model}</li>
                {/each}
              </ul>
            {/if}
            {#if skippedUnusedOcrModelLabels.length > 0}
              <p>Skipped active downloads:</p>
              <ul>
                {#each skippedUnusedOcrModelLabels as model}
                  <li>{model}</li>
                {/each}
              </ul>
            {/if}
            {#if skippedOcrProcessingJobModelLabels.length > 0}
              <p>Skipped running jobs:</p>
              <ul>
                {#each skippedOcrProcessingJobModelLabels as model}
                  <li>{model}</li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}
        {#if deleteUnusedOcrModelsError}
          <p class="group-hint group-hint--warn">Delete failed: {deleteUnusedOcrModelsError}</p>
        {/if}
      {:else if loadingOcrModelStatus}
        <p class="group-hint">Checking installed OCR models…</p>
      {:else}
        <p class="group-hint group-hint--warn">No OCR model status is available for the selected provider.</p>
      {/if}
    </div>

    <div class="settings-divider"></div>

    <div class="settings-group">
      <span class="group-label">Preview Cache</span>
      <SelectMenu
        value={String(draftPreviewCacheTtlSeconds)}
        onValueChange={(v) => { draftPreviewCacheTtlSeconds = parseInt(v, 10); }}
        label="Cache duration"
        options={[
          { value: "0",     label: "Disabled" },
          { value: "300",   label: "5 minutes" },
          { value: "900",   label: "15 minutes" },
          { value: "3600",  label: "1 hour (default)" },
          { value: "21600", label: "6 hours" },
          { value: "86400", label: "24 hours" },
        ]}
      />
      <p class="group-hint">
        In-memory cache for frame and image previews. Cached entries expire automatically after the selected duration.
        {#if draftPreviewCacheTtlSeconds === 0}
          <strong>Disabled</strong> — previews are fetched fresh every time.
        {/if}
      </p>
    </div>
    </section>

    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("transcription")}
          <div>
            <h2 class="card__title">Transcription</h2>
            <p class="card__subtitle">Local speech-to-text provider and model setup for microphone audio.</p>
          </div>
        </div>
        <button class="btn btn--ghost btn--sm" onclick={loadTranscriptionModelStatus} disabled={loadingTranscriptionModelStatus}>
          {loadingTranscriptionModelStatus ? "Checking" : "Refresh"}
        </button>
      </div>

      <div class="settings-group">
        <span class="group-label">Engine</span>
        <Switch
          bind:checked={draftTranscriptionEnabled}
          label="Enable audio transcription"
          description="Master switch for source-specific audio transcription"
        />
        <Switch
          bind:checked={draftTranscriptionMicrophoneEnabled}
          label="Transcribe microphone"
          description="Automatically queue transcription for committed microphone audio segments"
          disabled={!draftTranscriptionEnabled}
        />
        <Switch
          bind:checked={draftTranscriptionSystemAudioEnabled}
          label="Transcribe system audio"
          description="Transcribe system audio only when speech is detected."
          disabled={!draftTranscriptionEnabled}
        />
        <div class="settings-divider"></div>
        <RadioGroup
          value={draftTranscriptionProvider}
          onValueChange={chooseTranscriptionProvider}
          label="Provider"
          options={transcriptionProviderOptions.length > 0 ? transcriptionProviderOptions : [
            { value: "local_whisper", label: "Local Whisper", description: "Model status is loading" },
            { value: "apple_speech_on_device", label: "Apple Speech (on-device)", description: "Model status is loading" },
            { value: "parakeet", label: "Parakeet", description: "Model status is loading" },
          ]}
        />
        <div class="settings-divider"></div>
        <SelectMenu
          value={draftTranscriptionModelId ?? "__os_managed__"}
          onValueChange={chooseTranscriptionModel}
          label="Model"
          options={transcriptionModelOptions.length > 0 ? transcriptionModelOptions : [
            { value: draftTranscriptionModelId ?? "__os_managed__", label: "Loading model options" },
          ]}
        />
        <label class="field-label" for="transcription-language">Language</label>
        <input
          id="transcription-language"
          class="text-input"
          bind:value={draftTranscriptionLanguage}
          placeholder="auto"
        />
        {#if draftTranscriptionProvider === "parakeet"}
          <div class="settings-divider"></div>
          <RadioGroup
            value={draftTranscriptionMemoryMode}
            onValueChange={(value) => draftTranscriptionMemoryMode = value as AudioTranscriptionMemoryMode}
            label="Parakeet memory mode"
            options={[
              { value: "balanced", label: "Balanced", description: "Unload ONNX sessions after idle timeout" },
              { value: "low_memory", label: "Low memory", description: "Unload ONNX sessions after every transcription" },
              { value: "performance", label: "Performance", description: "Keep ONNX sessions loaded for fastest repeat jobs" },
            ]}
          />
          {#if draftTranscriptionMemoryMode === "balanced"}
            <label class="field-label" for="transcription-idle-unload">Idle unload seconds</label>
            <input
              id="transcription-idle-unload"
              class="text-input"
              type="number"
              min="0"
              max="86400"
              step="1"
              bind:value={draftTranscriptionIdleUnloadSeconds}
            />
          {/if}
          <label class="field-label" for="transcription-chunk-seconds">Chunk seconds</label>
          <input
            id="transcription-chunk-seconds"
            class="text-input"
            type="number"
            min="0"
            max="3600"
            step="1"
            bind:value={draftTranscriptionChunkSeconds}
          />
          <p class="group-hint">
            Choose the int8 Parakeet model for lower disk and runtime weight memory. Chunking limits peak activation memory; set chunk seconds to 0 to disable chunking.
          </p>
        {/if}
        <p class="group-hint">
          Use <strong>auto</strong> for automatic language detection, or enter a language hint such as <strong>en</strong>.
          Settings changes affect future audio segments; already-queued jobs keep their admitted provider/model payload.
        </p>
      </div>

      <div class="settings-group">
        <span class="group-label">Selected model status</span>
        {#if transcriptionModelError}
          <p class="group-hint group-hint--warn">Failed to load model status: {transcriptionModelError}</p>
        {:else if selectedTranscriptionModel}
          <div class="model-status" class:model-status--available={selectedTranscriptionModel.available}>
            <div>
              <div class="model-status__title">{selectedTranscriptionModel.displayName}</div>
              <div class="model-status__meta">{transcriptionStatusLabel(selectedTranscriptionModel)}</div>
            </div>
            <span class="model-status__pill">{selectedTranscriptionModel.available ? "available" : "unavailable"}</span>
          </div>
          <p class="group-hint">{selectedTranscriptionModel.description}</p>
          {#if selectedAppleSpeechPermissionStatus}
            <div class="permission-callout" class:permission-callout--ok={selectedAppleSpeechPermissionStatus === "available"}>
              <div class="permission-callout__copy">
                <span class="permission-callout__eyebrow">Apple Speech status</span>
                <strong>{appleSpeechPermissionLabel(selectedAppleSpeechPermissionStatus)}</strong>
                <p>{appleSpeechPermissionHint(selectedAppleSpeechPermissionStatus)}</p>
              </div>
              {#if selectedAppleSpeechNeedsPermission}
                {#if selectedAppleSpeechPermissionStatus === "permission_not_determined"}
                  <button
                    class="btn btn--ghost"
                    onclick={requestAppleSpeechPermission}
                    disabled={requestingAppleSpeechPermission}
                  >
                    {requestingAppleSpeechPermission ? "Requesting" : "Get permission"}
                  </button>
                {:else}
                  <button class="btn btn--ghost" onclick={openAppleSpeechPrivacySettings}>
                    Open System Settings
                  </button>
                {/if}
              {/if}
            </div>
            {#if appleSpeechPermissionError}
              <p class="group-hint group-hint--warn">Permission request failed: {appleSpeechPermissionError}</p>
            {/if}
          {/if}
          {#if selectedTranscriptionModel.installPath}
            <p class="group-hint"><strong>Install path:</strong> {selectedTranscriptionModel.installPath}</p>
          {/if}
          {#if selectedTranscriptionModel.missingFiles.length > 0}
            <p class="group-hint group-hint--warn"><strong>Missing files:</strong> {selectedTranscriptionModel.missingFiles.join(", ")}</p>
          {/if}
          {#if selectedTranscriptionModel.failureMessage}
            <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedTranscriptionModel.failureMessage}</p>
          {/if}
          {#if selectedTranscriptionModel.licenseLabel || selectedTranscriptionModel.sourceUrl}
            <p class="group-hint">
              {#if selectedTranscriptionModel.licenseLabel}<strong>License:</strong> {selectedTranscriptionModel.licenseLabel}. {/if}
              {#if selectedTranscriptionModel.sourceUrl}<strong>Source:</strong> {selectedTranscriptionModel.sourceUrl}{/if}
            </p>
          {/if}
          {#if selectedTranscriptionModel.management === "app_managed"}
            {#if selectedTranscriptionModel.download}
              {#if selectedTranscriptionDownloadRunning}
                <div class="download-progress" aria-live="polite">
                  <div class="download-progress__bar">
                    <span style={`width: ${selectedTranscriptionDownloadPercent ?? 8}%`}></span>
                  </div>
                  <p class="group-hint">
                    {selectedTranscriptionDownloadProgress?.status ?? "downloading"}
                    {#if selectedTranscriptionDownloadPercent !== null} · {selectedTranscriptionDownloadPercent}%{/if}
                    {#if selectedTranscriptionDownloadProgress?.message} · {selectedTranscriptionDownloadProgress.message}{/if}
                  </p>
                  <button class="btn btn--ghost" onclick={cancelSelectedTranscriptionModelDownload} disabled={cancellingTranscriptionDownload}>
                    {cancellingTranscriptionDownload ? "Cancelling" : "Cancel download"}
                  </button>
                </div>
              {:else}
                <button class="btn btn--ghost" onclick={startSelectedTranscriptionModelDownload} disabled={startingTranscriptionDownload || selectedTranscriptionModel.available}>
                  {startingTranscriptionDownload ? "Starting" : `Download (${formatBytes(selectedTranscriptionModel.download.byteSize)})`}
                </button>
              {/if}
              <p class="group-hint">Download support validates sha256 before marking this model installed.</p>
            {:else if !selectedTranscriptionModel.available}
              <p class="group-hint group-hint--warn">
                This app-managed model is missing, but no packaged download artifact is available in the current manifest.
              </p>
            {/if}
            {#if transcriptionDownloadError}
              <p class="group-hint group-hint--warn">Download failed: {transcriptionDownloadError}</p>
            {/if}
          {:else}
            <p class="group-hint">This provider is managed by macOS. There is no app-managed model download.</p>
          {/if}
          <div class="debug-log-actions">
            <button class="btn btn--danger" onclick={requestDeleteUnusedTranscriptionModels} disabled={deletingUnusedTranscriptionModels || selectedTranscriptionDownloadRunning}>
              Delete unused transcription models
            </button>
          </div>
          <p class="group-hint">Removes app-managed transcription model files except the model selected above.</p>
          {#if confirmingDeleteUnusedTranscriptionModels}
            <div class="delete-confirmation" role="alert">
              <strong>Delete unused transcription models?</strong>
              <p>This removes app-managed transcription model directories that are not currently selected. The selected model, active downloads, and running transcription jobs are kept. Queued and failed transcription jobs using deleted models are moved to the current transcription selection.</p>
              <div class="debug-log-actions">
                <button class="btn btn--danger" onclick={deleteUnusedTranscriptionModels} disabled={deletingUnusedTranscriptionModels}>
                  {deletingUnusedTranscriptionModels ? "Deleting" : "Confirm delete"}
                </button>
                <button class="btn btn--ghost" onclick={() => { confirmingDeleteUnusedTranscriptionModels = false; }} disabled={deletingUnusedTranscriptionModels}>
                  Cancel
                </button>
              </div>
            </div>
          {/if}
          {#if deleteUnusedTranscriptionModelsMessage}
            <div class="cleanup-result" aria-live="polite">
              <strong>{deleteUnusedTranscriptionModelsMessage}</strong>
              {#if deletedUnusedTranscriptionModelLabels.length > 0}
                <p>Deleted:</p>
                <ul>
                  {#each deletedUnusedTranscriptionModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedUnusedTranscriptionModelLabels.length > 0}
                <p>Skipped active downloads:</p>
                <ul>
                  {#each skippedUnusedTranscriptionModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedTranscriptionProcessingJobModelLabels.length > 0}
                <p>Skipped running jobs:</p>
                <ul>
                  {#each skippedTranscriptionProcessingJobModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
          {#if deleteUnusedTranscriptionModelsError}
            <p class="group-hint group-hint--warn">Delete failed: {deleteUnusedTranscriptionModelsError}</p>
          {/if}
        {:else if loadingTranscriptionModelStatus}
          <p class="group-hint">Checking installed transcription models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No model status is available for the selected provider.</p>
        {/if}
      </div>
    </section>

    <section class="card card--speaker">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("speaker")}
          <div>
            <h2 class="card__title">Speaker analysis</h2>
            <p class="card__subtitle">Anonymous diarization first; saved-person recognition only when you explicitly opt in.</p>
          </div>
        </div>
        <button class="btn btn--ghost btn--sm" onclick={loadSpeakerModelStatus} disabled={loadingSpeakerModelStatus}>
          {loadingSpeakerModelStatus ? "Checking" : "Refresh"}
        </button>
      </div>

      <div class="speaker-settings-hero">
        <div>
          <span class="group-label">Transcript speakers</span>
          <h3>Split the room before naming anyone.</h3>
          <p>Speaker separation runs locally after microphone transcription. Recognition uses only confirmed Person voice embeddings stored in this save directory.</p>
        </div>
        <div class="speaker-settings-hero__toggles">
          <Switch
            bind:checked={draftSpeakerSeparateSpeakers}
            label="Separate speakers in transcripts"
            description="Queue local diarization after successful microphone transcription"
          />
          <Switch
            bind:checked={draftSpeakerRecognizeSavedPeople}
            disabled={!draftSpeakerSeparateSpeakers}
            label="Recognize saved people"
            description="Opt in to matching against confirmed local Person voice profiles"
          />
        </div>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">Helper timeout</span>
        <Slider
          bind:value={draftSpeakerTimeoutMinutes}
          min={1}
          max={60}
          step={1}
          label="Timeout"
          unit="m"
          disabled={!draftSpeakerSeparateSpeakers}
        />
        <p class="group-hint">Stops speaker analysis if the local helper runs too long. Existing queued jobs keep the timeout they were created with.</p>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">Speaker model</span>
        {#if speakerModelError}
          <p class="group-hint group-hint--warn">Failed to load speaker model status: {speakerModelError}</p>
        {:else if selectedSpeakerModel}
          <div class="model-status" class:model-status--available={selectedSpeakerModel.available}>
            <div>
              <div class="model-status__title">{selectedSpeakerModel.displayName}</div>
              <div class="model-status__meta">{speakerStatusLabel(selectedSpeakerModel)}</div>
            </div>
            <span class="model-status__pill">{selectedSpeakerModel.available ? "available" : "unavailable"}</span>
          </div>
          <p class="group-hint">{selectedSpeakerModel.description}</p>
          {#if selectedSpeakerModel.installPath}
            <p class="group-hint"><strong>Install path:</strong> {selectedSpeakerModel.installPath}</p>
          {/if}
          {#if selectedSpeakerModel.missingFiles.length > 0}
            <p class="group-hint group-hint--warn"><strong>Missing files:</strong> {selectedSpeakerModel.missingFiles.join(", ")}</p>
          {/if}
          {#if selectedSpeakerModel.failureMessage}
            <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedSpeakerModel.failureMessage}</p>
          {/if}
          {#if selectedSpeakerModel.download}
            {#if selectedSpeakerDownloadRunning}
              <div class="download-progress" aria-live="polite">
                <div class="download-progress__bar">
                  <span style={`width: ${selectedSpeakerDownloadPercent ?? 8}%`}></span>
                </div>
                <p class="group-hint">
                  {selectedSpeakerDownloadProgress?.status ?? "downloading"}
                  {#if selectedSpeakerDownloadPercent !== null} · {selectedSpeakerDownloadPercent}%{/if}
                  {#if selectedSpeakerDownloadProgress?.message} · {selectedSpeakerDownloadProgress.message}{/if}
                </p>
                <button class="btn btn--ghost" onclick={cancelSelectedSpeakerModelDownload} disabled={cancellingSpeakerDownload}>
                  {cancellingSpeakerDownload ? "Cancelling" : "Cancel download"}
                </button>
              </div>
            {:else}
              <div class="debug-log-actions">
                <button class="btn btn--ghost" onclick={startSelectedSpeakerModelDownload} disabled={startingSpeakerDownload || selectedSpeakerModel.available}>
                  {startingSpeakerDownload ? "Starting" : "Download speaker model"}
                </button>
                <button class="btn btn--danger" onclick={deleteSelectedSpeakerModel} disabled={deletingSpeakerModel || selectedSpeakerDownloadRunning || !selectedSpeakerModel.available}>
                  {deletingSpeakerModel ? "Deleting" : "Delete speaker model"}
                </button>
              </div>
            {/if}
            <p class="group-hint">Downloads the pyannote segmentation bundle plus NeMo Titanet embedding model into app-managed storage.</p>
          {/if}
          {#if speakerDownloadError}
            <p class="group-hint group-hint--warn">Speaker model action failed: {speakerDownloadError}</p>
          {/if}
          {#if speakerModelDeleteMessage}
            <p class="group-hint">{speakerModelDeleteMessage}</p>
          {/if}
        {:else if loadingSpeakerModelStatus}
          <p class="group-hint">Checking installed speaker models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No speaker model status is available.</p>
        {/if}
      </div>
    </section>
    </div>
  {/if}

  {#if activeTab === "developer"}
    <div role="tabpanel" id="settings-panel-developer" aria-labelledby="settings-tab-developer" tabindex="0">
    <!-- ── Card: Developer & Logs ─────────────────────── -->
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          {@render settingsCardIcon("developer")}
          <div>
            <h2 class="card__title">Developer &amp; Logs</h2>
            <p class="card__subtitle">Debug surfaces, native capture diagnostics, and log files.</p>
          </div>
        </div>
      </div>

    <!-- ── Developer Options ─────────────────────────────────── -->
    <div class="settings-group">
      <span class="group-label">Developer Options</span>
      <Switch
        bind:checked={draftDeveloperOptionsEnabled}
        label="Enable developer options"
        description="Reveal the Debug surface in the navigation bar (raw session, system probe, idle policy, app-infra and background-job tools)"
      />
      <p class="group-hint">
        When disabled, the Debug page is hidden and visiting it redirects to the Timeline.
        Changes auto-save and apply immediately.
      </p>
    </div>

    <!-- ── Native Capture Debug Logging ──────────────────────── -->
    <div class="settings-group">
      <span class="group-label">Native Capture Debug Logging</span>
      <Switch
        bind:checked={draftNativeCaptureDebugLoggingEnabled}
        label="Enable debug logging"
        description="Write native capture diagnostic output to a log file on disk"
      />
      <p class="group-hint">
        When enabled, native capture internals are logged to a file for troubleshooting.
        Changes auto-save and apply immediately.
      </p>

      {#if debugLogStatus}
        <div class="debug-log-status">
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Status</span>
            <span class="debug-log-status__value">
              {#if debugLogStatus.enabled}
                <span class="debug-log-status__dot debug-log-status__dot--on"></span> Active
              {:else}
                <span class="debug-log-status__dot"></span> Inactive
              {/if}
            </span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Path</span>
            <span class="debug-log-status__path" title={debugLogStatus.path}>{debugLogStatus.path}</span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">File</span>
            <span class="debug-log-status__value">{debugLogStatus.exists ? "Exists on disk" : "Not found"}</span>
          </div>
        </div>

        {#if debugLogStatus.exists}
          <div class="debug-log-actions">
            <button
              class="btn btn--danger btn--sm"
              onclick={deleteDebugLog}
              disabled={deletingDebugLog}
            >
              {deletingDebugLog ? "Deleting…" : "Delete Log File"}
            </button>
            {#if debugLogDeleted}
              <span class="saved-badge">✓ Deleted</span>
            {/if}
          </div>
        {/if}
      {:else if loadingDebugLogStatus}
        <p class="loading-text">Loading log status…</p>
      {/if}

      {#if debugLogError}
        <div class="inline-error">
          <span class="inline-error__icon">⚠</span>
          <span class="inline-error__msg">{debugLogError}</span>
          <button class="btn btn--ghost btn--sm" onclick={() => debugLogError = null}>×</button>
        </div>
      {/if}
    </div>

    <!-- ── General Application Log ───────────────────────────── -->
    <div class="settings-group">
      <span class="group-label">General Application Log</span>
      <p class="group-hint">
        The general application log captures high-level runtime events and errors.
      </p>

      {#if generalLogStatus}
        <div class="debug-log-status">
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">Path</span>
            <span class="debug-log-status__path" title={generalLogStatus.path}>{generalLogStatus.path}</span>
          </div>
          <div class="debug-log-status__row">
            <span class="debug-log-status__label">File</span>
            <span class="debug-log-status__value">{generalLogStatus.exists ? "Exists on disk" : "Not found"}</span>
          </div>
        </div>

        <div class="debug-log-actions">
          <button
            class="btn btn--ghost btn--sm"
            onclick={openGeneralLog}
            disabled={openingGeneralLog}
          >
            {#if openingGeneralLog}
              Opening…
            {:else if generalLogStatus.exists}
              Open Log File
            {:else}
              Open Containing Folder
            {/if}
          </button>
          {#if generalLogStatus.exists}
            <button
              class="btn btn--danger btn--sm"
              onclick={deleteGeneralLog}
              disabled={deletingGeneralLog}
            >
              {deletingGeneralLog ? "Deleting…" : "Delete Log File"}
            </button>
          {/if}
          {#if generalLogDeleted}
            <span class="saved-badge">✓ Deleted</span>
          {/if}
        </div>
      {:else if loadingGeneralLogStatus}
        <p class="loading-text">Loading log status…</p>
      {/if}

      {#if generalLogError}
        <div class="inline-error">
          <span class="inline-error__icon">⚠</span>
          <span class="inline-error__msg">{generalLogError}</span>
          <button class="btn btn--ghost btn--sm" onclick={() => generalLogError = null}>×</button>
        </div>
      {/if}
    </div>
    </section>
    </div>
  {/if}

    {#if recError}
      <div class="inline-error">
        <span class="inline-error__icon">⚠</span>
        <span class="inline-error__msg">{recError}</span>
        <button class="btn btn--ghost btn--sm" onclick={() => recError = null}>×</button>
      </div>
    {/if}

    {#if keyboardBindingsError}
      <div class="inline-error">
        <span class="inline-error__icon">⚠</span>
        <span class="inline-error__msg">{keyboardBindingsError}</span>
        <button class="btn btn--ghost btn--sm" onclick={() => keyboardBindingsError = null}>×</button>
      </div>
    {/if}

    {#if recValidationErrors.length > 0}
      <div class="inline-validation">
        {#each recValidationErrors as err}
          <p class="inline-validation__item">
            <span class="inline-validation__icon">⚠</span>
            {err}
          </p>
        {/each}
      </div>
    {/if}
{/if}

<!-- ── Microphone settings ───────────────────────────────────────────────── -->
{#if activeTab === "audio"}
<div role="tabpanel" id="settings-panel-audio" aria-labelledby="settings-tab-audio" tabindex="0">
<section class="card">
  <div class="card__header">
    <div class="card__heading">
      {@render settingsCardIcon("audio")}
      <div>
        <h2 id="card-mic" class="card__title">Microphone Controller</h2>
        <p class="card__subtitle">Choose the active device and how disconnects are handled.</p>
      </div>
    </div>
    <button class="btn btn--ghost btn--sm" onclick={loadMicState} disabled={loadingMicState}>
      {loadingMicState ? "…" : "Reload"}
    </button>
  </div>

  {#if loadingMicState}
    <p class="loading-text">Loading microphone state…</p>
  {:else if micState}
    <!-- Effective device banner -->
    <div class="effective-device" class:effective-device--none={!micState.effectiveDevice}>
      <span class="effective-device__dot" class:effective-device__dot--on={!!micState.effectiveDevice}></span>
      <span class="effective-device__label">
        {#if micState.effectiveDevice}
          {micState.effectiveDevice.name}
          {#if micState.effectiveDevice.isDefault}
            <span class="badge badge--neutral badge--sm">default</span>
          {/if}
        {:else}
          No active device
        {/if}
      </span>
    </div>

    <!-- Available devices -->
    {#if micState.devices.length > 0}
      <div class="settings-group">
        <span class="group-label">Available Devices</span>
        <ul class="device-list">
          {#each micState.devices as device (device.id)}
            <li class="device-item" class:device-item--active={micState.effectiveDevice?.id === device.id}>
              <span class="device-item__dot" class:device-item__dot--active={micState.effectiveDevice?.id === device.id}></span>
              <span class="device-item__name">{device.name}</span>
              <div class="device-item__badges">
                {#if device.isDefault}
                  <span class="badge badge--neutral badge--sm">default</span>
                {/if}
                {#if micState.effectiveDevice?.id === device.id}
                  <span class="badge badge--ok badge--sm">active</span>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {:else}
      <p class="empty-state">No microphone devices found.</p>
    {/if}

    <div class="settings-divider"></div>

    <div class="settings-group">
      <RadioGroup
        bind:value={draftPreferenceMode}
        label="Preference"
        options={[
          { value: "default", label: "System Default", description: "Use the currently selected system microphone" },
          { value: "specific_device", label: "Specific Device", description: "Lock to a particular microphone" },
        ]}
      />
    </div>

    {#if draftPreferenceMode === "specific_device"}
      <div class="settings-group">
        <SelectMenu
          bind:value={draftDeviceId}
          label="Device"
          options={micDeviceOptions}
          placeholder="— pick a device —"
          warn={!draftDeviceId}
        />
        {#if !draftDeviceId}
          <p class="group-hint group-hint--warn">Select a device before saving Specific Device mode.</p>
        {/if}
      </div>
    {/if}

    <div class="settings-divider"></div>

    <div class="settings-group">
      <RadioGroup
        bind:value={draftDisconnectPolicy}
        label="On Disconnect"
        options={[
          { value: "fallback_to_default", label: "Fallback to Default", description: "Switch to system default when device disconnects" },
          { value: "wait_for_same_device", label: "Wait for Same Device", description: "Pause microphone capture until the device reconnects" },
        ]}
      />
    </div>

    {#if micError}
      <div class="inline-error">
        <span class="inline-error__icon">⚠</span>
        <span class="inline-error__msg">{micError}</span>
        <button class="btn btn--ghost btn--sm" onclick={() => micError = null}>×</button>
      </div>
    {/if}
  {:else}
    <p class="empty-state">Failed to load microphone state.</p>
    <button class="btn btn--ghost btn--sm" onclick={loadMicState}>Retry</button>
  {/if}
</section>
</div>
{/if}

</div><!-- /.settings-scroll -->

<style>
  /* ── Scroll region ────────────────────────────────────────────────────
     Wrapping all tab panels in a single flex child lets the page header
     and tab strip stay pinned while only this region scrolls. `flex: 1`
     claims the leftover viewport height inside `.app-content`, and
     `min-height: 0` is required for the child to shrink below its
     intrinsic content height in a flex column (otherwise the whole
     dedicated window would scroll instead). The negative-margin /
     positive-padding pair widens the scroll viewport to the page's
     full reading-column width so the scrollbar sits flush with the
     window edge while panel content keeps its 24px gutter. */
  .settings-scroll {
    flex: 1 1 0;
    min-height: 0;
    overflow-y: auto;
    /* Re-establish the vertical rhythm previously provided by
       `.app-content`'s flex `gap: 14px` so adjacent tab panels and the
       wrapper itself keep matching spacing on tab switches. */
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* ── Tab nav ──────────────────────────────────────────────────────────
     Compact horizontal strip; each tab shows index + label + tiny hint.
     The active tab gets the accent treatment so the user always knows
     which category is being edited. The wrapper carries a dashed
     separator on its bottom edge that matches `.page-header`'s divider
     so the pinned head of the dedicated window reads as a single
     stationary block above the scrolling panel area. */
  .tab-nav {
    margin: 0;
    padding-bottom: 12px;
    border-bottom: 1px dashed var(--app-border);
  }

  .tab-nav__list {
    display: flex;
    flex-wrap: nowrap;
    gap: 4px;
    padding: 6px;
    overflow-x: auto;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 6px;
  }

  .tab-nav__tab {
    flex: 1 1 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 14px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    text-align: center;
    white-space: nowrap;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
    color: var(--app-text-muted);
    min-width: 0;
  }

  .tab-nav__tab:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border);
    color: var(--app-text);
  }

  .tab-nav__tab:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .tab-nav__tab--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .tab-nav__tab--active:hover {
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
  }

  .tab-nav__label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--app-text-strong);
    line-height: 1.2;
  }

  .tab-nav__tab--active .tab-nav__label {
    color: var(--app-accent);
  }

  :global([data-theme="light"]) .tab-nav {
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .tab-nav__list {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .tab-nav__tab {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .tab-nav__tab:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .tab-nav__tab--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .tab-nav__label {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .tab-nav__tab--active .tab-nav__label {
    color: var(--app-accent-strong);
  }

  /* ── Page header ───────────────────────────────────────────── */
  .page-header {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 6px;
    padding-bottom: 12px;
    border-bottom: 1px dashed var(--app-border);
  }

  .page-header__head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
  }

  .page-header__title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--app-text-strong);
    line-height: 1.1;
  }

  .page-header__status {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }

  .page-header__status-text {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    position: relative;
    padding-left: 12px;
    white-space: nowrap;
  }

  .page-header__status-text::before {
    content: "";
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent-strong);
  }

  .page-header__status-text--blocked {
    color: var(--app-warn);
  }
  .page-header__status-text--blocked::before {
    background: var(--app-warn-strong);
  }

  .page-header__status-text--ok {
    color: var(--app-accent);
  }
  .page-header__status-text--ok::before {
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }

  .page-header__status-text--saving {
    color: var(--app-accent-strong);
  }
  .page-header__status-text--saving::before {
    background: var(--app-accent);
    animation: status-pulse 1.1s ease-in-out infinite;
  }

  .page-header__status-text--error {
    color: var(--app-danger);
  }
  .page-header__status-text--error::before {
    background: var(--app-danger);
    box-shadow: 0 0 6px var(--app-danger);
  }

  @keyframes status-pulse {
    0%, 100% { opacity: 0.35; transform: translateY(-50%) scale(0.85); }
    50% { opacity: 1; transform: translateY(-50%) scale(1.1); }
  }

  /* ── Status strip ─────────────────────────────────────────── */
  .status-strip {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0;
  }

  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
  }

  .status-pill__dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-border-strong);
    transition: background 0.15s;
  }

  .status-pill--on {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  .status-pill--on .status-pill__dot {
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }

  .status-pill--info {
    border-color: var(--app-border);
    background: var(--app-surface-raised);
  }

  .status-pill__label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .status-pill--on .status-pill__label {
    color: var(--app-accent-strong);
  }

  /* ── Card ──────────────────────────────────────────────────── */
  .card {
    position: relative;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 8px;
    padding: 22px 22px 18px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    overflow: hidden;
  }

  .card::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 1px;
    background: linear-gradient(90deg, transparent, var(--app-accent-strong) 20%, var(--app-accent) 50%, var(--app-accent-strong) 80%, transparent);
    opacity: 0.4;
  }

  .card--speaker::before {
    height: 2px;
    background: linear-gradient(90deg, transparent, #f59e0b 18%, var(--app-accent) 48%, #22d3ee 78%, transparent);
    opacity: 0.62;
  }

  .card__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .card__heading {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    min-width: 0;
  }

  .card-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 22px;
    padding: 0 6px;
    border: 1px solid var(--app-accent-border);
    border-radius: 3px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
    flex: 0 0 auto;
    margin-top: 1px;
  }

  .card-icon svg {
    width: 13px;
    height: 13px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .card:hover .card-icon {
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .card--combobox-open {
    overflow: visible;
    z-index: 10;
  }

  .card--speaker .card-icon {
    border-color: var(--app-accent-border);
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }

  .card__title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--app-text-strong);
    line-height: 1.2;
    text-transform: none;
  }

  .card__subtitle {
    font-size: 10px;
    color: var(--app-text-muted);
    letter-spacing: 0.02em;
    line-height: 1.5;
    margin-top: 3px;
  }

  /* ── Settings groups ──────────────────────────────────────── */
  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .settings-group:focus {
    outline: none;
  }

  .settings-group--attention .settings-stack {
    border-color: color-mix(in srgb, var(--app-accent) 45%, var(--app-border));
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--app-accent) 18%, transparent);
  }

  .group-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .settings-stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 14px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 4px;
  }

  .settings-divider {
    height: 1px;
    background: var(--app-border);
  }

  .speaker-settings-hero {
    display: grid;
    grid-template-columns: minmax(0, 0.9fr) minmax(18rem, 1.1fr);
    gap: 16px;
    align-items: start;
    padding: 16px;
    border: 1px solid color-mix(in srgb, var(--app-accent) 20%, var(--app-border));
    border-radius: 10px;
    background:
      radial-gradient(circle at 18% 20%, color-mix(in srgb, #f59e0b 12%, transparent), transparent 38%),
      radial-gradient(circle at 84% 8%, color-mix(in srgb, var(--app-accent) 14%, transparent), transparent 42%),
      color-mix(in srgb, var(--app-surface) 88%, transparent);
  }

  .speaker-settings-hero h3 {
    margin: 6px 0 6px;
    color: var(--app-text-strong);
    font-size: 17px;
    line-height: 1.15;
  }

  .speaker-settings-hero p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.55;
  }

  .speaker-settings-hero__toggles {
    display: grid;
    gap: 10px;
  }

  .group-hint {
    font-size: 10px;
    color: var(--app-text-faint);
    letter-spacing: 0.03em;
    line-height: 1.5;
  }

  .group-hint--warn {
    color: var(--app-warn);
    font-weight: 600;
  }

  .field-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .model-status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-warn) 8%, transparent);
  }

  .model-status--available {
    border-color: color-mix(in srgb, var(--app-accent) 42%, var(--app-border));
    background: color-mix(in srgb, var(--app-accent) 8%, transparent);
  }

  .model-status__title {
    font-size: 13px;
    font-weight: 700;
    color: var(--app-text);
  }

  .model-status__meta {
    margin-top: 2px;
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .model-status__pill {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .permission-callout {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 12px;
    border: 1px dashed var(--app-warn-border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-warn) 6%, transparent);
  }

  .permission-callout--ok {
    border-color: color-mix(in srgb, var(--app-accent) 34%, var(--app-border));
    background: color-mix(in srgb, var(--app-accent) 6%, transparent);
  }

  .permission-callout__copy {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 4px;
  }

  .permission-callout__eyebrow {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .permission-callout strong {
    font-size: 12px;
    color: var(--app-text);
  }

  .permission-callout p {
    margin: 0;
    font-size: 10px;
    line-height: 1.45;
    color: var(--app-text-faint);
  }

  .download-progress {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .download-progress__bar {
    height: 6px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
  }

  .download-progress__bar span {
    display: block;
    height: 100%;
    min-width: 8%;
    border-radius: inherit;
    background: var(--app-accent);
    transition: width 0.15s ease;
  }

  .cleanup-result {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--app-accent) 34%, var(--app-border));
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-accent) 7%, transparent);
    color: var(--app-text);
    font-size: 11px;
    line-height: 1.45;
  }

  .cleanup-result strong {
    font-size: 12px;
  }

  .cleanup-result p {
    margin: 0;
    color: var(--app-text-muted);
    font-weight: 700;
  }

  .cleanup-result ul {
    margin: 0;
    padding-left: 18px;
    color: var(--app-text-muted);
  }

  .delete-confirmation {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    background: var(--app-danger-bg-soft);
    color: var(--app-text);
  }

  .delete-confirmation strong {
    font-size: 12px;
  }

  .delete-confirmation p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.45;
  }

  /* ── Text input ────────────────────────────────────────────── */
  .input-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .text-input {
    flex: 1;
    padding: 7px 10px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    font-family: inherit;
    font-size: 12px;
    color: var(--app-text);
    outline: none;
    transition: border-color 0.12s;
  }

  .text-input:focus {
    border-color: var(--app-accent);
  }

  .text-input--empty {
    border-color: var(--app-warn-border);
  }

  .text-input::placeholder {
    color: var(--app-text-faint);
  }

  /* ── Buttons ───────────────────────────────────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  .saved-badge {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--app-accent);
    animation: fade-in-out 2.2s ease forwards;
  }

  @keyframes fade-in-out {
    0% { opacity: 0; transform: translateX(-4px); }
    15% { opacity: 1; transform: translateX(0); }
    80% { opacity: 1; }
    100% { opacity: 0; }
  }

  /* ── Badges ────────────────────────────────────────────────── */
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .badge--ok {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border: 1px solid var(--app-accent-border);
  }

  .badge--neutral {
    background: var(--app-neutral-bg);
    color: var(--app-neutral-text);
    border: 1px solid var(--app-neutral-border);
  }

  .badge--sm {
    padding: 0 5px;
    font-size: 9px;
  }

  /* ── Effective device ─────────────────────────────────────── */
  .effective-device {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: var(--app-source-mic-bg);
    border: 1px solid var(--app-source-mic-border);
    border-radius: 5px;
    transition: background 0.2s, border-color 0.2s;
  }

  .effective-device--none {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }

  .effective-device__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex-shrink: 0;
    transition: background 0.2s;
  }

  .effective-device__dot--on {
    background: var(--app-accent);
  }

  .effective-device__label {
    font-size: 12px;
    font-weight: 500;
    color: var(--app-text);
    display: flex;
    align-items: center;
    gap: 7px;
  }

  /* ── Device list ───────────────────────────────────────────── */
  .device-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .device-item {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 10px;
    border-radius: 4px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    transition: border-color 0.12s;
  }

  .device-item--active {
    border-color: var(--app-source-mic-border);
    background: var(--app-source-mic-bg);
  }

  .device-item__dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-border-strong);
    flex-shrink: 0;
    transition: background 0.15s;
  }

  .device-item__dot--active {
    background: var(--app-accent);
  }

  .device-item__name {
    font-size: 11px;
    color: var(--app-text);
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .device-item__badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  /* ── Inline error ─────────────────────────────────────────── */
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
  }

  .inline-error__icon {
    color: var(--app-danger);
    font-size: 11px;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .inline-error__msg {
    font-size: 11px;
    color: var(--app-danger-text);
    flex: 1;
    word-break: break-word;
  }

  /* ── Misc ──────────────────────────────────────────────────── */
  .loading-text {
    font-size: 11px;
    color: var(--app-text-faint);
    font-style: italic;
  }

  .empty-state {
    font-size: 11px;
    color: var(--app-text-faint);
    font-style: italic;
  }

  /* ── Capture source hints ─────────────────────────────────── */
  .capture-source-hint {
    font-size: 10px;
    color: var(--app-warn);
    letter-spacing: 0.03em;
    line-height: 1.5;
    margin-top: 2px;
  }

  /* ── Inline validation ────────────────────────────────────── */
  .inline-validation {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 10px 12px;
    background: var(--app-warn-bg);
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
  }

  .inline-validation__item {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: 11px;
    color: var(--app-warn);
    line-height: 1.5;
  }

  .inline-validation__icon {
    font-size: 10px;
    flex-shrink: 0;
    color: var(--app-warn-strong);
  }

  /* ── Resolution preset chips ──────────────────────────────────────── */
  .resolution-preset-grid {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .preset-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 16px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    font-family: inherit;
    transition: background 0.12s, border-color 0.12s;
    min-width: 72px;
  }

  .preset-chip:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }

  .preset-chip--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
  }

  .preset-chip:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .preset-chip__label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--app-text);
    text-transform: uppercase;
  }

  .preset-chip--active .preset-chip__label {
    color: var(--app-accent);
  }

  .preset-chip__dim {
    font-size: 9px;
    color: var(--app-text-subtle);
    letter-spacing: 0.04em;
  }

  .preset-chip--active .preset-chip__dim {
    color: var(--app-accent-strong);
  }

  /* ── Custom resolution inputs ─────────────────────────────────────── */
  .custom-resolution-inputs {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .custom-res-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .custom-res-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }

  .custom-res-input {
    width: 100%;
  }

  .custom-res-sep {
    font-size: 18px;
    font-weight: 300;
    color: var(--app-text-faint);
    padding-bottom: 7px;
    flex-shrink: 0;
    line-height: 1;
  }

  /* ── Resolution locked notice ─────────────────────────────────────── */
  .resolution-unsupported-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-neutral-bg);
    border: 1px solid var(--app-neutral-border);
    border-radius: 4px;
  }

  .resolution-unsupported-notice__icon {
    font-size: 11px;
    color: var(--app-neutral-text);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-unsupported-notice__text {
    font-size: 10px;
    color: var(--app-neutral-text);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-locked-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-info-bg);
    border: 1px solid var(--app-info-border);
    border-radius: 4px;
  }

  .resolution-locked-notice__icon {
    font-size: 11px;
    color: var(--app-info-strong);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-locked-notice__text {
    font-size: 10px;
    color: var(--app-info-strong);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-locked-notice__text strong {
    color: var(--app-info);
    font-weight: 700;
  }

  /* ── Resolution pending notice ────────────────────────────────────── */
  .resolution-pending-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-neutral-bg);
    border: 1px solid var(--app-neutral-border);
    border-radius: 4px;
  }

  .resolution-pending-notice__icon {
    font-size: 11px;
    color: var(--app-neutral-text);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-pending-notice__text {
    font-size: 10px;
    color: var(--app-neutral-text);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Resolution warn notice (support lookup failed) ───────────────────── */
  .resolution-warn-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-warn-bg);
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
  }

  .resolution-warn-notice__icon {
    font-size: 11px;
    color: var(--app-warn-strong);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-warn-notice__text {
    font-size: 10px;
    color: var(--app-warn);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .resolution-supported-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 4px;
  }

  .resolution-supported-notice__icon {
    font-size: 11px;
    color: var(--app-accent);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .resolution-supported-notice__text {
    font-size: 10px;
    color: var(--app-accent-strong);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Video Bitrate chips ──────────────────────────────────────────── */
  .bitrate-mode-chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .bitrate-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 16px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    cursor: pointer;
    outline: none;
    font-family: inherit;
    transition: background 0.12s, border-color 0.12s;
    min-width: 72px;
  }

  .bitrate-chip:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }

  .bitrate-chip--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
  }

  .bitrate-chip:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .bitrate-chip__label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--app-text);
    text-transform: uppercase;
  }

  .bitrate-chip--active .bitrate-chip__label {
    color: var(--app-accent);
  }

  .bitrate-chip__mbps {
    font-size: 9px;
    color: var(--app-text-subtle);
    letter-spacing: 0.04em;
  }

  .bitrate-chip--active .bitrate-chip__mbps {
    color: var(--app-accent-strong);
  }

  /* ── Bitrate preset hint ──────────────────────────────────────────── */
  .bitrate-preset-hint strong {
    color: var(--app-neutral-text);
    font-weight: 700;
  }

  /* ── Custom bitrate input row ─────────────────────────────────────── */
  .custom-bitrate-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }

  .custom-bitrate-input-wrap {
    display: flex;
    align-items: center;
    gap: 0;
  }

  .custom-bitrate-input {
    width: 120px;
    border-radius: 4px 0 0 4px;
    flex: unset;
  }

  .custom-bitrate-unit {
    padding: 7px 10px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-left: none;
    border-radius: 0 4px 4px 0;
    font-size: 11px;
    color: var(--app-text-subtle);
    letter-spacing: 0.06em;
    white-space: nowrap;
    font-weight: 600;
    text-transform: uppercase;
    user-select: none;
  }

  /* ── Bitrate compatibility notice ─────────────────────────────────── */
  .bitrate-compat-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-info-bg);
    border: 1px solid var(--app-info-border);
    border-radius: 4px;
  }

  .bitrate-compat-notice__icon {
    font-size: 11px;
    color: var(--app-info-strong);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .bitrate-compat-notice__text {
    font-size: 10px;
    color: var(--app-info-strong);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  /* ── Inactivity pause ─────────────────────────────────────────────── */
  .idle-timeout-row {
    margin-top: 2px;
  }

  /* ── Audio activity notice ────────────────────────────────────────── */
  .audio-activity-notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 12px;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 4px;
  }

  .audio-activity-notice__icon {
    font-size: 11px;
    color: var(--app-accent);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .audio-activity-notice__text {
    font-size: 10px;
    color: var(--app-accent-strong);
    letter-spacing: 0.02em;
    line-height: 1.55;
  }

  .audio-activity-notice__text strong {
    color: var(--app-accent);
    font-weight: 700;
  }

  /* ── Debug log status ────────────────────────────────────────────── */
  .debug-log-status {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 4px;
  }

  .debug-log-status__row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .debug-log-status__label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    width: 48px;
    flex-shrink: 0;
  }

  .debug-log-status__value {
    font-size: 11px;
    color: var(--app-text);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .debug-log-status__path {
    font-size: 10px;
    color: var(--app-info-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: monospace;
  }

  .debug-log-status__dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex-shrink: 0;
  }

  .debug-log-status__dot--on {
    background: var(--app-accent);
  }

  .debug-log-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .row-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .error-text {
    margin: 0;
    color: var(--app-danger-text);
    font-size: 11px;
    line-height: 1.5;
    word-break: break-word;
  }

  .excluded-apps-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .excluded-app-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 8px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
  }

  .excluded-app-row__meta {
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .excluded-app-row__name {
    overflow: hidden;
    color: var(--app-text);
    font-size: 12px;
    font-weight: 700;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .excluded-app-row__bundle {
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  /* ── Danger button variant ───────────────────────────────────────── */
  .btn--danger {
    background: var(--app-danger-bg-soft);
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }

  .btn--danger:not(:disabled):hover {
    background: var(--app-danger-bg);
    border-color: var(--app-danger);
  }

  /* ── Light theme overrides ────────────────────────────────────
     The dark palette above is the source of truth; this block flips
     just the surfaces, borders, and text colors that don't already
     consume layout-level semantic tokens. Keeping the override flat
     (one selector per affected rule) makes it easy to audit which
     tokens still hard-code dark values. */
  :global([data-theme="light"]) .page-header {
    border-bottom-color: var(--app-border);
  }
  :global([data-theme="light"]) .page-header__title {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .page-header__status-text {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .page-header__status-text::before {
    background: var(--app-accent-strong);
  }

  :global([data-theme="light"]) .status-pill {
    border-color: var(--app-border);
    background: var(--app-surface);
  }
  :global([data-theme="light"]) .status-pill--on {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  :global([data-theme="light"]) .status-pill--info {
    border-color: var(--app-border);
    background: var(--app-surface-raised);
  }
  :global([data-theme="light"]) .status-pill__dot {
    background: var(--app-text-faint);
  }
  :global([data-theme="light"]) .status-pill--on .status-pill__dot {
    background: var(--app-accent);
  }
  :global([data-theme="light"]) .status-pill__label {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .status-pill--on .status-pill__label {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .card {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .card__title {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .card__subtitle {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .group-label {
    color: var(--app-text-subtle);
  }
  :global([data-theme="light"]) .settings-stack {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .settings-divider {
    background: var(--app-border);
  }
  :global([data-theme="light"]) .group-hint {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .text-input {
    background: var(--app-surface);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .text-input:focus {
    border-color: var(--app-accent);
  }
  :global([data-theme="light"]) .text-input::placeholder {
    color: var(--app-text-faint);
  }

  :global([data-theme="light"]) .btn--ghost {
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .saved-badge {
    color: var(--app-accent-strong);
  }

  :global([data-theme="light"]) .badge--ok {
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
  }
  :global([data-theme="light"]) .badge--neutral {
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    border-color: var(--app-border);
  }

  :global([data-theme="light"]) .effective-device {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  :global([data-theme="light"]) .effective-device--none {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .effective-device__dot {
    background: var(--app-text-faint);
  }
  :global([data-theme="light"]) .effective-device__dot--on {
    background: var(--app-accent);
  }
  :global([data-theme="light"]) .effective-device__label {
    color: var(--app-text);
  }

  :global([data-theme="light"]) .device-item {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .device-item--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  :global([data-theme="light"]) .device-item__dot {
    background: var(--app-text-faint);
  }
  :global([data-theme="light"]) .device-item__dot--active {
    background: var(--app-accent);
  }
  :global([data-theme="light"]) .device-item__name {
    color: var(--app-text);
  }

  :global([data-theme="light"]) .preset-chip,
  :global([data-theme="light"]) .bitrate-chip {
    background: var(--app-surface-raised);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .preset-chip:hover,
  :global([data-theme="light"]) .bitrate-chip:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .preset-chip--active,
  :global([data-theme="light"]) .bitrate-chip--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
  }
  :global([data-theme="light"]) .preset-chip__label,
  :global([data-theme="light"]) .bitrate-chip__label {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .preset-chip--active .preset-chip__label,
  :global([data-theme="light"]) .bitrate-chip--active .bitrate-chip__label {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .preset-chip__dim,
  :global([data-theme="light"]) .bitrate-chip__mbps {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .custom-res-label {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .custom-res-sep {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .custom-bitrate-unit {
    background: var(--app-surface-raised);
    border-color: var(--app-border-strong);
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .resolution-unsupported-notice,
  :global([data-theme="light"]) .resolution-locked-notice,
  :global([data-theme="light"]) .resolution-pending-notice,
  :global([data-theme="light"]) .resolution-warn-notice,
  :global([data-theme="light"]) .resolution-supported-notice,
  :global([data-theme="light"]) .bitrate-compat-notice,
  :global([data-theme="light"]) .audio-activity-notice {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .resolution-supported-notice {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  :global([data-theme="light"]) .resolution-supported-notice__icon,
  :global([data-theme="light"]) .audio-activity-notice__icon {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .resolution-supported-notice__text,
  :global([data-theme="light"]) .audio-activity-notice__text {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .audio-activity-notice__text strong {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .resolution-locked-notice__text,
  :global([data-theme="light"]) .bitrate-compat-notice__text {
    color: var(--app-info-strong);
  }
  :global([data-theme="light"]) .resolution-locked-notice__text strong {
    color: var(--app-info);
  }

  :global([data-theme="light"]) .capture-source-hint {
    color: var(--app-warn);
  }
  :global([data-theme="light"]) .group-hint--warn {
    color: var(--app-warn);
  }

  :global([data-theme="light"]) .inline-validation {
    background: var(--app-warn-bg);
    border-color: var(--app-warn-border);
  }
  :global([data-theme="light"]) .inline-validation__item {
    color: var(--app-warn);
  }
  :global([data-theme="light"]) .inline-validation__icon {
    color: var(--app-warn-strong);
  }

  :global([data-theme="light"]) .inline-error {
    background: var(--app-danger-bg-soft);
    border-color: var(--app-danger-border);
  }
  :global([data-theme="light"]) .inline-error__icon {
    color: var(--app-danger-strong);
  }
  :global([data-theme="light"]) .inline-error__msg {
    color: var(--app-danger);
  }

  :global([data-theme="light"]) .loading-text,
  :global([data-theme="light"]) .empty-state {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .debug-log-status {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .debug-log-status__label {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .debug-log-status__value {
    color: var(--app-text);
  }
  :global([data-theme="light"]) .debug-log-status__path {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .debug-log-status__dot {
    background: var(--app-text-faint);
  }
  :global([data-theme="light"]) .debug-log-status__dot--on {
    background: var(--app-accent);
  }

  :global([data-theme="light"]) .btn--danger {
    background: var(--app-danger-bg-soft);
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }
  :global([data-theme="light"]) .btn--danger:not(:disabled):hover {
    background: var(--app-danger-bg);
    border-color: var(--app-danger-strong);
  }

  .privacy-disclosure {
    display: grid;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface-subtle);
  }

  .privacy-disclosure p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }

  .agent-access-callout {
    display: grid;
    gap: 4px;
    padding: 10px 12px;
    border: 1px solid color-mix(in srgb, var(--app-accent) 35%, var(--app-border));
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-accent) 12%, var(--app-surface));
  }

  .agent-access-callout strong {
    color: var(--app-text);
    font-size: 12px;
  }

  .agent-access-callout p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }

</style>
