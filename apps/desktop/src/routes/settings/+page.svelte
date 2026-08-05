<script lang="ts">
  // Settings shell — direction 02, Studio Shell.
  //
  // THE RAIL IS DELETED. Settings is ONE scrolling page: all five group panels
  // are mounted at once, each preceded by a sticky header that carries the
  // group's name and its position in the total ("9 – 21 of 48"). Navigation is
  // typing — a permanent filter field in the 30px tool strip (the phase-1 ⌘F
  // index, rehomed) — plus the 256px inspector on the right, which shows the
  // focused SETTING's detail rather than being a second nav.
  //
  // What that deletes: the group-switching state, the scroll-to-top-on-group
  // effect, and the whole IntersectionObserver scroll-spy. Nothing observes
  // "which section is active" any more — `position: sticky` answers it in CSS,
  // for free, and every anchor is always mounted so a deeplink is one
  // `scrollIntoView`.
  //
  // INVARIANTS preserved verbatim:
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
    resolveTabDeeplink,
    resolveFocusDeeplink,
    sectionForFocus,
    sectionAnchor,
    type SettingsSectionId,
  } from "$lib/settings/groups";
  import { settingsCensus } from "$lib/settings/row-census";
  // Shared `.settings-shell` styles, split per concern (≤800 lines each),
  // imported in SOURCE ORDER (cascade-critical; theme last). Map: settings-layout.css.
  import "$lib/settings/settings-layout.css";
  import "$lib/settings/settings-groups.css";
  import "$lib/settings/settings-controls.css";
  import "$lib/settings/settings-controls-fields.css";
  import "$lib/settings/settings-blocks.css";
  import "$lib/settings/settings-theme.css";
  import SettingsSaveChip from "$lib/settings/ui/SettingsSaveChip.svelte";
  import SettingsFindBar from "$lib/settings/ui/SettingsFindBar.svelte";
  import SettingsInspector from "$lib/settings/ui/SettingsInspector.svelte";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
  import { settingsInspector } from "$lib/settings/state/inspector.svelte";
  import IconPanel from "~icons/lucide/panel-right";
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

  // ─── Deeplink routing (driven by groups.ts) ─────────────────────────────────
  // Every panel is mounted, so a deeplink is a plain scroll to its anchor. No
  // group state, no spy, no suppression window — those existed only to keep a
  // rail highlight honest, and there is no rail.
  let scrollRegion = $state<HTMLDivElement | null>(null);
  let scrollRegionScrolling = $state(false);
  let scrollRegionScrollTimer: ReturnType<typeof setTimeout> | null = null;

  // The census answers both the sticky headers ("9 – 21 of 48") and the tool
  // strip's count, from the same enumeration the \u2318F index is tested against.
  const census = $derived(settingsCensus(settingsFind.active ? settingsFind.query : ""));

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

  // Scroll a section's anchor into view once its panel has painted. The sticky
  // header sits at the top of the scroll region, so land the anchor just under
  // it rather than behind it.
  function focusSettingsSection(section: SettingsSectionId, smooth = true) {
    void tick().then(() => {
      const el = document.getElementById(sectionAnchor(section));
      el?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
    });
  }

  // `$page.url`-reactive deeplink effect: resolve `?tab`/`?focus` to a section
  // (via groups.ts) and scroll there. A focus deeplink (cliAccess) also pops the
  // broker-authorization prompt, matching the legacy behavior.
  $effect(() => {
    const requestedTab = $page.url.searchParams.get("tab");
    const section = resolveTabDeeplink(requestedTab);
    if (section) {
      focusSettingsSection(section, false);
    }
    const focus = resolveFocusDeeplink($page.url.searchParams.get("focus"));
    if (focus) {
      void sectionForFocus(focus);
      c.brokerAuthorizationPromptVisible = true;
      void tick().then(() => {
        c.agentAccessSection?.scrollIntoView({ block: "start", behavior: "smooth" });
        c.agentAccessSection?.focus({ preventScroll: true });
      });
    }
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

<!-- ── Settings shell — direction 02 ───────────────────────────────────────
     Four fixed pieces, one scrolling region. The 38px title bar and the 24px
     status strip are the root layout's; this surface owns the 30px tool strip
     (filter · count · save chip · inspector toggle), the scroll, and the 256px
     inspector. There is no rail. -->
<div class="settings-shell" class:is-finding={settingsFind.active}>
  <!-- Page-level landmark heading for assistive tech: the shell otherwise has no
       <h1>, so the route reads as untitled to a screen reader. Visually hidden —
       the visible title is the window chrome + the sticky section headers. -->
  <h1 class="settings-page-title">Settings</h1>

  <!-- ── Tool strip. THE FILTER IS THE NAVIGATION (the rail it replaced could
       not find a setting whose section you have forgotten). -->
  <div class="ss-tstrip">
    <SettingsFindBar />
    <div class="ss-tstrip__sep"></div>
    <span class="t-meta is-mono settings-count">
      {#if settingsFind.active}
        {census.matched} of {census.total} · {census.matchedGroups}
        {census.matchedGroups === 1 ? "section" : "sections"}
      {:else}
        {census.total} settings · {census.groups.length} sections
      {/if}
    </span>
    <span class="ss-tstrip__spacer"></span>
    <!-- Autosave, top-anchored and unclippable (G7). The same state is mirrored
         into the window-edge status strip; there is no Undo beside it (G7). -->
    <SettingsSaveChip />
    <div class="ss-tstrip__sep"></div>
    <button
      class="btn btn--sm btn--icon"
      class:is-on={settingsInspector.open}
      type="button"
      aria-pressed={settingsInspector.open}
      aria-label={settingsInspector.open ? "Hide setting detail" : "Show setting detail"}
      onclick={() => settingsInspector.toggle()}
    >
      <IconPanel aria-hidden="true" />
    </button>
  </div>

  <!-- ── Body: the one scrolling region + the inspector welded to its right. -->
  <div class="ss-body">
    <div class="ss-main">
      <AppPrivacyExclusionPrompt
        controller={c.appPrivacyExclusion}
        onReview={() => focusSettingsSection("privacy")}
      />

      <div
        class="settings-scroll ss-setscroll"
        class:is-scrolling={scrollRegionScrolling}
        bind:this={scrollRegion}
        onscroll={handleScrollRegionScroll}
        data-find-query={settingsFind.query}
      >
        <!-- ONE page. Every group's panel is mounted at all times: that is what
             makes the scroll continuous, what lets ⌘F show a hit in any section
             WITH ITS LIVE CONTROL, and what makes a deeplink a plain scroll.
             Each group is introduced by a sticky header carrying its name and
             its position in the total — the address the rail used to hold. -->
        {#each census.groups as group (group.id)}
          {#if group.matches > 0}
            <div class="ss-stick">
              <span class="ss-stick__n">{group.label}</span>
              <span class="t-meta ss-stick__sub">{group.sections}</span>
              <span class="ss-stick__c">
                {#if settingsFind.active}
                  {group.matches} {group.matches === 1 ? "match" : "matches"}
                {:else}
                  {group.first} – {group.last} of {census.total}
                {/if}
              </span>
            </div>
          {/if}
          {#if group.id === "general"}
            <GeneralPanel />
          {:else if group.id === "capture"}
            <CapturePanel />
          {:else if group.id === "intelligence"}
            <IntelligencePanel />
          {:else if group.id === "data"}
            <DataPanel />
          {:else if group.id === "about"}
            <AboutPanel />
          {/if}
        {/each}
      </div>
    </div>

    {#if settingsInspector.open}
      <SettingsInspector />
    {/if}
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
    flex-direction: column;
    /* The Studio Shell's chrome is full-bleed: cancel the 8px/20px gutter the
       root layout puts on `.app-content--settings` so the tool strip, the
       sticky headers and the inspector reach the window edges. The layout is
       another agent's file, so this is the seam that does not touch it. */
    margin: -8px -20px 0;
  }

  /* The strip's own count, right of the filter — mono so it reads as a
     measurement, not a label. */
  .settings-count {
    white-space: nowrap;
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
