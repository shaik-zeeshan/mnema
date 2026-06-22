<script lang="ts">
  // Settings shell — Slice-5 shell-ification.
  //
  // The 12 legacy `{#if activeTab === ...}` panels were split into per-section
  // panel components grouped into 5 navigation groups (see lib/settings/groups.ts).
  // This shell is thin: it builds the single SettingsController (shared with every
  // panel via context), resolves `?tab`/`?focus` deeplinks to a (group, section
  // anchor) via groups.ts, runs the mount/autosave/validation/realtime effects,
  // and renders the rail + the active group's panel. All draft state, loaders,
  // helpers, and derivations live in the controller + the domain stores it owns;
  // the panels are dumb markup that read the controller.
  //
  // INVARIANTS preserved verbatim from the legacy page:
  //  • the mount `untrack(() => { ... })` block (see settings-mount-untrack.test),
  //  • the single debounced autosave driver $effect → engine.tick(),
  //  • the recording-validation coercion effects,
  //  • the realtime listeners + their teardown.

  import { page } from "$app/stores";
  import { tick, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import AppPrivacyExclusionPrompt from "$lib/components/AppPrivacyExclusionPrompt.svelte";
  import {
    RECORDING_AUTOSAVE_DOMAINS,
  } from "$lib/settings/state/autosave-core";
  import { parseCustomDimension } from "$lib/settings/state/recording-validation";
  import {
    SettingsController,
    setSettingsController,
  } from "$lib/settings/state/controller.svelte";
  import {
    SETTINGS_GROUPS,
    groupForSection,
    resolveTabDeeplink,
    resolveFocusDeeplink,
    sectionForFocus,
    sectionAnchor,
    type SettingsGroupId,
    type SettingsSectionId,
  } from "$lib/settings/groups";
  // Shared `.settings-shell` styles, split per concern (≤800 lines each),
  // imported in SOURCE ORDER (cascade-critical; theme last). Map: settings-layout.css.
  import "$lib/settings/settings-layout.css";
  import "$lib/settings/settings-groups.css";
  import "$lib/settings/settings-controls.css";
  import "$lib/settings/settings-blocks.css";
  import "$lib/settings/settings-theme.css";
  import SettingsRail from "$lib/settings/SettingsRail.svelte";
  import GeneralPanel from "$lib/settings/panels/general/GeneralPanel.svelte";
  import CapturePanel from "$lib/settings/panels/capture/CapturePanel.svelte";
  import IntelligencePanel from "$lib/settings/panels/intelligence/IntelligencePanel.svelte";
  import DataPanel from "$lib/settings/panels/data/DataPanel.svelte";
  import AboutPanel from "$lib/settings/panels/about/AboutPanel.svelte";
  import type {
    RecordingSettings,
    RecordingSettingsDomainUpdateResponse,
    AppUpdateStatus,
    OcrModelDownloadProgress,
    AudioTranscriptionModelDownloadProgress,
    SpeakerAnalysisModelDownloadProgress,
    SemanticSearchModelDownloadProgress,
  } from "$lib/types";

  const RECORDING_SETTINGS_CHANGED_EVENT = "recording_settings_changed";
  const RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT = "recording_settings_domain_changed";
  const APP_UPDATE_STATUS_CHANGED_EVENT = "app_update_status_changed";
  const AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT = "audio_transcription_model_download_progress";
  const SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT = "speaker_analysis_model_download_progress";
  const OCR_MODEL_DOWNLOAD_PROGRESS_EVENT = "ocr_model_download_progress";
  const SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT = "semantic_search_model_download_progress";

  // The one controller, shared with every panel via context.
  const c = new SettingsController();
  setSettingsController(c);

  // Loader-name aliases so the mount untrack block reads exactly as the legacy
  // page did (the settings-mount-untrack spec asserts these tokens). They are
  // the same store methods, just bound to local names for the mount effect.
  const rec = c.rec;
  const refreshAiProviderKeyPresence = () => c.aiRuntime.refreshAiProviderKeyPresence();
  const loadAiRuntimeStatus = () => c.aiRuntime.loadAiRuntimeStatus();
  const refreshUserContext = () => c.userContext.refreshUserContext();
  const loadCaptureSupport = () => c.loadCaptureSupport();
  const loadKeyboardBindingsSettings = () => c.keyboard.loadKeyboardBindingsSettings();
  const loadMicState = () => c.audio.loadMicState();
  const loadOcrModelStatus = () => c.loadOcrModelStatus();
  const loadTranscriptionModelStatus = () => c.loadTranscriptionModelStatus();
  const loadSpeakerModelStatus = () => c.loadSpeakerModelStatus();
  const loadSemanticSearchModelStatus = () => c.loadSemanticSearchModelStatus();
  const loadSemanticSearchSupportedModels = () => c.loadSemanticSearchSupportedModels();
  const loadPersonProfileCount = () => c.loadPersonProfileCount();
  const loadDebugLogStatus = () => c.logs.loadDebugLogStatus();
  const loadGeneralLogStatus = () => c.logs.loadGeneralLogStatus();
  const loadAppUpdateStatus = () => c.about.loadAppUpdateStatus();
  const loadThirdPartyNotices = () => c.about.loadThirdPartyNotices();
  const loadBrokerGrants = () => c.cliAccess.loadBrokerGrants();
  const loadMnemaCliStatus = () => c.cliAccess.loadMnemaCliStatus();
  const loadAskAiAvailability = () => c.askAi.loadAskAiAvailability();
  const loadSettingsModels = () => c.loadSettingsModels();

  // ─── Active group + deeplink routing (driven by groups.ts) ──────────────────
  let activeGroup = $state<SettingsGroupId>("capture");
  let settingsShell = $state<HTMLDivElement | null>(null);
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

  // Scroll a section's anchor into view after the group panel has mounted.
  function scrollToSection(section: SettingsSectionId, smooth: boolean) {
    void tick().then(() => {
      const el = document.getElementById(sectionAnchor(section));
      el?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
    });
  }

  // Select a section's group and scroll to it. Used by deeplink resolution.
  function focusSettingsSection(section: SettingsSectionId, smooth = true) {
    const group = groupForSection(section);
    const groupChanged = group !== activeGroup;
    activeGroup = group;
    // Reset the scroll to the top when the group itself changes, mirroring the
    // legacy "scroll to top on tab switch" behavior, then scroll to the anchor.
    if (groupChanged) scrollRegion?.scrollTo({ top: 0, behavior: "auto" });
    scrollToSection(section, smooth);
  }

  // `$page.url`-reactive deeplink effect: resolve `?tab`/`?focus` to a section
  // (via groups.ts) and route there. A focus deeplink (cliAccess) also pops the
  // broker-authorization prompt, matching the legacy behavior.
  $effect(() => {
    const requestedTab = $page.url.searchParams.get("tab");
    const section = resolveTabDeeplink(requestedTab);
    if (section) {
      focusSettingsSection(section, false);
    }
    const focus = resolveFocusDeeplink($page.url.searchParams.get("focus"));
    if (focus) {
      const focusSection = sectionForFocus(focus);
      c.brokerAuthorizationPromptVisible = true;
      activeGroup = groupForSection(focusSection);
      void tick().then(() => {
        c.agentAccessSection?.scrollIntoView({ block: "start", behavior: "smooth" });
        c.agentAccessSection?.focus({ preventScroll: true });
      });
    }
  });

  // Reset scroll to top when the active group changes (matches legacy tabbed
  // settings: a fresh group starts at the top unless a deeplink scrolled it).
  $effect(() => {
    activeGroup;
    untrack(() => scrollRegion?.scrollTo({ top: 0, behavior: "auto" }));
  });

  // ─── Auto-save (shared engine) ──────────────────────────────────────────────
  // Register one engine unit per autosaved surface. The recording store registers
  // one per recording domain (passing the controller's per-domain save, which
  // carries the retention-cleanup confirm flow); the keyboard + audio stores each
  // register their own domain unit.
  c.rec.registerAutosave(c.autosaveEngine, (domain) => c.saveRecordingDomain(domain));
  c.keyboard.registerAutosave(c.autosaveEngine);
  c.audio.registerAutosave(c.autosaveEngine);

  // The single reactive driver: read every unit's snapshot + the gating inputs
  // so Svelte re-runs this on any relevant change, then let the engine decide
  // what to (de)schedule. Reading the snapshots here subscribes the effect to the
  // underlying draft state — the engine stays framework-free.
  $effect(() => {
    if (c.rec.recordingSettings !== null) {
      for (const domain of RECORDING_AUTOSAVE_DOMAINS) {
        void c.rec.buildRecDomainSnapshot(domain);
        void c.rec.lastSavedRecSnapshots[domain];
        void c.rec.savingRecDomains[domain];
      }
    }
    if (c.keyboard.keyboardBindingsSettings !== null) void c.keyboard.buildKeyboardBindingsSnapshot();
    void c.keyboard.lastSavedKeyboardBindingsSnapshot;
    void c.keyboard.keyboardShortcutSaveBlocked;
    void c.keyboard.savingKeyboardBindings;
    if (c.audio.micState !== null) void c.audio.buildMicSnapshot();
    void c.audio.lastSavedMicSnapshot;
    void c.audio.micApplyBlocked;
    void c.audio.savingMicSettings;
    void c.appPrivacyExclusion.commandInFlight;
    c.autosaveEngine.tick();
  });

  // ─── Recording settings validation coercion ─────────────────────────────────
  // Invariant: system audio requires screen capture.
  $effect(() => {
    if (!c.rec.draftCaptureScreen && c.rec.draftCaptureSystemAudio) {
      c.rec.draftCaptureSystemAudio = false;
    }
  });

  // Invariant: coerce any non-original draft back to "original" only once we
  // have confirmed that non-original is unsupported (AVFoundation / pre-macOS 15).
  $effect(() => {
    if (c.rec.draftCaptureScreen && c.onlyOriginalResolutionSupported && c.rec.draftResolutionMode !== "original") {
      c.rec.draftResolutionMode = "original";
    }
  });

  // Parse custom resolution inputs as integers; keep null if invalid.
  $effect(() => {
    const w = parseCustomDimension(c.rec.customWidthRaw);
    c.rec.draftCustomWidth = w ?? null;
  });
  $effect(() => {
    const h = parseCustomDimension(c.rec.customHeightRaw);
    c.rec.draftCustomHeight = h ?? null;
  });

  // Parse custom bitrate input as an integer (Mbps); keep null if invalid.
  $effect(() => {
    if (!c.rec.draftCustomMbpsRaw) { c.rec.draftCustomMbps = null; return; }
    if (!/^\d+$/.test(c.rec.draftCustomMbpsRaw.trim())) { c.rec.draftCustomMbps = null; return; }
    const val = parseInt(c.rec.draftCustomMbpsRaw.trim(), 10);
    c.rec.draftCustomMbps = Number.isInteger(val) && val > 0 ? val : null;
  });

  // ─── Init: one-time mount load + realtime listeners ─────────────────────────
  $effect(() => {
    // One-time mount init. Wrapped in `untrack` because several of these loaders
    // synchronously read draft `$state` (e.g. refreshAiProviderKeyPresence reads
    // rec.draftAiProviders). Without untrack the effect would subscribe to those
    // drafts and re-run on every edit — re-firing loadRecordingSettings and
    // clobbering the in-flight draft back to the persisted value before autosave.
    untrack(() => {
      loadCaptureSupport();
      // refreshAiProviderKeyPresence reads rec.draftAiProviders, which loadRecordingSettings
      // only populates after its async fetch resolves. Chain it so the "key in keychain"
      // badge reflects saved keys on load instead of seeing a still-empty provider list.
      void rec.loadRecordingSettings().then(() => refreshAiProviderKeyPresence());
      loadKeyboardBindingsSettings();
      loadMicState();
      loadOcrModelStatus();
      loadTranscriptionModelStatus();
      loadSpeakerModelStatus();
      void loadSemanticSearchModelStatus();
      void loadSemanticSearchSupportedModels();
      void loadPersonProfileCount();
      loadDebugLogStatus();
      loadGeneralLogStatus();
      loadAppUpdateStatus();
      void loadThirdPartyNotices();
      void c.appPrivacyExclusion.loadPrivacyAppCandidates();
      void c.appPrivacyExclusion.loadSensitiveCaptureRecommendations();
      loadBrokerGrants();
      loadMnemaCliStatus();
      void loadAskAiAvailability();
      void loadSettingsModels();
      void loadAiRuntimeStatus();
      void refreshUserContext();
    });

    let unlistenUserContextChanged: (() => void) | undefined;
    let unlistenRecordingSettingsChanged: (() => void) | undefined;
    let unlistenRecordingSettingsDomainChanged: (() => void) | undefined;
    let unlistenAppUpdateStatusChanged: (() => void) | undefined;
    let unlistenOcrDownloadProgress: (() => void) | undefined;
    let unlistenTranscriptionDownloadProgress: (() => void) | undefined;
    let unlistenSpeakerDownloadProgress: (() => void) | undefined;
    let unlistenSemanticSearchDownloadProgress: (() => void) | undefined;
    let destroyed = false;

    // The microphone controller's two listeners live on the audio store.
    const stopMicListeners = c.audio.startListeners();

    listen<RecordingSettings>(RECORDING_SETTINGS_CHANGED_EVENT, (event) => {
      c.rec.onRecordingSettingsChanged(event.payload);
      void c.appPrivacyExclusion.loadSensitiveCaptureRecommendations();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsChanged = fn;
    });

    listen<RecordingSettingsDomainUpdateResponse>(
      RECORDING_SETTINGS_DOMAIN_CHANGED_EVENT,
      (event) => {
        c.rec.onRecordingSettingsDomainChanged(event.payload);
        if (event.payload.domain === "app_privacy_exclusion" || event.payload.domain === "metadata") {
          void c.appPrivacyExclusion.loadSensitiveCaptureRecommendations();
        }
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenRecordingSettingsDomainChanged = fn;
    });

    listen<AppUpdateStatus>(APP_UPDATE_STATUS_CHANGED_EVENT, (event) => {
      c.about.setAppUpdateStatus(event.payload);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenAppUpdateStatusChanged = fn;
    });

    listen<OcrModelDownloadProgress>(
      OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void c.handleOcrDownloadProgress(event.payload);
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenOcrDownloadProgress = fn;
    });

    listen<AudioTranscriptionModelDownloadProgress>(
      AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void c.handleTranscriptionDownloadProgress(event.payload);
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenTranscriptionDownloadProgress = fn;
    });

    listen<SpeakerAnalysisModelDownloadProgress>(
      SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void c.handleSpeakerDownloadProgress(event.payload);
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSpeakerDownloadProgress = fn;
    });

    listen<SemanticSearchModelDownloadProgress>(
      SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
      (event) => {
        void c.handleSemanticSearchDownloadProgress(event.payload);
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSemanticSearchDownloadProgress = fn;
    });

    listen("user_context_changed", () => {
      void refreshUserContext();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenUserContextChanged = fn;
    });

    return () => {
      destroyed = true;
      c.autosaveEngine.cancelAll();
      stopMicListeners();
      unlistenRecordingSettingsChanged?.();
      unlistenRecordingSettingsDomainChanged?.();
      unlistenAppUpdateStatusChanged?.();
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenSpeakerDownloadProgress?.();
      unlistenSemanticSearchDownloadProgress?.();
      unlistenUserContextChanged?.();
    };
  });
