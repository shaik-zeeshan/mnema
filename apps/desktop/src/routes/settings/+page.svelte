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
  import { setFlushBeforeStart } from "$lib/capture-controls.svelte";
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
    DEFAULT_SETTINGS_GROUP,
    DEFAULT_SETTINGS_SECTION,
    type SettingsGroupId,
    type SettingsSectionId,
  } from "$lib/settings/groups";
  import {
    SPY_LINE_PX,
    activeSectionAt,
    isAtScrollTarget,
    isScrolledToBottom,
  } from "$lib/settings/scroll-spy";
  // Shared `.settings-shell` styles, split per concern (≤800 lines each),
  // imported in SOURCE ORDER (cascade-critical; theme last). Map: settings-layout.css.
  import "$lib/settings/settings-layout.css";
  import "$lib/settings/settings-groups.css";
  import "$lib/settings/settings-controls.css";
  import "$lib/settings/settings-controls-fields.css";
  import "$lib/settings/settings-blocks.css";
  import "$lib/settings/settings-theme.css";
  import SettingsRail from "$lib/settings/SettingsRail.svelte";
  import SettingsBar from "$lib/settings/SettingsBar.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-cw";
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
  // The header gains a hairline once the column scrolls under it.
  let scrolled = $state(false);
  const activeGroupInfo = $derived(SETTINGS_GROUPS.find((g) => g.id === activeGroup)!);

  // Autosave failures (and the recording-settings load failure, which spans
  // many sections), one notice each (see the markup).
  type SaveError = { title: string; message: string; retry: (() => void) | null; dismiss: () => void };
  const saveErrors = $derived.by<SaveError[]>(() => {
    const out: SaveError[] = [];
    if (c.rec.recError) {
      out.push({
        title: "Couldn’t save recording settings",
        message: `${c.rec.recError.replace(/\.$/, "")}. Retrying on its own — or dismiss to put the last saved value back.`,
        retry: c.lastFailedSaveDomain ? () => c.retryFailedSave() : null,
        dismiss: () => c.dismissRecError(),
      });
    }
    if (c.rec.recLoadError) {
      out.push({
        title: "Couldn’t load recording settings",
        message: `${c.rec.recLoadError.replace(/\.$/, "")}. Capture, privacy and storage show their defaults until it loads.`,
        retry: () => void c.rec.loadRecordingSettings(),
        dismiss: () => (c.rec.recLoadError = null),
      });
    }
    if (c.keyboard.keyboardBindingsError) {
      out.push({
        title: "Couldn’t save shortcuts",
        message: c.keyboard.keyboardBindingsError,
        retry: null,
        dismiss: () => (c.keyboard.keyboardBindingsError = null),
      });
    }
    if (c.audio.micError) {
      out.push({
        title: "Microphone settings hit an error",
        message: c.audio.micError,
        retry: null,
        dismiss: () => (c.audio.micError = null),
      });
    }
    return out;
  });
  let scrollRegionScrolling = $state(false);
  let scrollRegionScrollTimer: ReturnType<typeof setTimeout> | null = null;

  // Scroll-spy suppression: while a programmatic scroll (deeplink / rail click)
  // is in flight, the scroll-spy must NOT fight it by re-deriving
  // `activeSection` from section positions mid-animation. The flag is raised
  // by `focusSettingsSection` and cleared on a scroll-SETTLE signal — once the
  // region's `scrollTop` reaches the target anchor (`spySuppressTarget`) — with
  // the timer only an UPPER bound so suppression can never get stuck (a long
  // smooth jump can outlast a fixed timeout, so a blind timer would clear
  // suppression mid-animation and flicker the rail highlight).
  let spySuppressed = $state(false);
  let spySuppressTimer: ReturnType<typeof setTimeout> | null = null;
  // The scroll region's expected `scrollTop` once the in-flight programmatic
  // scroll settles, or null when none is in flight / the target is unknown.
  let spySuppressTarget: number | null = null;

  function clearSpySuppression() {
    spySuppressed = false;
    spySuppressTarget = null;
    if (spySuppressTimer !== null) {
      clearTimeout(spySuppressTimer);
      spySuppressTimer = null;
    }
  }

  function suppressSpy() {
    spySuppressed = true;
    spySuppressTarget = null;
    if (spySuppressTimer !== null) clearTimeout(spySuppressTimer);
    // Safety upper bound only — settle normally clears via the scroll handler.
    spySuppressTimer = setTimeout(clearSpySuppression, 700);
  }

  // Both scroll-spy timers are armed from event handlers (suppress-on-navigate
  // and the onscroll is-scrolling flag), not from an
  // $effect, so that effect's cleanup never clears them. Clear on destroy so a
  // navigate-away mid-settle (e.g. "← Back to app" within the 700ms suppression
  // window) leaves no timer firing into a torn-down shell.
  onDestroy(() => {
    if (spySuppressTimer !== null) {
      clearTimeout(spySuppressTimer);
      spySuppressTimer = null;
    }
    if (scrollRegionScrollTimer !== null) {
      clearTimeout(scrollRegionScrollTimer);
      scrollRegionScrollTimer = null;
    }
  });

  // Record where the in-flight programmatic scroll is heading (the anchor's
  // offset, clamped to the region's max scrollTop) so the scroll handler can
  // detect settle. Called after `scrollIntoView` requests the scroll.
  function setSpyTarget(el: HTMLElement) {
    if (!scrollRegion) return;
    const maxTop = scrollRegion.scrollHeight - scrollRegion.clientHeight;
    const margin = parseFloat(getComputedStyle(el).scrollMarginTop) || 0;
    spySuppressTarget = Math.max(0, Math.min(el.offsetTop - margin, maxTop));
  }

  function handleScrollRegionScroll() {
    scrolled = (scrollRegion?.scrollTop ?? 0) > 4;
    scrollRegionScrolling = true;
    if (scrollRegionScrollTimer !== null) clearTimeout(scrollRegionScrollTimer);
    scrollRegionScrollTimer = setTimeout(() => {
      scrollRegionScrolling = false;
      scrollRegionScrollTimer = null;
    }, 800);

    if (!scrollRegion) return;
    // Clear scroll-spy suppression on a settle signal: once the programmatic
    // scroll has carried `scrollTop` to its target anchor, the spy may drive
    // `activeSection` again. No re-derive on this event, so a clicked section
    // that can't reach the line (target clamped at the bottom) stays lit.
    if (spySuppressed) {
      if (isAtScrollTarget(scrollRegion.scrollTop, spySuppressTarget)) clearSpySuppression();
      return;
    }

    // Scroll-spy: the section whose top has crossed a line just under the
    // header, the last one at the bottom. Only the active group's panel is
    // mounted, so these are its sections in render order.
    const line = scrollRegion.getBoundingClientRect().top + SPY_LINE_PX;
    const sections = [
      ...scrollRegion.querySelectorAll<HTMLElement>('[id^="settings-section-"]'),
    ].map((el) => ({
      id: el.id.replace(/^settings-section-/, "") as SettingsSectionId,
      top: el.getBoundingClientRect().top,
    }));
    const id = activeSectionAt(sections, line, isScrolledToBottom(scrollRegion));
    if (id && id !== activeSection && groupForSection(id) === activeGroup) activeSection = id;
  }

  // Scroll a section's anchor into view after the group panel has mounted, and
  // record the settle target so suppression clears on arrival (not a blind timer).
  function scrollToSection(section: SettingsSectionId, smooth: boolean) {
    void tick().then(() => {
      // A group's first section lands at the top, so the page head shows.
      if (SETTINGS_GROUPS.some((g) => g.sections[0].id === section)) {
        scrollRegion?.scrollTo({ top: 0, behavior: smooth ? "smooth" : "auto" });
        spySuppressTarget = 0;
        return;
      }
      const el = document.getElementById(sectionAnchor(section));
      el?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
      if (el) setSpyTarget(el);
    });
  }

  // Select a section's group + sub-section and scroll to it. Used by both the
  // rail (onNavigate) and deeplink resolution. Suppresses scroll-spy so it does
  // not fight the programmatic scroll. The scroll-to-top on a group change is
  // owned solely by the dedicated `activeGroup` $effect below (setting
  // `activeGroup` here triggers it); the deferred `scrollToSection` then wins.
  function focusSettingsSection(section: SettingsSectionId, smooth = true) {
    const group = groupForSection(section);
    activeGroup = group;
    activeSection = section;
    suppressSpy();
    scrollToSection(section, smooth);
  }

  // `$page.url`-reactive deeplink effect: resolve `?tab`/`?focus` to a section
  // (via groups.ts) and route there. `?focus=cliAccess` additionally moves focus
  // into the Access section's wrapper, so a deeplink lands the caret inside it.
  //
  // This arm used to ALSO light an arrival tint plus a "CLI access request"
  // callout in that panel. Under ADR 0059 an approval opens the CLI Access
  // Request window and nothing navigates Settings, so nothing produces `?focus=`
  // during a request — the cue advertised a live request nobody could have
  // arrived here to see. Deleted rather than re-timed; the request window is the
  // surface that carries the state.
  $effect(() => {
    const requestedTab = $page.url.searchParams.get("tab");
    const section = resolveTabDeeplink(requestedTab);
    if (section) {
      focusSettingsSection(section, false);
      if (!untrack(() => c.rec.recordingSettingsLoaded)) deeplinkAwaitingLoad = section;
    }
    const focus = resolveFocusDeeplink($page.url.searchParams.get("focus"));
    if (focus) {
      const focusSection = sectionForFocus(focus);
      activeGroup = groupForSection(focusSection);
      activeSection = focusSection;
      suppressSpy();
      void tick().then(() => {
        c.agentAccessSection?.scrollIntoView({ block: "start", behavior: "smooth" });
        c.agentAccessSection?.focus({ preventScroll: true });
        if (c.agentAccessSection) setSpyTarget(c.agentAccessSection);
      });
    }
  });

  // A deeplink to a lower section scrolls before recording settings arrive; the
  // sections above it then grow and push it down. Re-scroll once they've loaded
  // — unless the user has already scrolled elsewhere (scroll-spy moved on).
  let deeplinkAwaitingLoad: SettingsSectionId | null = null;
  $effect(() => {
    if (!c.rec.recordingSettingsLoaded) return;
    untrack(() => {
      const section = deeplinkAwaitingLoad;
      deeplinkAwaitingLoad = null;
      if (section && activeSection === section) focusSettingsSection(section, false);
    });
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

    // Start lands an in-flight sources save first (see capture-controls).
    setFlushBeforeStart(() => c.autosaveEngine.flush("capture_sources"));

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
      setFlushBeforeStart(null);
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

<!-- ── Settings shell ──────────────────────────────────────────────────────
     Full-window focus mode (settings.html): a full-height rail on the left; on
     the right a 52px drag header (crumb + save status) over the one scrolling
     column. One group panel is mounted at a time. -->
<div class="settings-shell">
  <SettingsRail
    {activeGroup}
    {activeSection}
    onNavigate={(section) => focusSettingsSection(section)}
  />

  <div class="settings-content">
    <SettingsBar {activeGroup} {activeSection} scrolled={scrolled} />

    <div
      class="settings-scroll"
      class:is-scrolling={scrollRegionScrolling}
      bind:this={scrollRegion}
      onscroll={handleScrollRegionScroll}
    >
      <div class="settings-page">
        <header class="settings-page__head">
          <h1 class="mx-h1">{activeGroupInfo.label}</h1>
          <p class="mx-prose">{activeGroupInfo.description}</p>
        </header>

        <!-- Autosave failures: the header shows one line; the message and its
             actions live here. Recording gets Retry (re-runs the failed domain
             save) + Dismiss (puts the last saved value back); shortcuts and the
             microphone re-attempt on their own, so they only offer Dismiss. -->
        {#each saveErrors as err (err.title)}
          <div class="mx-notice settings-page__notice" data-tone="danger" role="alert">
            <span class="mx-notice__icon"><IconAlert width="15" height="15" aria-hidden="true" /></span>
            <b class="mx-notice__title">{err.title}</b>
            <p class="mx-notice__text">{err.message}</p>
            <div class="mx-notice__acts">
              {#if err.retry}
                <button class="mx-btn mx-btn--sm" type="button" onclick={err.retry}>
                  <IconRetry width="12" height="12" aria-hidden="true" />Retry
                </button>
              {/if}
              <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={err.dismiss}>Dismiss</button>
            </div>
          </div>
        {/each}

        <AppPrivacyExclusionPrompt
          controller={c.appPrivacyExclusion}
          onReview={() => focusSettingsSection("privacy")}
        />

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
</div>

<style>
  /* The shell root rule lives here (its element is in this template); the
     content + scroll rules are in lib/settings/settings-layout.css. */
  .settings-shell {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
  }

  .settings-page {
    max-width: 760px;
    padding-bottom: var(--s-8);
  }
  .settings-page__head {
    display: grid;
    gap: var(--s-2);
    margin-bottom: var(--s-6);
  }
  .settings-page__notice {
    margin-bottom: var(--s-5);
  }
</style>
