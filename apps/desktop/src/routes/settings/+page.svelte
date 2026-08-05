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
  import { onDestroy, tick, untrack } from "svelte";
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
    groupForSection,
    resolveTabDeeplink,
    resolveFocusDeeplink,
    sectionForFocus,
    sectionAnchor,
    DEFAULT_SETTINGS_GROUP,
    DEFAULT_SETTINGS_SECTION,
    type SettingsGroupId,
    type SettingsSectionId,
  } from "$lib/settings/groups";
  // Shared `.settings-shell` styles, split per concern (≤800 lines each),
  // imported in SOURCE ORDER (cascade-critical; theme last). Map: settings-layout.css.
  import "$lib/settings/settings-layout.css";
  import "$lib/settings/settings-groups.css";
  import "$lib/settings/settings-controls.css";
  import "$lib/settings/settings-controls-fields.css";
  import "$lib/settings/settings-blocks.css";
  import "$lib/settings/settings-theme.css";
  // Direction 01 last: it re-skins the shared parts above onto the bento tile
  // grid + AppKit metrics, so it must win the cascade. Tokens only, no theme
  // branch (bento.css owns the light values).
  import "$lib/settings/settings-bento.css";
  import SettingsTabBar from "$lib/settings/ui/SettingsTabBar.svelte";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
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
  const refreshMcpServerSecretPresence = () => c.aiRuntime.refreshMcpServerSecretPresence();
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

  // ─── Active group + sub-section + deeplink routing (driven by groups.ts) ─────
  let activeGroup = $state<SettingsGroupId>(DEFAULT_SETTINGS_GROUP);
  // The currently-active sub-section (drives the rail's active item + scroll-spy).
  // Defaults to the first section of the default group.
  let activeSection = $state<SettingsSectionId>(DEFAULT_SETTINGS_SECTION);
  let scrollRegion = $state<HTMLDivElement | null>(null);
  let scrollRegionScrolling = $state(false);
  let scrollRegionScrollTimer: ReturnType<typeof setTimeout> | null = null;

  // The is-scrolling flag (auto-hiding scrollbar) is the only scroll bookkeeping
  // left. Direction 01 replaced the rail with toolbar tabs, so there is no
  // section highlight to keep in sync — the scroll-spy observer and its
  // programmatic-scroll suppression went with the rail.
  onDestroy(() => {
    if (scrollRegionScrollTimer !== null) {
      clearTimeout(scrollRegionScrollTimer);
      scrollRegionScrollTimer = null;
    }
  });

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
      document
        .getElementById(sectionAnchor(section))
        ?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
    });
  }

  // Select a section's group and scroll to it. Used by deeplink resolution and
  // by the privacy-exclusion prompt's "Review" action. The scroll-to-top on a
  // group change is owned solely by the dedicated `activeGroup` $effect below
  // (setting `activeGroup` here triggers it); the deferred `scrollToSection`
  // then wins.
  function focusSettingsSection(section: SettingsSectionId, smooth = true) {
    activeGroup = groupForSection(section);
    activeSection = section;
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
      activeSection = focusSection;
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
        // The video unit is gated by `resolutionSupportPendingForNonOriginal`
        // (page $state, not in any snapshot). Read it so the effect re-runs and
        // flushes a stranded edit once capture-support resolves and the gate
        // clears — mirroring the keyboard/audio per-unit gate reads below.
        if (domain === "video") void c.resolutionSupportPendingForNonOriginal;
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
      // refreshMcpServerSecretPresence shares that draft-list dependency (the MCP card's
      // "secret in keychain" badge, placeholder, and Clear button read the draft connector
      // list), so chain it off the same load.
      void rec.loadRecordingSettings().then(() => refreshAiProviderKeyPresence()).then(() => refreshMcpServerSecretPresence());
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
      void c.geckoUrlAccess.load();
      void c.systemAudioAccess.load();
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

    // Accessibility is granted outside the app (System Settings), so re-poll the
    // optional Gecko browser-URL access on window focus to pick up a grant without
    // making the user click Recheck. Skip once trusted; the store's in-flight latch
    // keeps refocus storms from double-firing.
    // System audio's evidence only moves while a recording runs, so re-poll it
    // on focus too — that is when a user who just recorded comes back to look.
    const onWindowFocus = () => {
      if (!c.geckoUrlAccess.trusted) void c.geckoUrlAccess.recheck();
      void c.systemAudioAccess.load();
    };
    const hasWindow = typeof window !== "undefined";
    if (hasWindow) window.addEventListener("focus", onWindowFocus);

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
      c.cancelPendingSaveRetries();
      stopMicListeners();
      unlistenRecordingSettingsChanged?.();
      unlistenRecordingSettingsDomainChanged?.();
      unlistenAppUpdateStatusChanged?.();
      unlistenOcrDownloadProgress?.();
      unlistenTranscriptionDownloadProgress?.();
      unlistenSpeakerDownloadProgress?.();
      unlistenSemanticSearchDownloadProgress?.();
      unlistenUserContextChanged?.();
      if (hasWindow) window.removeEventListener("focus", onWindowFocus);
    };
  });
</script>

<!-- ── Settings shell (direction 01 — Bento Native) ────────────────────────
     No rail. A sticky toolbar of five tabs sits under the window chrome and
     carries the autosave chip + the scoped ⌘F field with it, so neither can
     scroll away or clip off a short window (G7). Below it, one group panel is
     mounted at a time and lays its groups out as bento tiles in two columns. -->
<div class="settings-shell" class:is-finding={settingsFind.active}>
  <!-- Page-level landmark heading for assistive tech: the shell otherwise has no
       <h1>, so the route reads as untitled to a screen reader. Visually hidden —
       the visible title is the window chrome + the toolbar tabs. -->
  <h1 class="settings-page-title">Settings</h1>

  <SettingsTabBar {activeGroup} onSelect={(group) => (activeGroup = group)} />

  <!-- ── Content pane — only this column scrolls. -->
  <div class="settings-content">
    <AppPrivacyExclusionPrompt
      controller={c.appPrivacyExclusion}
      onReview={() => focusSettingsSection("privacy")}
    />

    <div
      class="settings-scroll scroll"
      class:is-scrolling={scrollRegionScrolling}
      bind:this={scrollRegion}
      onscroll={handleScrollRegionScroll}
      data-find-query={settingsFind.query}
    >
      {#if settingsFind.active}
        <!-- ⌘F: every group's panel is mounted so a hit in any section can
             render WITH ITS LIVE CONTROL; the rows/groups that don't match hide
             themselves (see `.is-finding` in settings-layout.css). -->
        <GeneralPanel />
        <CapturePanel />
        <IntelligencePanel />
        <DataPanel />
        <AboutPanel />
      {:else if activeGroup === "general"}
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
  /* One column: the sticky tab bar, then the scrolling pane. The bento skin
     (settings-bento.css) owns everything below the root. */
  .settings-shell {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* Visually-hidden page heading — present in the AT accessibility tree as the
     route's <h1> landmark, but removed from the visual layout (the flex shell's
     two columns are unaffected). */
  .settings-page-title {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
</style>