</script>

<!-- ── Settings shell ──────────────────────────────────────────────────────
     A fixed left rail lists the 5 groups; only the right-hand content pane
     scrolls. One group panel is mounted at a time, so the rail and window
     chrome stay pinned. -->
<div class="settings-shell" bind:this={settingsShell}>
  <SettingsRail bind:activeGroup shellEl={settingsShell} />

  <!-- ── Content pane — only this column scrolls. -->
  <div class="settings-content">
    <AppPrivacyExclusionPrompt
      controller={c.appPrivacyExclusion}
      onReview={() => focusSettingsSection("privacy")}
    />

    <div
      class="settings-scroll"
      class:is-scrolling={scrollRegionScrolling}
      bind:this={scrollRegion}
      onscroll={handleScrollRegionScroll}
    >
      {#if activeGroup === "general"}
        <GeneralPanel />
      {:else if activeGroup === "capture"}
        <CapturePanel />
      {:else if activeGroup === "intelligence"}
        <IntelligencePanel />
      {:else if activeGroup === "data"}
        <DataPanel />
      {:else if activeGroup === "about"}
        <AboutPanel />
      {/if}
    </div>
  </div>
</div>

<style>
  /* The shell root rule lives here (its element is in this template); all other
     settings CSS is the shared, `.settings-shell`-namespaced
     lib/settings/settings-{layout,groups,controls,blocks,theme}.css imported above. */
  .settings-shell {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    gap: 18px;
  }
</style>
