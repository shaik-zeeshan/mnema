<script lang="ts">
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import ScreenResolutionControl from "$lib/components/ScreenResolutionControl.svelte";
  import VideoBitrateControl from "$lib/components/VideoBitrateControl.svelte";
  import SceneShell from "./SceneShell.svelte";
  import ProgressArc from "./ProgressArc.svelte";
  import ArmStatus from "./ArmStatus.svelte";
  import AdvancedReveal from "./AdvancedReveal.svelte";
  import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
  import { isShortcutSuppressedTarget } from "$lib/keyboard";
  import { theme } from "$lib/theme.svelte";
  import type {
    ActivityMode,
    AudioTranscriptionMemoryMode,
    AudioTranscriptionModelDownloadProgress,
    AudioTranscriptionModelStatus,
    AudioTranscriptionModelStatusResponse,
    AudioTranscriptionProvider,
    ExcludedAppEntry,
    GetPermissionsResponse,
    OcrModelDownloadProgress,
    OcrModelStatus,
    OcrModelStatusResponse,
    OcrProvider,
    OcrRecognitionMode,
    OcrTesseractPageSegmentationMode,
    OcrTesseractPreprocessMode,
    PermissionStatus,
    RecordingSettings,
    RecordingSettingsDomainUpdateResponse,
    ResolutionMode,
    ResolutionPreset,
    RetentionPolicy,
    VideoBitrateMode,
    VideoBitratePreset,
  } from "$lib/types";

  type OnboardingState = {
    schemaVersion: number;
    completedAtUnixMs: number | null;
  };
  const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
  const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT = "audio_transcription_model_download_progress";
  const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT = "ocr_model_download_progress";
  const SELECTABLE_OCR_PROVIDERS: readonly OcrProvider[] = ["apple_vision", "tesseract"];

  type OnboardingStep = "about" | "permissions" | "sources" | "video" | "storage" | "privacy" | "processing" | "done";
  type ProcessingPanel = "ocr" | "transcription";
  type PermissionValue = PermissionStatus | "unsupported" | "unknown";
  type PermissionKey = "screen" | "microphone" | "systemAudio";

  // Evocative subsystem names (the literal noun lives in `bayMeta.eyebrow`).
  const steps: { id: OnboardingStep; label: string }[] = [
    { id: "about", label: "About" },
    { id: "permissions", label: "Access" },
    { id: "sources", label: "Capture" },
    { id: "video", label: "Lens" },
    { id: "storage", label: "Vault" },
    { id: "privacy", label: "Shield" },
    { id: "processing", label: "Mind" },
    { id: "done", label: "Ready" },
  ];
  const railSteps = steps.filter((step) => step.id !== "about" && step.id !== "done");

  // The `capture -> index -> recall` progress arc groups the six bays under the
  // product's promise loop, in step order so the arc lights left-to-right.
  const arcPhases: { id: string; label: string; stepIds: OnboardingStep[] }[] = [
    { id: "capture", label: "capture", stepIds: ["permissions", "sources"] },
    { id: "index", label: "index", stepIds: ["video", "storage"] },
    { id: "recall", label: "recall", stepIds: ["privacy", "processing"] },
  ];
  // Per-bay literal noun + descriptive subtitle for the SceneShell header. The
  // big evocative title comes from `steps[].label`.
  const bayMeta: Record<string, { eyebrow: string; subtitle: string }> = {
    permissions: { eyebrow: "Permissions", subtitle: "Bring the macOS capture permissions online before the recorder arms." },
    sources: { eyebrow: "Sources & cadence", subtitle: "Choose what the recorder takes in, and how often it samples." },
    video: { eyebrow: "Video", subtitle: "Set the screen output size and how hard frames are compressed." },
    storage: { eyebrow: "Storage", subtitle: "Choose where recordings and the searchable database live." },
    privacy: { eyebrow: "Privacy", subtitle: "Decide what the recorder must never see." },
    processing: { eyebrow: "OCR & transcription", subtitle: "Turn captured frames and audio into searchable memory." },
  };
  const processingTabs: { id: ProcessingPanel; label: string }[] = [
    { id: "ocr", label: "OCR" },
    { id: "transcription", label: "Transcription" },
  ];

  const activityModeOptions = [
    { value: "system_input_only", label: "Input only", description: "Keyboard and pointer activity keep recording active." },
    { value: "system_input_or_screen", label: "Input or screen change", description: "Input plus visible display changes keep recording active." },
    { value: "system_input_or_screen_or_audio", label: "Input, screen, or audio", description: "Input, display, microphone, and system audio can keep recording active." },
  ];
  const fallbackTranscriptionProviderOptions = [
    { value: "local_whisper", label: "Local Whisper", description: "Whisper models managed by mnema." },
    { value: "apple_speech_on_device", label: "Apple Speech", description: "On-device speech recognition managed by macOS." },
    { value: "parakeet", label: "Parakeet", description: "NVIDIA Parakeet ONNX models managed by mnema." },
  ];
  const fallbackOcrProviderOptions = [
    { value: "apple_vision", label: "Apple Vision", description: "Model status is loading." },
    { value: "tesseract", label: "Tesseract", description: "Model status is loading." },
  ];

  let activeStep = $state<OnboardingStep>("about");
  let activeProcessingPanel = $state<ProcessingPanel>("ocr");
  let settings = $state<RecordingSettings | null>(null);
  let permissions = $state<Record<PermissionKey, PermissionValue> | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let starting = $state(false);
  let completing = $state(false);
  let refreshingPerms = $state(false);
  let requestingPerm = $state<PermissionKey | null>(null);
  let applyingRecommended = $state(false);
  let error = $state<string | null>(null);

  let draftCaptureScreen = $state(true);
  let draftCaptureMicrophone = $state(false);
  let draftCaptureSystemAudio = $state(false);
  let draftFrameRate = $state(1);
  let draftSegmentDuration = $state(60);
  let draftResolutionMode = $state<ResolutionMode>("original");
  let draftResolutionPreset = $state<ResolutionPreset>("1080p");
  let draftCustomWidth = $state<number | null>(null);
  let draftCustomHeight = $state<number | null>(null);
  let customWidthRaw = $state("");
  let customHeightRaw = $state("");
  let draftBitrateMode = $state<VideoBitrateMode>("preset");
  let draftBitratePreset = $state<VideoBitratePreset>("medium");
  let draftCustomMbpsRaw = $state("");
  let draftCustomMbps = $state<number | null>(null);
  let draftSaveDirectory = $state("");
  let draftPreviewCacheTtlSeconds = $state(3600);
  let draftRetentionPolicy = $state<RetentionPolicy>("never");
  let draftAutoStart = $state(false);
  let draftPauseCaptureOnInactivity = $state(false);
  let draftIdleTimeoutSeconds = $state(30);
  let draftActivityMode = $state<ActivityMode>("system_input_only");
  let draftMicrophoneActivitySensitivity = $state(50);
  let draftSystemAudioActivitySensitivity = $state(50);
  let draftOcrEnabled = $state(true);
  let draftOcrProvider = $state<OcrProvider>("apple_vision");
  let draftOcrModelId = $state<string | null>(null);
  let draftOcrLanguage = $state("");
  let draftOcrRecognitionMode = $state<OcrRecognitionMode>("fast");
  let draftOcrLanguageCorrection = $state(false);
  let draftOcrTesseractPageSegmentationMode = $state<OcrTesseractPageSegmentationMode>("single_block");
  let draftOcrTesseractPreprocessMode = $state<OcrTesseractPreprocessMode>("grayscale");
  let draftOcrTesseractUpscaleFactor = $state(1);
  let ocrModelStatus = $state<OcrModelStatusResponse | null>(null);
  let loadingOcrModelStatus = $state(false);
  let ocrModelError = $state<string | null>(null);
  let ocrDownloadProgress = $state<OcrModelDownloadProgress | null>(null);
  let startingOcrDownload = $state(false);
  let cancellingOcrDownload = $state(false);
  let ocrDownloadError = $state<string | null>(null);
  let draftTranscriptionEnabled = $state(true);
  let draftTranscriptionProvider = $state<AudioTranscriptionProvider>("local_whisper");
  let draftTranscriptionModelId = $state<string | null>("base");
  let draftTranscriptionLanguage = $state("auto");
  let draftTranscriptionMemoryMode = $state<AudioTranscriptionMemoryMode>("balanced");
  let draftTranscriptionIdleUnloadSeconds = $state(300);
  let draftTranscriptionChunkSeconds = $state(30);
  let transcriptionModelStatus = $state<AudioTranscriptionModelStatusResponse | null>(null);
  let loadingTranscriptionModelStatus = $state(false);
  let transcriptionModelError = $state<string | null>(null);
  let transcriptionDownloadProgress = $state<AudioTranscriptionModelDownloadProgress | null>(null);
  let startingTranscriptionDownload = $state(false);
  let cancellingTranscriptionDownload = $state(false);
  let transcriptionDownloadError = $state<string | null>(null);
  let draftTranscriptionMicrophoneEnabled = $state(true);
  let draftTranscriptionSystemAudioEnabled = $state(false);
  let draftExcludedApps = $state<ExcludedAppEntry[]>([]);
  const appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => draftExcludedApps,
    onSettingsUpdated: (updated) => {
      settings = updated.settings;
      syncDrafts(updated.settings);
    },
    setError: (message) => {
      error = message;
    },
  });

  const activeIndex = $derived(steps.findIndex((step) => step.id === activeStep));
  const railActiveIndex = $derived(railSteps.findIndex((step) => step.id === activeStep));
  // Unique, correct bay index drawn from the rail order — fixes the old
  // duplicate "03" that `video` and `storage` both hard-coded.
  const bayIndex = $derived(String(railActiveIndex + 1).padStart(2, "0"));
  const arcModel = $derived(
    arcPhases.map((phase) => ({
      id: phase.id,
      label: phase.label,
      steps: phase.stepIds.map((id) => {
        const globalIndex = steps.findIndex((step) => step.id === id);
        const railIndex = railSteps.findIndex((step) => step.id === id);
        return {
          id,
          label: steps[globalIndex]?.label ?? id,
          num: String(railIndex + 1).padStart(2, "0"),
          state: (globalIndex < activeIndex
            ? "done"
            : globalIndex === activeIndex
              ? "active"
              : "future") as "done" | "active" | "future",
        };
      }),
    }))
  );
  const isWelcome = $derived(activeStep === "about");
  const isFinal = $derived(activeStep === "done");
  const showChrome = $derived(!isWelcome && !isFinal);
  const canGoBack = $derived(activeIndex > 0 && !saving && !starting && !completing && !applyingRecommended && !appPrivacyExclusion.commandInFlight);
  const selectedSourceCount = $derived(
    Number(draftCaptureScreen) + Number(draftCaptureMicrophone) + Number(draftCaptureSystemAudio)
  );
  const requiresOcrAvailability = $derived(draftOcrEnabled);
  const requiresTranscriptionAvailability = $derived(draftTranscriptionEnabled);
  const canGoNext = $derived(
    !loading && !saving && !starting && !completing && !applyingRecommended && !appPrivacyExclusion.commandInFlight && settings !== null
    && (activeStep !== "sources" || selectedSourceCount > 0)
    && (activeStep !== "storage" || draftSaveDirectory.trim().length > 0)
    && canProceedFromActiveStep()
  );
  const grantedCount = $derived(
    permissions === null ? 0
      : (["screen", "microphone", "systemAudio"] as const).filter((k) => permissions?.[k] === "granted").length
  );
  const customResolutionErrors = $derived(validateCustomResolution());
  const customBitrateErrors = $derived(validateCustomBitrate());
  $effect(() => { void initialize(); });
  $effect(() => {
    void loadOcrModelStatus();
    void loadTranscriptionModelStatus();

    let unlistenOcrDownloadProgress: (() => void) | undefined;
    let unlistenTranscriptionDownloadProgress: (() => void) | undefined;
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
    let destroyed = false;

    listen<OcrModelDownloadProgress>(
      OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => { void handleOcrDownloadProgress(event.payload); }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenOcrDownloadProgress = fn;
    });

    listen<AudioTranscriptionModelDownloadProgress>(
      AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => { void handleTranscriptionDownloadProgress(event.payload); }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenTranscriptionDownloadProgress = fn;
    });

    listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      settings = event.payload;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    });
    return () => {
      destroyed = true;
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenRecordingSettingsChanged?.();
    };
  });
  $effect(() => {
    if (!draftCaptureScreen && draftCaptureSystemAudio) draftCaptureSystemAudio = false;
  });
  $effect(() => {
    const parsed = parsePositiveInteger(customWidthRaw);
    draftCustomWidth = parsed !== null && parsed >= 320 && parsed <= 7680 ? parsed : null;
  });
  $effect(() => {
    const parsed = parsePositiveInteger(customHeightRaw);
    draftCustomHeight = parsed !== null && parsed >= 240 && parsed <= 4320 ? parsed : null;
  });
  $effect(() => {
    const parsed = parsePositiveInteger(draftCustomMbpsRaw);
    draftCustomMbps = parsed !== null && parsed >= 1 && parsed <= 40 ? parsed : null;
  });
  async function initialize(): Promise<void> {
    loading = true;
    error = null;
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
      settings = loadedSettings;
      permissions = permissionResponse.permissions as Record<PermissionKey, PermissionValue>;
      syncDrafts(loadedSettings);
      void appPrivacyExclusion.loadPrivacyAppCandidates();
      void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    } catch (err) {
      error = serializeError(err);
    } finally {
      loading = false;
    }
  }

  function serializeError(err: unknown): string {
    return typeof err === "string" ? err : (JSON.stringify(err) ?? "Unknown error");
  }

  function syncDrafts(next: RecordingSettings): void {
    draftCaptureScreen = next.captureScreen;
    draftCaptureMicrophone = next.captureMicrophone;
    draftCaptureSystemAudio = next.captureSystemAudio;
    draftFrameRate = next.screenFrameRate;
    draftSegmentDuration = next.segmentDurationSeconds;
    if (next.screenResolution.mode === "custom") {
      draftResolutionMode = "custom";
      draftCustomWidth = next.screenResolution.width;
      draftCustomHeight = next.screenResolution.height;
      customWidthRaw = String(next.screenResolution.width);
      customHeightRaw = String(next.screenResolution.height);
    } else if (next.screenResolution.preset === "original") {
      draftResolutionMode = "original";
      draftResolutionPreset = "1080p";
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    } else {
      draftResolutionMode = "preset";
      draftResolutionPreset = next.screenResolution.preset;
      draftCustomWidth = null;
      draftCustomHeight = null;
      customWidthRaw = "";
      customHeightRaw = "";
    }
    if (next.videoBitrate.mode === "custom") {
      draftBitrateMode = "custom";
      draftBitratePreset = "medium";
      draftCustomMbps = next.videoBitrate.customMbps;
      draftCustomMbpsRaw = String(next.videoBitrate.customMbps);
    } else {
      draftBitrateMode = "preset";
      draftBitratePreset = next.videoBitrate.preset;
      draftCustomMbps = null;
      draftCustomMbpsRaw = "";
    }
    draftSaveDirectory = next.saveDirectory;
    draftPreviewCacheTtlSeconds = next.previewCacheTtlSeconds ?? 3600;
    draftRetentionPolicy = next.retentionPolicy ?? "never";
    draftAutoStart = next.autoStart;
    draftPauseCaptureOnInactivity = next.pauseCaptureOnInactivity;
    draftIdleTimeoutSeconds = next.idleTimeoutSeconds;
    draftActivityMode = "system_input_or_screen_or_audio";
    draftMicrophoneActivitySensitivity = next.microphoneActivitySensitivity ?? 50;
    draftSystemAudioActivitySensitivity = next.systemAudioActivitySensitivity ?? 50;
    draftOcrEnabled = next.ocr?.enabled ?? true;
    const loadedOcrProvider = next.ocr?.provider;
    const loadedOcrProviderSelectable = isSelectableOcrProvider(loadedOcrProvider);
    draftOcrProvider = loadedOcrProviderSelectable ? loadedOcrProvider : "apple_vision";
    draftOcrModelId = loadedOcrProviderSelectable
      ? (next.ocr?.modelId ?? defaultOcrModelIdForProvider(draftOcrProvider))
      : defaultOcrModelIdForProvider(draftOcrProvider);
    draftOcrLanguage = loadedOcrProviderSelectable
      ? (next.ocr?.language ?? defaultOcrLanguageForProvider(draftOcrProvider) ?? "")
      : defaultOcrLanguageForProvider(draftOcrProvider) ?? "";
    draftOcrRecognitionMode = next.ocr?.recognitionMode ?? "fast";
    draftOcrLanguageCorrection = next.ocr?.languageCorrection ?? false;
    draftOcrTesseractPageSegmentationMode = next.ocr?.tesseractPageSegmentationMode ?? "single_block";
    draftOcrTesseractPreprocessMode = next.ocr?.tesseractPreprocessMode ?? "grayscale";
    draftOcrTesseractUpscaleFactor = next.ocr?.tesseractUpscaleFactor ?? 1;
    draftTranscriptionEnabled = next.transcription?.enabled ?? true;
    draftTranscriptionMicrophoneEnabled = next.transcription?.microphoneEnabled ?? true;
    draftTranscriptionSystemAudioEnabled = next.transcription?.systemAudioEnabled ?? false;
    draftTranscriptionProvider = next.transcription?.provider ?? "local_whisper";
    draftTranscriptionModelId = next.transcription?.modelId ?? defaultTranscriptionModelIdForProvider(draftTranscriptionProvider);
    draftTranscriptionLanguage = next.transcription?.language ?? "auto";
    draftTranscriptionMemoryMode = next.transcription?.memoryMode ?? "balanced";
    draftTranscriptionIdleUnloadSeconds = next.transcription?.idleUnloadSeconds ?? 300;
    draftTranscriptionChunkSeconds = next.transcription?.chunkSeconds ?? 30;
    draftExcludedApps = [...(next.privacy?.excludedApps ?? [])];
  }

  function buildSettingsRequest(): RecordingSettings {
    const base = settings;
    if (base === null) throw new Error("Recording settings are not loaded.");
    return {
      ...base,
      captureScreen: draftCaptureScreen,
      captureMicrophone: draftCaptureMicrophone,
      captureSystemAudio: draftCaptureScreen && draftCaptureSystemAudio,
      screenFrameRate: draftFrameRate,
      screenResolution: draftResolutionMode === "custom"
        ? { mode: "custom", width: draftCustomWidth!, height: draftCustomHeight! }
        : { mode: "preset", preset: draftResolutionMode === "original" ? "original" : draftResolutionPreset },
      videoBitrate: draftBitrateMode === "custom"
        ? { mode: "custom", preset: null, customMbps: draftCustomMbps! }
        : { mode: "preset", preset: draftBitratePreset, customMbps: null },
      segmentDurationSeconds: draftSegmentDuration,
      saveDirectory: draftSaveDirectory.trim(),
      previewCacheTtlSeconds: draftPreviewCacheTtlSeconds,
      retentionPolicy: draftRetentionPolicy,
      appearance: theme.loaded ? theme.appearance : base.appearance,
      autoStart: draftAutoStart,
      pauseCaptureOnInactivity: draftPauseCaptureOnInactivity,
      idleTimeoutSeconds: draftIdleTimeoutSeconds,
      activityMode: "system_input_or_screen_or_audio",
      microphoneActivitySensitivity: draftMicrophoneActivitySensitivity,
      systemAudioActivitySensitivity: draftSystemAudioActivitySensitivity,
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
        tesseractCharWhitelist: null,
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
    };
  }

  async function saveSettings(): Promise<void> {
    saving = true;
    error = null;
    try {
      const request = buildSettingsRequest();
      let updated = settings;
      const domainUpdates: Array<[string, Record<string, unknown>]> = [
        [
          "update_capture_source_settings",
          {
            captureScreen: request.captureScreen,
            captureMicrophone: request.captureMicrophone,
            captureSystemAudio: request.captureSystemAudio,
          },
        ],
        [
          "update_capture_timing_settings",
          {
            segmentDurationSeconds: request.segmentDurationSeconds,
            autoStart: request.autoStart,
          },
        ],
        [
          "update_video_settings",
          {
            screenFrameRate: request.screenFrameRate,
            screenResolution: request.screenResolution,
            videoBitrate: request.videoBitrate,
          },
        ],
        [
          "update_storage_settings",
          {
            saveDirectory: request.saveDirectory,
            retentionPolicy: request.retentionPolicy,
          },
        ],
        [
          "update_display_settings",
          {
            appearance: request.appearance,
          },
        ],
        [
          "update_inactivity_settings",
          {
            pauseCaptureOnInactivity: request.pauseCaptureOnInactivity,
            idleTimeoutSeconds: request.idleTimeoutSeconds,
            microphoneActivitySensitivity: request.microphoneActivitySensitivity,
            systemAudioActivitySensitivity: request.systemAudioActivitySensitivity,
          },
        ],
        [
          "update_processing_settings",
          {
            previewCacheTtlSeconds: request.previewCacheTtlSeconds,
            ocr: request.ocr,
            transcription: request.transcription,
          },
        ],
      ];

      for (const [command, domainRequest] of domainUpdates) {
        const response = await invoke<RecordingSettingsDomainUpdateResponse>(command, { request: domainRequest });
        updated = response.settings;
      }

      if (updated !== null) {
        settings = updated;
        syncDrafts(updated);
      }
    } catch (err) {
      error = serializeError(err);
      throw err;
    } finally {
      saving = false;
    }
  }

  async function refreshPermissions(): Promise<void> {
    error = null;
    refreshingPerms = true;
    try {
      const response = await invoke<GetPermissionsResponse>("get_capture_permissions");
      permissions = response.permissions as Record<PermissionKey, PermissionValue>;
    } catch (err) {
      error = serializeError(err);
    } finally {
      refreshingPerms = false;
    }
  }

  // Granted/unsupported need no action. macOS won't re-prompt once denied, so
  // those rows deep-link to System Settings instead of re-requesting.
  function permissionAction(
    value: PermissionValue | undefined,
  ): { label: string; mode: "request" | "settings" } | null {
    if (value === "granted" || value === "unsupported") return null;
    if (value === "denied" || value === "restricted") return { label: "Open Settings", mode: "settings" };
    return { label: "Grant access", mode: "request" };
  }

  async function requestPermission(key: PermissionKey): Promise<void> {
    if (requestingPerm) return;
    error = null;
    requestingPerm = key;
    try {
      const action = permissionAction(permissions?.[key]);
      if (action?.mode === "settings") {
        await invoke("open_capture_privacy_settings", { kind: key });
      } else {
        const response = await invoke<GetPermissionsResponse>("request_capture_permission", { kind: key });
        permissions = response.permissions as Record<PermissionKey, PermissionValue>;
      }
    } catch (err) {
      error = serializeError(err);
    } finally {
      requestingPerm = null;
    }
  }

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
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadOcrModelStatus();
    }
  }

  async function loadTranscriptionModelStatus(): Promise<void> {
    loadingTranscriptionModelStatus = true;
    transcriptionModelError = null;
    try {
      transcriptionModelStatus = await invoke<AudioTranscriptionModelStatusResponse>("get_audio_transcription_model_status");
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
        }
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

  async function handleTranscriptionDownloadProgress(progress: AudioTranscriptionModelDownloadProgress): Promise<void> {
    transcriptionDownloadProgress = progress;
    if (["completed", "failed", "cancelled"].includes(progress.status)) {
      await loadTranscriptionModelStatus();
    }
  }

  async function nextStep(): Promise<void> {
    if (!canGoNext) return;
    if (activeStep === "sources" || activeStep === "video" || activeStep === "storage" || activeStep === "processing") {
      try { await saveSettings(); } catch { return; }
    }
    activeStep = steps[Math.min(activeIndex + 1, steps.length - 1)].id;
  }

  function previousStep(): void {
    if (!canGoBack) return;
    activeStep = steps[Math.max(activeIndex - 1, 0)].id;
  }

  async function finish(startRecording: boolean): Promise<void> {
    if (!canGoNext || settings === null) return;
    completing = true;
    starting = startRecording;
    error = null;
    try {
      await saveSettings();
      if (startRecording) {
        await invoke("start_native_capture", {
          request: {
            captureScreen: draftCaptureScreen,
            captureMicrophone: draftCaptureMicrophone,
            captureSystemAudio: draftCaptureScreen && draftCaptureSystemAudio,
          },
        });
      }
      await invoke("complete_onboarding");
    } catch (err) {
      error = serializeError(err);
      completing = false;
      starting = false;
    }
  }

  function permissionLabel(value: PermissionValue | undefined): string {
    switch (value) {
      case "granted": return "Granted";
      case "denied": return "Denied";
      case "not_determined": return "Not requested";
      case "restricted": return "Restricted";
      case "unsupported": return "Unsupported";
      default: return "Unknown";
    }
  }

  function permissionTone(value: PermissionValue | undefined): "ok" | "pending" | "blocked" {
    if (value === "granted") return "ok";
    if (value === "not_determined") return "pending";
    return "blocked";
  }

  function formatDuration(v: number): string {
    if (v >= 60) {
      const m = Math.floor(v / 60);
      const s = v % 60;
      return s ? `${m}m ${s}s` : `${m}m`;
    }
    return `${v}s`;
  }

  function parsePositiveInteger(raw: string): number | null {
    const trimmed = raw.trim();
    if (!/^\d+$/.test(trimmed)) return null;
    const parsed = Number.parseInt(trimmed, 10);
    return Number.isFinite(parsed) ? parsed : null;
  }

  function validateCustomResolution(): string[] {
    if (draftResolutionMode !== "custom") return [];
    const errors: string[] = [];
    if (draftCustomWidth === null) errors.push("Width must be between 320 and 7680 pixels.");
    if (draftCustomHeight === null) errors.push("Height must be between 240 and 4320 pixels.");
    return errors;
  }

  function validateCustomBitrate(): string[] {
    if (draftBitrateMode !== "custom") return [];
    return draftCustomMbps === null ? ["Bitrate must be a whole number from 1 to 40 Mbps."] : [];
  }

  const ocrProviderOptions = $derived(
    (ocrModelStatus?.providers ?? [])
      .filter((provider) => isSelectableOcrProvider(provider.provider))
      .map((provider) => ({
        value: provider.provider,
        label: provider.displayName,
        description: provider.models.some((model) => model.available)
          ? "At least one model is available"
          : "No available model detected",
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

  const selectedTranscriptionModels = $derived(selectedTranscriptionProviderStatus?.models ?? []);

  const transcriptionModelOptions = $derived(
    selectedTranscriptionModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: `${model.displayName} · ${transcriptionStatusLabel(model)}`,
    }))
  );

  const selectedTranscriptionModel = $derived(
    selectedTranscriptionModels.find((model) => model.modelId === draftTranscriptionModelId) ?? selectedTranscriptionModels[0] ?? null
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

  const processingReady = $derived(
    (!requiresOcrAvailability || (!!selectedOcrModel && selectedOcrModel.available && !selectedOcrDownloadRunning))
    && (!requiresTranscriptionAvailability || (!!selectedTranscriptionModel && selectedTranscriptionModel.available && !selectedTranscriptionDownloadRunning))
  );

  // "Armed" signals per bay — each reflects real draft/derived state, never a
  // synthetic flag. Privacy is informational (no hard prerequisite gates it).
  const armedSources = $derived(selectedSourceCount > 0);
  const armedVideo = $derived(customResolutionErrors.length === 0 && customBitrateErrors.length === 0);
  const armedStorage = $derived(draftSaveDirectory.trim().length > 0);
  const armedPrivacy = $derived(draftExcludedApps.length > 0);

  // Advanced reveals always start collapsed — every bay opens to its essential
  // controls, and the secondary tuning stays tucked behind its disclosure until
  // the user asks for it.

  function goToStep(id: string): void {
    const target = id as OnboardingStep;
    const index = steps.findIndex((step) => step.id === target);
    if (index >= 0 && index <= activeIndex) activeStep = target;
  }

  // One-tap "use recommended setup": set smart defaults, apply recommended
  // privacy exclusions, persist via the existing pipeline, then land on the
  // first unsatisfied true prerequisite — mirroring `canGoNext`. It never
  // bypasses the finale's `complete_onboarding`/`start_native_capture`.
  async function applyRecommendedSetup(): Promise<void> {
    if (settings === null || saving || starting || completing || applyingRecommended || appPrivacyExclusion.commandInFlight) {
      return;
    }
    applyingRecommended = true;
    error = null;
    try {
      // Apply recommended privacy exclusions first. Each command syncs drafts
      // from its server response, so we must set the smart defaults *after* it
      // resolves — otherwise syncDrafts would clobber them before the save.
      // Safe no-op when nothing is pending.
      await appPrivacyExclusion.applyAllRecommendedPrivacyApps();
      draftCaptureScreen = true;
      draftOcrEnabled = true;
      chooseOcrProvider("apple_vision");
      draftTranscriptionEnabled = true;
      chooseTranscriptionProvider("local_whisper");
      draftTranscriptionModelId = "base";
      await saveSettings();
    } catch {
      return;
    } finally {
      applyingRecommended = false;
    }
    if (draftSaveDirectory.trim().length === 0) {
      activeStep = "storage";
      return;
    }
    if (!processingReady) {
      activeStep = "processing";
      return;
    }
    activeStep = "done";
  }

  function canProceedFromActiveStep(): boolean {
    if (activeStep === "video") {
      return customResolutionErrors.length === 0 && customBitrateErrors.length === 0;
    }
    if (activeStep === "processing" || activeStep === "done") {
      return processingReady;
    }
    return true;
  }

  function ocrStatusLabel(model: OcrModelStatus): string {
    if (model.available) return "Available";
    if (model.status === "os_managed") return "OS managed";
    if (model.status === "installed") return "Installed";
    if (model.status === "downloading") return "Downloading";
    if (model.status === "failed") return "Failed";
    return "Missing";
  }

  function transcriptionStatusLabel(model: AudioTranscriptionModelStatus): string {
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

  function chooseOcrProvider(value: string): void {
    if (!isSelectableOcrProvider(value)) return;
    draftOcrProvider = value;
    draftOcrModelId = preferredOcrModelIdForProvider(draftOcrProvider);
    draftOcrLanguage = defaultOcrLanguageForProvider(draftOcrProvider) ?? "";
  }

  function chooseOcrModel(value: string): void {
    draftOcrModelId = value === "__os_managed__" ? null : value;
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

  function chooseTranscriptionProvider(value: string): void {
    draftTranscriptionProvider = value as AudioTranscriptionProvider;
    draftTranscriptionModelId = preferredTranscriptionModelIdForProvider(draftTranscriptionProvider);
  }

  function chooseTranscriptionModel(value: string): void {
    draftTranscriptionModelId = value === "__os_managed__" ? null : value;
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

  function handleProcessingTabKeydown(event: KeyboardEvent): void {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[role="tab"]')
      : null;
    const focusedTabId = focusedTab?.id?.replace(/^processing-tab-/, "") ?? null;
    const focusedIndex = processingTabs.findIndex((tab) => tab.id === focusedTabId);
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : processingTabs.findIndex((tab) => tab.id === activeProcessingPanel);
    if (currentIndex === -1) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % processingTabs.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + processingTabs.length) % processingTabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = processingTabs.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    event.stopPropagation();
    const nextTab = processingTabs[nextIndex];
    activeProcessingPanel = nextTab.id;
    document.getElementById(`processing-tab-${nextTab.id}`)?.focus();
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (
      e.altKey &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.shiftKey &&
      e.key === "ArrowLeft" &&
      canGoBack &&
      !isShortcutSuppressedTarget(e.target)
    ) {
      e.preventDefault();
      previousStep();
      return;
    }
    if (isShortcutSuppressedTarget(e.target)) return;
    if (e.key === "Enter" && !e.shiftKey && activeStep !== "done" && canGoNext) {
      e.preventDefault();
      void nextStep();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<section class="ob" class:ob--welcome={isWelcome} class:ob--final={isFinal}>
  {#if showChrome}
    <header class="ob__head">
      <div class="ob__title">
        <span class="ob__index">{String(railActiveIndex + 1).padStart(2, "0")}/{String(railSteps.length).padStart(2, "0")}</span>
        <h1>Set up mnema</h1>
      </div>
      <span class="ob__status" data-tone={canGoNext ? "ok" : "pending"}>
        {loading ? "Loading" : applyingRecommended ? "Arming" : saving ? "Saving" : starting ? "Starting" : "First run"}
      </span>
    </header>

    <ProgressArc
      phases={arcModel}
      navDisabled={loading || saving || starting || completing || applyingRecommended}
      onNavigate={goToStep}
    />
  {/if}

  <div class="ob__body" class:ob__body--combobox-open={appPrivacyExclusion.comboboxOpen && activeStep === "privacy"}>
    {#if loading}
      <div class="card card--loading">
        <span class="loader" aria-hidden="true"></span>
        <span class="loading-text">Loading settings…</span>
      </div>
    {:else if settings}
      {#if isWelcome || isFinal}
        {#key activeStep}
          <div class="step-anim">
            {#if isWelcome}
              <section class="welcome" aria-labelledby="welcome-title">
                <div class="welcome__bg" aria-hidden="true">
                  <div class="welcome__grid"></div>
                  <div class="welcome__halo"></div>
                </div>
                <div class="welcome__inner">
                  <span class="welcome__eyebrow">
                    <span class="welcome__pulse"></span>
                    Welcome
                  </span>
                  <h2 id="welcome-title" class="welcome__title">
                    Your <em>memory</em>,
                    <br />on rewind.
                  </h2>
                  <p class="welcome__tag">
                    mnema quietly records your screen so you can scrub back to anything you've seen — searchable, local, and yours.
                  </p>
                  <ul class="welcome__loop" aria-hidden="true">
                    <li><span></span>capture</li>
                    <li><span></span>index</li>
                    <li><span></span>recall</li>
                  </ul>
                  <div class="welcome__cta">
                    <button type="button" class="btn btn--primary btn--lg" onclick={nextStep} disabled={!canGoNext}>
                      Begin setup
                      <span class="btn__arrow" aria-hidden="true">→</span>
                    </button>
                    <span class="welcome__meta">≈ 60 seconds · 6 quick steps</span>
                  </div>
                  <button
                    type="button"
                    class="btn btn--link welcome__accel"
                    onclick={applyRecommendedSetup}
                    disabled={!canGoNext}
                  >
                    {applyingRecommended ? "Arming the recorder…" : "Use recommended setup →"}
                  </button>
                </div>
              </section>
            {:else}
              <section class="finale" aria-labelledby="finale-title">
                <div class="finale__bg" aria-hidden="true">
                  <div class="finale__rings"></div>
                  <div class="finale__rings finale__rings--alt"></div>
                </div>
                <div class="finale__inner">
                  <span class="finale__crest">
                    <span class="finale__crest-dot"></span>
                    All set
                  </span>
                  <h2 id="finale-title" class="finale__title">Press record.</h2>
                  <p class="finale__tag">
                    Setup is complete. {selectedSourceCount > 0 ? `${selectedSourceCount} source${selectedSourceCount === 1 ? "" : "s"} armed` : "No sources armed"} · {draftFrameRate} fps · {formatDuration(draftSegmentDuration)} segments{draftPauseCaptureOnInactivity ? ` · idle pause @ ${formatDuration(draftIdleTimeoutSeconds)}` : ""}.
                  </p>
                  {#if !processingReady}
                    <p class="hint hint--warn">A selected OCR or transcription model still needs to finish installing before setup can complete.</p>
                  {/if}

                  <div class="finale__chips">
                    <span class="chip chip--lg" data-on={draftCaptureScreen}>Screen</span>
                    <span class="chip chip--lg" data-on={draftCaptureMicrophone}>Mic</span>
                    <span class="chip chip--lg" data-on={draftCaptureSystemAudio && draftCaptureScreen}>Sys audio</span>
                    <span class="chip chip--lg" data-on={draftOcrEnabled}>OCR</span>
                    <span class="chip chip--lg" data-on={draftTranscriptionEnabled}>Transcript</span>
                  </div>

                  <div class="finale__cta">
                    <button
                      type="button"
                      class="btn btn--primary btn--cta"
                      onclick={() => finish(true)}
                      disabled={!canGoNext}
                    >
                      <span class="btn__rec btn__rec--lg" aria-hidden="true"></span>
                      {starting ? "Starting…" : "Start recording"}
                    </button>
                    <button
                      type="button"
                      class="btn btn--link"
                      onclick={() => finish(false)}
                      disabled={!canGoNext}
                    >
                      {completing && !starting ? "Opening…" : "Just open the dashboard →"}
                    </button>
                  </div>

                  <p class="finale__foot">You can change anything later in <em>Settings</em>.</p>
                </div>
              </section>
            {/if}
          </div>
        {/key}
      {:else}
        <div class="stage">
          <div class="stage__bg" aria-hidden="true">
            <div class="scene-grid"></div>
            <div class="scene-halo"></div>
          </div>
          {#key activeStep}
            <div class="stage__view">
              {#if activeStep === "permissions"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.permissions.eyebrow}
                  title="Access"
                  subtitle={bayMeta.permissions.subtitle}
                >
                  {#snippet status()}
                    <ArmStatus armed={grantedCount > 0} pendingLabel="Awaiting access" armedLabel={`${grantedCount}/3 ready`} />
                  {/snippet}

                  <ul class="perm-list">
                    {#each [
                      { key: "screen", name: "Screen recording" },
                      { key: "microphone", name: "Microphone" },
                      { key: "systemAudio", name: "System audio" },
                    ] as p}
                      {@const value = permissions?.[p.key as PermissionKey]}
                      {@const tone = permissionTone(value)}
                      {@const action = permissionAction(value)}
                      <li class="perm perm--{tone}">
                        <span class="perm__name">{p.name}</span>
                        <span class="perm__right">
                          <span class="perm__pill">
                            <span class="perm__dot"></span>{permissionLabel(value)}
                          </span>
                          {#if action}
                            <button
                              type="button"
                              class="btn btn--ghost btn--sm perm__action"
                              onclick={() => requestPermission(p.key as PermissionKey)}
                              disabled={requestingPerm !== null}
                            >
                              {requestingPerm === p.key ? "Requesting…" : action.label}
                            </button>
                          {/if}
                        </span>
                      </li>
                    {/each}
                  </ul>

                  <p class="hint">Grant each source to arm the recorder now — macOS also prompts when recording starts. Once a permission is denied, use <em>Open Settings</em> to enable it under <em>Privacy &amp; Security</em>.</p>

                  <div class="row">
                    <button type="button" class="btn btn--ghost btn--sm" onclick={refreshPermissions} disabled={refreshingPerms}>
                      {refreshingPerms ? "Checking…" : "Refresh"}
                    </button>
                    <button type="button" class="btn btn--ghost btn--sm" onclick={applyRecommendedSetup} disabled={!canGoNext}>
                      {applyingRecommended ? "Arming…" : "Use recommended setup"}
                    </button>
                  </div>
                </SceneShell>
              {:else if activeStep === "sources"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.sources.eyebrow}
                  title="Capture"
                  subtitle={bayMeta.sources.subtitle}
                >
                  {#snippet status()}
                    <ArmStatus armed={armedSources} pendingLabel="No sources" armedLabel={`${selectedSourceCount} armed`} />
                  {/snippet}

                  <div class="settings-stack">
                    <Switch bind:checked={draftCaptureScreen} label="Screen" description="Capture the display" />
                    <div class="settings-divider"></div>
                    <Switch bind:checked={draftCaptureMicrophone} label="Microphone" description="Capture microphone audio" />
                    <div class="settings-divider"></div>
                    <Switch
                      bind:checked={draftCaptureSystemAudio}
                      disabled={!draftCaptureScreen}
                      label="System audio"
                      description="Capture Mac system audio when supported"
                    />
                  </div>
                  {#if !draftCaptureScreen}
                    <p class="hint hint--warn">System audio requires screen capture.</p>
                  {/if}

                  <div class="grid-2">
                    <div class="settings-group">
                      <span class="group-label">Frame rate</span>
                      <Slider bind:value={draftFrameRate} min={1} max={120} step={1} label="FPS" unit=" fps" />
                    </div>
                    <div class="settings-group">
                      <span class="group-label">Segment</span>
                      <Slider
                        bind:value={draftSegmentDuration}
                        min={10}
                        max={300}
                        step={10}
                        label="Duration"
                        formatValue={formatDuration}
                      />
                    </div>
                  </div>

                  <AdvancedReveal label="Idle handling">
                    <div class="settings-stack">
                      <Switch
                        bind:checked={draftPauseCaptureOnInactivity}
                        label="Pause when idle"
                        description="Resume automatically when activity returns"
                      />
                    </div>
                    {#if draftPauseCaptureOnInactivity}
                      <div class="settings-group">
                        <span class="group-label">Idle timeout</span>
                        <Slider
                          bind:value={draftIdleTimeoutSeconds}
                          min={5}
                          max={300}
                          step={5}
                          label="Timeout"
                          formatValue={formatDuration}
                        />
                      </div>
                    {/if}
                  </AdvancedReveal>

                  {#if selectedSourceCount === 0}
                    <p class="hint hint--err">Enable at least one source to continue.</p>
                  {/if}
                </SceneShell>
              {:else if activeStep === "video"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.video.eyebrow}
                  title="Lens"
                  subtitle={bayMeta.video.subtitle}
                >
                  {#snippet status()}
                    <ArmStatus armed={armedVideo} pendingLabel="Check inputs" armedLabel="Calibrated" />
                  {/snippet}

                  <div class="video-grid">
                    <div class="settings-group">
                      <span class="group-label">Screen resolution</span>
                      <ScreenResolutionControl
                        bind:mode={draftResolutionMode}
                        bind:preset={draftResolutionPreset}
                        bind:widthRaw={customWidthRaw}
                        bind:heightRaw={customHeightRaw}
                        customErrors={customResolutionErrors}
                      />
                    </div>

                    <div class="settings-group">
                      <span class="group-label">Video bitrate</span>
                      <VideoBitrateControl
                        bind:mode={draftBitrateMode}
                        bind:preset={draftBitratePreset}
                        bind:customMbpsRaw={draftCustomMbpsRaw}
                        customMbps={draftCustomMbps}
                        customErrors={customBitrateErrors}
                      />
                      <p class="hint">Bitrate applies on the ScreenCaptureKit path. Older systems keep the macOS default.</p>
                    </div>
                  </div>
                </SceneShell>
              {:else if activeStep === "storage"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.storage.eyebrow}
                  title="Vault"
                  subtitle={bayMeta.storage.subtitle}
                >
                  {#snippet status()}
                    <ArmStatus armed={armedStorage} pendingLabel="Path needed" armedLabel="Vault set" />
                  {/snippet}

                  <div class="settings-group">
                    <span class="group-label">Save directory</span>
                    <input
                      type="text"
                      class="text-input"
                      class:text-input--empty={!draftSaveDirectory.trim()}
                      bind:value={draftSaveDirectory}
                      placeholder="/Users/you/mnema"
                      spellcheck="false"
                      autocomplete="off"
                    />
                    <p class="hint">Layout: <code>&lt;dir&gt;/db/app.sqlite3</code> · <code>&lt;dir&gt;/recordings/YYYY/MM/DD/</code></p>
                  </div>

                  <div class="settings-group">
                    <SelectMenu
                      value={draftRetentionPolicy}
                      onValueChange={(v) => { draftRetentionPolicy = v as RetentionPolicy; }}
                      label="Retention"
                      options={[
                        { value: "never", label: "Never" },
                        { value: "days_7", label: "7 days" },
                        { value: "days_14", label: "14 days" },
                        { value: "days_30", label: "30 days" },
                      ]}
                    />
                  </div>

                  <AdvancedReveal label="Cache & startup">
                    <div class="settings-group">
                      <SelectMenu
                        value={String(draftPreviewCacheTtlSeconds)}
                        onValueChange={(v) => { draftPreviewCacheTtlSeconds = parseInt(v, 10); }}
                        label="Preview cache"
                        options={[
                          { value: "0", label: "Disabled" },
                          { value: "300", label: "5 minutes" },
                          { value: "900", label: "15 minutes" },
                          { value: "3600", label: "1 hour" },
                          { value: "21600", label: "6 hours" },
                          { value: "86400", label: "24 hours" },
                        ]}
                      />
                    </div>
                    <div class="settings-stack">
                      <Switch
                        bind:checked={draftAutoStart}
                        label="Auto-start on launch"
                        description="Begin recording when app opens"
                      />
                    </div>
                  </AdvancedReveal>
                </SceneShell>
              {:else if activeStep === "privacy"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.privacy.eyebrow}
                  title="Shield"
                  subtitle={bayMeta.privacy.subtitle}
                  comboboxOpen={appPrivacyExclusion.comboboxOpen}
                >
                  {#snippet status()}
                    <ArmStatus armed={armedPrivacy} pendingLabel="Open" armedLabel={`${draftExcludedApps.length} excluded`} />
                  {/snippet}

                  <AppPrivacyExclusion
                    controller={appPrivacyExclusion}
                    comboboxListId="onboarding-privacy-app-combobox-list"
                  />
                </SceneShell>
              {:else if activeStep === "processing"}
                <SceneShell
                  index={bayIndex}
                  eyebrow={bayMeta.processing.eyebrow}
                  title="Mind"
                  subtitle={bayMeta.processing.subtitle}
                >
                  {#snippet status()}
                    <ArmStatus armed={processingReady} pendingLabel="Preparing" armedLabel="Ready" />
                  {/snippet}

                  <div
                    class="process-tabs"
                    role="tablist"
                    aria-label="Processing settings"
                    tabindex="-1"
                    onkeydown={handleProcessingTabKeydown}
                  >
                    {#each processingTabs as tab (tab.id)}
                      <button
                        type="button"
                        id={`processing-tab-${tab.id}`}
                        class="process-tab"
                        class:process-tab--active={activeProcessingPanel === tab.id}
                        role="tab"
                        aria-selected={activeProcessingPanel === tab.id}
                        aria-controls={`processing-panel-${tab.id}`}
                        tabindex={activeProcessingPanel === tab.id ? 0 : -1}
                        onkeydown={handleProcessingTabKeydown}
                        onclick={() => { activeProcessingPanel = tab.id; }}
                      >
                        {tab.label}
                      </button>
                    {/each}
                  </div>

                  {#if requiresOcrAvailability && selectedOcrDownloadRunning}
                    <p class="hint hint--warn">Finish is blocked until the selected OCR model download completes.</p>
                  {:else if requiresTranscriptionAvailability && selectedTranscriptionDownloadRunning}
                    <p class="hint hint--warn">Finish is blocked until the selected transcription model download completes.</p>
                  {:else if requiresOcrAvailability && !selectedOcrModel?.available}
                    <p class="hint hint--warn">Finish is blocked until the selected OCR model is available.</p>
                  {:else if requiresTranscriptionAvailability && !selectedTranscriptionModel?.available}
                    <p class="hint hint--warn">Finish is blocked until the selected transcription model is available.</p>
                  {/if}

                  <div class="processing-grid">
                    {#if activeProcessingPanel === "ocr"}
                    <div
                      class="settings-group"
                      id="processing-panel-ocr"
                      role="tabpanel"
                      aria-labelledby="processing-tab-ocr"
                      tabindex="0"
                    >
                      <div class="settings-stack">
                        <Switch
                          bind:checked={draftOcrEnabled}
                          label="Enable OCR"
                          description="Queue OCR for captured screen frames."
                        />
                      </div>
                      <RadioGroup
                        value={draftOcrProvider}
                        onValueChange={chooseOcrProvider}
                        disabled={!draftOcrEnabled}
                        label="Provider"
                        options={ocrProviderOptions.length > 0 ? ocrProviderOptions : fallbackOcrProviderOptions}
                      />
                      <SelectMenu
                        value={draftOcrModelId ?? "__os_managed__"}
                        onValueChange={chooseOcrModel}
                        disabled={!draftOcrEnabled}
                        label="Model"
                        options={ocrModelOptions.length > 0 ? ocrModelOptions : [
                          { value: draftOcrModelId ?? "__os_managed__", label: "Loading model options" },
                        ]}
                      />
                      {#if draftOcrEnabled && draftOcrProvider === "apple_vision"}
                        <AdvancedReveal label="OCR tuning">
                          <RadioGroup
                            bind:value={draftOcrRecognitionMode}
                            disabled={!draftOcrEnabled}
                            label="Recognition mode"
                            options={[
                              {
                                value: "fast",
                                label: "Fast",
                                description: "Lower CPU usage for continuous capture.",
                              },
                              {
                                value: "accurate",
                                label: "Accurate",
                                description: "Higher recognition quality with more processing cost.",
                              },
                            ]}
                          />
                          <div class="settings-stack">
                            <Switch
                              bind:checked={draftOcrLanguageCorrection}
                              disabled={!draftOcrEnabled}
                              label="Language correction"
                              description="Spend extra OCR work correcting recognized text."
                            />
                          </div>
                        </AdvancedReveal>
                      {/if}
                      {#if draftOcrEnabled}
                        <div class="mini-status">
                          {#if ocrModelError}
                            <p class="hint hint--warn">Failed to load OCR model status: {ocrModelError}</p>
                          {:else if selectedOcrModel}
                            <div class="model-status" class:model-status--available={selectedOcrModel.available}>
                              <div>
                                <div class="model-status__title">{selectedOcrModel.displayName}</div>
                                <div class="model-status__meta">{ocrStatusLabel(selectedOcrModel)}</div>
                              </div>
                              <span class="model-status__pill">{selectedOcrModel.available ? "available" : "unavailable"}</span>
                            </div>
                            {#if selectedOcrModel.management === "app_managed"}
                              {#if selectedOcrModel.download}
                                {#if selectedOcrDownloadRunning}
                                  <div class="download-progress" aria-live="polite">
                                    <div class="download-progress__bar">
                                      <span style={`width: ${selectedOcrDownloadPercent ?? 8}%`}></span>
                                    </div>
                                    <p class="hint">
                                      {selectedOcrDownloadProgress?.status ?? "downloading"}
                                      {#if selectedOcrDownloadPercent !== null} · {selectedOcrDownloadPercent}%{/if}
                                      {#if selectedOcrDownloadProgress?.message} · {selectedOcrDownloadProgress.message}{/if}
                                    </p>
                                    <button type="button" class="btn btn--ghost btn--sm" onclick={cancelSelectedOcrModelDownload} disabled={cancellingOcrDownload}>
                                      {cancellingOcrDownload ? "Cancelling" : "Cancel download"}
                                    </button>
                                  </div>
                                {:else}
                                  <button type="button" class="btn btn--ghost btn--sm" onclick={startSelectedOcrModelDownload} disabled={startingOcrDownload || selectedOcrModel.available}>
                                    {startingOcrDownload ? "Starting" : `Download OCR model (${formatBytes(selectedOcrModel.download.byteSize)})`}
                                  </button>
                                {/if}
                              {:else if !selectedOcrModel.available}
                                <p class="hint hint--warn">
                                  {selectedOcrModel.provider === "tesseract"
                                    ? "Tesseract still needs a published self-contained runtime bundle before in-app download can work."
                                    : "No downloadable OCR artifact is available for this model."}
                                </p>
                              {/if}
                              {#if ocrDownloadError}
                                <p class="hint hint--warn">Download failed: {ocrDownloadError}</p>
                              {/if}
                            {:else}
                              <p class="hint">This OCR provider is managed by macOS.</p>
                            {/if}
                          {:else if loadingOcrModelStatus}
                            <p class="hint">Checking OCR models…</p>
                          {:else}
                            <p class="hint hint--warn">No OCR model status is available.</p>
                          {/if}
                        </div>
                      {:else}
                        <p class="hint">Screen recording can start without OCR while this is disabled. Existing OCR results remain visible.</p>
                      {/if}
                    </div>
                    {/if}

                    {#if activeProcessingPanel === "transcription"}
                    <div
                      class="settings-group"
                      id="processing-panel-transcription"
                      role="tabpanel"
                      aria-labelledby="processing-tab-transcription"
                      tabindex="0"
                    >
                      <div class="settings-stack">
                        <Switch
                          bind:checked={draftTranscriptionEnabled}
                          label="Enable transcription"
                          description="Master switch for source-specific speech-to-text."
                        />
                      </div>
                      <RadioGroup
                        value={draftTranscriptionProvider}
                        onValueChange={chooseTranscriptionProvider}
                        disabled={!draftTranscriptionEnabled}
                        label="Provider"
                        options={transcriptionProviderOptions.length > 0 ? transcriptionProviderOptions : fallbackTranscriptionProviderOptions}
                      />
                      <SelectMenu
                        value={draftTranscriptionModelId ?? "__os_managed__"}
                        onValueChange={chooseTranscriptionModel}
                        disabled={!draftTranscriptionEnabled}
                        label="Model"
                        options={transcriptionModelOptions.length > 0 ? transcriptionModelOptions : [
                          { value: draftTranscriptionModelId ?? "__os_managed__", label: "Loading model options" },
                        ]}
                      />

                      {#if draftTranscriptionEnabled}
                        <AdvancedReveal label="Sources & tuning">
                          <div class="settings-stack">
                            <Switch
                              bind:checked={draftTranscriptionMicrophoneEnabled}
                              label="Transcribe microphone"
                              description="Queue speech-to-text for committed microphone audio."
                            />
                            <Switch
                              bind:checked={draftTranscriptionSystemAudioEnabled}
                              label="Transcribe system audio"
                              description="Transcribe system audio only when speech is detected."
                            />
                          </div>
                          <label class="field-label" for="onboarding-transcription-language">Language</label>
                          <input
                            id="onboarding-transcription-language"
                            class="text-input"
                            bind:value={draftTranscriptionLanguage}
                            placeholder="auto"
                            spellcheck="false"
                            autocomplete="off"
                          />
                          {#if draftTranscriptionProvider === "parakeet"}
                            <RadioGroup
                              value={draftTranscriptionMemoryMode}
                              onValueChange={(value) => draftTranscriptionMemoryMode = value as AudioTranscriptionMemoryMode}
                              label="Memory mode"
                              options={[
                                { value: "balanced", label: "Balanced", description: "Unload ONNX sessions after idle timeout." },
                                { value: "low_memory", label: "Low memory", description: "Unload sessions after every transcription." },
                                { value: "performance", label: "Performance", description: "Keep sessions loaded for repeat jobs." },
                              ]}
                            />
                            {#if draftTranscriptionMemoryMode === "balanced"}
                              <label class="field-label" for="onboarding-transcription-idle-unload">Idle unload seconds</label>
                              <input
                                id="onboarding-transcription-idle-unload"
                                class="text-input"
                                type="number"
                                min="0"
                                max="86400"
                                step="1"
                                bind:value={draftTranscriptionIdleUnloadSeconds}
                              />
                            {/if}
                            <label class="field-label" for="onboarding-transcription-chunk-seconds">Chunk seconds</label>
                            <input
                              id="onboarding-transcription-chunk-seconds"
                              class="text-input"
                              type="number"
                              min="0"
                              max="3600"
                              step="1"
                              bind:value={draftTranscriptionChunkSeconds}
                            />
                          {/if}
                        </AdvancedReveal>
                      {/if}

                      {#if draftTranscriptionEnabled}
                        <div class="mini-status">
                          {#if transcriptionModelError}
                            <p class="hint hint--warn">Failed to load transcription model status: {transcriptionModelError}</p>
                          {:else if selectedTranscriptionModel}
                            <div class="model-status" class:model-status--available={selectedTranscriptionModel.available}>
                              <div>
                                <div class="model-status__title">{selectedTranscriptionModel.displayName}</div>
                                <div class="model-status__meta">{transcriptionStatusLabel(selectedTranscriptionModel)}</div>
                              </div>
                              <span class="model-status__pill">{selectedTranscriptionModel.available ? "available" : "unavailable"}</span>
                            </div>
                            {#if selectedTranscriptionModel.management === "app_managed"}
                              {#if selectedTranscriptionModel.download}
                                {#if selectedTranscriptionDownloadRunning}
                                  <div class="download-progress" aria-live="polite">
                                    <div class="download-progress__bar">
                                      <span style={`width: ${selectedTranscriptionDownloadPercent ?? 8}%`}></span>
                                    </div>
                                    <p class="hint">
                                      {selectedTranscriptionDownloadProgress?.status ?? "downloading"}
                                      {#if selectedTranscriptionDownloadPercent !== null} · {selectedTranscriptionDownloadPercent}%{/if}
                                      {#if selectedTranscriptionDownloadProgress?.message} · {selectedTranscriptionDownloadProgress.message}{/if}
                                    </p>
                                    <button type="button" class="btn btn--ghost btn--sm" onclick={cancelSelectedTranscriptionModelDownload} disabled={cancellingTranscriptionDownload}>
                                      {cancellingTranscriptionDownload ? "Cancelling" : "Cancel download"}
                                    </button>
                                  </div>
                                {:else}
                                  <button type="button" class="btn btn--ghost btn--sm" onclick={startSelectedTranscriptionModelDownload} disabled={startingTranscriptionDownload || selectedTranscriptionModel.available}>
                                    {startingTranscriptionDownload ? "Starting" : `Download transcription model (${formatBytes(selectedTranscriptionModel.download.byteSize)})`}
                                  </button>
                                {/if}
                              {:else if !selectedTranscriptionModel.available}
                                <p class="hint hint--warn">No downloadable artifact is available for this model.</p>
                              {/if}
                              {#if transcriptionDownloadError}
                                <p class="hint hint--warn">Download failed: {transcriptionDownloadError}</p>
                              {/if}
                            {:else}
                              <p class="hint">This provider is managed by macOS.</p>
                            {/if}
                          {:else if loadingTranscriptionModelStatus}
                            <p class="hint">Checking transcription models…</p>
                          {:else}
                            <p class="hint hint--warn">No transcription model status is available.</p>
                          {/if}
                        </div>
                      {:else}
                        <p class="hint">Microphone audio can be captured without transcription while this is disabled.</p>
                      {/if}
                    </div>
                    {/if}
                  </div>
                </SceneShell>
              {/if}
            </div>
          {/key}
        </div>
      {/if}
    {/if}
  </div>

  {#if !isWelcome}
    <footer class="ob__foot" class:ob__foot--minimal={!showChrome}>
      {#if error}
        <p class="ob__error">{error}</p>
      {:else if showChrome}
        <p class="ob__hint" aria-hidden="true"><kbd>↵</kbd> next</p>
      {:else}
        <span class="ob__foot-spacer"></span>
      {/if}
      <button type="button" class="btn btn--ghost" onclick={previousStep} disabled={!canGoBack}>Back</button>
      {#if showChrome}
        <button type="button" class="btn btn--primary" onclick={nextStep} disabled={!canGoNext}>
          {saving ? "Saving…" : "Continue"}
        </button>
      {/if}
    </footer>
  {:else if error}
    <footer class="ob__foot ob__foot--minimal">
      <p class="ob__error">{error}</p>
    </footer>
  {/if}
</section>

<style>
  /* ── Layout shell — match settings page rhythm ─────────────── */
  .ob {
    height: 100%;
    min-height: 100%;
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    gap: 10px;
  }

  /* ── Header (compact, like .page-header) ───────────────────── */
  .ob__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-bottom: 8px;
    border-bottom: 1px dashed var(--app-border);
  }
  .ob__title {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .ob__index {
    padding: 2px 6px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    border-radius: 3px;
    color: var(--app-accent);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    font-variant-numeric: tabular-nums;
  }
  .ob h1 {
    margin: 0;
    color: var(--app-text-strong);
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 1.1;
  }
  .ob__status {
    position: relative;
    padding-left: 12px;
    color: var(--app-text-subtle);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .ob__status::before {
    content: "";
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-subtle);
  }
  .ob__status[data-tone="ok"] { color: var(--app-accent); }
  .ob__status[data-tone="ok"]::before { background: var(--app-accent); box-shadow: 0 0 6px var(--app-accent-glow); }
  .ob__status[data-tone="pending"]::before { background: var(--app-warn); }

  /* ── Stage — full-bleed bay frame + persistent ambient backdrop ─
     One framed scene for the six middle bays. The backdrop (grid + halo)
     is a *sibling* of the keyed bay content, rendered once outside the
     `{#key}` so it stays continuous across steps. Critically, `.stage`
     carries no `overflow`/`transform`/`filter`: those would clip the
     non-portaled privacy combobox. The grid clips itself via `.stage__bg`. */
  .stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface-raised);
  }
  .stage::before {
    content: "";
    position: absolute;
    top: 0;
    left: 12px;
    right: 12px;
    height: 1px;
    background: linear-gradient(90deg, transparent, var(--app-accent-strong) 20%, var(--app-accent) 50%, var(--app-accent-strong) 80%, transparent);
    opacity: 0.4;
    z-index: 2;
  }
  .stage__bg {
    position: absolute;
    inset: 0;
    overflow: hidden;
    border-radius: inherit;
    pointer-events: none;
  }
  /* Dimmer than the welcome/finale backdrop — the middle stays "committed",
     never fully drenched, so the bookends keep their punch. */
  .scene-grid {
    position: absolute;
    inset: -2px;
    background-image:
      linear-gradient(var(--app-border) 1px, transparent 1px),
      linear-gradient(90deg, var(--app-border) 1px, transparent 1px);
    background-size: 22px 22px;
    opacity: 0.18;
    mask-image: radial-gradient(ellipse at 26% 0%, black 0%, transparent 72%);
  }
  .scene-halo {
    position: absolute;
    width: 340px;
    height: 340px;
    right: -90px;
    top: -120px;
    background: radial-gradient(circle, var(--app-accent-glow) 0%, transparent 65%);
    filter: blur(10px);
    opacity: 0.4;
    animation: drift 16s ease-in-out infinite;
  }
  .stage__view {
    position: relative;
    z-index: 1;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    animation: step-in 0.22s ease-out;
  }

  /* ── Compact bay density ───────────────────────────────────────
     Each step must fit the fixed frame without scrolling, so inside the
     stage we drop the secondary control *descriptions* (the labels carry the
     meaning on first run) and tighten the shared radio / disclosure / list
     paddings. Scoped to `.stage`, so the welcome and finale bookends keep
     their roomier treatment. */
  .stage :global(.switch-description) {
    display: none;
  }
  .stage :global(.rg-item) {
    padding: 5px 10px;
  }
  .stage :global(.settings-list-item) {
    padding: 6px;
  }
  .stage :global(.privacy-disclosure) {
    gap: 3px;
    padding: 8px 11px;
  }
  .stage :global(.privacy-disclosure p) {
    font-size: 10px;
    line-height: 1.4;
  }

  /* ── Body / scroll ─────────────────────────────────────────── */
  .ob__body {
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  /* While the Shield app-picker is open, lift the body above the footer and
     stop clipping so the non-portaled dropdown paints over both. The Shield
     bay's content is short, so dropping the scroll clip is safe. */
  .ob__body--combobox-open {
    overflow: visible;
    position: relative;
    z-index: 30;
  }
  .step-anim {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    animation: step-in 0.22s ease-out;
  }
  .ob--welcome .step-anim > *,
  .ob--final .step-anim > * { flex: 1 1 auto; min-height: 0; }
  @keyframes step-in {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ── Card — matches settings .card exactly ─────────────────── */
  .card {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 16px 14px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-raised);
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
  .card--loading {
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 28px;
  }
  .loader {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--app-border);
    border-top-color: var(--app-accent);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .loading-text {
    color: var(--app-text-muted);
    font-size: 11px;
  }

  /* ── Permissions list (compact rows) ───────────────────────── */
  .perm-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .perm {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 12px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
  }
  .perm__name {
    color: var(--app-text-strong);
    font-size: 12px;
    font-weight: 600;
  }
  .perm__right {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .perm__action {
    white-space: nowrap;
  }
  .perm__pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .perm__dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.6;
  }
  .perm--ok { border-color: var(--app-accent-border); }
  .perm--ok .perm__pill { color: var(--app-accent); border-color: var(--app-accent-border); background: var(--app-accent-bg); }
  .perm--ok .perm__dot { opacity: 1; box-shadow: 0 0 4px var(--app-accent-glow); }
  .perm--pending .perm__pill { color: var(--app-warn); border-color: var(--app-warn-border); background: var(--app-warn-bg); }
  .perm--blocked .perm__pill { color: var(--app-danger); border-color: var(--app-danger-border); background: var(--app-danger-bg); }

  /* ── Settings groups ───────────────────────────────────────── */
  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .group-label {
    color: var(--app-text-subtle);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  .settings-stack {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 11px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 4px;
  }
  .settings-divider {
    height: 1px;
    background: var(--app-border);
  }
  .process-tabs {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 4px;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
  }
  .process-tab {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    height: 26px;
    padding: 0 10px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .process-tab:not(.process-tab--active):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
  }
  .process-tab--active {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .process-tab:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }
  .grid-2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  /* Lens bay: resolution + bitrate side by side so the step fits the frame
     without scrolling. The onboarding window is never narrower than 820px;
     the single-column fallback only matters if that minimum ever changes. */
  .video-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px 18px;
    align-items: start;
  }
  @media (max-width: 640px) {
    .video-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .processing-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }

  /* ── Hints ─────────────────────────────────────────────────── */
  .hint {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.5;
  }
  .hint code {
    padding: 0 4px;
    border-radius: 2px;
    background: var(--app-surface);
    color: var(--app-text);
    font-family: inherit;
    font-size: 10px;
  }
  .hint em { font-style: normal; color: var(--app-text); }
  .hint--warn { color: var(--app-warn); font-weight: 600; }
  .hint--err { color: var(--app-danger); font-weight: 600; }

  /* ── Inputs ────────────────────────────────────────────────── */
  .text-input {
    width: 100%;
    padding: 7px 10px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    color: var(--app-text);
    font-family: inherit;
    font-size: 12px;
    outline: none;
    transition: border-color 0.12s;
  }
  .text-input:focus { border-color: var(--app-accent); }
  .text-input--empty { border-color: var(--app-warn-border); }
  .text-input::placeholder { color: var(--app-text-faint); }
  .field-label {
    color: var(--app-text-subtle);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  .mini-status {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 2px;
  }
  .model-status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-warn) 8%, transparent);
  }
  .model-status--available {
    border-color: color-mix(in srgb, var(--app-accent) 42%, var(--app-border));
    background: color-mix(in srgb, var(--app-accent) 8%, transparent);
  }
  .model-status__title {
    color: var(--app-text);
    font-size: 12px;
    font-weight: 700;
  }
  .model-status__meta {
    margin-top: 2px;
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .model-status__pill {
    flex-shrink: 0;
    color: var(--app-text-muted);
    font-size: 8px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .download-progress {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .download-progress__bar {
    height: 6px;
    overflow: hidden;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-hover);
  }
  .download-progress__bar span {
    display: block;
    height: 100%;
    min-width: 8%;
    border-radius: inherit;
    background: var(--app-accent);
    transition: width 0.15s ease;
  }

  /* ── Chip (used in finale) ─────────────────────────────────── */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 3px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-faint);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .chip::before {
    content: "";
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.5;
  }
  .chip[data-on="true"] {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .chip[data-on="true"]::before { opacity: 1; box-shadow: 0 0 4px var(--app-accent-glow); }

  /* ── Rows / footer ─────────────────────────────────────────── */
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ob__foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 8px;
    border-top: 1px dashed var(--app-border);
  }
  .ob__hint {
    flex: 1;
    margin: 0;
    color: var(--app-text-faint);
    font-size: 9px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  .ob__hint kbd {
    display: inline-block;
    padding: 0 5px;
    margin-right: 4px;
    border: 1px solid var(--app-border-strong);
    border-radius: 3px;
    background: var(--app-surface);
    color: var(--app-text);
    font-family: inherit;
    font-size: 9px;
  }
  .ob__error {
    flex: 1;
    margin: 0;
    padding: 5px 8px;
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    background: var(--app-danger-bg);
    color: var(--app-danger);
    font-size: 10.5px;
    line-height: 1.35;
  }

  /* ── Buttons (match settings) ──────────────────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 7px 14px;
    border: 1px solid transparent;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    outline: none;
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s;
  }
  .btn:disabled { opacity: 0.35; cursor: not-allowed; }
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
  .btn--primary {
    background: var(--app-accent);
    color: var(--app-bg);
    border-color: var(--app-accent);
  }
  .btn--primary:not(:disabled):hover {
    background: var(--app-accent-strong);
    border-color: var(--app-accent-strong);
  }
  .btn--sm { padding: 4px 10px; font-size: 9px; }
  .btn__rec {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-bg);
    box-shadow: 0 0 0 1.5px var(--app-bg);
    opacity: 0.7;
  }
  .btn:focus-visible {
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  /* ── Welcome (first step) ──────────────────────────────────── */
  .ob--welcome,
  .ob--final {
    grid-template-rows: minmax(0, 1fr) auto;
  }
  .ob--welcome .ob__body,
  .ob--final .ob__body {
    overflow: hidden;
  }

  .welcome,
  .finale {
    position: relative;
    height: 100%;
    min-height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface-raised);
    overflow: hidden;
  }
  .welcome__bg,
  .finale__bg {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
  }
  .welcome__grid {
    position: absolute;
    inset: -2px;
    background-image:
      linear-gradient(var(--app-border) 1px, transparent 1px),
      linear-gradient(90deg, var(--app-border) 1px, transparent 1px);
    background-size: 22px 22px;
    opacity: 0.35;
    mask-image: radial-gradient(ellipse at 30% 35%, black 0%, transparent 75%);
  }
  .welcome__halo {
    position: absolute;
    width: 360px;
    height: 360px;
    left: -80px;
    top: -120px;
    background: radial-gradient(circle, var(--app-accent-glow) 0%, transparent 65%);
    filter: blur(8px);
    opacity: 0.7;
    animation: drift 14s ease-in-out infinite;
  }
  @keyframes drift {
    0%, 100% { transform: translate(0, 0); }
    50% { transform: translate(40px, 25px); }
  }

  .welcome__inner {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 32px 36px;
    max-width: 520px;
  }
  .welcome__eyebrow {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    align-self: flex-start;
    padding: 3px 10px 3px 8px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.18em;
    text-transform: uppercase;
  }
  .welcome__pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 0 0 var(--app-accent-glow);
    animation: pulse 1.8s ease-out infinite;
  }
  @keyframes pulse {
    0% { box-shadow: 0 0 0 0 var(--app-accent-glow); }
    70% { box-shadow: 0 0 0 8px transparent; }
    100% { box-shadow: 0 0 0 0 transparent; }
  }
  .welcome__title {
    margin: 0;
    color: var(--app-text-strong);
    font-size: 38px;
    font-weight: 700;
    line-height: 1.02;
    letter-spacing: -0.01em;
  }
  .welcome__title em {
    font-style: normal;
    color: var(--app-accent);
    position: relative;
  }
  .welcome__title em::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 2px;
    height: 6px;
    background: var(--app-accent-glow);
    opacity: 0.5;
    z-index: -1;
  }
  .welcome__tag {
    margin: 0;
    color: var(--app-text);
    font-size: 12.5px;
    line-height: 1.55;
    max-width: 44ch;
  }
  .welcome__loop {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }
  .welcome__loop li {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .welcome__loop li:not(:last-child)::after {
    content: "→";
    margin: 0 8px;
    color: var(--app-accent);
    opacity: 0.6;
  }
  .welcome__loop li span {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 4px var(--app-accent-glow);
  }
  .welcome__cta {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 8px;
  }
  .welcome__meta {
    color: var(--app-text-muted);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }
  .welcome__accel {
    align-self: flex-start;
    margin-top: -4px;
    padding-left: 0;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    font-size: 9.5px;
    font-weight: 700;
  }

  /* ── Finale (last step) ────────────────────────────────────── */
  .finale__rings {
    position: absolute;
    width: 540px;
    height: 540px;
    border-radius: 50%;
    border: 1px solid var(--app-accent-border);
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    opacity: 0.35;
    animation: ring-pulse 4s ease-in-out infinite;
  }
  .finale__rings--alt {
    width: 360px;
    height: 360px;
    border-color: var(--app-accent);
    opacity: 0.18;
    animation-delay: 1.6s;
  }
  @keyframes ring-pulse {
    0%, 100% { transform: translate(-50%, -50%) scale(0.92); opacity: 0.15; }
    50% { transform: translate(-50%, -50%) scale(1.05); opacity: 0.5; }
  }

  .finale__inner {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 14px;
    padding: 32px 36px;
    max-width: 520px;
  }
  .finale__crest {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 3px 12px;
    border-radius: 999px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.18em;
    text-transform: uppercase;
  }
  .finale__crest-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }
  .finale__title {
    margin: 0;
    color: var(--app-text-strong);
    font-size: 42px;
    font-weight: 700;
    letter-spacing: -0.015em;
    line-height: 1;
  }
  .finale__tag {
    margin: 0;
    color: var(--app-text);
    font-size: 12px;
    line-height: 1.5;
    max-width: 46ch;
  }
  .finale__chips {
    display: inline-flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin: 4px 0 8px;
  }
  .chip--lg {
    padding: 4px 10px;
    font-size: 10px;
  }

  .finale__cta {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
  }
  .finale__foot {
    margin: 6px 0 0;
    color: var(--app-text-muted);
    font-size: 10px;
  }
  .finale__foot em { font-style: normal; color: var(--app-text); }

  .btn--lg {
    padding: 10px 20px;
    font-size: 12px;
  }
  .btn--cta {
    padding: 12px 26px;
    font-size: 13px;
    letter-spacing: 0.12em;
    box-shadow: 0 0 0 0 var(--app-accent-glow);
    animation: cta-glow 2.4s ease-in-out infinite;
  }
  @keyframes cta-glow {
    0%, 100% { box-shadow: 0 0 0 0 var(--app-accent-glow); }
    50% { box-shadow: 0 0 0 6px transparent; }
  }
  .btn__arrow {
    display: inline-block;
    transition: transform 0.18s ease;
  }
  .btn--lg:not(:disabled):hover .btn__arrow { transform: translateX(3px); }
  .btn__rec--lg { width: 9px; height: 9px; }
  .btn--link {
    background: transparent;
    border-color: transparent;
    color: var(--app-text-muted);
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: none;
    padding: 4px 8px;
  }
  .btn--link:not(:disabled):hover {
    color: var(--app-accent);
    background: transparent;
  }

  .ob__foot--minimal { border-top: 0; padding-top: 0; }
  .ob__foot-spacer { flex: 1; }

  /* ── Narrow widths (min window 640px) ──────────────────────── */
  @media (max-width: 600px) {
    .grid-2 { grid-template-columns: 1fr; }
    .welcome__inner,
    .finale__inner { padding: 22px 20px; }
    .welcome__title { font-size: 30px; }
    .finale__title { font-size: 32px; }
    .welcome__cta { flex-direction: column; align-items: flex-start; gap: 8px; }
  }

  @media (prefers-reduced-motion: reduce) {
    .step-anim, .stage__view, .loader,
    .scene-halo, .welcome__halo, .welcome__pulse,
    .finale__rings, .btn--cta { animation: none; }
  }
</style>
