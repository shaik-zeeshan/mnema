<script lang="ts">
  import { page } from "$app/stores";
  import { tick } from "svelte";
  import { Portal } from "bits-ui";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import AppPrivacyExclusionPrompt from "$lib/components/AppPrivacyExclusionPrompt.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
  import { createAppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
  import {
    DEFAULT_KEYBOARD_BINDINGS,
    EDITABLE_SHORTCUT_ACTIONS,
    getShortcutBinding,
    normalizeShortcutBinding,
    parseShortcutBinding,
    reservedShortcutConflict,
    setShortcutBinding,
    shortcutBindingFromKeyboardEvent,
    shortcutConflictScope,
    shortcutScopesConflict,
    withKeyboardBindingDefaults,
    type EditableShortcutAction,
    type EditableShortcutActionId,
  } from "$lib/keyboard-bindings.svelte";
  import { setDeveloperOptionsEnabled } from "$lib/developer-options.svelte";
  import { setAppearance } from "$lib/theme.svelte";
  import type {
    ActivityMode,
    AppearanceSetting,
    AskAiModel,
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
    RecordingSettingsDomainUpdateResponse,
    SettingsOwnershipDomain,
    UpdateAccessSettingsRequest,
    UpdateCaptureSourceSettingsRequest,
    UpdateCaptureTimingSettingsRequest,
    UpdateDeveloperSettingsRequest,
    UpdateDisplaySettingsRequest,
    UpdateInactivitySettingsRequest,
    UpdateMetadataSettingsRequest,
    UpdateProcessingSettingsRequest,
    UpdateStorageSettingsRequest,
    UpdateVideoSettingsRequest,
    UpdateAiRuntimeSettingsRequest,
    UpdateUserContextSettingsRequest,
    DerivationBudgetTier,
    AiEngineKind,
    AiCloudProvider,
    AiLocalKind,
    AiRuntimeStatus,
    AiRuntimeTestResult,
    UserContextStatus,
    UserContextDerivationRunResult,
    Activity,
    Conclusion,
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
    PersonProfileDto,
    KeyboardBindingsSettings,
    AppUpdateChannel,
    AppUpdateStatus,
  } from "$lib/types";

  const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
  const RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT = "recording_settings_domain_changed";
  const APP_UPDATE_STATUS_CHANGED_EVENT = "app_update_status_changed";
  const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT = "audio_transcription_model_download_progress";
  const SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT = "speaker_analysis_model_download_progress";
  const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT = "ocr_model_download_progress";
  const SELECTABLE_OCR_PROVIDERS: readonly OcrProvider[] = ["apple_vision", "tesseract"];

  // Canonical project links surfaced in the About tab. The GitHub repo is the
  // app's source of truth for releases — the updater pulls latest.json from it.
  const ABOUT_REPO_URL = "https://github.com/shaik-zeeshan/mnema";
  const ABOUT_RELEASES_URL = "https://github.com/shaik-zeeshan/mnema/releases";

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

  type PiRuntimeStatus = {
    source: "managed" | "path" | "unmanaged" | "missing";
    executablePath: string | null;
    version: string | null;
    minimumVersion: string;
    versionOk: boolean;
    authJsonPath: string;
    authJsonExists: boolean;
    ready: boolean;
    reason: "pi_not_found" | "pi_version_unavailable" | "pi_version_too_old" | "pi_auth_missing" | string | null;
    providerCount?: number;
    providerConfigured?: boolean;
    authJsonProviderCount?: number;
    authJsonProviderConfigured?: boolean;
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

  type AutosaveRecordingDomain = Extract<
    SettingsOwnershipDomain,
    | "capture_sources"
    | "capture_timing"
    | "video"
    | "storage"
    | "display"
    | "metadata"
    | "inactivity"
    | "processing"
    | "developer"
    | "access"
    | "ai_runtime"
    | "user_context"
  >;
  type RecordingSettingsDraftDomain = AutosaveRecordingDomain | "app_privacy_exclusion";

  type RecordingDomainRequest =
    | UpdateCaptureSourceSettingsRequest
    | UpdateCaptureTimingSettingsRequest
    | UpdateVideoSettingsRequest
    | UpdateStorageSettingsRequest
    | UpdateDisplaySettingsRequest
    | UpdateMetadataSettingsRequest
    | UpdateInactivitySettingsRequest
    | UpdateProcessingSettingsRequest
    | UpdateDeveloperSettingsRequest
    | UpdateAccessSettingsRequest
    | UpdateAiRuntimeSettingsRequest
    | UpdateUserContextSettingsRequest;

  const RECORDING_AUTOSAVE_DOMAINS: readonly AutosaveRecordingDomain[] = [
    "capture_sources",
    "capture_timing",
    "video",
    "storage",
    "display",
    "metadata",
    "inactivity",
    "processing",
    "developer",
    "access",
    "ai_runtime",
    "user_context",
  ];

  const RECORDING_DRAFT_DOMAINS: readonly RecordingSettingsDraftDomain[] = [
    ...RECORDING_AUTOSAVE_DOMAINS,
    "app_privacy_exclusion",
  ];

  const RECORDING_DOMAIN_COMMANDS: Record<AutosaveRecordingDomain, string> = {
    capture_sources: "update_capture_source_settings",
    capture_timing: "update_capture_timing_settings",
    video: "update_video_settings",
    storage: "update_storage_settings",
    display: "update_display_settings",
    metadata: "update_metadata_settings",
    inactivity: "update_inactivity_settings",
    processing: "update_processing_settings",
    developer: "update_developer_settings",
    access: "update_access_settings",
    ai_runtime: "update_ai_runtime_settings",
    user_context: "update_user_context_settings",
  };

  function makeRecordingDomainState<T>(value: T): Record<RecordingSettingsDraftDomain, T> {
    return Object.fromEntries(
      RECORDING_DRAFT_DOMAINS.map((domain) => [domain, value])
    ) as Record<RecordingSettingsDraftDomain, T>;
  }

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
  let draftAskAiEnabled = $state(false);
  // Tool-call cap. Persisted as a single number where 0 = no cap; the UI splits
  // that into a "limit on/off" toggle plus the numeric value (kept around for
  // re-enabling so toggling off then on restores the previous number).
  const ASK_AI_DEFAULT_TOOL_CALL_LIMIT = 12;
  let draftAskAiLimitToolCalls = $state(true);
  let draftAskAiMaxToolCalls = $state(ASK_AI_DEFAULT_TOOL_CALL_LIMIT);
  // Effective persisted value: 0 when the cap is off, else the chosen number
  // (floored to 1 so an empty/invalid input never silently becomes unlimited).
  let effectiveAskAiMaxToolCalls = $derived(
    draftAskAiLimitToolCalls
      ? Math.max(1, Math.floor(draftAskAiMaxToolCalls || ASK_AI_DEFAULT_TOOL_CALL_LIMIT))
      : 0,
  );
  // Quick Recall model selection. Empty string means "let PI pick its default".
  // `askAiModels` is the list discovered from the user's PI runtime.
  let draftAskAiModel = $state("");
  let askAiModels = $state<AskAiModel[]>([]);
  let askAiModelsLoading = $state(false);
  let askAiModelsError = $state<string | null>(null);
  // Editable combobox state: the text the user types to filter, whether the
  // dropdown is open, and the keyboard-highlighted row. The panel is portaled to
  // <body> and fixed-positioned (computed from the input rect) so no overflow or
  // transform ancestor in the settings layout can clip it.
  let askAiModelOpen = $state(false);
  let askAiModelQuery = $state("");
  let askAiModelHighlight = $state(0);
  let askAiModelInputEl = $state<HTMLInputElement | null>(null);
  let askAiModelPanelStyle = $state("");

  function updateAskAiModelPanelPosition() {
    const el = askAiModelInputEl;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    // Anchor the panel above the input: pin its bottom just over the input's top
    // so it opens upward, away from the clipped bottom edge of the settings card.
    askAiModelPanelStyle = `position: fixed; bottom: ${window.innerHeight - rect.top + 4}px; left: ${rect.left}px; width: ${rect.width}px;`;
  }

  function openAskAiModelMenu() {
    askAiModelOpen = true;
    updateAskAiModelPanelPosition();
  }

  // While the menu is open, keep it pinned under the input as the page scrolls or
  // resizes (capture phase catches scrolling inner containers, not just window).
  $effect(() => {
    if (!askAiModelOpen) return;
    const handler = () => updateAskAiModelPanelPosition();
    window.addEventListener("scroll", handler, true);
    window.addEventListener("resize", handler);
    return () => {
      window.removeEventListener("scroll", handler, true);
      window.removeEventListener("resize", handler);
    };
  });

  function askAiModelLabel(value: string): string {
    if (!value) return "Use PI default";
    const match = askAiModels.find((model) => model.value === value);
    if (!match) return value;
    return match.provider ? `${match.name} (${match.provider})` : match.name;
  }

  // All selectable entries: the "PI default" sentinel plus every discovered
  // model. `sublabel` shows the provider:id so ids stay recognizable.
  let askAiModelEntries = $derived([
    {
      value: "",
      label: "Use PI default",
      sublabel: "Follows the model configured in your PI runtime",
    },
    ...askAiModels.map((model) => ({
      value: model.value,
      label: model.provider ? `${model.name} (${model.provider})` : model.name,
      sublabel: model.value,
    })),
  ]);

  // Substring filter on the typed query. When the query still equals the
  // committed selection's label (e.g. just focused), show the whole list.
  let askAiModelFiltered = $derived.by(() => {
    const query = askAiModelQuery.trim().toLowerCase();
    if (!query || query === askAiModelLabel(draftAskAiModel).toLowerCase()) {
      return askAiModelEntries;
    }
    return askAiModelEntries.filter(
      (entry) =>
        entry.label.toLowerCase().includes(query) ||
        entry.value.toLowerCase().includes(query),
    );
  });

  // Keep the input text in sync with the committed selection while the dropdown
  // is closed (covers settings/model loads that change the resolved label).
  $effect(() => {
    if (!askAiModelOpen) {
      askAiModelQuery = askAiModelLabel(draftAskAiModel);
    }
  });

  function commitAskAiModel(value: string) {
    draftAskAiModel = value;
    askAiModelQuery = askAiModelLabel(value);
    askAiModelOpen = false;
  }

  function closeAskAiModelSoon() {
    // Delay so an option's click (mousedown → click) lands before we close.
    setTimeout(() => {
      askAiModelOpen = false;
      askAiModelQuery = askAiModelLabel(draftAskAiModel);
    }, 120);
  }

  function handleAskAiModelKeydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      openAskAiModelMenu();
      askAiModelHighlight = Math.min(askAiModelHighlight + 1, askAiModelFiltered.length - 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      askAiModelHighlight = Math.max(askAiModelHighlight - 1, 0);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const choice = askAiModelFiltered[askAiModelHighlight];
      if (choice) {
        commitAskAiModel(choice.value);
      } else {
        // No list match: accept a typed provider:id as a custom model.
        const typed = askAiModelQuery.trim();
        if (typed.includes(":")) commitAskAiModel(typed);
      }
    } else if (event.key === "Escape") {
      askAiModelOpen = false;
      askAiModelQuery = askAiModelLabel(draftAskAiModel);
    }
  }
  let retentionCleanupSummary = $state<RetentionCleanupSummary | null>(null);
  let retentionCleanupRunning = $state(false);
  let retentionCleanupError = $state<string | null>(null);
  let brokerGrants = $state<BrokerGrant[]>([]);
  let brokerGrantLoading = $state(false);
  let brokerGrantSaving = $state(false);
  let brokerGrantError = $state<string | null>(null);
  let mnemaCliStatus = $state<MnemaCliStatus | null>(null);
  let piRuntimeStatus = $state<PiRuntimeStatus | null>(null);
  let mnemaCliLoading = $state(false);
  let piRuntimeLoading = $state(false);
  let mnemaCliInstalling = $state(false);
  let mnemaCliError = $state<string | null>(null);
  let piRuntimeError = $state<string | null>(null);

  // Reasoning Engine (AI runtime) drafts. Autosaved through the `ai_runtime`
  // domain, EXCEPT the provider API key — that is stored only in the OS keychain
  // and saved/cleared through explicit invokes (see below), never autosaved.
  const DEFAULT_AI_CLOUD_MODEL = "claude-haiku-4-5";
  const DEFAULT_AI_LOCAL_ENDPOINT = "http://localhost:11434";
  let draftAiEnabled = $state(false);
  let draftAiEngineKind = $state<AiEngineKind>("cloud");
  let draftAiCloudProvider = $state<AiCloudProvider>("anthropic");
  let draftAiCloudModel = $state(DEFAULT_AI_CLOUD_MODEL);
  let draftAiCloudBaseUrl = $state("");
  let draftAiLocalKind = $state<AiLocalKind>("ollama");
  let draftAiLocalEndpoint = $state(DEFAULT_AI_LOCAL_ENDPOINT);
  let draftAiLocalModel = $state("");

  // User Context (derivation) drafts. Autosaved through the `user_context`
  // domain, mirroring the `ai_runtime` draft-state pattern. The settings card
  // UI is a later slice; these keep types/snapshots consistent for now.
  const DEFAULT_USER_CONTEXT_BUDGET_TIER: DerivationBudgetTier = "balanced";
  const DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS = 30;
  let draftUserContextBudgetTier = $state<DerivationBudgetTier>(
    DEFAULT_USER_CONTEXT_BUDGET_TIER
  );
  let draftUserContextBackfillWindowDays = $state(
    DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS
  );
  let draftUserContextBackfillGoDeeper = $state(false);

  // Reasoning Engine availability snapshot + the cloud-key entry box. The key is
  // not part of any draft/autosave; it has its own loading/error/saved state and
  // explicit save/clear invokes mirroring installMnemaCli/loadPiRuntimeStatus.
  let aiRuntimeStatus = $state<AiRuntimeStatus | null>(null);
  let aiRuntimeStatusLoading = $state(false);
  let aiRuntimeStatusError = $state<string | null>(null);
  let aiProviderKeyInput = $state("");
  let aiProviderKeySaved = $state(false);
  let aiProviderKeySaving = $state(false);
  let aiProviderKeyError = $state<string | null>(null);
  let aiRuntimeTestRunning = $state(false);
  let aiRuntimeTestResult = $state<AiRuntimeTestResult | null>(null);
  let aiRuntimeTestError = $state<string | null>(null);

  // Human-facing label for an AiRuntimeStatus.reason code.
  function aiRuntimeReasonLabel(reason: string | null | undefined): string {
    switch (reason) {
      case "ai_runtime_disabled":
        return "Reasoning Engine is turned off.";
      case "no_model":
        return "No model is configured.";
      case "no_cloud_key":
        return "No API key saved for this provider.";
      case "no_base_url":
        return "Add the base URL for the OpenAI-compatible provider.";
      case "local_no_model":
        return "No local model is configured.";
      case "local_endpoint_unreachable":
        return "The local endpoint could not be reached.";
      default:
        return reason ? reason : "Unavailable";
    }
  }

  async function loadAiRuntimeStatus() {
    aiRuntimeStatusLoading = true;
    aiRuntimeStatusError = null;
    try {
      aiRuntimeStatus = await invoke<AiRuntimeStatus>("get_ai_runtime_status");
    } catch (error) {
      aiRuntimeStatusError = error instanceof Error ? error.message : String(error);
    } finally {
      aiRuntimeStatusLoading = false;
    }
  }

  // Re-check whether a key is stored for the currently selected cloud provider.
  async function refreshAiProviderKeyPresence() {
    try {
      aiProviderKeySaved = await invoke<boolean>("ai_runtime_has_provider_key", {
        request: { provider: draftAiCloudProvider },
      });
    } catch (error) {
      aiProviderKeyError = error instanceof Error ? error.message : String(error);
    }
  }

  async function saveAiProviderKey() {
    const key = aiProviderKeyInput.trim();
    if (!key) {
      aiProviderKeyError = "Enter an API key first.";
      return;
    }
    aiProviderKeySaving = true;
    aiProviderKeyError = null;
    try {
      await invoke("ai_runtime_set_provider_key", {
        request: { provider: draftAiCloudProvider, key },
      });
      aiProviderKeyInput = "";
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
    } catch (error) {
      aiProviderKeyError = error instanceof Error ? error.message : String(error);
    } finally {
      aiProviderKeySaving = false;
    }
  }

  async function clearAiProviderKey() {
    aiProviderKeySaving = true;
    aiProviderKeyError = null;
    try {
      await invoke("ai_runtime_clear_provider_key", {
        request: { provider: draftAiCloudProvider },
      });
      aiProviderKeyInput = "";
      await refreshAiProviderKeyPresence();
      await loadAiRuntimeStatus();
    } catch (error) {
      aiProviderKeyError = error instanceof Error ? error.message : String(error);
    } finally {
      aiProviderKeySaving = false;
    }
  }

  async function runAiRuntimeTestConnection() {
    aiRuntimeTestRunning = true;
    aiRuntimeTestError = null;
    aiRuntimeTestResult = null;
    try {
      aiRuntimeTestResult = await invoke<AiRuntimeTestResult>("ai_runtime_test_connection");
    } catch (error) {
      aiRuntimeTestError = error instanceof Error ? error.message : String(error);
    } finally {
      aiRuntimeTestRunning = false;
      void loadAiRuntimeStatus();
    }
  }

  // ----- User Context (issue #93): status + recent Activity preview + run-now -
  // Read-only surface inside the Reasoning Engine card; the derivation worker
  // runs in the background and emits `user_context_changed` to refresh this.
  let userContextStatus = $state<UserContextStatus | null>(null);
  let userContextStatusError = $state<string | null>(null);
  let userContextActivities = $state<Activity[]>([]);
  let userContextActivitiesError = $state<string | null>(null);
  let userContextConclusions = $state<Conclusion[]>([]);
  let userContextConclusionsError = $state<string | null>(null);
  let userContextRunNowRunning = $state(false);
  let userContextRunNowMessage = $state<string | null>(null);

  async function loadUserContextStatus() {
    try {
      userContextStatus = await invoke<UserContextStatus>("get_user_context_status");
      userContextStatusError = null;
    } catch (error) {
      userContextStatusError = error instanceof Error ? error.message : String(error);
    }
  }

  async function loadUserContextActivities() {
    try {
      userContextActivities = await invoke<Activity[]>("list_user_context_activities", {
        limit: 8,
        offset: 0,
      });
      userContextActivitiesError = null;
    } catch (error) {
      userContextActivitiesError = error instanceof Error ? error.message : String(error);
    }
  }

  async function loadUserContextConclusions() {
    try {
      // Include faded Conclusions so the dossier preview shows the full picture:
      // a faded Conclusion (below the display floor, #95) leaves the visible
      // dossier but is dimmed + tagged here, with its confidence % still shown.
      userContextConclusions = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      userContextConclusionsError = null;
    } catch (error) {
      userContextConclusionsError = error instanceof Error ? error.message : String(error);
    }
  }

  async function refreshUserContext() {
    await Promise.all([
      loadUserContextStatus(),
      loadUserContextActivities(),
      loadUserContextConclusions(),
    ]);
  }

  // Count a Conclusion's supporting/contradicting evidence links for the row.
  function conclusionEvidenceCount(conclusion: Conclusion): number {
    return conclusion.evidence.length;
  }

  // Render a confidence in [0,1] as a whole-number percent.
  function formatConfidencePercent(confidence: number): string {
    return `${Math.round(confidence * 100)}%`;
  }

  async function runUserContextDerivationNow() {
    userContextRunNowRunning = true;
    userContextRunNowMessage = null;
    try {
      const result = await invoke<UserContextDerivationRunResult>(
        "user_context_run_derivation_now"
      );
      userContextRunNowMessage = result.message;
      await refreshUserContext();
    } catch (error) {
      userContextRunNowMessage = error instanceof Error ? error.message : String(error);
    } finally {
      userContextRunNowRunning = false;
    }
  }

  // Compact label for an ActivityCategory (or "—" when uncategorized).
  function activityCategoryLabel(category: string | null | undefined): string {
    if (!category) return "—";
    return category.charAt(0).toUpperCase() + category.slice(1);
  }

  // "MMM D, h:mm a" range for an Activity row (best-effort; locale formatting).
  function formatActivityRange(startedAtMs: number, endedAtMs: number): string {
    const fmt = (ms: number) =>
      new Date(ms).toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
    const start = fmt(startedAtMs);
    if (endedAtMs <= startedAtMs) return start;
    return `${start} – ${fmt(endedAtMs)}`;
  }

  function formatLastDerived(ms: number | null | undefined): string {
    if (!ms) return "never";
    return new Date(ms).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

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
  // Saved-person count drives the preset-switch warning (over-warns for any
  // profile, not strictly the current Voiceprint Space — acceptable for V1).
  let personProfileCount = $state(0);
  let switchingSpeakerModel = $state(false);
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

  // App update status
  let appUpdateStatus = $state<AppUpdateStatus | null>(null);
  let checkingAppUpdate = $state(false);
  let switchingAppUpdateChannel = $state(false);
  let installingAppUpdate = $state(false);
  let restartingAfterUpdate = $state(false);
  let appUpdateActionError = $state<string | null>(null);
  let previewConfirmationVisible = $state(false);

  // About tab: transient "Copied" confirmation plus a local error slot for the
  // copy/open-link actions (kept separate from update-action errors).
  let aboutDetailsCopied = $state(false);
  let aboutActionError = $state<string | null>(null);
  let aboutDetailsCopiedTimer: ReturnType<typeof setTimeout> | null = null;

  // Loading / error state
  let loadingRecSettings = $state(false);
  let savingRecDomains = $state<Record<RecordingSettingsDraftDomain, boolean>>(
    makeRecordingDomainState(false)
  );
  const savingRecSettings = $derived(
    RECORDING_DRAFT_DOMAINS.some((domain) => savingRecDomains[domain])
  );
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
    | "about"
    | "capture"
    | "video"
    | "access"
    | "privacy"
    | "shortcuts"
    | "audio"
    | "processing"
    | "storage"
    | "appearance"
    | "developer";

  type SettingsFocus = "cliAccess";

  let activeTab = $state<SettingsTab>("capture");
  let brokerAuthorizationPromptVisible = $state(false);
  let shortcutCaptureActionId = $state<EditableShortcutActionId | null>(null);
  let shortcutCaptureError = $state<{ actionId: EditableShortcutActionId; message: string } | null>(null);
  const keyboardPlatform = detectKeyboardPlatform();
  let agentAccessSection = $state<HTMLElement | null>(null);

  // Scroll-region element. The wrapper persists across tab switches (only
  // the inner `{#if activeTab === ...}` panel re-mounts), so without an
  // explicit reset the previous tab's `scrollTop` would carry over and
  // strand the user mid-page on the next tab. Reset to the top whenever
  // `activeTab` changes — matches the typical tabbed-settings expectation.
  let scrollRegion = $state<HTMLDivElement | null>(null);
  let scrollRegionScrolling = $state(false);
  let scrollRegionScrollTimer: ReturnType<typeof setTimeout> | null = null;

  function handleScrollRegionScroll() {
    scrollRegionScrolling = true;
    if (scrollRegionScrollTimer !== null) clearTimeout(scrollRegionScrollTimer);
    scrollRegionScrollTimer = setTimeout(() => {
      scrollRegionScrolling = false;
      scrollRegionScrollTimer = null;
    }, 800);
  }

  $effect(() => {
    // Track `activeTab` so this fires on every switch.
    activeTab;
    scrollRegion?.scrollTo({ top: 0, behavior: "auto" });
  });

  // ─── Sidebar collapse ──────────────────────────────────────────────────────
  // The category rail can collapse to an icon-only strip. Two inputs decide the
  // rendered state: an explicit, persisted user preference (toggled from the
  // header button or ⌘/Ctrl-B) and an automatic collapse when the shell is too
  // narrow to show the labelled rail beside the content. Auto wins while it
  // applies, so a cramped window always yields the compact rail; widen it again
  // and the user's preference is restored.
  const SIDEBAR_COLLAPSE_STORAGE_KEY = "mnema.settings.sidebarCollapsed";
  const SIDEBAR_AUTO_COLLAPSE_WIDTH = 640;

  let userSidebarCollapsed = $state(false);
  let settingsShell = $state<HTMLDivElement | null>(null);
  let shellWidth = $state(Number.POSITIVE_INFINITY);

  const autoSidebarCollapsed = $derived(shellWidth < SIDEBAR_AUTO_COLLAPSE_WIDTH);
  const sidebarCollapsed = $derived(autoSidebarCollapsed || userSidebarCollapsed);

  // Load the persisted preference once (client-only; reads nothing reactive so
  // this runs a single time after mount).
  $effect(() => {
    if (typeof localStorage === "undefined") return;
    const stored = localStorage.getItem(SIDEBAR_COLLAPSE_STORAGE_KEY);
    if (stored !== null) userSidebarCollapsed = stored === "1";
  });

  // Measure the shell itself (not the viewport) so the breakpoint reflects the
  // width actually available to the rail + content, independent of window
  // chrome, page padding, or the centered reading column.
  $effect(() => {
    const el = settingsShell;
    if (!el || typeof ResizeObserver === "undefined") return;
    shellWidth = el.clientWidth; // seed before the first callback to avoid a flash
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) shellWidth = entry.contentRect.width;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });

  function toggleSidebar() {
    // While auto-collapsed there is no room to expand, so the toggle is inert
    // (and disabled in the UI); guard here so the keyboard shortcut matches.
    if (autoSidebarCollapsed) return;
    userSidebarCollapsed = !userSidebarCollapsed;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(SIDEBAR_COLLAPSE_STORAGE_KEY, userSidebarCollapsed ? "1" : "0");
    }
  }

  // ⌘B / Ctrl-B toggles the rail, matching the editor-sidebar convention. Skipped
  // while a field has focus so it never swallows text input.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onKeydown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      if (event.key !== "b" && event.key !== "B") return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target?.isContentEditable) return;
      event.preventDefault();
      toggleSidebar();
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
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
    { id: "shortcuts",  label: "Shortcuts",   description: "View and customize keys" },
    { id: "video",      label: "Video",       description: "Frame rate, resolution, bitrate" },
    { id: "audio",      label: "Audio",       description: "Microphone devices & disconnects" },
    { id: "processing", label: "Processing",  description: "OCR, transcription, speakers" },
    { id: "storage",    label: "Storage",     description: "Save path, retention, cache" },
    { id: "appearance", label: "Appearance",  description: "Theme and timeline display" },
    { id: "developer",  label: "Developer",   description: "Debug toggles & logs" },
    { id: "about",      label: "About",       description: "Version and updates" },
  ];

  function normalizeSettingsTab(value: string | null | undefined): SettingsTab | null {
    if (value === "about") return "about";
    if (value === "capture" || value === "behavior") return "capture";
    if (value === "access" || value === "cliAccess" || value === "cli-access") return "access";
    if (value === "privacy" || value === "metadata") return "privacy";
    if (value === "shortcuts" || value === "keyboard" || value === "keyboard-shortcuts" || value === "keyboard_bindings") return "shortcuts";
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

  let lastSavedRecSnapshots = $state<Record<RecordingSettingsDraftDomain, string | null>>(
    makeRecordingDomainState<string | null>(null)
  );
  let lastSavedKeyboardBindingsSnapshot = $state<string | null>(null);
  let lastSavedMicSnapshot = $state<string | null>(null);
  const recAutoSaveTimers = new Map<AutosaveRecordingDomain, ReturnType<typeof setTimeout>>();
  let keyboardBindingsAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let micAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;

  const appPrivacyExclusion = createAppPrivacyExclusionController({
    getExcludedApps: () => draftExcludedApps,
    onSettingsUpdated: (response) => {
      recordingSettings = response.settings;
      syncRecordingDomainFromCanonical(response.domain, response.settings, true);
    },
    setError: (message) => {
      recError = message;
    },
    beforePrivacyCommand: () => {
      for (const timer of recAutoSaveTimers.values()) {
        clearTimeout(timer);
      }
      recAutoSaveTimers.clear();
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

  function syncCaptureSourceDrafts(s: RecordingSettings) {
    draftCaptureScreen = s.captureScreen;
    draftCaptureMicrophone = s.captureMicrophone;
    draftCaptureSystemAudio = s.captureSystemAudio;
  }

  function syncCaptureTimingDrafts(s: RecordingSettings) {
    draftSegmentDuration = s.segmentDurationSeconds;
    draftAutoStart = s.autoStart;
  }

  function syncVideoDrafts(s: RecordingSettings) {
    draftFrameRate = s.screenFrameRate;
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
  }

  function syncStorageDrafts(s: RecordingSettings) {
    draftSaveDirectory = s.saveDirectory;
    draftRetentionPolicy = s.retentionPolicy ?? "never";
  }

  function syncDisplayDrafts(s: RecordingSettings) {
    draftFollowTimelineLive = s.followTimelineLive ?? false;
    draftAppearance = s.appearance ?? "system";
  }

  function syncMetadataDrafts(s: RecordingSettings) {
    draftMetadataEnabled = s.metadata?.enabled ?? true;
    draftBrowserUrlMode = s.metadata?.browserUrlMode ?? "sanitized";
  }

  function syncPrivacyDrafts(s: RecordingSettings) {
    draftExcludedApps = [...(s.privacy?.excludedApps ?? [])];
  }

  function syncAccessDrafts(s: RecordingSettings) {
    draftAskAiEnabled = s.access?.askAiEnabled ?? false;
    const cap = s.access?.askAiMaxToolCalls ?? ASK_AI_DEFAULT_TOOL_CALL_LIMIT;
    draftAskAiLimitToolCalls = cap > 0;
    draftAskAiMaxToolCalls = cap > 0 ? cap : ASK_AI_DEFAULT_TOOL_CALL_LIMIT;
    draftAskAiModel = s.access?.askAiModel ?? "";
  }

  function syncAiRuntimeDrafts(s: RecordingSettings) {
    draftAiEnabled = s.aiRuntime?.enabled ?? false;
    draftAiEngineKind = s.aiRuntime?.engineKind ?? "cloud";
    draftAiCloudProvider = s.aiRuntime?.cloudProvider ?? "anthropic";
    draftAiCloudModel = s.aiRuntime?.cloudModel ?? DEFAULT_AI_CLOUD_MODEL;
    draftAiCloudBaseUrl = s.aiRuntime?.cloudBaseUrl ?? "";
    draftAiLocalKind = s.aiRuntime?.localKind ?? "ollama";
    draftAiLocalEndpoint = s.aiRuntime?.localEndpoint ?? DEFAULT_AI_LOCAL_ENDPOINT;
    draftAiLocalModel = s.aiRuntime?.localModel ?? "";
  }

  function syncUserContextDrafts(s: RecordingSettings) {
    draftUserContextBudgetTier =
      s.userContext?.derivationBudgetTier ?? DEFAULT_USER_CONTEXT_BUDGET_TIER;
    draftUserContextBackfillWindowDays =
      s.userContext?.backfillWindowDays ?? DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS;
    draftUserContextBackfillGoDeeper = s.userContext?.backfillGoDeeper ?? false;
  }

  function syncInactivityDrafts(s: RecordingSettings) {
    draftPauseCaptureOnInactivity = s.pauseCaptureOnInactivity;
    draftIdleTimeoutSeconds = s.idleTimeoutSeconds;
    draftActivityMode = "system_input_or_screen_or_audio";
    draftMicrophoneActivitySensitivity = s.microphoneActivitySensitivity ?? 50;
    draftSystemAudioActivitySensitivity = s.systemAudioActivitySensitivity ?? 50;
    draftMicrophoneVadAdapter = s.audioSpeechDetection?.detector ?? s.microphoneVadAdapter ?? "silero";
  }

  function syncDeveloperDrafts(s: RecordingSettings) {
    draftNativeCaptureDebugLoggingEnabled = s.nativeCaptureDebugLoggingEnabled ?? false;
    draftDeveloperOptionsEnabled = s.developerOptionsEnabled ?? false;
  }

  function syncProcessingDrafts(s: RecordingSettings) {
    draftPreviewCacheTtlSeconds = s.previewCacheTtlSeconds ?? 3600;
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
  }

  function syncRecDrafts(s: RecordingSettings) {
    for (const domain of RECORDING_DRAFT_DOMAINS) {
      syncRecDomainDrafts(domain, s);
      setRecDomainBaseline(domain, s);
    }
  }

  function syncRecDomainDrafts(domain: RecordingSettingsDraftDomain, s: RecordingSettings) {
    switch (domain) {
      case "capture_sources":
        syncCaptureSourceDrafts(s);
        break;
      case "capture_timing":
        syncCaptureTimingDrafts(s);
        break;
      case "video":
        syncVideoDrafts(s);
        break;
      case "storage":
        syncStorageDrafts(s);
        break;
      case "display":
        syncDisplayDrafts(s);
        break;
      case "metadata":
        syncMetadataDrafts(s);
        break;
      case "app_privacy_exclusion":
        syncPrivacyDrafts(s);
        break;
      case "inactivity":
        syncInactivityDrafts(s);
        break;
      case "processing":
        syncProcessingDrafts(s);
        break;
      case "developer":
        syncDeveloperDrafts(s);
        break;
      case "access":
        syncAccessDrafts(s);
        break;
      case "ai_runtime":
        syncAiRuntimeDrafts(s);
        break;
      case "user_context":
        syncUserContextDrafts(s);
        break;
    }
  }

  function syncKeyboardBindingsDrafts(s: KeyboardBindingsSettings) {
    keyboardBindingsSettings = withKeyboardBindingDefaults(s);
    draftGlobalShortcutsEnabled = keyboardBindingsSettings.globalShortcuts.enabled;
    lastSavedKeyboardBindingsSnapshot = buildKeyboardBindingsSnapshot();
  }

  function syncMicDrafts(s: MicrophoneControllerState) {
    draftPreferenceMode = s.preference.mode;
    draftDeviceId = s.preference.deviceId ?? null;
    draftDisconnectPolicy = s.disconnectPolicy;
    lastSavedMicSnapshot = buildMicSnapshot();
  }

  function buildScreenResolutionRequest(): UpdateVideoSettingsRequest["screenResolution"] {
    return draftResolutionMode === "custom"
      ? {
          mode: "custom" as const,
          width: draftCustomWidth!,
          height: draftCustomHeight!,
        }
      : {
          mode: "preset" as const,
          preset: draftResolutionMode === "original" ? "original" as const : draftResolutionPreset,
        };
  }

  function buildVideoBitrateRequest(): UpdateVideoSettingsRequest["videoBitrate"] {
    return draftBitrateMode === "custom"
      ? { mode: "custom" as const, preset: null, customMbps: draftCustomMbps! }
      : { mode: "preset" as const, preset: draftBitratePreset, customMbps: null };
  }

  function buildProcessingRequest(): UpdateProcessingSettingsRequest {
    return {
      previewCacheTtlSeconds: draftPreviewCacheTtlSeconds,
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
    };
  }

  function buildRecDomainRequest(domain: AutosaveRecordingDomain): RecordingDomainRequest {
    switch (domain) {
      case "capture_sources":
        return {
          captureScreen: draftCaptureScreen,
          captureMicrophone: draftCaptureMicrophone,
          captureSystemAudio: draftCaptureSystemAudio,
        };
      case "capture_timing":
        return {
          segmentDurationSeconds: draftSegmentDuration,
          autoStart: draftAutoStart,
        };
      case "video":
        return {
          screenFrameRate: draftFrameRate,
          screenResolution: buildScreenResolutionRequest(),
          videoBitrate: buildVideoBitrateRequest(),
        };
      case "storage":
        return {
          saveDirectory: draftSaveDirectory,
          retentionPolicy: draftRetentionPolicy,
        };
      case "display":
        return {
          appearance: draftAppearance,
          followTimelineLive: draftFollowTimelineLive,
        };
      case "metadata":
        return {
          enabled: draftMetadataEnabled,
          browserUrlMode: draftBrowserUrlMode,
        };
      case "inactivity":
        return {
          pauseCaptureOnInactivity: draftPauseCaptureOnInactivity,
          idleTimeoutSeconds: draftIdleTimeoutSeconds,
          microphoneActivitySensitivity: draftMicrophoneActivitySensitivity,
          systemAudioActivitySensitivity: draftSystemAudioActivitySensitivity,
          audioSpeechDetection: {
            detector: draftMicrophoneVadAdapter,
          },
        };
      case "processing":
        return buildProcessingRequest();
      case "developer":
        return {
          developerOptionsEnabled: draftDeveloperOptionsEnabled,
          nativeCaptureDebugLoggingEnabled: draftNativeCaptureDebugLoggingEnabled,
        };
      case "access":
        return {
          askAiEnabled: draftAskAiEnabled,
          askAiMaxToolCalls: effectiveAskAiMaxToolCalls,
          askAiModel: draftAskAiModel,
        };
      case "ai_runtime":
        return {
          enabled: draftAiEnabled,
          engineKind: draftAiEngineKind,
          cloudProvider: draftAiCloudProvider,
          cloudModel: draftAiCloudModel,
          cloudBaseUrl: draftAiCloudBaseUrl,
          localKind: draftAiLocalKind,
          localEndpoint: draftAiLocalEndpoint,
          localModel: draftAiLocalModel,
        };
      case "user_context":
        return {
          derivationBudgetTier: draftUserContextBudgetTier,
          backfillWindowDays: draftUserContextBackfillWindowDays,
          backfillGoDeeper: draftUserContextBackfillGoDeeper,
        };
    }
  }

  function buildKeyboardBindingsRequest(): KeyboardBindingsSettings {
    const current = withKeyboardBindingDefaults(keyboardBindingsSettings ?? DEFAULT_KEYBOARD_BINDINGS);
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
      brokerGrantError = describeError(err);
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

  async function loadPiRuntimeStatus() {
    piRuntimeLoading = true;
    piRuntimeError = null;
    try {
      piRuntimeStatus = await invoke<PiRuntimeStatus>("get_pi_runtime_status");
    } catch (err) {
      piRuntimeError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      piRuntimeLoading = false;
    }
    // Refresh the selectable model list whenever PI status is (re)checked.
    void loadAskAiModels();
  }

  async function loadAskAiModels() {
    askAiModelsLoading = true;
    askAiModelsError = null;
    try {
      askAiModels = await invoke<AskAiModel[]>("ask_ai_list_models");
    } catch (err) {
      askAiModels = [];
      askAiModelsError = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    } finally {
      askAiModelsLoading = false;
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
      brokerGrantError = describeError(err);
    } finally {
      brokerGrantSaving = false;
    }
  }

  function describeError(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err && typeof err.message === "string") return err.message;
    if (err instanceof Error && err.message) return err.message;
    return "Something went wrong. Please try again.";
  }

  async function loadAppUpdateStatus() {
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("get_app_update_status");
    } catch (err) {
      appUpdateActionError = describeError(err);
    }
  }

  async function checkForAppUpdate() {
    checkingAppUpdate = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("check_for_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      checkingAppUpdate = false;
    }
  }

  async function useAppUpdateChannel(channel: AppUpdateChannel) {
    if (appUpdateStatus?.channel === channel && !previewConfirmationVisible) return;
    switchingAppUpdateChannel = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("set_app_update_channel", { channel });
      previewConfirmationVisible = false;
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      switchingAppUpdateChannel = false;
    }
  }

  function chooseAppUpdateChannel(channel: AppUpdateChannel) {
    if (channel === "preview" && appUpdateStatus?.channel !== "preview") {
      previewConfirmationVisible = true;
      return;
    }
    void useAppUpdateChannel(channel);
  }

  async function installAppUpdate() {
    installingAppUpdate = true;
    appUpdateActionError = null;
    try {
      appUpdateStatus = await invoke<AppUpdateStatus>("install_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
    } finally {
      installingAppUpdate = false;
    }
  }

  async function restartAfterAppUpdate() {
    restartingAfterUpdate = true;
    appUpdateActionError = null;
    try {
      await invoke("restart_after_app_update");
    } catch (err) {
      appUpdateActionError = describeError(err);
      await loadAppUpdateStatus();
    } finally {
      restartingAfterUpdate = false;
    }
  }

  function appUpdateStateLabel(status: AppUpdateStatus | null): string {
    if (!status) return "Loading";
    if (status.recordingActive && (status.state === "available" || status.state === "recordingBlocked")) {
      return "Recording active";
    }
    switch (status.state) {
      case "idle": return "Not checked";
      case "checking": return "Checking";
      case "upToDate": return "Up to date";
      case "available": return "Update available";
      case "downloading": return "Downloading";
      case "installing": return "Installing";
      case "restartRequired": return "Restart required";
      case "recordingBlocked": return "Recording active";
      case "incompatible": return "Incompatible";
      case "failed": return "Failed";
      default: return "Unknown";
    }
  }

  function appUpdateStatusMessage(status: AppUpdateStatus | null): string {
    if (!status) return "Loading update status.";
    if (status.recordingActive && (status.state === "available" || status.state === "recordingBlocked")) {
      return "Stop recording to install this update.";
    }
    if (status.error?.message) return status.error.message;
    switch (status.state) {
      case "idle": return "Mnema has not checked for updates in this app session.";
      case "checking": return "Checking the selected update channel.";
      case "upToDate": return "Mnema is current on the selected channel.";
      case "available": return `Version ${status.update?.version ?? "newer"} is ready to download and install.`;
      case "downloading": return "Downloading the update package.";
      case "installing": return "Installing the update. Keep Mnema open until this finishes.";
      case "restartRequired": return "Restart Mnema when you are ready to finish updating.";
      case "incompatible": return "No compatible update is available for this Mac.";
      case "failed": return "The last update operation failed. You can retry.";
      default: return "Update status is unavailable.";
    }
  }

  function updateChannelLabel(channel: AppUpdateChannel | null | undefined): string {
    return channel === "preview" ? "Preview" : "Stable";
  }

  function platformLabel(status: AppUpdateStatus | null): string {
    if (!status) return "Unknown";
    const os = status.app.platform === "macos" ? "macOS" : status.app.platform;
    const arch = status.app.arch === "aarch64" ? "Apple Silicon" : status.app.arch;
    return `${os} · ${arch}`;
  }

  // A single-line, paste-ready summary for bug reports: product, version,
  // build target, and bundle identifier.
  function aboutDetailsText(status: AppUpdateStatus | null): string {
    const app = status?.app;
    const product = app?.productName ?? "mnema";
    const version = app?.version ?? "unknown";
    const target = app ? `${app.platform}/${app.arch}` : "unknown";
    const identifier = app?.identifier ?? "unknown";
    return `${product} ${version} (${target}) ${identifier}`;
  }

  async function copyAboutDetails() {
    aboutActionError = null;
    try {
      await writeText(aboutDetailsText(appUpdateStatus));
      aboutDetailsCopied = true;
      if (aboutDetailsCopiedTimer !== null) clearTimeout(aboutDetailsCopiedTimer);
      aboutDetailsCopiedTimer = setTimeout(() => {
        aboutDetailsCopied = false;
        aboutDetailsCopiedTimer = null;
      }, 2000);
    } catch (err) {
      aboutActionError = describeError(err);
    }
  }

  async function openExternalUrl(url: string) {
    aboutActionError = null;
    try {
      await openUrl(url);
    } catch (err) {
      aboutActionError = describeError(err);
    }
  }

  function formatUpdateDate(value: string | null | undefined): string | null {
    if (!value) return null;
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  }

  function formatCheckedAt(value: number | null | undefined): string {
    if (!value) return "Not checked yet";
    return new Date(value).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  function appUpdateProgressText(status: AppUpdateStatus | null): string {
    const progress = status?.progress;
    if (!progress) return "";
    if (progress.contentLengthBytes) {
      return `${formatBytes(progress.downloadedBytes)} of ${formatBytes(progress.contentLengthBytes)}`;
    }
    return `${formatBytes(progress.downloadedBytes)} downloaded`;
  }

  function appUpdateProgressPercent(status: AppUpdateStatus | null): number {
    const progress = status?.progress;
    if (!progress?.contentLengthBytes) return 8;
    const percent = (progress.downloadedBytes / progress.contentLengthBytes) * 100;
    return Math.max(4, Math.min(100, percent));
  }

  function canInstallAppUpdate(status: AppUpdateStatus | null): boolean {
    return !!status?.update
      && (status.state === "available" || status.state === "recordingBlocked" || status.state === "failed")
      && !status.recordingActive
      && !installingAppUpdate
      && !checkingAppUpdate
      && !switchingAppUpdateChannel;
  }

  function canRestartAfterUpdate(status: AppUpdateStatus | null): boolean {
    return status?.state === "restartRequired" && !status.recordingActive && !restartingAfterUpdate;
  }

  type GrantStatus = "active" | "expired" | "revoked";

  function grantStatus(grant: BrokerGrant): GrantStatus {
    if (grant.revoked) return "revoked";
    if (grant.expiresAtUnixMs <= Date.now()) return "expired";
    return "active";
  }

  function formatGrantScope(scope: BrokerGrant["scope"]): string {
    if (scope === "all_retained_history") return "All retained history";
    if (scope && typeof scope === "object" && "recent_days" in scope) {
      const days = (scope as { recent_days?: { days?: number } }).recent_days?.days ?? 0;
      return days <= 1 ? "Last day" : `Last ${days} days`;
    }
    return "Limited scope";
  }

  function formatGrantTime(unixMs: number): string {
    const diffMs = unixMs - Date.now();
    const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
    const abs = Math.abs(diffMs);
    if (abs < 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 60000), "minute");
    if (abs < 24 * 60 * 60 * 1000) return rtf.format(Math.round(diffMs / 3600000), "hour");
    return rtf.format(Math.round(diffMs / 86400000), "day");
  }

  function grantStatusLabel(grant: BrokerGrant): string {
    const status = grantStatus(grant);
    if (status === "revoked") return "Revoked";
    if (status === "expired") return `Expired ${formatGrantTime(grant.expiresAtUnixMs)}`;
    return `Expires ${formatGrantTime(grant.expiresAtUnixMs)}`;
  }

  function formatPiRuntimeSource(source: PiRuntimeStatus["source"]): string {
    if (source === "managed") return "managed";
    if (source === "path") return "PATH";
    if (source === "unmanaged") return "configured path";
    return "not found";
  }

  function formatPiRuntimeReason(status: PiRuntimeStatus): string {
    if (status.ready) return "ready";
    if (status.reason === "pi_not_found") return "pi was not found in PATH";
    if (status.reason === "pi_version_unavailable") return "pi --version did not return a usable version";
    if (status.reason === "pi_version_too_old") return `pi ${status.version ?? "unknown"} is older than ${status.minimumVersion}`;
    if (status.reason === "pi_auth_missing") return `PI auth is missing at ${status.authJsonPath}`;
    if (status.reason === "pi_auth_empty") return `PI auth has no providers at ${status.authJsonPath}`;
    if (status.reason === "pi_auth_malformed") return `PI auth is not valid JSON at ${status.authJsonPath}`;
    if (status.reason === "pi_auth_misconfigured") return `PI auth has no configured provider at ${status.authJsonPath}`;
    return "PI is not ready";
  }

  function piProviderConfigured(status: PiRuntimeStatus | null): boolean {
    return status?.providerConfigured ?? status?.authJsonProviderConfigured ?? false;
  }

  function piProviderCount(status: PiRuntimeStatus | null): number {
    return status?.providerCount ?? status?.authJsonProviderCount ?? 0;
  }

  function askAiStatusLabel(status: PiRuntimeStatus | null): string {
    return draftAskAiEnabled && status?.ready ? "Available" : "Unavailable";
  }

  function askAiStatusDetail(status: PiRuntimeStatus | null): string {
    if (!draftAskAiEnabled) return "Ask AI is off. Enable it here after PI is set up.";
    if (status === null) return "Checking PI setup.";
    if (status.ready) return `PI ready via ${formatPiRuntimeSource(status.source)}${status.executablePath ? ` at ${status.executablePath}` : ""}.`;
    if (!status.versionOk) return formatPiRuntimeReason(status);
    if (!piProviderConfigured(status)) return `Set up a PI provider in ${status.authJsonPath}; no credentials are collected by Mnema.`;
    return formatPiRuntimeReason(status);
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

  function buildRecDomainRequestFromSettings(
    domain: RecordingSettingsDraftDomain,
    s: RecordingSettings,
  ): unknown {
    switch (domain) {
      case "capture_sources":

        return {
          captureScreen: s.captureScreen,
          captureMicrophone: s.captureMicrophone,
          captureSystemAudio: s.captureSystemAudio,
        };
      case "capture_timing":
        return {
          segmentDurationSeconds: s.segmentDurationSeconds,
          autoStart: s.autoStart,
        };
      case "video":
        return {
          screenFrameRate: s.screenFrameRate,
          screenResolution: s.screenResolution,
          videoBitrate: s.videoBitrate,
        };
      case "storage":
        return {
          saveDirectory: s.saveDirectory,
          retentionPolicy: s.retentionPolicy ?? "never",
        };
      case "display":
        return {
          appearance: s.appearance ?? "system",
          followTimelineLive: s.followTimelineLive ?? false,
        };
      case "metadata":
        return {
          enabled: s.metadata?.enabled ?? true,
          browserUrlMode: s.metadata?.browserUrlMode ?? "sanitized",
        };
      case "app_privacy_exclusion":
        return {
          excludedApps: s.privacy?.excludedApps ?? [],
        };
      case "inactivity":
        return {
          pauseCaptureOnInactivity: s.pauseCaptureOnInactivity,
          idleTimeoutSeconds: s.idleTimeoutSeconds,
          microphoneActivitySensitivity: s.microphoneActivitySensitivity ?? 50,
          systemAudioActivitySensitivity: s.systemAudioActivitySensitivity ?? 50,
          audioSpeechDetection: {
            detector: s.audioSpeechDetection?.detector ?? s.microphoneVadAdapter ?? "silero",
          },
        };
      case "processing":
        return {
          previewCacheTtlSeconds: s.previewCacheTtlSeconds ?? 3600,
          ocr: s.ocr,
          transcription: s.transcription,
          speakerAnalysis: s.speakerAnalysis,
        };
      case "developer":
        return {
          developerOptionsEnabled: s.developerOptionsEnabled ?? false,
          nativeCaptureDebugLoggingEnabled: s.nativeCaptureDebugLoggingEnabled ?? false,
        };
      case "access":
        return {
          askAiEnabled: s.access?.askAiEnabled ?? false,
          askAiMaxToolCalls: s.access?.askAiMaxToolCalls ?? ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
          askAiModel: s.access?.askAiModel ?? "",
        };
      case "ai_runtime":
        return {
          enabled: s.aiRuntime?.enabled ?? false,
          engineKind: s.aiRuntime?.engineKind ?? "cloud",
          cloudProvider: s.aiRuntime?.cloudProvider ?? "anthropic",
          cloudModel: s.aiRuntime?.cloudModel ?? DEFAULT_AI_CLOUD_MODEL,
          cloudBaseUrl: s.aiRuntime?.cloudBaseUrl ?? "",
          localKind: s.aiRuntime?.localKind ?? "ollama",
          localEndpoint: s.aiRuntime?.localEndpoint ?? DEFAULT_AI_LOCAL_ENDPOINT,
          localModel: s.aiRuntime?.localModel ?? "",
        };
      case "user_context":
        return {
          derivationBudgetTier:
            s.userContext?.derivationBudgetTier ?? DEFAULT_USER_CONTEXT_BUDGET_TIER,
          backfillWindowDays:
            s.userContext?.backfillWindowDays ?? DEFAULT_USER_CONTEXT_BACKFILL_WINDOW_DAYS,
          backfillGoDeeper: s.userContext?.backfillGoDeeper ?? false,
        };
    }
  }

  function buildRecDomainSnapshot(domain: RecordingSettingsDraftDomain): string {
    if (domain === "app_privacy_exclusion") {
      return JSON.stringify({ excludedApps: draftExcludedApps });
    }
    return JSON.stringify(buildRecDomainRequest(domain));
  }

  function buildRecDomainSnapshotFromSettings(
    domain: RecordingSettingsDraftDomain,
    s: RecordingSettings,
  ): string {
    return JSON.stringify(buildRecDomainRequestFromSettings(domain, s));
  }

  function setRecDomainBaseline(domain: RecordingSettingsDraftDomain, s: RecordingSettings): void {
    lastSavedRecSnapshots = {
      ...lastSavedRecSnapshots,
      [domain]: buildRecDomainSnapshotFromSettings(domain, s),
    };
  }

  function syncRecordingDomainFromCanonical(
    domain: SettingsOwnershipDomain,
    s: RecordingSettings,
    force = false,
  ): void {
    if (!RECORDING_DRAFT_DOMAINS.includes(domain as RecordingSettingsDraftDomain)) return;
    const draftDomain = domain as RecordingSettingsDraftDomain;
    const baseline = lastSavedRecSnapshots[draftDomain];
    const dirty = baseline !== null && buildRecDomainSnapshot(draftDomain) !== baseline;

    if (force || !dirty) {
      syncRecDomainDrafts(draftDomain, s);
    }
    setRecDomainBaseline(draftDomain, s);

    if (draftDomain === "display" && (force || !dirty)) {
      setAppearance(s.appearance ?? "system");
    }
    if (draftDomain === "developer" && (force || !dirty)) {
      setDeveloperOptionsEnabled(s.developerOptionsEnabled ?? false);
      void loadDebugLogStatus();
    }
    if (draftDomain === "ai_runtime" && (force || !dirty)) {
      void refreshAiProviderKeyPresence();
      void loadAiRuntimeStatus();
    }
  }

  // A domain-less `recording_settings_changed` (the legacy full-settings
  // command) carries no per-domain payload, so resync every domain's drafts and
  // baselines from the canonical settings. Dirty domains keep their in-flight
  // edits — only their baseline is refreshed so the next autosave diff stays
  // correct — which prevents a later same-domain save from shipping stale
  // companion fields back over the external change. App-wide appearance and
  // developer-mode side effects are handled by the dedicated theme /
  // developer-options stores listening on the same event, so we skip them (and
  // their per-domain IPC) here.
  function resyncRecordingDraftsFromCanonical(s: RecordingSettings): void {
    for (const domain of RECORDING_DRAFT_DOMAINS) {
      const baseline = lastSavedRecSnapshots[domain];
      const dirty = baseline !== null && buildRecDomainSnapshot(domain) !== baseline;
      if (!dirty) {
        syncRecDomainDrafts(domain, s);
      }
      setRecDomainBaseline(domain, s);
    }
  }

  function buildKeyboardBindingsSnapshot(): string {
    return JSON.stringify(buildKeyboardBindingsRequest());
  }

  function shortcutCategoryLabel(category: string): string {
    if (category === "global") return "Recording & window";
    if (category === "app") return "App";
    if (category === "dashboard") return "Dashboard";
    return "Audio Drawer";
  }

  function shortcutCategoryActions(category: string): EditableShortcutAction[] {
    return EDITABLE_SHORTCUT_ACTIONS.filter((action) => action.category === category);
  }

  function shortcutDraftBinding(actionId: EditableShortcutActionId): string {
    return getShortcutBinding(buildKeyboardBindingsRequest(), actionId);
  }

  function bindingHasNonShiftModifier(binding: string): boolean {
    const parsed = parseShortcutBinding(binding);
    return parsed?.primary === true || parsed?.alt === true;
  }

  function shortcutIssues(): Record<string, string> {
    const settings = buildKeyboardBindingsRequest();
    const issues: Record<string, string> = {};
    const seen = new Map<string, EditableShortcutAction[]>();

    for (const action of EDITABLE_SHORTCUT_ACTIONS) {
      const raw = getShortcutBinding(settings, action.id).trim();
      if (!raw) continue;
      const normalized = normalizeShortcutBinding(raw);
      if (!normalized) {
        issues[action.id] = "Use a valid shortcut such as J, ⌘K, or ⌥⌘P.";
        continue;
      }
      if (action.nativeBackground && !bindingHasNonShiftModifier(normalized)) {
        issues[action.id] = "Background shortcuts must include Command/Control or Alt.";
        continue;
      }
      const reserved = reservedShortcutConflict(action, normalized);
      if (reserved) {
        issues[action.id] = `Reserved to ${reserved.label}.`;
        continue;
      }
      const key = normalized.toLowerCase();
      const previousActions = seen.get(key) ?? [];
      const conflictingPreviousActions = previousActions.filter((previous) =>
        shortcutScopesConflict(shortcutConflictScope(previous), shortcutConflictScope(action)),
      );
      if (conflictingPreviousActions.length > 0) {
        issues[action.id] = `Conflicts with ${conflictingPreviousActions[0].label}.`;
        for (const previous of conflictingPreviousActions) {
          issues[previous.id] = `Conflicts with ${action.label}.`;
        }
      }
      previousActions.push(action);
      seen.set(key, previousActions);
    }

    return issues;
  }

  const keyboardShortcutIssues = $derived(shortcutIssues());
  const keyboardShortcutSaveBlocked = $derived(Object.keys(keyboardShortcutIssues).length > 0 || shortcutCaptureActionId !== null);

  function shortcutIssueFor(actionId: EditableShortcutActionId): string | null {
    if (shortcutCaptureError?.actionId === actionId) return shortcutCaptureError.message;
    return keyboardShortcutIssues[actionId] ?? null;
  }

  function setShortcutDraft(actionId: EditableShortcutActionId, binding: string): void {
    const base = withKeyboardBindingDefaults(keyboardBindingsSettings ?? DEFAULT_KEYBOARD_BINDINGS);
    keyboardBindingsSettings = setShortcutBinding(base, actionId, binding);
  }

  function clearShortcut(actionId: EditableShortcutActionId): void {
    setShortcutDraft(actionId, "");
  }

  function resetShortcut(actionId: EditableShortcutActionId): void {
    setShortcutDraft(actionId, getShortcutBinding(DEFAULT_KEYBOARD_BINDINGS, actionId));
  }

  async function restoreDefaultShortcuts(): Promise<void> {
    const ok = await ask("Restore all keyboard shortcuts to their defaults?", {
      title: "Restore default shortcuts",
      kind: "warning",
      okLabel: "Restore defaults",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    keyboardBindingsSettings = withKeyboardBindingDefaults(DEFAULT_KEYBOARD_BINDINGS);
    draftGlobalShortcutsEnabled = DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.enabled;
  }

  function shortcutKeyTokens(binding: string): string[] | null {
    const parsed = parseShortcutBinding(binding);
    if (!parsed) return null;
    return formatShortcut(parsed, keyboardPlatform);
  }

  function startShortcutCapture(actionId: EditableShortcutActionId): void {
    shortcutCaptureError = null;
    shortcutCaptureActionId = shortcutCaptureActionId === actionId ? null : actionId;
  }

  function cancelShortcutCapture(): void {
    shortcutCaptureError = null;
    shortcutCaptureActionId = null;
  }

  function captureShortcut(actionId: EditableShortcutActionId, event: KeyboardEvent): void {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
    if (event.key === "Escape") {
      shortcutCaptureError = null;
      shortcutCaptureActionId = null;
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      shortcutCaptureError = null;
      clearShortcut(actionId);
      shortcutCaptureActionId = null;
      return;
    }
    const binding = shortcutBindingFromKeyboardEvent(event, keyboardPlatform);
    if (!binding) {
      if (keyboardPlatform === "macos" && event.ctrlKey && event.key !== "Control") {
        shortcutCaptureError = { actionId, message: "Control shortcuts are not supported on macOS. Use Command or Option." };
      } else if (event.key !== "Meta" && event.key !== "Control" && event.key !== "Alt" && event.key !== "Shift") {
        shortcutCaptureError = { actionId, message: "That key is not supported for shortcuts." };
      }
      return;
    }
    shortcutCaptureError = null;
    setShortcutDraft(actionId, binding);
    shortcutCaptureActionId = null;
  }

  // While listening for a new shortcut we intercept keys at the window in the
  // capture phase. This is required because (a) WebKit (Tauri's WKWebView) does
  // not focus a <button> on click, so a button-local onkeydown never fires, and
  // (b) the layout's window keydown handler would otherwise swallow plain keys
  // like "1"/"J"/"K" as app shortcuts before this row could record them.
  $effect(() => {
    const actionId = shortcutCaptureActionId;
    if (actionId === null) return;

    const onKeydown = (event: KeyboardEvent) => {
      captureShortcut(actionId, event);
    };
    const onPointerDown = (event: Event) => {
      const target = event.target;
      if (target instanceof Element && target.closest(`[data-shortcut-capture="${actionId}"]`)) {
        return;
      }
      cancelShortcutCapture();
    };

    window.addEventListener("keydown", onKeydown, { capture: true });
    window.addEventListener("pointerdown", onPointerDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKeydown, { capture: true });
      window.removeEventListener("pointerdown", onPointerDown, { capture: true });
    };
  });

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

  // Best-effort saved-person count for the preset-switch warning. A failed load
  // simply leaves the count at 0 (no warning), never blocking preset selection.
  async function loadPersonProfileCount() {
    try {
      const profiles = await invoke<PersonProfileDto[]>("list_person_profiles");
      personProfileCount = profiles.length;
    } catch {
      personProfileCount = 0;
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

  function recDomainSaveBlocked(domain: AutosaveRecordingDomain): boolean {
    if (domain === "capture_sources") {
      return (
        (!draftCaptureScreen && !draftCaptureMicrophone && !draftCaptureSystemAudio)
        || (draftCaptureSystemAudio && !draftCaptureScreen)
      );
    }
    if (domain === "video") {
      return resolutionSupportPendingForNonOriginal || customResolutionBlocked || customBitrateBlocked;
    }
    if (domain === "storage") {
      return !draftSaveDirectory;
    }
    return false;
  }

  async function saveRecordingDomain(domain: AutosaveRecordingDomain) {
    if (appPrivacyExclusion.commandInFlight) return;
    if (recDomainSaveBlocked(domain)) {
      if (domain === "video" && resolutionSupportPendingForNonOriginal) {
        recError = "Wait for capture support to load before saving preset/custom resolution.";
      }
      return;
    }

    const previousRetentionPolicy = recordingSettings?.retentionPolicy ?? "never";

    if (domain === "storage" && previousRetentionPolicy === "never" && draftRetentionPolicy !== "never") {
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

    savingRecDomains = { ...savingRecDomains, [domain]: true };
    recError = null;
    recSaved = false;
    try {
      const response = await invoke<RecordingSettingsDomainUpdateResponse>(RECORDING_DOMAIN_COMMANDS[domain], {
        request: buildRecDomainRequest(domain),
      });
      const updated = response.settings;
      recordingSettings = updated;
      syncRecordingDomainFromCanonical(response.domain, updated, true);
      recSaved = true;
      setTimeout(() => { recSaved = false; }, 2200);

      if (domain === "storage" && previousRetentionPolicy !== updated.retentionPolicy && updated.retentionPolicy !== "never") {
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
      savingRecDomains = { ...savingRecDomains, [domain]: false };
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
    if (recordingSettings === null) return;

    for (const domain of RECORDING_AUTOSAVE_DOMAINS) {
      const baseline = lastSavedRecSnapshots[domain];
      if (baseline === null) continue;
      const current = buildRecDomainSnapshot(domain);
      const existingTimer = recAutoSaveTimers.get(domain);

      if (current === baseline || recDomainSaveBlocked(domain) || appPrivacyExclusion.commandInFlight || savingRecDomains[domain]) {
        if (existingTimer) {
          clearTimeout(existingTimer);
          recAutoSaveTimers.delete(domain);
        }
        continue;
      }

      if (existingTimer) clearTimeout(existingTimer);
      const timer = setTimeout(() => {
        recAutoSaveTimers.delete(domain);
        const latestBaseline = lastSavedRecSnapshots[domain];
        if (latestBaseline === null) return;
        if (recDomainSaveBlocked(domain) || appPrivacyExclusion.commandInFlight || savingRecDomains[domain]) return;
        if (buildRecDomainSnapshot(domain) === latestBaseline) return;
        void saveRecordingDomain(domain);
      }, RECORDING_AUTOSAVE_DEBOUNCE_MS);
      recAutoSaveTimers.set(domain, timer);
    }
  });

  $effect(() => {
    if (keyboardBindingsSettings === null || lastSavedKeyboardBindingsSnapshot === null) return;
    const current = buildKeyboardBindingsSnapshot();
    if (current === lastSavedKeyboardBindingsSnapshot) return;
    if (keyboardShortcutSaveBlocked) return;
    if (savingKeyboardBindings) return;

    if (keyboardBindingsAutoSaveTimer !== null) clearTimeout(keyboardBindingsAutoSaveTimer);
    keyboardBindingsAutoSaveTimer = setTimeout(() => {
      keyboardBindingsAutoSaveTimer = null;
      if (savingKeyboardBindings || keyboardShortcutSaveBlocked) return;
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

  // Preset picker options. Each curated Speaker Model Preset surfaces its
  // download size from the same model-status descriptor the status panel uses
  // (`model.download.byteSize`), formatted via the shared `formatBytes` helper.
  const speakerModelOptions = $derived(
    selectedSpeakerModels.map((model) => ({
      value: model.modelId ?? "__os_managed__",
      label: model.download
        ? `${model.displayName} · ${formatBytes(model.download.byteSize)}`
        : model.displayName,
    }))
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

  // Switching Speaker Model Presets is non-destructive and reversible, so the
  // warning is purely informational: it fires only when the user is moving AWAY
  // from the saved preset while saved-person recognition is on and at least one
  // Person profile exists. Confirming proceeds (the existing autosave persists
  // the new modelId); cancelling leaves draftSpeakerModelId unchanged so the
  // controlled picker reverts to the saved selection. It NEVER auto-migrates,
  // re-enrolls, or blocks the choice.
  async function chooseSpeakerModel(value: string) {
    const next = value === "__os_managed__" ? null : value;
    if (next === draftSpeakerModelId) return;

    const savedModelId = recordingSettings?.speakerAnalysis?.modelId ?? null;
    const switchingAwayFromSaved = next !== savedModelId;
    const needsWarning =
      switchingAwayFromSaved && draftSpeakerRecognizeSavedPeople && personProfileCount > 0;

    if (needsWarning) {
      switchingSpeakerModel = true;
      try {
        const ok = await ask(
          "Switching the speaker model is safe and reversible — your saved people are not deleted. "
            + "But saved voices won't be recognized under the new model until you re-tag each person once. "
            + "Switching back to the previous model restores them. Switch anyway?",
          { title: "Switch speaker model?", kind: "warning", okLabel: "Switch", cancelLabel: "Keep current" }
        );
        if (!ok) return;
      } finally {
        switchingSpeakerModel = false;
      }
    }

    draftSpeakerModelId = next;
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
    void loadPersonProfileCount();
    loadDebugLogStatus();
    loadGeneralLogStatus();
    loadAppUpdateStatus();
    void appPrivacyExclusion.loadPrivacyAppCandidates();
    void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    loadBrokerGrants();
    loadMnemaCliStatus();
    loadPiRuntimeStatus();
    void loadAiRuntimeStatus();
    void refreshAiProviderKeyPresence();
    void refreshUserContext();

    let unlistenControllerChanged: (() => void) | undefined;
    let unlistenUserContextChanged: (() => void) | undefined;
    let unlistenAutoDisconnectFailure: (() => void) | undefined;
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
    let unlistenRecordingSettingsDomainChanged: (() => void) | undefined;
    let unlistenOpenSettingsTab: (() => void) | undefined;
    let unlistenAppUpdateStatusChanged: (() => void) | undefined;
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
      resyncRecordingDraftsFromCanonical(event.payload);
      recError = null;
      void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    });

    listen<RecordingSettingsDomainUpdateResponse>(
      RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT,
      (event) => {
        recordingSettings = event.payload.settings;
        syncRecordingDomainFromCanonical(event.payload.domain, event.payload.settings);
        recError = null;
        if (event.payload.domain === "app_privacy_exclusion" || event.payload.domain === "metadata") {
          void appPrivacyExclusion.loadSensitiveCaptureRecommendations();
        }
      }
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsDomainChanged = fn;
    });


    listen<{ tab: string; focus?: string }>("open_settings_tab", (event) => {
      handleSettingsTabEvent(event.payload.tab, event.payload.focus);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenOpenSettingsTab = fn;
    });

    listen<AppUpdateStatus>(APP_UPDATE_STATUS_CHANGED_EVENT, (event) => {
      appUpdateStatus = event.payload;
      appUpdateActionError = null;
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenAppUpdateStatusChanged = fn;
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

    listen("user_context_changed", () => {
      void refreshUserContext();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenUserContextChanged = fn;
    });

    return () => {
      destroyed = true;
      for (const timer of recAutoSaveTimers.values()) clearTimeout(timer);
      recAutoSaveTimers.clear();
      if (keyboardBindingsAutoSaveTimer !== null) clearTimeout(keyboardBindingsAutoSaveTimer);
      unlistenControllerChanged?.();
      unlistenAutoDisconnectFailure?.();
      unlistenRecordingSettingsChanged?.();
      unlistenRecordingSettingsDomainChanged?.();
      unlistenOpenSettingsTab?.();
      unlistenAppUpdateStatusChanged?.();
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenSpeakerDownloadProgress?.();
      unlistenUserContextChanged?.();
    };
  });
</script>

<!-- Sidebar nav glyphs — one per category, drawn on a 24 viewBox with a
     1.8 stroke so the rail reads as one icon family. -->
{#snippet navIcon(kind: SettingsTab)}
  <span class="settings-nav__icon" aria-hidden="true">
    {#if kind === "about"}
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v6" />
        <path d="M12 7h.01" />
      </svg>
    {:else if kind === "capture"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8" />
        <path d="M12 17v4" />
      </svg>
    {:else if kind === "access"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="m7 9 3 3-3 3" />
        <path d="M13 15h4" />
      </svg>
    {:else if kind === "privacy"}
      <svg viewBox="0 0 24 24">
        <path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6Z" />
        <path d="M9 12h6" />
        <path d="M12 9v6" />
      </svg>
    {:else if kind === "shortcuts"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="6" width="18" height="12" rx="2" />
        <path d="M7 10h.01" />
        <path d="M11 10h.01" />
        <path d="M15 10h.01" />
        <path d="M8 14h8" />
      </svg>
    {:else if kind === "video"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="14" rx="2" />
        <path d="m10 9 5 3-5 3Z" />
      </svg>
    {:else if kind === "audio"}
      <svg viewBox="0 0 24 24">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0" />
        <path d="M12 18v3" />
        <path d="M9 21h6" />
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
    {:else if kind === "developer"}
      <svg viewBox="0 0 24 24">
        <path d="m8 9-4 3 4 3" />
        <path d="m16 9 4 3-4 3" />
        <path d="m14 5-4 14" />
      </svg>
    {/if}
  </span>
{/snippet}

<!-- Capture-summary glyphs — same 24 viewBox / 1.8 stroke family as the nav
     icons. Hidden while the rail is expanded (the dot + label carry the state
     there); they become the whole indicator once the rail collapses. -->
{#snippet captureStatIcon(kind: "screen" | "mic" | "sysaudio")}
  <span class="status-pill__icon" aria-hidden="true">
    {#if kind === "screen"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8" />
        <path d="M12 17v4" />
      </svg>
    {:else if kind === "mic"}
      <svg viewBox="0 0 24 24">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0" />
        <path d="M12 18v3" />
        <path d="M9 21h6" />
      </svg>
    {:else}
      <svg viewBox="0 0 24 24">
        <path d="M11 5 6 9H3v6h3l5 4Z" />
        <path d="M16 9a5 5 0 0 1 0 6" />
        <path d="M19 7a8 8 0 0 1 0 10" />
      </svg>
    {/if}
  </span>
{/snippet}

<!-- ── Settings shell ──────────────────────────────────────────────────────
     A fixed left rail lists the categories; only the right-hand content pane
     scrolls. One panel is mounted at a time (see the `{#if activeTab === ...}`
     guards below), so the rail and window chrome stay pinned and changes
     inside an unselected category don't trigger reactivity elsewhere. The rail
     replaces the previous top tab strip, which overflowed into a horizontal
     scrollbar once nine tabs no longer fit the minimum window width. -->
<div class="settings-shell" bind:this={settingsShell}>
  <aside
    id="settings-sidebar"
    class="settings-sidebar"
    class:settings-sidebar--collapsed={sidebarCollapsed}
  >
    <div class="settings-sidebar__head">
      <div class="settings-sidebar__titlebar">
        <span class="settings-sidebar__glyph" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <path d="m5 8 4 4-4 4" />
            <path d="M13 16h6" />
          </svg>
        </span>
        <h1 class="settings-sidebar__title">Settings</h1>
        <button
          class="settings-sidebar__toggle"
          type="button"
          onclick={toggleSidebar}
          disabled={autoSidebarCollapsed}
          aria-expanded={!sidebarCollapsed}
          aria-controls="settings-sidebar"
          aria-keyshortcuts="Meta+B Control+B"
          aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          title={autoSidebarCollapsed
            ? "Widen the window to expand"
            : sidebarCollapsed
              ? "Expand sidebar (⌘B)"
              : "Collapse sidebar (⌘B)"}
        >
          <svg class="settings-sidebar__toggle-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M15 6l-6 6 6 6" />
          </svg>
        </button>
      </div>
      <div class="settings-sidebar__status" aria-live="polite">
        {#if recError || keyboardBindingsError || micError}
          <span class="status-text status-text--error"><span class="status-text__label">save failed</span></span>
        {:else if recSaveBlocked || micApplyBlocked}
          <span class="status-text status-text--blocked"><span class="status-text__label">resolve issues</span></span>
        {:else if savingRecSettings || savingKeyboardBindings || savingMicSettings}
          <span class="status-text status-text--saving"><span class="status-text__label">saving</span></span>
        {:else if recSaved || keyboardBindingsSaved || micSaved}
          <span class="status-text status-text--ok"><span class="status-text__label">saved</span></span>
        {:else}
          <span class="status-text"><span class="status-text__label">auto-save on</span></span>
        {/if}
      </div>
    </div>

    <nav class="settings-nav" aria-label="Settings categories">
      <div class="settings-nav__list" role="tablist" tabindex="-1" onkeydown={handleTabKeydown}>
        {#each tabs as tab}
          <button
            class="settings-nav__item"
            class:settings-nav__item--active={activeTab === tab.id}
            role="tab"
            aria-selected={activeTab === tab.id}
            aria-controls="settings-panel-{tab.id}"
            aria-label={sidebarCollapsed ? tab.label : null}
            id="settings-tab-{tab.id}"
            tabindex={activeTab === tab.id ? 0 : -1}
            title={sidebarCollapsed ? tab.label : null}
            onkeydown={handleTabKeydown}
            onclick={() => { activeTab = tab.id; }}
            type="button"
          >
            {@render navIcon(tab.id)}
            <span class="settings-nav__text">
              <span class="settings-nav__label">{tab.label}</span>
              <span class="settings-nav__hint">{tab.description}</span>
            </span>
          </button>
        {/each}
      </div>
    </nav>

    {#if recordingSettings}
      <div class="settings-sidebar__foot">
        <span class="settings-sidebar__foot-label">Capture summary</span>
        <ul class="status-strip" aria-label="Current capture summary">
          <li
            class="status-pill"
            class:status-pill--on={draftCaptureScreen}
            title={sidebarCollapsed ? `Screen ${draftCaptureScreen ? "on" : "off"}` : null}
          >
            {@render captureStatIcon("screen")}
            <span class="status-pill__dot"></span>
            <span class="status-pill__label">Screen</span>
          </li>
          <li
            class="status-pill"
            class:status-pill--on={draftCaptureMicrophone}
            title={sidebarCollapsed ? `Mic ${draftCaptureMicrophone ? "on" : "off"}` : null}
          >
            {@render captureStatIcon("mic")}
            <span class="status-pill__dot"></span>
            <span class="status-pill__label">Mic</span>
          </li>
          <li
            class="status-pill"
            class:status-pill--on={draftCaptureSystemAudio}
            title={sidebarCollapsed ? `System audio ${draftCaptureSystemAudio ? "on" : "off"}` : null}
          >
            {@render captureStatIcon("sysaudio")}
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
      </div>
    {/if}
  </aside>

  <!-- ── Content pane ────────────────────────────────────────────────────
       Only this right-hand column scrolls; the sidebar and window chrome
       stay pinned. `flex: 1` claims the leftover height inside the shell and
       `min-height: 0` lets `.settings-scroll` shrink below its content's
       intrinsic height so the inner overflow (not the window) scrolls. -->
  <div class="settings-content">
    <AppPrivacyExclusionPrompt
      controller={appPrivacyExclusion}
      onReview={() => { activeTab = "privacy"; }}
    />

    <div class="settings-scroll" class:is-scrolling={scrollRegionScrolling} bind:this={scrollRegion} onscroll={handleScrollRegionScroll}>

<!-- ── About & updates ─────────────────────────────────────────────────── -->
{#if activeTab === "about"}
  <div role="tabpanel" id="settings-panel-about" aria-labelledby="settings-tab-about" tabindex="0">
    <section class="card about-card">
      <div class="about-id">
        <div class="about-id__mark">
          <h2 class="about-id__name">mnema</h2>
          {#if appUpdateStatus?.app.version}
            <span class="about-id__version">v{appUpdateStatus.app.version}</span>
          {:else}
            <span class="about-id__version about-id__version--pending">checking…</span>
          {/if}
          <span class="badge badge--neutral badge--sm about-id__channel">
            {updateChannelLabel(appUpdateStatus?.channel)} channel
          </span>
        </div>
        <p class="about-id__tag">
          Your memory, on rewind. Mnema records your screen so you can scrub back to
          anything you've seen: searchable, local, and yours.
        </p>
      </div>

      <div class="settings-divider"></div>

      <dl class="about-meta">
        <div class="about-meta__row">
          <dt>Platform</dt>
          <dd>{platformLabel(appUpdateStatus)}</dd>
        </div>
        <div class="about-meta__row">
          <dt>Identifier</dt>
          <dd>{appUpdateStatus?.app.identifier ?? "Unknown"}</dd>
        </div>
        <div class="about-meta__row">
          <dt>License</dt>
          <dd>MIT</dd>
        </div>
      </dl>

      <div class="about-footer">
        <div class="about-links">
          <button type="button" class="about-link" onclick={() => openExternalUrl(ABOUT_REPO_URL)}>
            Source<span class="about-link__arrow" aria-hidden="true">↗</span>
          </button>
          <button type="button" class="about-link" onclick={() => openExternalUrl(ABOUT_RELEASES_URL)}>
            Release notes<span class="about-link__arrow" aria-hidden="true">↗</span>
          </button>
        </div>
        <button
          type="button"
          class="btn btn--ghost btn--sm about-copy"
          onclick={copyAboutDetails}
          aria-label="Copy version and build details to the clipboard"
        >
          {aboutDetailsCopied ? "Copied" : "Copy details"}
        </button>
      </div>

      {#if aboutActionError}
        <p class="error-text about-error" role="alert">{aboutActionError}</p>
      {/if}
    </section>

    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          <h2 class="card__title">Updates</h2>
          <p class="card__subtitle">Mnema checks the selected channel at startup after onboarding.</p>
        </div>
        <button
          class="btn btn--primary btn--sm"
          onclick={checkForAppUpdate}
          disabled={checkingAppUpdate || switchingAppUpdateChannel || installingAppUpdate || appUpdateStatus?.state === "downloading" || appUpdateStatus?.state === "installing" || appUpdateStatus?.state === "restartRequired"}
        >
          {checkingAppUpdate || appUpdateStatus?.state === "checking" ? "Checking" : "Check for Updates"}
        </button>
      </div>

      <div class="settings-group">
        <span class="group-label">Update channel</span>
        <div class="update-channel-control" role="radiogroup" aria-label="Update channel">
          <button
            type="button"
            class:update-channel-control__option--active={appUpdateStatus?.channel !== "preview"}
            class="update-channel-control__option"
            aria-pressed={appUpdateStatus?.channel !== "preview"}
            onclick={() => chooseAppUpdateChannel("stable")}
            disabled={switchingAppUpdateChannel || installingAppUpdate}
          >
            <span>Stable</span>
            <small>Published releases</small>
          </button>
          <button
            type="button"
            class:update-channel-control__option--active={appUpdateStatus?.channel === "preview"}
            class="update-channel-control__option"
            aria-pressed={appUpdateStatus?.channel === "preview"}
            onclick={() => chooseAppUpdateChannel("preview")}
            disabled={switchingAppUpdateChannel || installingAppUpdate}
          >
            <span>Preview</span>
            <small>Opt-in prereleases</small>
          </button>
        </div>
        {#if switchingAppUpdateChannel}
          <p class="group-hint">Saving channel and checking for updates.</p>
        {:else}
          <p class="group-hint">Current channel: {updateChannelLabel(appUpdateStatus?.channel)}. Switching channels checks immediately.</p>
        {/if}

        {#if previewConfirmationVisible}
          <div class="preview-warning" role="alert">
            <div>
              <strong>Use preview updates?</strong>
              <p>Preview builds may be less stable and may show macOS security warnings until Developer ID signing and notarization are available.</p>
            </div>
            <div class="row-actions">
              <button class="btn btn--primary btn--sm" type="button" onclick={() => void useAppUpdateChannel("preview")} disabled={switchingAppUpdateChannel}>
                Use Preview Updates
              </button>
              <button class="btn btn--ghost btn--sm" type="button" onclick={() => { previewConfirmationVisible = false; }}>
                Keep Stable
              </button>
            </div>
          </div>
        {/if}
      </div>

      <div class="settings-divider"></div>

      <div class="update-status-panel" class:update-status-panel--error={appUpdateStatus?.state === "failed" || appUpdateStatus?.state === "incompatible"}>
        <div class="update-status-panel__main">
          <div class="update-status-panel__headline">
            <span class="badge badge--neutral badge--sm">{appUpdateStateLabel(appUpdateStatus)}</span>
            {#if appUpdateStatus?.update}
              <strong>Version {appUpdateStatus.update.version}</strong>
            {:else}
              <strong>{appUpdateStatus?.app.version ?? "Mnema"}</strong>
            {/if}
          </div>
          <p>{appUpdateStatusMessage(appUpdateStatus)}</p>
          <span class="update-status-panel__meta">Last checked: {formatCheckedAt(appUpdateStatus?.lastCheckedAtUnixMs)}</span>
        </div>

        {#if appUpdateStatus?.update?.date}
          <span class="update-status-panel__date">{formatUpdateDate(appUpdateStatus.update.date)}</span>
        {/if}
      </div>

      {#if appUpdateStatus?.progress}
        <div class="download-progress" aria-live="polite">
          <div class="download-progress__bar">
            <span style={`width: ${appUpdateProgressPercent(appUpdateStatus)}%`}></span>
          </div>
          <p class="group-hint">{appUpdateProgressText(appUpdateStatus)}</p>
        </div>
      {/if}

      {#if appUpdateStatus?.update?.notes}
        <div class="release-notes">
          <span class="group-label">Release notes</span>
          <p>{appUpdateStatus.update.notes}</p>
        </div>
      {/if}

      <div class="row-actions">
        {#if appUpdateStatus?.state === "restartRequired"}
          <button class="btn btn--primary" type="button" onclick={restartAfterAppUpdate} disabled={!canRestartAfterUpdate(appUpdateStatus)}>
            {restartingAfterUpdate ? "Restarting" : "Restart to Update"}
          </button>
        {:else}
          <button class="btn btn--primary" type="button" onclick={installAppUpdate} disabled={!canInstallAppUpdate(appUpdateStatus)}>
            {installingAppUpdate || appUpdateStatus?.state === "downloading" || appUpdateStatus?.state === "installing" ? "Installing" : "Install Update"}
          </button>
        {/if}
        {#if appUpdateStatus?.recordingActive && appUpdateStatus?.update}
          <span class="action-hint action-hint--warn">Stop recording to install this update.</span>
        {/if}
      </div>

      {#if appUpdateActionError}
        <div class="inline-error">
          <span class="inline-error__icon">⚠</span>
          <span class="inline-error__msg">{appUpdateActionError}</span>
          <button class="btn btn--ghost btn--sm" onclick={() => appUpdateActionError = null}>×</button>
        </div>
      {/if}
    </section>
  </div>
{/if}

<!-- ── Access ───────────────────────────────────────────────────────────── -->
{#if activeTab === "access"}
  <div role="tabpanel" id="settings-panel-access" aria-labelledby="settings-tab-access" tabindex="0">
    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          <h2 class="card__title">Access</h2>
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
            <button class="btn btn--ghost btn--sm" type="button" disabled={brokerGrantSaving || brokerGrantLoading || mnemaCliLoading || piRuntimeLoading} onclick={() => { void loadBrokerGrants(); void loadMnemaCliStatus(); void loadPiRuntimeStatus(); }}>
              Refresh
            </button>
          </div>
          {#if mnemaCliError}
            <p class="error-text">{mnemaCliError}</p>
          {/if}
          {#if piRuntimeError}
            <p class="error-text">{piRuntimeError}</p>
          {/if}
          {#if brokerGrantError}
            <p class="error-text">{brokerGrantError}</p>
          {/if}
          {#if brokerGrantLoading && brokerGrants.length === 0}
            <p class="group-hint">Loading grants…</p>
          {:else if brokerGrants.length > 0}
            <ul class="grant-list">
              {#each brokerGrants as grant (grant.id)}
                {@const status = grantStatus(grant)}
                <li class="grant-row" class:grant-row--inactive={status !== "active"}>
                  <span class="grant-row__status grant-row__status--{status}" aria-hidden="true"></span>
                  <div class="grant-row__meta">
                    <span class="grant-row__name" title={grant.label}>{grant.label}</span>
                    <span class="grant-row__detail">
                      <span class="grant-row__scope">{formatGrantScope(grant.scope)}</span>
                      <span class="grant-row__sep" aria-hidden="true">·</span>
                      <span title={new Date(grant.expiresAtUnixMs).toLocaleString()}>{grantStatusLabel(grant)}</span>
                    </span>
                  </div>
                  <button
                    class="btn btn--ghost btn--sm"
                    type="button"
                    disabled={brokerGrantSaving || status !== "active"}
                    onclick={() => revokeAgentBrokerGrant(grant.id)}
                  >
                    Revoke
                  </button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="group-hint">No CLI Access grants yet. Tools you approve will appear here.</p>
          {/if}
        </div>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">Ask AI</span>
        <div class="settings-stack">
          <Switch
            bind:checked={draftAskAiEnabled}
            label="Enable Ask AI"
            description="Allow Mnema to send your questions plus redacted capture context to your configured PI provider. Off by default."
          />
          <div class="privacy-disclosure">
            <p>Ask AI can answer with redacted screen text, audio transcripts, and timeline results from your retained history after redaction.</p>
            <p>When enabled, questions and the redacted context needed to answer them are sent through PI to your configured provider/cloud. Mnema never asks for or stores provider credentials here.</p>
          </div>
          <Switch
            bind:checked={draftAskAiLimitToolCalls}
            label="Limit tool calls per question"
            description="Cap how many follow-up searches Ask AI can run for one question. Off means no cap."
          />
          {#if draftAskAiLimitToolCalls}
            <label class="field-label" for="ask-ai-max-tool-calls">Max tool calls per question</label>
            <input
              id="ask-ai-max-tool-calls"
              class="text-input"
              type="number"
              min="1"
              max="500"
              step="1"
              bind:value={draftAskAiMaxToolCalls}
            />
            <p class="group-hint">
              Each tool call is one brokered query into your redacted capture history. A lower cap bounds how much a single answer can pull; the default is {ASK_AI_DEFAULT_TOOL_CALL_LIMIT}.
            </p>
          {:else}
            <p class="group-hint group-hint--warn">
              No cap: a single question can issue unlimited brokered queries into your retained capture history.
            </p>
          {/if}
          <label class="field-label" for="ask-ai-model">Quick Recall model</label>
          <div class="model-combobox">
            <input
              id="ask-ai-model"
              class="text-input model-combobox__input"
              role="combobox"
              aria-expanded={askAiModelOpen}
              aria-controls="ask-ai-model-list"
              aria-autocomplete="list"
              autocomplete="off"
              placeholder="Use PI default"
              disabled={!draftAskAiEnabled}
              bind:this={askAiModelInputEl}
              bind:value={askAiModelQuery}
              oninput={() => { openAskAiModelMenu(); askAiModelHighlight = 0; }}
              onfocus={(event) => { openAskAiModelMenu(); event.currentTarget.select(); }}
              onblur={closeAskAiModelSoon}
              onkeydown={handleAskAiModelKeydown}
            />
            {#if askAiModelOpen && draftAskAiEnabled}
              <Portal>
                <div
                  class="model-combobox__panel"
                  id="ask-ai-model-list"
                  role="listbox"
                  style={askAiModelPanelStyle}
                >
                  {#if askAiModelsLoading}
                    <span class="model-combobox__empty">Loading models from PI…</span>
                  {:else if askAiModelFiltered.length > 0}
                    {#each askAiModelFiltered as entry, index (entry.value)}
                      <button
                        class="model-combobox__option"
                        class:model-combobox__option--active={index === askAiModelHighlight}
                        type="button"
                        role="option"
                        aria-selected={entry.value === draftAskAiModel}
                        onmousedown={(event) => event.preventDefault()}
                        onmouseenter={() => { askAiModelHighlight = index; }}
                        onclick={() => commitAskAiModel(entry.value)}
                      >
                        <span class="model-combobox__option-main">
                          <span class="model-combobox__name">{entry.label}</span>
                          {#if entry.sublabel}
                            <span class="model-combobox__sub">{entry.sublabel}</span>
                          {/if}
                        </span>
                        {#if entry.value === draftAskAiModel}
                          <span class="model-combobox__check" aria-hidden="true">✓</span>
                        {/if}
                      </button>
                    {/each}
                  {:else}
                    <span class="model-combobox__empty">
                      {askAiModelQuery.trim().includes(":")
                        ? `Press Enter to use "${askAiModelQuery.trim()}"`
                        : "No matching models"}
                    </span>
                  {/if}
                </div>
              </Portal>
            {/if}
          </div>
          {#if askAiModelsError}
            <p class="group-hint group-hint--warn">
              Could not list PI models, so only the PI default is guaranteed. Set up PI and refresh status — you can still type a model id as provider:id.
            </p>
          {:else}
            <p class="group-hint">
              Type to filter the models from your PI runtime. "Use PI default" follows the model configured in PI.
            </p>
          {/if}
          <div class="model-status" class:model-status--available={draftAskAiEnabled && piRuntimeStatus?.ready}>
            <div>
              <div class="model-status__title">Ask AI {askAiStatusLabel(piRuntimeStatus)}</div>
              <div class="model-status__meta">{askAiStatusDetail(piRuntimeStatus)}</div>
            </div>
            <span class="model-status__pill">{draftAskAiEnabled && piRuntimeStatus?.ready ? "available" : "unavailable"}</span>
          </div>
          {#if piRuntimeStatus}
            <p class="group-hint">
              PI: {piRuntimeStatus.ready
                ? `ready via ${formatPiRuntimeSource(piRuntimeStatus.source)}${piRuntimeStatus.executablePath ? ` at ${piRuntimeStatus.executablePath}` : ""}`
                : formatPiRuntimeReason(piRuntimeStatus)}
            </p>
            <p class="group-hint">
              PI auth: {piRuntimeStatus.authJsonExists ? `found at ${piRuntimeStatus.authJsonPath}` : `not found at ${piRuntimeStatus.authJsonPath}`}.
              Providers configured: {piProviderCount(piRuntimeStatus)}.
            </p>
            {#if draftAskAiEnabled && !piRuntimeStatus.ready}
              <p class="group-hint group-hint--warn">Set up PI and configure a provider in PI auth, then refresh status. Do not enter provider credentials in Mnema.</p>
            {/if}
          {:else if piRuntimeLoading}
            <p class="group-hint">Checking PI setup…</p>
          {:else}
            <p class="group-hint group-hint--warn">Refresh PI status before enabling Ask AI.</p>
          {/if}
          <div class="row-actions">
            <button class="btn btn--ghost btn--sm" type="button" disabled={piRuntimeLoading} onclick={loadPiRuntimeStatus}>
              {piRuntimeLoading ? "Checking" : "Refresh PI status"}
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="card">
      <div class="card__header">
        <div class="card__heading">
          <h2 class="card__title">Reasoning Engine</h2>
          <p class="card__subtitle">
            The model Mnema uses to derive understanding from your capture history. Pick a cloud
            provider with your own key, or a local model that never leaves your machine.
          </p>
        </div>
      </div>

      <div class="settings-group">
        <span class="group-label">Engine</span>
        <div class="settings-stack">
          <div class="privacy-disclosure">
            <p>A cloud engine sends redacted capture text to your chosen provider over HTTPS to reason about it. That means continuous outbound egress and per-token cost billed to your provider account.</p>
            <p>A local engine runs entirely on this machine through Ollama or Llamafile — nothing is sent anywhere and no API key is needed.</p>
          </div>
          <Switch
            bind:checked={draftAiEnabled}
            label="Enable Reasoning Engine"
            description="Allow Mnema to run the selected model. Off by default."
          />
          <RadioGroup
            value={draftAiEngineKind}
            onValueChange={(value) => (draftAiEngineKind = value as AiEngineKind)}
            label="Engine"
            disabled={!draftAiEnabled}
            options={[
              { value: "cloud", label: "Cloud", description: "HTTPS to a provider with your own key. Sends redacted text off-device." },
              { value: "local", label: "Local", description: "Ollama or Llamafile on this machine. No key, no egress." },
            ]}
          />

          {#if draftAiEngineKind === "cloud"}
            <div class="settings-divider"></div>
            <RadioGroup
              value={draftAiCloudProvider}
              onValueChange={(value) => {
                draftAiCloudProvider = value as AiCloudProvider;
                void refreshAiProviderKeyPresence();
              }}
              label="Provider"
              disabled={!draftAiEnabled}
              options={[
                { value: "anthropic", label: "Anthropic", description: "Claude models" },
                { value: "openai", label: "OpenAI", description: "GPT models" },
                { value: "openai_compatible", label: "OpenAI-compatible", description: "Fireworks, OpenRouter, Together — custom base URL + key" },
              ]}
            />
            {#if draftAiCloudProvider === "openai_compatible"}
              <label class="field-label" for="ai-cloud-base-url">Base URL</label>
              <input
                id="ai-cloud-base-url"
                class="text-input"
                autocomplete="off"
                placeholder="https://api.fireworks.ai/inference/v1"
                disabled={!draftAiEnabled}
                bind:value={draftAiCloudBaseUrl}
              />
            {/if}
            <label class="field-label" for="ai-cloud-model">Model</label>
            <input
              id="ai-cloud-model"
              class="text-input"
              autocomplete="off"
              placeholder={draftAiCloudProvider === "openai_compatible" ? "accounts/fireworks/models/…" : "claude-haiku-4-5"}
              disabled={!draftAiEnabled}
              bind:value={draftAiCloudModel}
            />
            <label class="field-label" for="ai-cloud-key">API key</label>
            <div class="row-actions">
              <input
                id="ai-cloud-key"
                class="text-input"
                type="password"
                autocomplete="off"
                placeholder={aiProviderKeySaved ? "A key is saved — enter a new one to replace it" : "Paste your provider API key"}
                disabled={!draftAiEnabled || aiProviderKeySaving}
                bind:value={aiProviderKeyInput}
              />
              {#if aiProviderKeySaved}
                <span class="saved-badge">✓ key saved</span>
              {/if}
            </div>
            <div class="row-actions">
              <button
                class="btn btn--ghost btn--sm"
                type="button"
                disabled={!draftAiEnabled || aiProviderKeySaving || aiProviderKeyInput.trim().length === 0}
                onclick={saveAiProviderKey}
              >
                {aiProviderKeySaving ? "Saving" : "Save key"}
              </button>
              <button
                class="btn btn--ghost btn--sm"
                type="button"
                disabled={aiProviderKeySaving || !aiProviderKeySaved}
                onclick={clearAiProviderKey}
              >
                Clear
              </button>
            </div>
            <p class="group-hint">Your key is stored only in the macOS keychain — never in Mnema's settings, config, or save directory.</p>
            {#if aiProviderKeyError}
              <p class="error-text">{aiProviderKeyError}</p>
            {/if}
          {:else}
            <div class="settings-divider"></div>
            <RadioGroup
              value={draftAiLocalKind}
              onValueChange={(value) => (draftAiLocalKind = value as AiLocalKind)}
              label="Local runtime"
              disabled={!draftAiEnabled}
              options={[
                { value: "ollama", label: "Ollama", description: "Default endpoint http://localhost:11434" },
                { value: "llamafile", label: "Llamafile", description: "OpenAI-compatible local server" },
              ]}
            />
            <label class="field-label" for="ai-local-endpoint">Endpoint</label>
            <input
              id="ai-local-endpoint"
              class="text-input"
              autocomplete="off"
              placeholder="http://localhost:11434"
              disabled={!draftAiEnabled}
              bind:value={draftAiLocalEndpoint}
            />
            <label class="field-label" for="ai-local-model">Model</label>
            <input
              id="ai-local-model"
              class="text-input"
              autocomplete="off"
              placeholder="e.g. llama3.1"
              disabled={!draftAiEnabled}
              bind:value={draftAiLocalModel}
            />
          {/if}
        </div>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">Status</span>
        <div class="settings-stack">
          <div class="model-status" class:model-status--available={aiRuntimeStatus?.available}>
            <div>
              <div class="model-status__title">Reasoning Engine {aiRuntimeStatus?.available ? "ready" : "unavailable"}</div>
              <div class="model-status__meta">
                {#if aiRuntimeStatusLoading}
                  Checking engine…
                {:else if aiRuntimeStatus?.available}
                  {aiRuntimeStatus.engineKind === "cloud" ? "Cloud" : "Local"} engine configured and reachable.
                {:else}
                  {aiRuntimeReasonLabel(aiRuntimeStatus?.reason)}
                {/if}
              </div>
            </div>
            <span class="model-status__pill">{aiRuntimeStatus?.available ? "available" : "unavailable"}</span>
          </div>
          {#if aiRuntimeStatusError}
            <p class="error-text">{aiRuntimeStatusError}</p>
          {/if}
          <div class="row-actions">
            <button class="btn btn--ghost btn--sm" type="button" disabled={aiRuntimeStatusLoading} onclick={loadAiRuntimeStatus}>
              {aiRuntimeStatusLoading ? "Refreshing" : "Refresh"}
            </button>
            <button
              class="btn btn--ghost btn--sm"
              type="button"
              disabled={!draftAiEnabled || aiRuntimeTestRunning}
              onclick={runAiRuntimeTestConnection}
            >
              {aiRuntimeTestRunning ? "Testing" : "Test connection"}
            </button>
          </div>
          {#if aiRuntimeTestResult}
            <div class="cleanup-result" aria-live="polite">
              <strong>{aiRuntimeTestResult.message || "Connection succeeded."}</strong>
              <p>Model: {aiRuntimeTestResult.model || "(none)"}</p>
              {#if aiRuntimeTestResult.rawJson}
                <pre class="ai-runtime-raw">{aiRuntimeTestResult.rawJson}</pre>
              {/if}
            </div>
          {/if}
          {#if aiRuntimeTestError}
            <p class="group-hint group-hint--warn">Test connection failed.</p>
            <p class="error-text">{aiRuntimeTestError}</p>
          {/if}
        </div>
      </div>

      <div class="settings-divider"></div>

      <div class="settings-group">
        <span class="group-label">User Context</span>
        <div class="settings-stack">
          <div
            class="model-status"
            class:model-status--available={userContextStatus?.engineAvailable}
          >
            <div>
              <div class="model-status__title">
                {userContextStatus?.engineAvailable ? "Deriving Activities" : "Derivation paused"}
              </div>
              <div class="model-status__meta">
                {#if userContextStatus}
                  {userContextStatus.activityCount}
                  {userContextStatus.activityCount === 1 ? "Activity" : "Activities"} ·
                  {userContextStatus.conclusionCount}
                  {userContextStatus.conclusionCount === 1 ? "Conclusion" : "Conclusions"} ·
                  last run {formatLastDerived(userContextStatus.lastDerivedAtMs)}
                  {#if !userContextStatus.engineAvailable}
                    · {aiRuntimeReasonLabel(userContextStatus.reason)}
                  {/if}
                {:else}
                  Loading…
                {/if}
              </div>
            </div>
            <span class="model-status__pill">
              {userContextStatus?.engineAvailable ? "active" : "paused"}
            </span>
          </div>

          {#if userContextStatus}
            <p class="group-hint">
              ≈ {userContextStatus.tokenUsage.totalTokens.toLocaleString()} tokens used across
              {userContextStatus.tokenUsage.runCount}
              derivation {userContextStatus.tokenUsage.runCount === 1 ? "pass" : "passes"}
              (estimated).
            </p>
          {/if}

          {#if userContextStatusError}
            <p class="error-text">{userContextStatusError}</p>
          {/if}

          <div class="row-actions">
            <button
              class="btn btn--ghost btn--sm"
              type="button"
              disabled={userContextRunNowRunning || !userContextStatus?.engineAvailable}
              onclick={runUserContextDerivationNow}
            >
              {userContextRunNowRunning ? "Deriving" : "Run derivation now"}
            </button>
            <button
              class="btn btn--ghost btn--sm"
              type="button"
              onclick={refreshUserContext}
            >
              Refresh
            </button>
          </div>

          {#if userContextRunNowMessage}
            <p class="group-hint" aria-live="polite">{userContextRunNowMessage}</p>
          {/if}

          <div class="settings-stack">
            <span class="field-label">Recent activity</span>
            {#if userContextActivitiesError}
              <p class="error-text">{userContextActivitiesError}</p>
            {:else if userContextActivities.length === 0}
              <p class="group-hint">No Activities derived yet.</p>
            {:else}
              <ul class="user-context-activities">
                {#each userContextActivities as activity (activity.id)}
                  <li class="user-context-activity">
                    <div class="user-context-activity__title">{activity.title}</div>
                    <div class="user-context-activity__meta">
                      {#if activity.category}
                        <span class="user-context-activity__category"
                          >{activityCategoryLabel(activity.category)}</span
                        >
                      {/if}
                      <span>{formatActivityRange(activity.startedAtMs, activity.endedAtMs)}</span>
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>

          <div class="settings-stack">
            <span class="field-label">Conclusions</span>
            {#if userContextConclusionsError}
              <p class="error-text">{userContextConclusionsError}</p>
            {:else if userContextConclusions.length === 0}
              <p class="group-hint">No Conclusions distilled yet.</p>
            {:else}
              <ul class="user-context-conclusions">
                {#each userContextConclusions as conclusion (conclusion.id)}
                  <li
                    class="user-context-conclusion"
                    class:user-context-conclusion--faded={conclusion.status === "faded"}
                  >
                    <div class="user-context-conclusion__statement">
                      {conclusion.statement}
                      {#if conclusion.status === "faded"}
                        <span class="user-context-conclusion__tag" title="Below the display floor; its history is kept">faded</span>
                      {/if}
                    </div>
                    <div class="user-context-conclusion__meta">
                      <span class="user-context-conclusion__subject">{conclusion.subject}</span>
                      <span class="user-context-conclusion__confidence"
                        >{formatConfidencePercent(conclusion.confidence)} confidence</span
                      >
                      <span>
                        {conclusionEvidenceCount(conclusion)}
                        {conclusionEvidenceCount(conclusion) === 1 ? "Activity" : "Activities"}
                      </span>
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
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
      <h2 id="card-capture" class="card__title">Capture</h2>
      <p class="card__subtitle">What gets recorded and how often segments roll over.</p>
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

  {/if}
</section>
</div>
{/if}

{#if activeTab === "shortcuts"}
<div role="tabpanel" id="settings-panel-shortcuts" aria-labelledby="settings-tab-shortcuts" tabindex="0">
<section class="card">
  <div class="card__header">
    <div class="card__heading">
      <h2 class="card__title">Keyboard Shortcuts</h2>
      <p class="card__subtitle">View and customize Mnema keyboard shortcuts.</p>
    </div>
    <div class="card__actions">
      <button class="btn btn--ghost btn--sm" onclick={loadKeyboardBindingsSettings} disabled={savingKeyboardBindings}>
        Reload
      </button>
      <button class="btn btn--ghost btn--sm" onclick={restoreDefaultShortcuts} disabled={savingKeyboardBindings}>
        Restore defaults
      </button>
    </div>
  </div>

  {#if keyboardBindingsSettings === null}
    <p class="loading-text">Loading shortcuts…</p>
  {:else}
    <div class="settings-group">
      <span class="group-label">Background shortcuts</span>
      <Switch
        bind:checked={draftGlobalShortcutsEnabled}
        label="Global shortcuts"
        description="Use system-wide shortcuts for recording and showing Mnema while it is in the background"
      />
      <p class="group-hint">Background shortcuts require Command/Control or Alt. Foreground shortcuts are ignored while typing in text fields.</p>
      <p class="group-hint">Click a shortcut to rebind it, then press the keys. <strong>Esc</strong> cancels, <strong>⌫</strong> clears. Changes save automatically.</p>
    </div>

    {#if keyboardShortcutSaveBlocked && Object.keys(keyboardShortcutIssues).length > 0}
      <div class="inline-error" role="alert">
        <span class="inline-error__icon" aria-hidden="true">⚠</span>
        <span class="inline-error__msg">Resolve shortcut conflicts or invalid shortcuts before changes are saved.</span>
      </div>
    {/if}

    {#each ["global", "app", "dashboard", "audioDrawer"] as category (category)}
      <div class="settings-divider"></div>
      <div class="settings-group">
        <span class="group-label">{shortcutCategoryLabel(category)}</span>
        <div class="shortcut-editor-list">
          {#each shortcutCategoryActions(category) as action (action.id)}
            {@const binding = shortcutDraftBinding(action.id)}
            {@const issue = shortcutIssueFor(action.id)}
            {@const tokens = shortcutKeyTokens(binding)}
            {@const listening = shortcutCaptureActionId === action.id}
            <div class="shortcut-editor-row" class:shortcut-editor-row--error={issue !== null} class:shortcut-editor-row--listening={listening}>
              <div class="shortcut-editor-row__main">
                <span class="shortcut-editor-row__title">{action.label}</span>
                <span class="shortcut-editor-row__description">{action.description}</span>
                {#if issue}
                  <span class="shortcut-editor-row__error">{issue}</span>
                {/if}
              </div>
              <div class="shortcut-editor-row__controls">
                <button
                  class="shortcut-capture"
                  class:shortcut-capture--recording={listening}
                  class:shortcut-capture--empty={!tokens && !listening}
                  type="button"
                  data-shortcut-capture={action.id}
                  aria-label={listening ? `Listening for ${action.label} shortcut` : `Set shortcut for ${action.label}`}
                  onclick={(event) => { startShortcutCapture(action.id); event.currentTarget.focus(); }}
                >
                  {#if listening}
                    <span class="shortcut-capture__pulse" aria-hidden="true"></span>
                    <span class="shortcut-capture__hint">Press keys…</span>
                  {:else if tokens}
                    <span class="shortcut-capture__keys">
                      {#each tokens as token, i (i)}
                        <kbd class="shortcut-cap">{token}</kbd>
                      {/each}
                    </span>
                  {:else}
                    <span class="shortcut-capture__hint">Set shortcut</span>
                  {/if}
                </button>
                <button
                  class="shortcut-icon-btn"
                  type="button"
                  title="Reset to default"
                  aria-label={`Reset ${action.label} to default`}
                  onclick={() => resetShortcut(action.id)}
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M4 4v5h5" />
                    <path d="M4 9a8 8 0 1 1-1.5 5" />
                  </svg>
                </button>
                <button
                  class="shortcut-icon-btn"
                  type="button"
                  title="Clear shortcut"
                  aria-label={`Clear ${action.label}`}
                  disabled={!binding}
                  onclick={() => clearShortcut(action.id)}
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="m6 6 12 12" />
                    <path d="m18 6-12 12" />
                  </svg>
                </button>
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/each}
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
            <h2 class="card__title">Privacy</h2>
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
          <h2 id="card-video" class="card__title">Video Output</h2>
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
          <h2 id="card-storage" class="card__title">Storage &amp; Startup</h2>
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
          <h2 class="card__title">Appearance</h2>
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
          <h2 id="card-inactivity" class="card__title">Inactivity</h2>
          <p class="card__subtitle">Pause &amp; resume rules when you step away.</p>
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
          <h2 class="card__title">OCR &amp; Previews</h2>
          <p class="card__subtitle">Choose the OCR engine, inspect model availability, and tune preview caching.</p>
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
          <h2 class="card__title">Transcription</h2>
          <p class="card__subtitle">Local speech-to-text provider and model setup for microphone audio.</p>
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
          <h2 class="card__title">Speaker analysis</h2>
          <p class="card__subtitle">Anonymous diarization first; saved-person recognition only when you explicitly opt in.</p>
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
        <SelectMenu
          value={draftSpeakerModelId ?? "__os_managed__"}
          onValueChange={chooseSpeakerModel}
          disabled={!draftSpeakerSeparateSpeakers || switchingSpeakerModel}
          label="Preset"
          options={speakerModelOptions.length > 0 ? speakerModelOptions : [
            { value: draftSpeakerModelId ?? "__os_managed__", label: "Loading preset options" },
          ]}
        />
        <p class="group-hint">
          Pick a preset by intent. Each preset's download size is shown in the list. Recognition is scoped per preset:
          switching is safe and reversible, but saved voices need a one-time re-tag under the new preset.
        </p>
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
                  {startingSpeakerDownload ? "Starting" : `Download (${formatBytes(selectedSpeakerModel.download.byteSize)})`}
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
          <h2 class="card__title">Developer &amp; Logs</h2>
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
      <h2 id="card-mic" class="card__title">Microphone Controller</h2>
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
  </div><!-- /.settings-content -->
</div><!-- /.settings-shell -->

<style>
  /* ── Settings shell ───────────────────────────────────────────────────
     Two columns: a fixed nav rail on the left, one scrolling content pane
     on the right. The shell is the sole child of `.app-content` (a flex
     column with `min-height: 0`), so `flex: 1` + `min-height: 0` hand the
     leftover height down to `.settings-scroll`, which owns the overflow —
     the window and rail never scroll. */
  .settings-shell {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    gap: 18px;
  }

  /* ── Sidebar rail ─────────────────────────────────────────────────── */
  .settings-sidebar {
    position: relative;
    flex: 0 0 230px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    gap: 14px;
    padding-right: 18px;
    /* Only flex-basis + padding animate on collapse. Labels are toggled, not
       reflowed, so this stays a single cheap interpolation. */
    transition:
      flex-basis 0.24s cubic-bezier(0.22, 1, 0.36, 1),
      padding 0.24s cubic-bezier(0.22, 1, 0.36, 1);
  }

  /* Rail divider: a dashed hairline that fades toward the top and bottom
     edges. Uses a repeating gradient for the dashes and a mask for the fade. */
  .settings-sidebar::after {
    content: "";
    position: absolute;
    inset: 0 0 0 auto;
    width: 1px;
    background: repeating-linear-gradient(
      to bottom,
      transparent 0px,
      transparent 3px,
      var(--app-border-strong) 3px,
      var(--app-border-strong) 8px
    );
    -webkit-mask-image: linear-gradient(
      to bottom,
      transparent,
      black 14%,
      black 86%,
      transparent
    );
    mask-image: linear-gradient(
      to bottom,
      transparent,
      black 14%,
      black 86%,
      transparent
    );
  }

  .settings-sidebar--collapsed {
    flex-basis: 60px;
    padding-right: 8px;
  }

  /* The nav list scrolls on its own if the categories ever outgrow the
     rail; the brand and capture footer stay pinned. */
  .settings-nav {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .settings-nav__list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 1px;
  }

  .settings-nav__item {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    padding: 9px 11px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    cursor: pointer;
    font-family: inherit;
    text-align: left;
    color: var(--app-text-muted);
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .settings-nav__item:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  .settings-nav__item:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .settings-nav__item--active,
  .settings-nav__item--active:hover {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .settings-nav__icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .settings-nav__item:hover .settings-nav__icon {
    color: var(--app-text-strong);
    border-color: var(--app-border-strong);
  }

  .settings-nav__item--active .settings-nav__icon {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
    box-shadow: 0 0 10px -3px var(--app-accent-glow);
  }

  .settings-nav__icon svg {
    width: 15px;
    height: 15px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .settings-nav__text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .settings-nav__label {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    line-height: 1.25;
    color: inherit;
  }

  .settings-nav__hint {
    font-size: 10px;
    line-height: 1.3;
    letter-spacing: 0.01em;
    color: var(--app-text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .settings-nav__item--active .settings-nav__hint {
    color: color-mix(in srgb, var(--app-accent) 55%, var(--app-text-muted));
  }

  /* ── Sidebar footer: at-a-glance capture summary ──────────────────── */
  .settings-sidebar__foot {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    /* Left gutter matches the nav-icon column (1px list + 11px item) so the
       summary lines up under the categories above it. */
    padding: 12px 2px 0 12px;
  }

  /* Footer divider: a dashed hairline that fades at both ends, matching the
     rail divider treatment. */
  .settings-sidebar__foot::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 1px;
    background: repeating-linear-gradient(
      to right,
      transparent 0px,
      transparent 3px,
      var(--app-border-strong) 3px,
      var(--app-border-strong) 8px
    );
    -webkit-mask-image: linear-gradient(
      to right,
      transparent,
      black 22%,
      black 78%,
      transparent
    );
    mask-image: linear-gradient(
      to right,
      transparent,
      black 22%,
      black 78%,
      transparent
    );
  }

  .settings-sidebar__foot-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  /* ── Content pane + scroll region ─────────────────────────────────── */
  .settings-content {
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .settings-scroll {
    flex: 1 1 0;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    /* Keep panel content clear of the scrollbar gutter on the right. */
    padding-right: 6px;
  }

  /* One tab panel renders at a time. When a panel stacks several cards
     (Processing, Capture) lay them out in a column with the same 14px gap the
     scroll region uses between sibling panels, for one consistent rhythm. */
  [role="tabpanel"] {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* Auto-hiding scrollbars: invisible at rest, fade in on hover or scroll,
     accent-tinted on direct thumb interaction. */
  .settings-scroll::-webkit-scrollbar,
  .settings-nav::-webkit-scrollbar {
    width: 8px;
  }
  .settings-scroll::-webkit-scrollbar-track,
  .settings-nav::-webkit-scrollbar-track {
    background: transparent;
  }
  .settings-scroll::-webkit-scrollbar-thumb,
  .settings-nav::-webkit-scrollbar-thumb {
    background: transparent;
    border: 2px solid transparent;
    background-clip: padding-box;
    border-radius: 999px;
  }
  /* Show thumb while the container is hovered or actively scrolling. */
  .settings-scroll:hover::-webkit-scrollbar-thumb,
  .settings-scroll.is-scrolling::-webkit-scrollbar-thumb,
  .settings-nav:hover::-webkit-scrollbar-thumb {
    background: var(--app-border-hover);
    background-clip: padding-box;
  }
  /* Accent highlight when hovering directly over the thumb. */
  .settings-scroll::-webkit-scrollbar-thumb:hover,
  .settings-nav::-webkit-scrollbar-thumb:hover {
    background: var(--app-accent);
    background-clip: padding-box;
  }
  .settings-scroll,
  .settings-nav {
    scrollbar-width: thin;
    scrollbar-color: transparent transparent;
  }
  .settings-scroll:hover,
  .settings-scroll.is-scrolling,
  .settings-nav:hover {
    scrollbar-color: var(--app-border-hover) transparent;
  }

  /* ── Sidebar head: brand mark, title, collapse toggle, save status ─── */
  .settings-sidebar__head {
    display: flex;
    flex-direction: column;
    gap: 9px;
    /* Left gutter matches the nav-icon column so the brand mark and save
       status line up under the categories below. */
    padding: 2px 2px 0 12px;
  }

  .settings-sidebar__titlebar {
    display: flex;
    align-items: center;
    /* Match the nav item's icon-to-label gap so "Settings" aligns with the
       category labels. */
    gap: 11px;
  }

  /* Terminal-prompt brand chip: an SVG prompt (chevron + cursor underline)
     rather than literal ">_" text, which sat low and cramped against the
     chip's baseline. Sized to the nav-icon chip and accent-tinted so the brand
     mark heads the same icon column as the categories below. */
  .settings-sidebar__glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: 0 0 10px -3px var(--app-accent-glow);
  }

  .settings-sidebar__glyph svg {
    width: 15px;
    height: 15px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .settings-sidebar__title {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--app-text-strong);
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Collapse / expand control. The chevron points "into" the rail to collapse
     and flips 180° to point out when collapsed (see the collapsed block). */
  .settings-sidebar__toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 26px;
    height: 26px;
    padding: 0;
    border-radius: 6px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .settings-sidebar__toggle:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
  }
  .settings-sidebar__toggle:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }
  .settings-sidebar__toggle:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .settings-sidebar__toggle-icon {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.9;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 0.24s cubic-bezier(0.22, 1, 0.36, 1);
  }

  .settings-sidebar__status {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }

  .status-text {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    position: relative;
    padding-left: 12px;
    white-space: nowrap;
  }

  .status-text::before {
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

  .status-text--blocked {
    color: var(--app-warn);
  }
  .status-text--blocked::before {
    background: var(--app-warn-strong);
  }

  .status-text--ok {
    color: var(--app-accent);
  }
  .status-text--ok::before {
    background: var(--app-accent);
    box-shadow: 0 0 6px var(--app-accent-glow);
  }

  .status-text--saving {
    color: var(--app-accent-strong);
  }
  .status-text--saving::before {
    background: var(--app-accent);
    animation: status-pulse 1.1s ease-in-out infinite;
  }

  .status-text--error {
    color: var(--app-danger);
  }
  .status-text--error::before {
    background: var(--app-danger);
    box-shadow: 0 0 6px var(--app-danger);
  }

  @keyframes status-pulse {
    0%, 100% { opacity: 0.35; transform: translateY(-50%) scale(0.85); }
    50% { opacity: 1; transform: translateY(-50%) scale(1.1); }
  }

  /* ── Collapsed rail ────────────────────────────────────────────────────
     The rail narrows to an icon-only strip. Width + padding animate; labels
     are dropped outright (not faded) so the nav can never spill into a
     horizontal scrollbar mid-transition and item focus rings are never
     clipped. The chevron flips to point outward as the cue to expand. */
  .settings-sidebar--collapsed .settings-sidebar__toggle-icon {
    transform: rotate(180deg);
  }

  .settings-sidebar--collapsed .settings-sidebar__head {
    align-items: center;
    padding: 2px 0 0;
  }
  .settings-sidebar--collapsed .settings-sidebar__titlebar {
    justify-content: center;
    width: 100%;
  }
  .settings-sidebar--collapsed .settings-sidebar__glyph,
  .settings-sidebar--collapsed .settings-sidebar__title {
    display: none;
  }

  .settings-sidebar--collapsed .settings-nav__item {
    justify-content: center;
    padding: 9px 0;
    gap: 0;
  }
  .settings-sidebar--collapsed .settings-nav__text {
    display: none;
  }

  /* Save status shrinks to a single state-coloured dot (the element itself
     becomes the dot; its ::before and label are dropped). */
  .settings-sidebar--collapsed .settings-sidebar__status {
    justify-content: center;
  }
  .settings-sidebar--collapsed .status-text {
    width: 7px;
    height: 7px;
    padding-left: 0;
    border-radius: 50%;
    background: currentColor;
  }
  .settings-sidebar--collapsed .status-text::before,
  .settings-sidebar--collapsed .status-text__label {
    display: none;
  }
  .settings-sidebar--collapsed .status-text--ok,
  .settings-sidebar--collapsed .status-text--saving {
    box-shadow: 0 0 6px var(--app-accent-glow);
  }
  .settings-sidebar--collapsed .status-text--error {
    box-shadow: 0 0 6px var(--app-danger);
  }
  .settings-sidebar--collapsed .status-text--saving {
    animation: status-dot-pulse 1.1s ease-in-out infinite;
  }
  @keyframes status-dot-pulse {
    0%, 100% { opacity: 0.4; }
    50% { opacity: 1; }
  }

  /* Capture summary becomes a vertical column of source icon chips: a lit
     accent chip while capturing, a muted outline when not. The fps/resolution
     pills are text-only, so they drop out of the icon rail. */
  .settings-sidebar--collapsed .settings-sidebar__foot {
    align-items: center;
    padding-left: 0;
    padding-right: 0;
  }
  .settings-sidebar--collapsed .settings-sidebar__foot-label {
    display: none;
  }
  .settings-sidebar--collapsed .status-strip {
    flex-direction: column;
    align-items: center;
    flex-wrap: nowrap;
    gap: 6px;
  }
  .settings-sidebar--collapsed .status-pill {
    width: 28px;
    height: 28px;
    padding: 0;
    justify-content: center;
    border-radius: 6px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
  }
  .settings-sidebar--collapsed .status-pill--on {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: 0 0 10px -3px var(--app-accent-glow);
  }
  .settings-sidebar--collapsed .status-pill--info {
    display: none;
  }
  .settings-sidebar--collapsed .status-pill__dot,
  .settings-sidebar--collapsed .status-pill__label {
    display: none;
  }
  .settings-sidebar--collapsed .status-pill__icon {
    display: inline-flex;
  }

  @media (prefers-reduced-motion: reduce) {
    .settings-sidebar,
    .settings-sidebar__toggle-icon {
      transition: none;
    }
    .settings-sidebar--collapsed .status-text--saving {
      animation: none;
    }
  }

  /* (The former top page-header + horizontal tab strip are gone; their
     title, save-status, and capture-summary styles now live in the sidebar
     rail above.) */

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

  /* Source glyph: hidden while the rail is expanded (the dot + label carry the
     state there); becomes the whole indicator once the rail collapses. */
  .status-pill__icon {
    display: none;
    align-items: center;
    justify-content: center;
  }
  .status-pill__icon svg {
    width: 15px;
    height: 15px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
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
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow: hidden;
  }

  .card::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 1px;
    background: linear-gradient(90deg, transparent, var(--app-accent-strong) 20%, var(--app-accent) 50%, var(--app-accent-strong) 80%, transparent);
    opacity: 0.2;
  }

  .card--speaker::before {
    height: 2px;
    background: linear-gradient(90deg, transparent, #f59e0b 18%, var(--app-accent) 48%, #22d3ee 78%, transparent);
    opacity: 0.62;
  }

  .card__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .card__heading {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .card__actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }

  .card--combobox-open {
    overflow: visible;
    z-index: 10;
  }

  .card__title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--app-text-strong);
    line-height: 1.3;
    text-transform: none;
  }

  .card__subtitle {
    font-size: 11px;
    color: var(--app-text-muted);
    letter-spacing: 0.01em;
    line-height: 1.45;
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
    gap: 10px;
    padding: 10px 12px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 6px;
  }

  .settings-divider {
    height: 1px;
    background: var(--app-border);
  }

  .shortcut-editor-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .shortcut-editor-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 10px 14px;
    align-items: center;
    padding: 9px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
  }

  .shortcut-editor-row--error {
    border-color: var(--app-danger-border);
  }

  .shortcut-editor-row__main {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .shortcut-editor-row__title {
    color: var(--app-text);
    font-size: 12px;
    font-weight: 700;
    line-height: 1.3;
  }

  .shortcut-editor-row__description {
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.35;
  }

  .shortcut-editor-row__error {
    color: var(--app-danger-text);
    font-size: 10px;
    line-height: 1.35;
  }

  .shortcut-editor-row__controls {
    display: flex;
    gap: 6px;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
  }

  .shortcut-editor-row--listening {
    border-color: var(--app-accent-border);
  }

  .shortcut-capture {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 104px;
    min-height: 30px;
    padding: 0 10px;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    background: var(--app-surface-raised);
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
    cursor: pointer;
    transition: border-color 0.12s ease, box-shadow 0.12s ease, color 0.12s ease, background 0.12s ease;
  }

  .shortcut-capture:hover,
  .shortcut-capture:focus-visible {
    border-color: var(--app-accent-border);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
    outline: none;
  }

  .shortcut-capture--empty {
    border-style: dashed;
    color: var(--app-text-muted);
    font-weight: 600;
  }

  .shortcut-capture--recording,
  .shortcut-capture--recording:hover {
    border-style: solid;
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .shortcut-capture__keys {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  .shortcut-cap {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 4px;
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    background: var(--app-bg);
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    line-height: 1;
  }

  .shortcut-capture__hint {
    line-height: 1;
  }

  .shortcut-capture__pulse {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--app-accent);
    animation: shortcut-capture-pulse 1.3s ease-out infinite;
  }

  @keyframes shortcut-capture-pulse {
    0% { box-shadow: 0 0 0 0 var(--app-accent-glow); opacity: 1; }
    70% { box-shadow: 0 0 0 6px transparent; opacity: 0.55; }
    100% { box-shadow: 0 0 0 0 transparent; opacity: 1; }
  }

  @media (prefers-reduced-motion: reduce) {
    .shortcut-capture__pulse {
      animation: none;
    }
  }

  .shortcut-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease, background 0.12s ease;
  }

  .shortcut-icon-btn svg {
    width: 15px;
    height: 15px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .shortcut-icon-btn:hover:not(:disabled),
  .shortcut-icon-btn:focus-visible {
    border-color: var(--app-border-strong);
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    outline: none;
  }

  .shortcut-icon-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  @media (max-width: 720px) {
    .shortcut-editor-row {
      grid-template-columns: 1fr;
    }

    .shortcut-editor-row__controls {
      justify-content: flex-start;
    }
  }


  .about-card {
    gap: 14px;
  }

  .about-id {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .about-id__mark {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px 10px;
  }

  .about-id__name {
    margin: 0;
    font-size: 24px;
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }

  .about-id__version {
    padding: 2px 8px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.01em;
  }

  .about-id__version--pending {
    border-color: var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-weight: 600;
  }

  .about-id__channel {
    align-self: center;
  }

  .about-id__tag {
    max-width: 56ch;
    color: var(--app-text-muted);
    font-size: 11.5px;
    line-height: 1.55;
  }

  .about-meta {
    display: grid;
    gap: 7px;
    margin: 0;
  }

  .about-meta__row {
    display: grid;
    grid-template-columns: 84px minmax(0, 1fr);
    align-items: baseline;
    gap: 12px;
  }

  .about-meta dt {
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .about-meta dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--app-text);
    font-size: 11.5px;
    line-height: 1.4;
  }

  .about-footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px 14px;
  }

  .about-links {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
  }

  .about-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
    cursor: pointer;
    transition: color 0.12s;
  }

  .about-link:hover:not(:disabled),
  .about-link:focus-visible {
    outline: none;
    color: var(--app-accent);
  }

  .about-link__arrow {
    font-size: 10px;
    opacity: 0.7;
  }

  .about-error {
    margin: 0;
  }

  .update-channel-control {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }

  .update-channel-control__option {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
    padding: 10px 12px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    font: inherit;
    text-align: left;
    transition: border-color 0.12s, background 0.12s, color 0.12s;
  }

  .update-channel-control__option:hover:not(:disabled) {
    border-color: var(--app-border-strong);
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  .update-channel-control__option:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .update-channel-control__option:disabled {
    cursor: not-allowed;
    opacity: 0.7;
  }

  .update-channel-control__option--active {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }

  .update-channel-control__option span {
    font-size: 12px;
    font-weight: 700;
  }

  .update-channel-control__option small {
    color: var(--app-text-faint);
    font-size: 10px;
    line-height: 1.4;
  }

  .preview-warning {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--app-warn-border);
    border-radius: 7px;
    background: color-mix(in srgb, var(--app-warn) 8%, transparent);
  }

  .preview-warning strong {
    color: var(--app-text-strong);
    font-size: 12px;
  }

  .preview-warning p {
    margin: 3px 0 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }

  .update-status-panel {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
    padding: 12px;
    border: 1px solid color-mix(in srgb, var(--app-accent) 28%, var(--app-border));
    border-radius: 7px;
    background: color-mix(in srgb, var(--app-accent) 6%, transparent);
  }

  .update-status-panel--error {
    border-color: var(--app-warn-border);
    background: color-mix(in srgb, var(--app-warn) 7%, transparent);
  }

  .update-status-panel__main {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 6px;
  }

  .update-status-panel__headline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .update-status-panel__headline strong {
    color: var(--app-text-strong);
    font-size: 13px;
  }

  .update-status-panel p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }

  .update-status-panel__meta,
  .update-status-panel__date {
    color: var(--app-text-faint);
    font-size: 10px;
    line-height: 1.4;
  }

  .update-status-panel__date {
    flex: 0 0 auto;
  }

  .release-notes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
  }

  .release-notes p {
    display: -webkit-box;
    margin: 0;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.55;
    white-space: pre-line;
  }

  .action-hint {
    color: var(--app-text-faint);
    font-size: 10px;
    line-height: 1.4;
  }

  .action-hint--warn {
    color: var(--app-warn);
    font-weight: 600;
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

  /* Editable, filterable model picker (combobox): a text input over a floating
     listbox panel, matching the App Privacy Exclusion combobox idiom. */
  .model-combobox {
    position: relative;
    min-width: 0;
  }

  .model-combobox__input {
    width: 100%;
  }

  /* Positioning (position/top/left/width) is supplied inline because the panel
     is portaled to <body>; only its appearance lives here. */
  .model-combobox__panel {
    z-index: 9999;
    display: flex;
    max-height: 260px;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    background: var(--app-surface-raised);
    box-shadow: 0 12px 30px color-mix(in srgb, var(--app-bg) 34%, transparent);
  }

  .model-combobox__option {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 7px 9px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: transparent;
    color: var(--app-text);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }

  .model-combobox__option--active,
  .model-combobox__option:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }

  .model-combobox__option-main {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .model-combobox__name {
    overflow: hidden;
    color: var(--app-text-strong);
    font-size: 12px;
    font-weight: 700;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .model-combobox__sub {
    overflow: hidden;
    color: var(--app-text-faint);
    font-size: 10px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .model-combobox__check {
    flex: 0 0 auto;
    color: var(--app-accent);
    font-size: 12px;
    font-weight: 800;
  }

  .model-combobox__empty {
    padding: 10px;
    color: var(--app-text-faint);
    font-size: 11px;
    font-style: italic;
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

  /* User Context recent-Activity preview list (read-only). */
  .user-context-activities {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .user-context-activity {
    padding: 8px 10px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-accent) 4%, transparent);
  }

  .user-context-activity__title {
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text);
    line-height: 1.4;
  }

  .user-context-activity__meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 3px;
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--app-text-muted);
  }

  .user-context-activity__category {
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-accent);
  }

  /* User Context Conclusion preview list (read-only; #94). Row layout matches
     the Activity preview so the two lists read as one dossier. Pin/Dismiss
     controls are intentionally absent here (they belong to #99). */
  .user-context-conclusions {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .user-context-conclusion {
    padding: 8px 10px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-accent) 4%, transparent);
  }

  /* A faded Conclusion (#95) is below the display floor: it leaves the bright
     dossier but is kept (its Confidence History persists), so it reads as dimmed
     here rather than absent. */
  .user-context-conclusion--faded {
    opacity: 0.62;
    background: transparent;
    border-style: dashed;
  }

  .user-context-conclusion__statement {
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text);
    line-height: 1.4;
  }

  .user-context-conclusion__tag {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 5px;
    border: 1px solid var(--app-border);
    border-radius: 3px;
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-muted);
    vertical-align: middle;
  }

  .user-context-conclusion__meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 3px;
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--app-text-muted);
  }

  .user-context-conclusion__subject {
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-accent);
  }

  .user-context-conclusion__confidence {
    font-variant-numeric: tabular-nums;
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

  .ai-runtime-raw {
    margin: 0;
    padding: 8px 10px;
    border: 1px solid color-mix(in srgb, var(--app-accent) 24%, var(--app-border));
    border-radius: 4px;
    background: color-mix(in srgb, var(--app-accent) 4%, transparent);
    color: var(--app-text-muted);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10px;
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 160px;
    overflow: auto;
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

  /* The About tab's update actions are the only primary buttons in this
     page. Without this rule they fall back to the UA default button face
     (a light fill) which looks foreign in the dark shell — match the soft
     green primary used by the debug and access-request pages. */
  .btn--primary {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  .btn--primary:not(:disabled):hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
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

  /* ── CLI Access grant rows ───────────────────────────────────────── */
  .grant-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .grant-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 9px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
  }

  .grant-row--inactive {
    background: transparent;
  }

  .grant-row__status {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--app-text-subtle);
  }

  .grant-row__status--active {
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .grant-row__status--expired {
    background: var(--app-warn);
  }

  .grant-row__status--revoked {
    background: var(--app-danger);
  }

  .grant-row__meta {
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .grant-row__name {
    overflow: hidden;
    color: var(--app-text);
    font-size: 12px;
    font-weight: 700;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grant-row--inactive .grant-row__name {
    color: var(--app-text-muted);
    font-weight: 600;
  }

  .grant-row__detail {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.35;
  }

  .grant-row__scope {
    color: var(--app-text);
    font-weight: 600;
  }

  .grant-row--inactive .grant-row__scope {
    color: var(--app-text-muted);
    font-weight: 500;
  }

  .grant-row__sep {
    color: var(--app-text-faint);
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
  /* Sidebar rail, nav, and save status read from semantic tokens that
     already flip per theme; only the status text needs a touch more
     contrast against the light surface. */
  :global([data-theme="light"]) .status-text {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .settings-nav__item:hover .settings-nav__icon {
    border-color: var(--app-border-strong);
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

  :global([data-theme="light"]) .about-id__version {
    color: var(--app-accent-strong);
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
