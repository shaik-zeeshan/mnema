<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { tick, type Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { isMainAppRoute, normalizeAppPathname } from "$lib/route-path";
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";
  import { closeCurrentWindow, getLastMainSurface, isDedicatedSurfaceWindow, isQuickRecallWindow, openDebugWindow, openSettings } from "$lib/surface-windows";
  import { createSettingsDeeplink } from "$lib/settings/deeplink.svelte";
  import {
    bootstrapCaptureControls,
    captureControls,
    sourceSelection,
    pauseCapture,
    resumeCapture,
    startCapture,
    stopCapture,
    subscribeRuntimeSources,
    toggleSourceSelected,
  } from "$lib/capture-controls.svelte";
  import { initTheme } from "$lib/theme.svelte";
  import { theme, persistAppearance } from "$lib/theme.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
  import RecordPill from "$lib/components/RecordPill.svelte";
  import type { AppearanceSetting } from "$lib/types";
  import {
    appNotifications,
    clearAppNotification,
    clearAppNotifications,
    dismissAppNotificationError,
    initAppNotifications,
    noteAppNotificationError,
    reloadAppNotifications,
    type AppNotification,
  } from "$lib/notifications.svelte";
  import { initLicenseStatus } from "$lib/licensing-store.svelte";
  import LicenseBanner from "$lib/LicenseBanner.svelte";
  import LicenseDeepLinkModal from "$lib/LicenseDeepLinkModal.svelte";
  import {
    GLOBAL_SHORTCUTS,
    getEffectiveGlobalShortcut,
    getGlobalShortcutAction,
    type GlobalShortcutId,
  } from "$lib/global-shortcuts";
  import { initKeyboardBindings } from "$lib/keyboard-bindings.svelte";
  import { askAiClock } from "$lib/askAiClock";
  import {
    detectKeyboardPlatform,
    formatShortcut,
    getFocusableElements,
    isShortcutSuppressedTarget,
    trapTabKey,
    type KeyboardPlatform,
    type ShortcutDefinition,
  } from "$lib/keyboard";
  import { keyboardHelp, type KeyboardHelpGroup } from "$lib/keyboard-help.svelte";
  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  // The listener `$effect` below re-runs on every in-window navigation (it reads
  // `$page.url.pathname` transitively), but the cold-start handoff drains
  // (insights peek + settings drain) must fire ONCE on mount, not on every
  // route change — re-issuing those drain/peek IPC calls would replay stale
  // handoffs. This non-reactive flag gates them to the first run.
  let coldDrainsDone = false;

  const normalizedPathname = $derived(normalizeAppPathname($page.url.pathname));
  const isMainRoute = $derived(isMainAppRoute($page.url.pathname));
  const isInsightsRoute = $derived(normalizeAppPathname($page.url.pathname).startsWith("/insights"));
  const isTriggersRoute = $derived(normalizedPathname.startsWith("/triggers"));
  const isSettings = $derived(normalizedPathname.startsWith("/settings"));
  // Settings renders inside the Main window as the `/settings` route. The Main
  // titlebar (record controls, source pills, surface toggle, gear) stays visible
  // on Settings too — it is the Main window's persistent top nav — and Settings
  // renders its own sidebar shell in the content area below it. Native traffic
  // lights stay (overlay titlebar), reserved for by the titlebar's left inset.
  const isSettingsRoute = $derived(normalizedPathname === "/settings");
  const isDebug = $derived(normalizedPathname.startsWith("/debug"));
  const isPanelSurface = isQuickRecallWindow();
  // The Main window hosts the story shell (`/insights` + `/triggers`, sharing
  // the rail) and the raw Timeline (`/`, behind the titlebar clock door). The
  // shared main titlebar (record controls, source pills, settings) renders on
  // all of them.
  const isMainSurfaceRoute = $derived(isMainRoute || isInsightsRoute || isTriggersRoute);
  const showMainTitlebar = $derived((isMainSurfaceRoute || isSettingsRoute) && !isPanelSurface);
  const showDedicatedTitlebar = isDedicatedSurfaceWindow();
  const transparentSurface = $derived(showDedicatedTitlebar || isPanelSurface);
  const isMainWindow = $derived(!showDedicatedTitlebar && !isPanelSurface);
  // Shown across the main surfaces (the home moved off `/`, so gating on the
  // Timeline alone would hide the help from the app's front page).
  const canShowShortcutsHelp = $derived(isMainWindow && isMainSurfaceRoute);
  let windowPlatform = $state<KeyboardPlatform>(detectKeyboardPlatform());
  let notificationsOpen = $state(false);
  let notificationsOpenedByKeyboard = false;
  let notificationsButtonEl = $state<HTMLButtonElement | null>(null);
  let notificationsPopoverEl = $state<HTMLDivElement | null>(null);
  let settingsButtonEl = $state<HTMLButtonElement | null>(null);
  let restartingPrivacyCapture = $state(false);
  let shortcutsHelpOpen = $state(false);
  let shortcutsHelpPanelEl = $state<HTMLDivElement | null>(null);
  let shortcutsHelpCloseEl = $state<HTMLButtonElement | null>(null);
  let shortcutsHelpReturnFocusEl: HTMLElement | null = null;
  let chromeAppearance = $state<AppearanceSetting>("system");
  let savingChromeAppearance = $state(false);

  $effect(() => {
    if (typeof document === "undefined") return;

    document.documentElement.classList.toggle("dedicated-surface-window", transparentSurface);

    return () => {
      document.documentElement.classList.remove("dedicated-surface-window");
    };
  });

  const devEnabled = $derived(developerOptions.value);
  const devLoaded = $derived(developerOptions.loaded);

  // Initialize the global theme runtime during layout creation so theme
  // resolution starts before the shell's first render instead of waiting for a
  // post-render `$effect`. `initTheme` is idempotent and remains safe in the
  // SPA-only setup.
  initTheme();
  initAppNotifications();
  initKeyboardBindings();
  initLicenseStatus();

  $effect(() => {
    chromeAppearance = theme.appearance;
  });

  async function setChromeAppearance(next: AppearanceSetting): Promise<void> {
    savingChromeAppearance = true;
    try {
      await persistAppearance(next);
    } finally {
      savingChromeAppearance = false;
    }
  }

  $effect(() => {
    loadDeveloperOptions();
  });

  // Stamp the frontend's local UTC offset so the distillation worker can label
  // Activity times in the user's local clock (the Rust `time` crate can't read
  // the local offset soundly under Tauri — the frontend is the sound source,
  // mirroring `askAiClock`). Once on start + on window focus (catches DST /
  // travel). Fire-and-forget: a failed stamp must never break startup.
  $effect(() => {
    const stamp = () => {
      void invoke("user_context_stamp_local_offset", {
        offsetMinutes: askAiClock().utcOffsetMinutes,
      }).catch(() => {});
    };
    stamp();
    window.addEventListener("focus", stamp);
    return () => window.removeEventListener("focus", stamp);
  });

  // Bootstrap shared capture state once for the whole app — the title bar
  // status indicator and record/stop action depend on it. The route pages
  // (e.g. dashboard, debug) also call `bootstrapCaptureControls`, but each
  // call is guarded by `captureControls.bootstrapped`, so this is idempotent.
  $effect(() => {
    if (captureControls.bootstrapped) return;
    void bootstrapCaptureControls();
  });

  // Settings deeplink transport, owned by `$lib/settings/deeplink.svelte`. The
  // Main window turns an `open_settings_tab` deeplink (live event + a cold-window
  // drain) into a `/settings` navigation; the module holds the listener + drain
  // and reads the live shell state through these getters so reactivity and the
  // exact navigation semantics are preserved. The cold drain stays sequenced with
  // the insights peek below via the single `coldDrainsDone` one-shot gate.
  const settingsDeeplink = createSettingsDeeplink({
    currentPathname: () => $page.url.pathname,
    goto,
    isMainWindow: () => isMainWindow,
    isSettings: () => isSettings,
  });

  $effect(() => {
    let destroyed = false;
    let unlistenBrokerOpenCaptureResult: (() => void) | undefined;
    let unlistenInsightsOpenConversation: (() => void) | undefined;

    // Settings deeplink transport (the `open_settings_tab` listener). Cleanup is
    // the module's returned unlisten, torn down alongside the others below.
    const unlistenOpenSettingsTab = settingsDeeplink.listen();

    listen("broker_open_capture_result", () => {
      if (isMainWindow && !isMainRoute) {
        void goto("/");
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenBrokerOpenCaptureResult = fn;
    });

    // Quick Recall → Chat handoff (issue #111, ADR 0031): navigate the main
    // window to the Insights surface so its Chat tab can select the handed-off
    // conversation. The Insights page itself owns switching to the Chat tab and
    // selecting the conversation (live event + a cold-window drain on mount);
    // here we only ensure the route is on `/insights`.
    listen("insights_open_conversation", () => {
      if (isMainWindow && !isInsightsRoute) {
        void goto("/insights");
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenInsightsOpenConversation = fn;
    });

    // One-shot cold-start handoff drains. This `$effect` re-runs on every
    // in-window navigation (it reads `$page.url.pathname` transitively), but the
    // cold-window peek/drain below must run only once on mount — re-issuing them
    // on later navigations would replay stale handoffs. Gate them behind a
    // non-reactive flag set after the first run.
    if (!coldDrainsDone) {
      coldDrainsDone = true;

    // Story-first home (Warm Paper Slice 2): the static entry still boots on
    // `/` (the raw Timeline), but the app's front page is the Today shell —
    // redirect the cold main window there once. Later navigations to `/` (the
    // titlebar clock door) are untouched; dedicated/panel windows never hit
    // this (isMainWindow false), and onboarding boots on its own route.
    if (isMainWindow && isMainRoute) {
      void goto("/insights", { replaceState: true });
    }

    // Cold-window inverse: a freshly-opened main window boots on Timeline (`/`),
    // and the live `insights_open_conversation` event may have already fired
    // before the listener above attached — so without this the handoff would
    // strand on Timeline and the Insights surface (which owns the drain) would
    // never mount. Peek the queue on mount and, if a handoff is pending, route
    // to `/insights` so its on-mount drain runs. Non-draining: the Insights page
    // still owns consuming the queue.
    if (isMainWindow && !isInsightsRoute) {
      // Snapshot the route at peek time. The peek is async, so the user may
      // navigate during the drain window; if the route changed underneath us we
      // bail rather than yanking them back to /insights (self-healing, but the
      // bounce is jarring). Comparing the captured pathname keeps the re-route
      // intent tied to the route this peek was started for.
      const peekPathname = normalizeAppPathname($page.url.pathname);
      void invoke<boolean>("has_pending_insights_open_conversations")
        .then((pending) => {
          const routeUnchanged =
            normalizeAppPathname($page.url.pathname) === peekPathname;
          if (!destroyed && pending && routeUnchanged && !isInsightsRoute) {
            void goto("/insights");
          }
        })
        .catch(() => {
          // Best-effort: leave the route as-is if the peek is unavailable.
        });
    }

    // Cold-window Settings deeplink drain — owned by the settings-deeplink
    // module, kept sequenced after the insights peek under this same one-shot
    // gate. `() => !destroyed` mirrors the inline `destroyed` bail the drain made
    // inside its resolved `.then` (this run's cleanup flips `destroyed`).
    settingsDeeplink.drainColdWindow(() => !destroyed);
    }

    return () => {
      destroyed = true;
      unlistenBrokerOpenCaptureResult?.();
      unlistenInsightsOpenConversation?.();
      unlistenOpenSettingsTab();
    };
  });

  // Gate direct visits to `/debug` behind developer-options. We wait until
  // the flag has actually loaded to avoid a flash-redirect when the persisted
  // value is `true` but the IPC hasn't returned yet.
  $effect(() => {
    if (!devLoaded) return;
    if (isDebug && !devEnabled) {
      goto("/insights", { replaceState: true });
    }
  });

  // Hide the gated Debug surface until we know whether developer options
  // are enabled, and while we're redirecting a disabled user away from it.
  // Non-gated routes always render immediately.
  const showChildren = $derived(!isDebug || (devLoaded && devEnabled));

  // Routes that want a centered, padded reading column. Settings is excluded:
  // it renders full-bleed inside the Main window with its own sidebar shell.
  // Onboarding is excluded too (Slice 3): the accordion shell fills the window
  // and owns its own scroll region — the narrow column's max-width/padding would
  // shrink it and break the shell's `height:100%` fill.
  const isNarrow = $derived(isDebug);
  const notificationCount = $derived(appNotifications.count);
  const hasNotifications = $derived(notificationCount > 0);
  const notificationLoadError = $derived(appNotifications.loadError);
  const notificationActionError = $derived(appNotifications.actionError);
  // The bell must also stay reachable when the initial load failed (count 0 but
  // a recoverable error) so the failure isn't indistinguishable from "no
  // notifications" and a retry remains available.
  const hasNotificationIndicator = $derived(
    hasNotifications || notificationLoadError !== null,
  );
  const hasErrorNotification = $derived(
    appNotifications.items.some((n) => n.severity === "error"),
  );
  const hasWarningNotification = $derived(
    appNotifications.items.some((n) => n.severity === "warning"),
  );
  // The count + worst-severity badge is `aria-hidden` (decorative), so assistive
  // tech otherwise hears only "Open notifications" with no sense of how many or
  // how urgent. Fold the live summary into the button name and mirror it into a
  // dedicated live region (assertive when an error is present) so a new alert is
  // announced even while the popover is closed.
  const notificationSummary = $derived.by<string>(() => {
    if (notificationLoadError) {
      return "Notifications failed to load — open to retry.";
    }
    if (notificationCount === 0) return "";
    const noun = notificationCount === 1 ? "notification" : "notifications";
    const severity = hasErrorNotification
      ? ", including an error"
      : hasWarningNotification
        ? ", including a warning"
        : "";
    return `${notificationCount} ${noun}${severity}`;
  });
  const notificationsAriaLabel = $derived(
    notificationSummary ? `Open notifications — ${notificationSummary}` : "Open notifications",
  );
  const notificationLiveTone = $derived(
    hasErrorNotification || notificationLoadError !== null ? "assertive" : "polite",
  );

  $effect(() => {
    if (!hasNotificationIndicator) notificationsOpen = false;
  });

  async function runNotificationAction(notification: AppNotification): Promise<void> {
    if (notification.action?.type !== "open_settings_tab") return;
    try {
      await openSettings(notification.action.tab);
    } catch {
      // Navigation failed — keep the notification and the popover so the user
      // can see the action did not complete and retry.
      noteAppNotificationError("Couldn't open settings. Try again.");
      return;
    }
    // Only dismiss + close once the navigation succeeded; if the clear itself
    // fails it surfaces its own error and we leave the popover open.
    const cleared = await clearAppNotification(notification.id);
    if (cleared) notificationsOpen = false;
  }

  function notificationActionLabel(notification: AppNotification): string {
    if (notification.action?.type !== "open_settings_tab") return "Open";
    if (notification.action.tab === "about") return "Open update settings";
    if (notification.action.tab === "processing") return "Open OCR settings";
    if (notification.action.tab === "transcription") return "Open transcription settings";
    if (notification.action.tab === "speakers") return "Open speaker settings";
    if (notification.action.tab === "shortcuts") return "Open shortcut settings";
    return "Open settings";
  }

  // ── Record state mirrored from the shared capture-controls seam ────
  // The titlebar's record pill (RecordPill.svelte) owns the on/off-the-record
  // display and the timed off-the-record menu; the layout keeps only what the
  // keyboard shortcuts and per-source indicators need.
  const isCapturing = $derived(captureControls.running);
  const captureLoadingStart = $derived(captureControls.loadingStart);
  const captureLoadingStop = $derived(captureControls.loadingStop);
  const captureLoadingPause = $derived(captureControls.loadingPause);
  const captureLoadingSettings = $derived(captureControls.loadingSettings);

  // ── Per-source runtime indicators ──────────────────────────────────────
  // While a capture session is running, fetch `get_idle_debug` periodically
  // through the shared seam so the title bar can show small per-source
  // icons (screen / microphone / system audio) with running vs paused
  // state. The subscription auto-clears when the session stops.
  $effect(() => {
    if (!showMainTitlebar || !isCapturing) return;
    const release = subscribeRuntimeSources();
    return release;
  });

  type SourceLane = {
    key: "screen" | "microphone" | "systemAudio";
    label: string;
  };
  type PrivacyVisualCaptureStatus = {
    modifier: "retrying" | "restart-required";
    label: string;
    detail: string;
  };
  const sourceLanes: SourceLane[] = [
    { key: "screen", label: "Screen" },
    { key: "microphone", label: "Microphone" },
    { key: "systemAudio", label: "System audio" },
  ];

  // While recording, each pill reflects the *live* runtime status of the
  // source. While idle/stopped, each pill reflects whether that source is
  // *selected* for the next session — clicking the pill toggles the
  // corresponding `RecordingSettings` flag through the same Tauri command
  // the settings page uses.
  type LiveState = "running" | "paused" | "starting" | "off";
  type SelectState = "selected" | "unselected";

  function liveStateFor(key: SourceLane["key"]): LiveState {
    const rs = captureControls.runtimeSources;
    if (!rs) return "off";
    const src = rs[key];
    if (!src.requested) return "off";
    if (src.paused) return "paused";
    if (src.sessionActive && src.writerActive) return "running";
    return "starting";
  }
  function selectStateFor(key: SourceLane["key"]): SelectState {
    return sourceSelection.isSelected(key) ? "selected" : "unselected";
  }

  function liveTitleFor(lane: SourceLane, state: LiveState): string {
    const verb =
      state === "running"
        ? "on the record"
        : state === "paused"
          ? "paused"
          : state === "starting"
            ? "starting…"
            : "off";
    return `${lane.label}: ${verb}`;
  }

  function isPrivacySuspensionReason(reason: string | null): boolean {
    return (
      reason === "privacy_filter_apply_failed" ||
      reason === "privacy_recovery_restart_required"
    );
  }

  function isPrivacySuspendedSource(src: {
    requested: boolean;
    reason: string | null;
  }): boolean {
    return src.requested && isPrivacySuspensionReason(src.reason);
  }

  function formatSuspendedVisualSources(keys: SourceLane["key"][]): string {
    const labels = keys.map((key) => {
      if (key === "systemAudio") return "system audio";
      return key;
    });
    if (labels.length === 0) return "visual capture";
    if (labels.length === 1) return labels[0];
    return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
  }

  const privacyVisualCaptureStatus = $derived.by<PrivacyVisualCaptureStatus | null>(() => {
    if (!isCapturing) return null;
    const rs = captureControls.runtimeSources;
    if (!rs) return null;

    const suspendedSources: SourceLane["key"][] = [];
    if (isPrivacySuspendedSource(rs.screen)) {
      suspendedSources.push("screen");
    }
    if (isPrivacySuspendedSource(rs.systemAudio)) {
      suspendedSources.push("systemAudio");
    }
    if (suspendedSources.length === 0) return null;

    const reason = suspendedSources
      .map((key) => rs[key].reason)
      .find((value) => value === "privacy_recovery_restart_required")
      ?? "privacy_filter_apply_failed";
    const sources = formatSuspendedVisualSources(suspendedSources);
    const suffix = rs.microphone.requested
      ? " Microphone can keep recording."
      : "";

    if (reason === "privacy_recovery_restart_required") {
      return {
        modifier: "restart-required",
        label: `Restart recording to resume ${sources}`,
        detail: `Privacy filter recovery failed. Stop and start recording to resume ${sources}.${suffix}`,
      };
    }

    return {
      modifier: "retrying",
      label: `Privacy filter failed; retrying ${sources}`,
      detail: `Mnema stopped ${sources} because the privacy filter could not be applied. It is retrying recovery.${suffix}`,
    };
  });

  function selectTitleFor(lane: SourceLane, state: SelectState): string {
    return state === "selected"
      ? `${lane.label}: on the record — click to leave out next session`
      : `${lane.label}: off the record — click to include next session`;
  }

  const canUseGlobalShortcuts = $derived(isMainWindow && isMainRoute);
  const canToggleSourcesByShortcut = $derived(
    canUseGlobalShortcuts && !isCapturing && !captureLoadingSettings,
  );
  const canToggleRecordingByShortcut = $derived(
    isCapturing ? !captureLoadingStop : !captureLoadingStart && !captureLoadingSettings,
  );

  function sourceShortcutIdFor(key: SourceLane["key"]): GlobalShortcutId {
    if (key === "screen") return "toggleSourceScreen";
    if (key === "microphone") return "toggleSourceMicrophone";
    return "toggleSourceSystemAudio";
  }

  function shortcutDisplay(id: GlobalShortcutId): string {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, windowPlatform).join("") : "—";
  }

  function shortcutWithLabel(
    definition: ShortcutDefinition,
    label: string,
  ): ShortcutDefinition {
    return { ...definition, label };
  }

  const globalShortcutHelpGroup = $derived.by<KeyboardHelpGroup>(() => {
    const rows: KeyboardHelpGroup["rows"] = [];

    if (canToggleRecordingByShortcut) {
      rows.push(
        shortcutWithLabel(
          getEffectiveGlobalShortcut("toggleRecording"),
          isCapturing ? "Stop the record" : "Go on the record",
        ),
      );
    }

    if (isCapturing) {
      rows.push(
        shortcutWithLabel(
          getEffectiveGlobalShortcut("pauseResumeRecording"),
          captureControls.isUserPaused ? "Back on the record" : "Go off the record",
        ),
      );
    }

    rows.push(getEffectiveGlobalShortcut("toggleMainWindow"));
    rows.push(getEffectiveGlobalShortcut("toggleQuickRecall"));
    rows.push(getEffectiveGlobalShortcut("openSettings"));

    if (devEnabled) {
      rows.push(getEffectiveGlobalShortcut("openDebug"));
    }

    if (canToggleSourcesByShortcut) {
      rows.push(
        getEffectiveGlobalShortcut("toggleSourceScreen"),
        getEffectiveGlobalShortcut("toggleSourceMicrophone"),
        getEffectiveGlobalShortcut("toggleSourceSystemAudio"),
      );
    }

    rows.push(getEffectiveGlobalShortcut("toggleShortcutsHelp"), GLOBAL_SHORTCUTS.closeShortcutsHelp);

    return {
      id: "global",
      title: "Global",
      rows,
    };
  });

  const shortcutHelpGroups = $derived.by<KeyboardHelpGroup[]>(() => {
    const groups = [globalShortcutHelpGroup, ...keyboardHelp.contextualGroups]
      .map((group) => ({
        ...group,
        rows: group.rows.filter((row) => row.enabled !== false && row.bindings.length > 0),
      }))
      .filter((group) => group.rows.length > 0);
    return groups;
  });

  async function toggleRecordingShortcut(): Promise<void> {
    if (!canToggleRecordingByShortcut) return;
    if (isCapturing) {
      await stopCapture();
      return;
    }
    await startCapture();
  }

  async function toggleSourceShortcut(key: SourceLane["key"]): Promise<void> {
    if (!canToggleSourcesByShortcut || sourceSelection.isSaving(key)) return;
    await toggleSourceSelected(key);
  }

  async function pauseResumeRecordingShortcut(): Promise<void> {
    if (!isCapturing || captureLoadingPause || captureLoadingStop || captureLoadingStart) return;
    if (captureControls.isUserPaused) {
      await resumeCapture();
    } else {
      await pauseCapture();
    }
  }

  async function restartCaptureForPrivacyRecovery(): Promise<void> {
    if (captureLoadingStart || captureLoadingStop || restartingPrivacyCapture || !isCapturing) return;
    restartingPrivacyCapture = true;
    try {
      // stop/start funnel any failure into the shared capture-error dialog
      // (capture-controls.svelte.ts) — so a failed restart is now visible.
      await stopCapture();
      if (!captureControls.isRunning) {
        await startCapture();
      }
    } finally {
      restartingPrivacyCapture = false;
    }
  }

  // ── Main surface navigation ────────────────────────────────────────────
  // The Main window hosts the story shell (`/insights` + `/triggers`, sharing
  // the rail) and the raw Timeline (`/`) behind the titlebar clock door — the
  // segmented surface switcher is gone (Warm Paper Slice 2).
  function goToSurface(surface: "timeline" | "insights" | "triggers"): void {
    const target =
      surface === "insights" ? "/insights" : surface === "triggers" ? "/triggers" : "/";
    if (normalizeAppPathname($page.url.pathname) === target) return;
    void goto(target);
  }

  // The clock door is a toggle: from anywhere it opens the raw Timeline; from
  // the Timeline it returns to the story shell.
  function onTimelineButtonClick(): void {
    goToSurface(isMainRoute ? "insights" : "timeline");
  }

  // The gear is a real toggle: opening Settings from a surface, then clicking
  // the gear again returns to the surface it was opened from (Timeline,
  // Insights, or Triggers) instead of being a no-op with no obvious exit.
  function onSettingsButtonClick(): void {
    if (isSettings) {
      const last = normalizeAppPathname(getLastMainSurface());
      goToSurface(
        last.startsWith("/insights")
          ? "insights"
          : last.startsWith("/triggers")
            ? "triggers"
            : "timeline",
      );
      return;
    }
    void openSettings();
  }

  // Quick Recall has no in-app door otherwise — it is only summonable via the
  // global ⌥Space shortcut, which a new user can't discover. The titlebar
  // affordance asks Rust to toggle the Quick Recall panel (the same path the
  // global shortcut takes); the shortcut stays the canonical fallback if the
  // command is unavailable.
  async function summonQuickRecall(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // Best-effort: leave the global ⌥Space shortcut as the summon path.
    }
  }

  function openNotifications(openedByKeyboard = false): void {
    if (!hasNotificationIndicator) return;
    notificationsOpenedByKeyboard = openedByKeyboard;
    notificationsOpen = true;
  }

  // Relative age for each notification row so a stale alert is distinguishable
  // from a fresh one. Recomputed against `notificationsNow`, which ticks while
  // the popover is open.
  let notificationsNow = $state(Date.now());
  $effect(() => {
    if (!notificationsOpen) return;
    notificationsNow = Date.now();
    const handle = setInterval(() => {
      notificationsNow = Date.now();
    }, 30_000);
    return () => clearInterval(handle);
  });

  function formatNotificationAge(createdAtUnixMs: number): string {
    const deltaMs = Math.max(0, notificationsNow - createdAtUnixMs);
    const seconds = Math.floor(deltaMs / 1000);
    if (seconds < 45) return "just now";
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.round(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.round(hours / 24);
    return `${days}d ago`;
  }

  function formatNotificationTimestamp(createdAtUnixMs: number): string {
    try {
      return new Date(createdAtUnixMs).toLocaleString();
    } catch {
      return "";
    }
  }

  function closeNotifications(): void {
    notificationsOpen = false;
  }

  function toggleNotifications(openedByKeyboard = false): void {
    if (notificationsOpen) {
      closeNotifications();
      return;
    }
    openNotifications(openedByKeyboard);
  }

  function onNotificationsButtonKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" || event.key === " ") {
      notificationsOpenedByKeyboard = true;
    }
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!notificationsOpen) return;
    const target = event.target as Node | null;
    if (!target) return;
    if (notificationsPopoverEl?.contains(target)) return;
    if (notificationsButtonEl?.contains(target)) return;
    closeNotifications();
  }

  function closeShortcutsHelp(): void {
    shortcutsHelpOpen = false;
  }

  function onShortcutsHelpPointerDown(event: PointerEvent): void {
    if (event.target === event.currentTarget) closeShortcutsHelp();
  }

  function toggleShortcutsHelp(): void {
    if (!canShowShortcutsHelp) return;
    const willOpen = !shortcutsHelpOpen;
    if (willOpen) {
      closeNotifications();
    }
    shortcutsHelpOpen = willOpen;
  }

  function isDedicatedWindowCloseSuppressedTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(target.closest([
      "input",
      "textarea",
      "select",
      '[contenteditable="true"]',
      '[role="textbox"]',
      '[role="searchbox"]',
      '[role="combobox"]',
      "[data-shortcuts-ignore]",
    ].join(", ")));
  }

  function dismissQuickRecallOnEscape(event: KeyboardEvent): boolean {
    if (!isPanelSurface) return false;
    if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return false;
    if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
    event.preventDefault();
    event.stopPropagation();
    void closeCurrentWindow();
    return true;
  }

  function closeDedicatedWindowOnEscape(event: KeyboardEvent): boolean {
    if (!showDedicatedTitlebar || (!isSettings && !isDebug)) return false;
    if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return false;
    if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
    if (isDedicatedWindowCloseSuppressedTarget(event.target)) return false;

    event.preventDefault();
    event.stopPropagation();
    void closeCurrentWindow();
    return true;
  }

  function onShortcutsHelpKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeShortcutsHelp();
      return;
    }
    trapTabKey(event, shortcutsHelpPanelEl);
  }

  function onNotificationsPopoverKeydown(event: KeyboardEvent): void {
    trapTabKey(event, notificationsPopoverEl);
  }

  function handleGlobalShortcutKeydown(event: KeyboardEvent): void {
    if (dismissQuickRecallOnEscape(event)) return;
    if (closeDedicatedWindowOnEscape(event)) return;

    const action = getGlobalShortcutAction(event, {
      devEnabled,
      isIdle: canToggleSourcesByShortcut,
      isMainRoute,
      isMainWindow,
      isShortcutSuppressedTarget: isShortcutSuppressedTarget(event.target),
      shortcutsHelpOpen,
    }, windowPlatform);
    if (!action) {
      if (
        event.key === "Escape" &&
        !shortcutsHelpOpen &&
        notificationsOpen
      ) {
        event.preventDefault();
        event.stopPropagation();
        closeNotifications();
      }
      return;
    }

    event.preventDefault();

    if (action.type === "closeShortcutsHelp") {
      event.stopPropagation();
      closeShortcutsHelp();
      return;
    }

    if (action.type === "toggleRecording") {
      void toggleRecordingShortcut();
      return;
    }

    if (action.type === "pauseResumeRecording") {
      void pauseResumeRecordingShortcut();
      return;
    }

    if (action.type === "toggleMainWindow") {
      void invoke("toggle_main_window_visibility_command");
      return;
    }

    if (action.type === "openSettings") {
      void openSettings();
      return;
    }

    if (action.type === "openDebug") {
      void openDebugWindow();
      return;
    }

    if (action.type === "toggleSource") {
      void toggleSourceShortcut(action.source);
      return;
    }

    toggleShortcutsHelp();
  }

  $effect(() => {
    if (shortcutsHelpOpen && !canShowShortcutsHelp) {
      shortcutsHelpOpen = false;
    }
  });

  $effect(() => {
    if (!shortcutsHelpOpen || !canShowShortcutsHelp) return;
    shortcutsHelpReturnFocusEl = document.activeElement as HTMLElement | null;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled) return;
      const first = getFocusableElements(shortcutsHelpPanelEl)[0] ?? shortcutsHelpCloseEl;
      first?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (
        !active ||
        active === document.body ||
        active.closest(".shortcut-help")
      ) {
        shortcutsHelpReturnFocusEl?.focus({ preventScroll: true });
      }
    };
  });

  $effect(() => {
    if (!notificationsOpen) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !notificationsOpen || !notificationsOpenedByKeyboard) return;
      getFocusableElements(notificationsPopoverEl)[0]?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (
        (notificationsOpenedByKeyboard && (!active || active === document.body)) ||
        (active && notificationsPopoverEl?.contains(active))
      ) {
        // Clearing the last notification removes the bell, so fall back to a
        // stable neighbour (the settings button) instead of dropping focus to
        // <body>.
        const target = notificationsButtonEl?.isConnected
          ? notificationsButtonEl
          : settingsButtonEl;
        target?.focus({ preventScroll: true });
      }
      notificationsOpenedByKeyboard = false;
    };
  });
</script>

<svelte:window onkeydown={handleGlobalShortcutKeydown} onpointerdown={onWindowPointerDown} />
<svelte:body class:dedicated-surface-window={transparentSurface} />

<div
  class="app-shell"
  class:app-shell--bounded={isMainSurfaceRoute || isSettingsRoute}
  class:app-shell--dedicated={showDedicatedTitlebar}
  class:app-shell--macos={showDedicatedTitlebar && windowPlatform === "macos"}
  class:app-shell--windows={showDedicatedTitlebar && windowPlatform === "windows"}
>
  <!--
    Custom desktop title bar. The Tauri window uses macOS's overlay title-bar
    style, so the OS still draws native traffic lights in the top-left; this
    bar reserves space for them via `.titlebar` left padding. The drag region
    is restricted to the inert filler area (`data-tauri-drag-region`); every
    interactive control sits outside that region so clicks/taps reach the
    button.
  -->
  {#if showMainTitlebar}
  <header class="titlebar">
    <div class="titlebar__group titlebar__group--left">
      {#if showMainTitlebar}
        <!-- The record pill (Warm Paper Slice 7): one door for the on/off-the-
             record state + the timed off-the-record menu. Start/stop stay
             reachable via keyboard shortcuts and the tray. -->
        <RecordPill />
        {#snippet sourceIcon(key: SourceLane["key"])}
          {#if key === "screen"}
            <svg
              class="titlebar__source-icon"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <rect x="2" y="4" width="20" height="13" rx="2" />
              <path d="M8 21h8" />
              <path d="M12 17v4" />
            </svg>
          {:else if key === "microphone"}
            <svg
              class="titlebar__source-icon"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <rect x="9" y="2.5" width="6" height="12" rx="3" />
              <path d="M5.5 11a6.5 6.5 0 0 0 13 0" />
              <path d="M12 17.5v3.5" />
              <path d="M9 21h6" />
            </svg>
          {:else}
            <svg
              class="titlebar__source-icon"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M11 5 6.5 9H3v6h3.5L11 19z" />
              <path d="M15.5 8.5a5 5 0 0 1 0 7" />
              <path d="M18.5 5.5a9 9 0 0 1 0 13" />
            </svg>
          {/if}
        {/snippet}
        {#each sourceLanes as lane (lane.key)}
          {#if isCapturing}
            {@const state = liveStateFor(lane.key)}
            <span
              class="titlebar__source titlebar__source--{lane.key} titlebar__source--{state}"
              use:tip={liveTitleFor(lane, state)}
              aria-label={liveTitleFor(lane, state)}
              role="status"
            >
              {@render sourceIcon(lane.key)}
              <span class="titlebar__source-state" aria-hidden="true">
                {#if state === "running"}
                  <span class="titlebar__source-dot"></span>
                {:else if state === "paused"}
                  <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true">
                    <rect x="1" y="1" width="2" height="6" rx="0.5" fill="currentColor" />
                    <rect x="5" y="1" width="2" height="6" rx="0.5" fill="currentColor" />
                  </svg>
                {:else if state === "starting"}
                  <span class="titlebar__source-ring"></span>
                {:else}
                  <span class="titlebar__source-slash"></span>
                {/if}
              </span>
            </span>
          {:else}
            {@const state = selectStateFor(lane.key)}
            <button
              type="button"
              class="titlebar__source titlebar__source--toggle titlebar__source--{lane.key} titlebar__source--{state}"
              use:tip={`${selectTitleFor(lane, state)} (${shortcutDisplay(sourceShortcutIdFor(lane.key))})`}
              aria-label={selectTitleFor(lane, state)}
              aria-pressed={state === "selected"}
              disabled={sourceSelection.isSaving(lane.key) || captureControls.loadingSettings}
              onclick={() => toggleSourceSelected(lane.key)}
            >
              {@render sourceIcon(lane.key)}
              <span class="titlebar__source-state" aria-hidden="true">
                {#if state === "selected"}
                  <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true">
                    <path
                      d="M1.5 4.2 3.2 5.9 6.5 2.5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.6"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                {:else}
                  <span class="titlebar__source-slash"></span>
                {/if}
              </span>
            </button>
          {/if}
        {/each}
        {#if privacyVisualCaptureStatus}
          <span
            class="titlebar__privacy-warning titlebar__privacy-warning--{privacyVisualCaptureStatus.modifier}"
            use:tip={privacyVisualCaptureStatus.detail}
            aria-label={privacyVisualCaptureStatus.detail}
            aria-live="polite"
            role="status"
          >
            <svg
              class="titlebar__privacy-warning-icon"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M12 9v4" />
              <path d="M12 17h.01" />
              <path d="M10.3 3.9 2.4 17.5A2 2 0 0 0 4.1 20h15.8a2 2 0 0 0 1.7-2.5L13.7 3.9a2 2 0 0 0-3.4 0Z" />
            </svg>
            <span class="titlebar__privacy-warning-label">{privacyVisualCaptureStatus.label}</span>
            {#if privacyVisualCaptureStatus.modifier === "restart-required"}
              <button
                type="button"
                class="titlebar__privacy-warning-action"
                use:tip={privacyVisualCaptureStatus.detail}
                aria-label={privacyVisualCaptureStatus.detail}
                aria-busy={restartingPrivacyCapture}
                disabled={captureLoadingStart || captureLoadingStop || restartingPrivacyCapture}
                onclick={restartCaptureForPrivacyRecovery}
              >
                {restartingPrivacyCapture ? "Restarting…" : "Restart"}
              </button>
            {/if}
          </span>
        {/if}
      {/if}
    </div>

    <!-- Inert centre area carries the drag region + the Quick Recall (Search)
         door. The old Timeline⇄Insights⇄Triggers segmented switcher is gone
         (Warm Paper Slice 2): the rail owns surface nav, and the raw Timeline
         is a titlebar icon door in the right cluster. -->
    <div class="titlebar__drag" data-tauri-drag-region>
      <!-- Quick Recall door — otherwise summonable only via the global ⌥Space
           shortcut, which a new user can't discover. -->
      <button
        type="button"
        class="titlebar__search"
        use:tip={`Search · Recall (${shortcutDisplay("toggleQuickRecall")})`}
        aria-label={`Search and recall (${shortcutDisplay("toggleQuickRecall")})`}
        onclick={() => void summonQuickRecall()}
      >
        <svg
          class="titlebar__search-icon"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <span class="titlebar__search-label">Search</span>
        <kbd class="titlebar__search-kbd" aria-hidden="true">{shortcutDisplay("toggleQuickRecall")}</kbd>
      </button>
    </div>

    <div class="titlebar__group titlebar__group--right">
      {#if showMainTitlebar}
        <!-- Raw Timeline door (Warm Paper Slice 2): the frame timeline is a
             full surface behind this clock icon — a sibling of theme +
             settings — instead of a segmented switcher entry. Clicking it
             from the timeline returns to the story shell. -->
        <button
          type="button"
          class="titlebar__settings"
          class:active={isMainRoute}
          aria-label={isMainRoute ? "Back to Today" : "Raw timeline"}
          aria-current={isMainRoute ? "page" : undefined}
          use:tip={isMainRoute ? "Back to Today" : "Raw timeline"}
          onclick={onTimelineButtonClick}
        >
          <svg
            class="titlebar__settings-icon"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.75"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="9" />
            <path d="M12 7v5l3 2" />
          </svg>
        </button>
        <!-- Persistent live regions: announce a new/cleared alert even while the
             bell popover is closed. Two always-mounted regions (one polite, one
             assertive) so the summary routes into the matching politeness — some
             screen readers don't re-register an attribute-only aria-live change
             on a mounted node. The count badge itself stays aria-hidden. -->
        <span class="sr-only" aria-live="polite" aria-atomic="true">
          {notificationLiveTone === "polite" ? notificationSummary : ""}
        </span>
        <span class="sr-only" aria-live="assertive" aria-atomic="true">
          {notificationLiveTone === "assertive" ? notificationSummary : ""}
        </span>
        <!-- Persistent bell slot: the button stays mounted with a quiet rest
             state (no count dot) so the neighbouring gear/help/theme icons
             don't shift when alerts arrive or clear. The count dot + popover
             stay gated on a live indicator. -->
        <div class="titlebar__notifications">
          <button
            bind:this={notificationsButtonEl}
            type="button"
            class="titlebar__settings titlebar__notifications-button"
            class:active={notificationsOpen}
            class:titlebar__notifications-button--quiet={!hasNotificationIndicator}
            aria-label={notificationsAriaLabel}
            aria-expanded={notificationsOpen}
            aria-controls="notification-popover"
            use:tip={hasNotificationIndicator ? "Notifications" : "No notifications"}
            onkeydown={onNotificationsButtonKeydown}
            onpointerdown={() => { notificationsOpenedByKeyboard = false; }}
            onclick={() => toggleNotifications(notificationsOpenedByKeyboard)}
          >
            <svg
              class="titlebar__settings-icon"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.75"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
              <path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9" />
            </svg>
            {#if hasNotificationIndicator}
              <span
                class="titlebar__notification-dot"
                class:titlebar__notification-dot--warning={hasWarningNotification && !hasErrorNotification && !notificationLoadError}
                class:titlebar__notification-dot--error={hasErrorNotification || notificationLoadError !== null}
                aria-hidden="true"
              >{notificationCount > 0 ? notificationCount : "!"}</span>
            {/if}
          </button>
          {#if notificationsOpen}
            <div
              id="notification-popover"
              class="notification-popover"
              role="dialog"
              aria-label="Notifications"
              tabindex="-1"
              bind:this={notificationsPopoverEl}
              onkeydown={onNotificationsPopoverKeydown}
            >
              <div class="notification-popover__head">
                <span>Notifications</span>
                {#if hasNotifications}
                  <button type="button" class="notification-popover__clear" onclick={() => void clearAppNotifications()}>
                    Clear all
                  </button>
                {/if}
              </div>
              {#if notificationActionError}
                <div class="notification-popover__error" role="alert">
                  <span class="notification-popover__error-text">{notificationActionError}</span>
                  <button
                    type="button"
                    class="notification-popover__error-dismiss"
                    onclick={dismissAppNotificationError}
                  >
                    Dismiss
                  </button>
                </div>
              {/if}
              <div class="notification-popover__list">
                {#if notificationLoadError}
                  <div class="notification-item notification-item--error" role="alert">
                    <div class="notification-item__body">
                      <span class="notification-item__title">Couldn't load notifications</span>
                      <span class="notification-item__message">{notificationLoadError}</span>
                      <button
                        type="button"
                        class="notification-item__action"
                        onclick={() => void reloadAppNotifications()}
                      >
                        Try again
                      </button>
                    </div>
                  </div>
                {/if}
                {#each appNotifications.items as notification (notification.id)}
                  <div class="notification-item notification-item--{notification.severity}">
                    <div class="notification-item__body">
                      <span class="notification-item__title">{notification.title}</span>
                      <span class="notification-item__message">{notification.message}</span>
                      <time
                        class="notification-item__time"
                        datetime={new Date(notification.createdAtUnixMs).toISOString()}
                        use:tip={formatNotificationTimestamp(notification.createdAtUnixMs)}
                      >{formatNotificationAge(notification.createdAtUnixMs)}</time>
                      {#if notification.action?.type === "open_settings_tab"}
                        <button
                          type="button"
                          class="notification-item__action"
                          onclick={() => void runNotificationAction(notification)}
                        >
                          {notificationActionLabel(notification)}
                        </button>
                      {/if}
                    </div>
                    <button
                      type="button"
                      class="notification-item__clear"
                      aria-label="Clear notification"
                      onclick={() => void clearAppNotification(notification.id)}
                    >
                      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
                        <path d="M2.5 2.5 9.5 9.5" />
                        <path d="M9.5 2.5 2.5 9.5" />
                      </svg>
                    </button>
                  </div>
                {/each}
              </div>
            </div>
          {/if}
        </div>
        {#if canShowShortcutsHelp}
          <button
            type="button"
            class="titlebar__settings titlebar__settings--help"
            class:active={shortcutsHelpOpen}
            aria-label="Keyboard shortcuts"
            aria-haspopup="dialog"
            aria-expanded={shortcutsHelpOpen}
            use:tip={`Keyboard shortcuts (${shortcutDisplay("toggleShortcutsHelp")})`}
            onclick={() => toggleShortcutsHelp()}
          >
            <svg
              class="titlebar__settings-icon"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.75"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <circle cx="12" cy="12" r="9" />
              <path d="M9.4 9.2a2.6 2.6 0 0 1 5 .9c0 1.7-2.4 2-2.4 3.6" />
              <path d="M12 17h.01" />
            </svg>
          </button>
        {/if}
        <button
          bind:this={settingsButtonEl}
          type="button"
          class="titlebar__settings"
          class:active={isSettings}
          aria-label={isSettings ? "Close settings" : "Open settings"}
          aria-current={isSettings ? "page" : undefined}
          use:tip={isSettings ? "Close settings" : `Settings (${shortcutDisplay("openSettings")})`}
          onclick={onSettingsButtonClick}
        >
          <svg
            class="titlebar__settings-icon"
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.75"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
        <div class="titlebar__theme">
          <ThemeModeControl
            bind:value={chromeAppearance}
            compact
            disabled={!theme.loaded || savingChromeAppearance}
            onChange={setChromeAppearance}
          />
        </div>
        {#if devEnabled}
          <button
            type="button"
            class="titlebar__settings"
            aria-label="Open debug"
            use:tip={`Debug (${shortcutDisplay("openDebug")})`}
            onclick={() => void openDebugWindow()}
          >
            <svg
              class="titlebar__settings-icon"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.75"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M9 3h6" />
              <path d="M10 9V7a2 2 0 1 1 4 0v2" />
              <rect x="5" y="9" width="14" height="10" rx="2" />
              <path d="M8 13h.01" />
              <path d="M16 13h.01" />
              <path d="M9 19v2" />
              <path d="M15 19v2" />
              <path d="M2 12h3" />
              <path d="M19 12h3" />
            </svg>
          </button>
        {/if}
      {/if}
    </div>
  </header>
  {/if}

  {#if showDedicatedTitlebar}
  <header class="surface-titlebar">
    <div class="surface-titlebar__drag" data-tauri-drag-region></div>
    <div class="surface-titlebar__actions">
      <ThemeModeControl
        bind:value={chromeAppearance}
        compact
        disabled={!theme.loaded || savingChromeAppearance}
        onChange={setChromeAppearance}
      />
      <button
        type="button"
        class="surface-titlebar__close"
        aria-label="Close window"
        use:tip={"Close"}
        onclick={() => void closeCurrentWindow()}
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
          <path d="M2.5 2.5 9.5 9.5" />
          <path d="M9.5 2.5 2.5 9.5" />
        </svg>
        <span>Close</span>
      </button>
    </div>
  </header>
  {/if}

  {#if isMainWindow}
    <!-- App-wide licensing banner (final-week trial teach-in + Read-Only Mode).
         Renders nothing outside a trial's final week / Read-Only Mode. Main
         window only — Quick Recall / onboarding / dedicated surfaces stay clean. -->
    <LicenseBanner />
    <!-- Deep-link receipt: the visible acknowledgement when a mnema://license/*
         deep link bounces the user back into the app. Main window only — that's
         the window the dispatcher surfaces. -->
    <LicenseDeepLinkModal />
  {/if}

  <main class="app-content" class:app-content--narrow={isNarrow} class:app-content--dedicated={showDedicatedTitlebar} class:app-content--panel={isPanelSurface} class:app-content--settings={isSettingsRoute && !showDedicatedTitlebar}>
    {#if showChildren}
      {@render children()}
    {/if}
  </main>

  {#if shortcutsHelpOpen && canShowShortcutsHelp}
    <div class="shortcut-help" role="presentation" onpointerdown={onShortcutsHelpPointerDown}>
      <div
        class="shortcut-help__panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcut-help-title"
        tabindex="-1"
        bind:this={shortcutsHelpPanelEl}
        onkeydown={onShortcutsHelpKeydown}
      >
        <header class="shortcut-help__header">
          <div>
            <p class="shortcut-help__eyebrow">focused window</p>
            <h2 id="shortcut-help-title">Keyboard shortcuts</h2>
          </div>
          <button
            bind:this={shortcutsHelpCloseEl}
            type="button"
            class="shortcut-help__close"
            aria-label="Close keyboard shortcuts"
            onclick={closeShortcutsHelp}
          >
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
              <path d="M2.5 2.5 9.5 9.5" />
              <path d="M9.5 2.5 2.5 9.5" />
            </svg>
          </button>
        </header>

        <div class="shortcut-help__groups">
          {#each shortcutHelpGroups as group (group.id)}
            <section class="shortcut-help__group" aria-labelledby={`shortcut-help-group-${group.id}`}>
              <h3 id={`shortcut-help-group-${group.id}`}>{group.title}</h3>
              <dl class="shortcut-help__list">
                {#each group.rows as row (row.id)}
                  <div class="shortcut-help__row">
                    <dt>
                      {#each formatShortcut(row.bindings[0], windowPlatform) as token}
                        <kbd>{token}</kbd>
                      {/each}
                    </dt>
                    <dd>{row.label}</dd>
                  </div>
                {/each}
              </dl>
            </section>
          {/each}
        </div>

        <p class="shortcut-help__note">
          Shortcuts pause while focus is inside inputs, sliders, selects, text areas, or buttons.
        </p>
      </div>
    </div>
  {/if}
</div>

<style>
  :global(*, *::before, *::after) {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  /* ── Semantic theme tokens ─────────────────────────────────────
     Tokens live on `:root` so any descendant — including portaled or
     `:global` styled content — can consume them. Three themes are defined
     (Warm Paper redesign, docs/mockups/unified-shell/DESIGN.md):

       · Warm Paper light — the default face, defined on `:root`. Cream
         paper stack (#faf8f2 shell → #fffdf7 cards), botanical green
         accent, warm-red record family.
       · Warm Paper dark — `[data-theme="dark"]`. Warm charcoal, never
         blue; ivory ink; lifted greens.
       · Terminal — `[data-theme="terminal"]`. The legacy dark-terminal
         identity, kept selectable until Warm Paper Dark reaches parity.

     The active set is selected by `data-theme` on `<html>`, written by
     `$lib/theme.svelte`. We deliberately avoid `prefers-color-scheme`
     media queries here because the runtime owns the decision (the user
     can pin a theme explicitly via `appearance`). */
  :global(:root) {
    /* Warm Paper light — values from story-first-v5.html (pinned mockup). */
    --app-bg: #faf8f2;
    --app-fg: #2b2a24;
    --app-fg-muted: #6d675a;
    --app-fg-subtle: #9a927e;

    --app-titlebar-bg: #faf8f2;
    --app-titlebar-border: #ece6d6;
    --app-titlebar-title: #9a927e;

    --app-status-bg: #fbf9f1;
    --app-status-border: #e4ddcb;
    --app-status-fg: #6d675a;
    --app-status-dot: #d6cdb6;

    --app-status-running-fg: #8a3629;
    --app-status-running-border: #e6c4b8;
    --app-status-running-dot: #b0483b;
    --app-status-running-dot-glow: rgba(176, 72, 59, 0.16);

    --app-status-paused-fg: #9a6b1f;
    --app-status-paused-border: #e2d3ae;
    --app-status-paused-dot: #9a6b1f;
    --app-status-paused-dot-glow: rgba(154, 107, 31, 0.18);

    --app-record-start-bg: #fffdf7;
    --app-record-start-fg: #8a3629;
    --app-record-start-border: #e6c4b8;
    --app-record-start-bg-hover: #f7e7e1;
    --app-record-start-fg-hover: #6d2a1f;
    --app-record-start-border-hover: #d8a898;

    --app-record-stop-bg: #f7e7e1;
    --app-record-stop-fg: #8a3629;
    --app-record-stop-border: #e6c4b8;
    --app-record-stop-bg-hover: #efd7cd;
    --app-record-stop-border-hover: #cb9c8d;

    --app-record-glyph-start: #b0483b;
    --app-record-glyph-stop: #8a3629;

    --app-icon-fg: #6d675a;
    --app-icon-fg-hover: #2b2a24;
    --app-icon-bg-hover: #ece8da;
    --app-icon-border-hover: #d6cdb6;
    --app-icon-bg-active: #ece8da;
    --app-icon-border-active: #d6cdb6;

    /* Surface / control tokens shared by the dashboard, settings, and the
       shared bits-ui-backed controls (Switch, Select, RadioGroup, Slider).
       Keeping these centralized means each component declares its palette
       once via these tokens and the dark/terminal sets below flip them in
       one place — no per-component palette duplication. */
    --app-surface: #fffdf7;
    --app-surface-subtle: #f4f1e7;
    --app-surface-raised: #fffefb;
    --app-surface-hover: #ece8da;
    --app-surface-active: #f0f6f0;
    --app-border: #e4ddcb;
    --app-border-strong: #d6cdb6;
    --app-border-hover: #bfb394;
    --app-text-strong: #2b2a24;
    --app-text: #45412f;
    /* Secondary conveyed text — clears the AA 4.5:1 floor on the card
       surface (#5f5949 ≈ 6.5:1 on #fffdf7). */
    --app-text-muted: #5f5949;
    /* Tertiary conveyed text / structural labels — dimmer than muted but
       still AA (#6d675a ≈ 5.2:1). */
    --app-text-subtle: #6d675a;
    /* Placeholder / decorative ONLY (intentionally sub-AA). Never use for text
       a user must read. */
    --app-text-faint: #9a927e;
    --app-accent: #1f6f4a;
    --app-accent-strong: #175a3b;
    --app-accent-bg: #e7f1e9;
    --app-accent-border: #c9dfd0;
    --app-accent-glow: rgba(31, 111, 74, 0.16);
    /* Ink for text placed ON the accent fill. The Warm Paper light accent is
       a deep green, so its ink is paper-light; the dark/terminal accents are
       bright greens carrying dark ink (overridden per theme below). */
    --app-accent-contrast: #f6faf6;

    /* Brand faces (mode-independent). Serif carries human narrative —
       greetings, titles, digest, activity prose; mono is strictly machine
       data — timestamps, durations, provenance, status; sans is the body /
       control face. The legacy terminal theme overrides `--app-font-body`
       back to mono to keep its identity. */
    --app-font-mono: "Berkeley Mono", "TX-02", "Monaspace Neon", ui-monospace,
      monospace;
    --app-font-serif: "Iowan Old Style", "Palatino Linotype", Palatino, Georgia,
      "Times New Roman", serif;
    --app-font-sans: -apple-system, BlinkMacSystemFont, "SF Pro Text",
      "Segoe UI", "Helvetica Neue", Arial, sans-serif;
    --app-font-body: var(--app-font-sans);
    /* Narrative face — greetings, titles, digest, activity prose. Serif in
       the warm themes; terminal keeps mono so the legacy look is untouched. */
    --app-font-narrative: var(--app-font-serif);

    /* Shared focus-visible rings (mode-independent; the accent-glow they key
       off is per-mode, so the ring adapts to the active theme automatically). */
    --app-ring: 0 0 0 3px var(--app-accent-glow);
    --app-ring-danger: 0 0 0 3px
      color-mix(in srgb, var(--app-danger) 30%, transparent);

    /* Canonical disabled-control opacity (mode-independent) — one source of
       truth so dimmed controls stop drifting across 0.35/0.38/0.4/0.45. */
    --app-disabled-opacity: 0.4;

    /* In-flight / saving (`cursor: progress`) controls dim less than a true
       disabled control so the action still reads as "busy, not unavailable". */
    --app-busy-opacity: 0.6;

    /* Shared popover / tooltip elevation. Page depth is normally surface
       lightness, but floating layers lift off with this one shadow. */
    --app-shadow-popover: 0 2px 8px rgba(60, 52, 32, 0.1),
      0 14px 44px rgba(60, 52, 32, 0.16);

    /* Type scale (mode-independent). 6 integer steps consumed app-wide. */
    --text-xs: 10px;
    --text-sm: 11px;
    --text-base: 12px;
    --text-md: 13px;
    --text-lg: 16px;
    --text-xl: 20px;

    --app-warn: #9a6b1f;
    --app-warn-strong: #7f5510;
    --app-warn-bg: #f6efdd;
    --app-warn-border: #e2d3ae;

    --app-danger: #b0483b;
    --app-danger-strong: #8a3629;
    --app-danger-bg: #f7e7e1;
    --app-danger-bg-soft: #fbf1ed;
    --app-danger-border: #e6c4b8;
    --app-danger-text: #a03d30;

    --app-info: #3a6ea8;
    --app-info-strong: #2d578a;
    --app-info-bg: #ecf2f9;
    --app-info-border: #c4d5e8;

    --app-neutral-bg: #f0ecdf;
    --app-neutral-border: #dcd3bd;
    --app-neutral-text: #6d675a;

    --app-source-screen: #6d5a8a;
    --app-source-screen-strong: #59487a;
    --app-source-screen-bg: #f0ebf7;
    --app-source-screen-border: #d5c9e8;

    --app-source-mic: #2a8a60;
    --app-source-mic-strong: #1f6f4a;
    --app-source-mic-bg: #e7f1e9;
    --app-source-mic-border: #c9dfd0;

    --app-source-sysaudio: #9a6b1f;
    --app-source-sysaudio-strong: #7f5510;
    --app-source-sysaudio-bg: #f6efdd;
    --app-source-sysaudio-border: #e2d3ae;

    --app-overlay-bg: rgba(250, 248, 242, 0.8);
    --app-overlay-bg-strong: rgba(250, 248, 242, 0.88);
    --app-overlay-border: rgba(60, 52, 32, 0.12);

    /* Frame-thumbnail stage (quick-recall thumbs) — a dark media well is
       deliberate in every theme (screenshots read best on a dark stage);
       only its hue follows the theme's temperature. */
    --app-thumb-stage: #23211a;
    --app-thumb-stage-fg: #9a927e;

    /* Recessed inner shadow for form-control insets (Input/Select/Combobox/
       Stepper). Soft on the near-white paper fields; the dark themes below
       deepen it. */
    --app-input-recess: rgba(60, 52, 32, 0.08);

    --app-ocr-box: rgba(31, 111, 74, 0.42);
    --app-ocr-box-hover: rgba(31, 111, 74, 0.88);
    --app-ocr-box-fill: transparent;
    --app-ocr-chip-bg: rgba(255, 253, 247, 0.92);
    --app-ocr-chip-text: #175a3b;
    --app-ocr-chip-border: rgba(31, 111, 74, 0.24);
    --app-ocr-hover-shadow: rgba(60, 52, 32, 0.18);
    --app-ocr-hover-inset: transparent;
    --app-ocr-chip-text-shadow: none;

    /* Insights chart tokens (Warm Paper light). Warm grayscale "free tier"
       ramp (light → dark so bars read on the bright paper), the engine
       category palette, and focus heat — consumed by the SVG chart
       primitives in `$lib/insights/charts/`. Flipping `data-theme` reskins
       them via the dark/terminal overrides below. */
    --chart-grey-1: #ddd6c4;
    --chart-grey-2: #c4bba5;
    --chart-grey-3: #a29a84;
    --chart-grey-4: #7e7767;
    --chart-grey-5: #55503e;

    /* Category palette per the v5 mockup, extended for the categories the
       mockup doesn't carry. "Creating" is deliberately NOT the exact accent
       green and "Entertainment" not the danger red, so a category color
       never reads as a semantic signal. */
    --cat-creating: #2a8a60;
    --cat-communication: #3a6ea8;
    --cat-meetings: #8a5a2a;
    --cat-research: #6d5a8a;
    --cat-learning: #217f74;
    --cat-organizing: #9a6b1f;
    --cat-personal: #a4641c;
    --cat-entertainment: #b65c35;

    --focus-deep: #1f6f4a;
    --focus-mid: #9a6b1f;
    --focus-distracted: #b0483b;
  }

  /* Warm Paper dark — warm charcoal, never blue; ivory ink; lifted greens.
     Values from the v5 mockup's dark block. */
  :global([data-theme="dark"]) {
    --app-bg: #17150f;
    --app-fg: #ede8d9;
    --app-fg-muted: #b5ac95;
    --app-fg-subtle: #978d79;

    --app-titlebar-bg: #17150f;
    --app-titlebar-border: #262218;
    --app-titlebar-title: #978d79;

    --app-status-bg: #1a1710;
    --app-status-border: #2d2920;
    --app-status-fg: #b5ac95;
    --app-status-dot: #3e382a;

    --app-status-running-fg: #e39a8b;
    --app-status-running-border: #4d2c22;
    --app-status-running-dot: #d97f6e;
    --app-status-running-dot-glow: rgba(217, 127, 110, 0.18);

    --app-status-paused-fg: #d3a75c;
    --app-status-paused-border: #4a3c1c;
    --app-status-paused-dot: #d3a75c;
    --app-status-paused-dot-glow: rgba(211, 167, 92, 0.18);

    --app-record-start-bg: #1e1b13;
    --app-record-start-fg: #e39a8b;
    --app-record-start-border: #4d2c22;
    --app-record-start-bg-hover: #2d1b15;
    --app-record-start-fg-hover: #eeb0a2;
    --app-record-start-border-hover: #6a3c2e;

    --app-record-stop-bg: #2d1b15;
    --app-record-stop-fg: #ede8d9;
    --app-record-stop-border: #4d2c22;
    --app-record-stop-bg-hover: #3a231a;
    --app-record-stop-border-hover: #6a3c2e;

    --app-record-glyph-start: #d97f6e;
    --app-record-glyph-stop: #e39a8b;

    --app-icon-fg: #b5ac95;
    --app-icon-fg-hover: #ede8d9;
    --app-icon-bg-hover: #221f15;
    --app-icon-border-hover: #3e382a;
    --app-icon-bg-active: #201d14;
    --app-icon-border-active: #3e382a;

    --app-surface: #1e1b13;
    --app-surface-subtle: #131109;
    --app-surface-raised: #262115;
    --app-surface-hover: #2d2820;
    --app-surface-active: #1c2f23;
    --app-border: #2d2920;
    --app-border-strong: #3e382a;
    --app-border-hover: #55503e;
    --app-text-strong: #ede8d9;
    --app-text: #d8d1bc;
    /* Secondary conveyed text — #b5ac95 ≈ 7.5:1 on #1e1b13. */
    --app-text-muted: #b5ac95;
    /* Tertiary conveyed text / structural labels — #978d79 ≈ 5:1, AA. */
    --app-text-subtle: #978d79;
    /* Placeholder / decorative ONLY (intentionally sub-AA). */
    --app-text-faint: #6a6250;
    --app-accent: #5cbd8d;
    --app-accent-strong: #7ed3a8;
    --app-accent-bg: #1c2f23;
    --app-accent-border: #2f4d3b;
    --app-accent-glow: rgba(92, 189, 141, 0.18);
    /* The lifted green fill carries dark ink. */
    --app-accent-contrast: #0f231a;

    --app-shadow-popover: 0 2px 8px rgba(0, 0, 0, 0.4),
      0 14px 44px rgba(0, 0, 0, 0.55);

    --app-warn: #d3a75c;
    --app-warn-strong: #e0b975;
    --app-warn-bg: #2b2312;
    --app-warn-border: #4a3c1c;

    --app-danger: #d97f6e;
    --app-danger-strong: #e39a8b;
    --app-danger-bg: #2d1b15;
    --app-danger-bg-soft: #221713;
    --app-danger-border: #4d2c22;
    --app-danger-text: #e39a8b;

    --app-info: #7aa8d8;
    --app-info-strong: #9dbfe4;
    --app-info-bg: #16202d;
    --app-info-border: #2a3c52;

    --app-neutral-bg: #221f15;
    --app-neutral-border: #3e382a;
    --app-neutral-text: #978d79;

    --app-source-screen: #ab94d3;
    --app-source-screen-strong: #8f76bd;
    --app-source-screen-bg: #241f31;
    --app-source-screen-border: #3b3153;

    --app-source-mic: #4fb383;
    --app-source-mic-strong: #3d9a6d;
    --app-source-mic-bg: #16281e;
    --app-source-mic-border: #2b4a37;

    --app-source-sysaudio: #d0a452;
    --app-source-sysaudio-strong: #ba8f3e;
    --app-source-sysaudio-bg: #2b2312;
    --app-source-sysaudio-border: #4a3c1c;

    --app-overlay-bg: rgba(14, 12, 7, 0.78);
    --app-overlay-bg-strong: rgba(14, 12, 7, 0.85);
    --app-overlay-border: rgba(237, 232, 217, 0.08);

    --app-thumb-stage: #0f0d08;
    --app-thumb-stage-fg: #978d79;

    --app-input-recess: rgba(0, 0, 0, 0.35);

    --app-ocr-box: rgba(120, 220, 160, 0.45);
    --app-ocr-box-hover: rgba(120, 220, 160, 0.95);
    --app-ocr-box-fill: rgba(120, 220, 160, 0.1);
    --app-ocr-chip-bg: rgba(12, 10, 5, 0.96);
    --app-ocr-chip-text: #eaffef;
    --app-ocr-chip-border: rgba(120, 220, 160, 0.6);
    --app-ocr-hover-shadow: rgba(0, 0, 0, 0.45);
    --app-ocr-hover-inset: rgba(255, 255, 255, 0.04);
    --app-ocr-chip-text-shadow: none;

    /* Charts — warm dark ramp + the mockup's dark category palette. */
    --chart-grey-1: #322d1f;
    --chart-grey-2: #453e2b;
    --chart-grey-3: #5d553d;
    --chart-grey-4: #7b7257;
    --chart-grey-5: #a3997c;

    --cat-creating: #4fb383;
    --cat-communication: #7aa8d8;
    --cat-meetings: #cf9c5c;
    --cat-research: #ab94d3;
    --cat-learning: #5cc0b2;
    --cat-organizing: #d0a452;
    --cat-personal: #c98f4a;
    --cat-entertainment: #d98b62;

    --focus-deep: #5cbd8d;
    --focus-mid: #d3a75c;
    --focus-distracted: #d97f6e;
  }

  /* Terminal — the legacy dark-terminal identity, lifted verbatim from the
     pre-Warm-Paper chrome. Kept selectable until Warm Paper Dark reaches
     parity (see PLAN.md Further Notes); superseded, not deleted. */
  :global([data-theme="terminal"]) {
    --app-bg: #0c0c0e;
    --app-fg: #e2e2e8;
    --app-fg-muted: #8a8aaa;
    --app-fg-subtle: #45455a;

    --app-titlebar-bg: #08080c;
    --app-titlebar-border: #15151f;
    --app-titlebar-title: #45455a;

    --app-status-bg: #0a0a10;
    --app-status-border: #161624;
    --app-status-fg: #6f6f90;
    --app-status-dot: #2a2a3a;

    --app-status-running-fg: #ff5d6c;
    --app-status-running-border: #3a1820;
    --app-status-running-dot: #ff3148;
    --app-status-running-dot-glow: rgba(255, 49, 72, 0.18);

    --app-status-paused-fg: #d6a14a;
    --app-status-paused-border: #3a2818;
    --app-status-paused-dot: #d6a14a;
    --app-status-paused-dot-glow: rgba(214, 161, 74, 0.16);

    --app-record-start-bg: #1a0f12;
    --app-record-start-fg: #ff8a96;
    --app-record-start-border: #3a1820;
    --app-record-start-bg-hover: #2a1218;
    --app-record-start-fg-hover: #ffb0b9;
    --app-record-start-border-hover: #5a2030;

    --app-record-stop-bg: #170d0f;
    --app-record-stop-fg: #f0f0f5;
    --app-record-stop-border: #4a1c26;
    --app-record-stop-bg-hover: #2a1218;
    --app-record-stop-border-hover: #6a2434;

    --app-record-glyph-start: #ff3148;
    --app-record-glyph-stop: #ff8a96;

    --app-icon-fg: #8a8aaa;
    --app-icon-fg-hover: #e2e2e8;
    --app-icon-bg-hover: #1a1a2a;
    --app-icon-border-hover: #2a2a3a;
    --app-icon-bg-active: #14141f;
    --app-icon-border-active: #2a2a3a;

    --app-surface: #0e0e16;
    --app-surface-subtle: #101018;
    --app-surface-raised: #13131a;
    --app-surface-hover: #1a1a2a;
    --app-surface-active: #131320;
    --app-border: #1e1e2e;
    --app-border-strong: #2a2a3a;
    --app-border-hover: #3a3a5a;
    --app-text-strong: #e2e2e8;
    --app-text: #c0c0d0;
    --app-text-muted: #9696ae;
    --app-text-subtle: #7e7e98;
    --app-text-faint: #33334a;
    --app-accent: #3dffa0;
    --app-accent-strong: #2a8a60;
    --app-accent-bg: #0d1f15;
    --app-accent-border: #1a4a30;
    --app-accent-glow: rgba(61, 255, 160, 0.18);
    --app-accent-contrast: #07120c;

    /* The terminal identity keeps its monospace body and narrative. */
    --app-font-body: var(--app-font-mono);
    --app-font-narrative: var(--app-font-mono);

    --app-shadow-popover: 0 8px 24px rgba(0, 0, 0, 0.22);

    --app-warn: #d6a14a;
    --app-warn-strong: #c47a30;
    --app-warn-bg: #1a1208;
    --app-warn-border: #7a4a18;

    --app-danger: #ff6b7a;
    --app-danger-strong: #ff4455;
    --app-danger-bg: #2e0f14;
    --app-danger-bg-soft: #0e0a0a;
    --app-danger-border: #4a1a20;
    --app-danger-text: #ff8090;

    --app-info: #60b0ff;
    --app-info-strong: #4a6aaa;
    --app-info-bg: #0c1a2e;
    --app-info-border: #1a3050;

    --app-neutral-bg: #1a1a2a;
    --app-neutral-border: #2a2a3a;
    --app-neutral-text: #7070a0;

    --app-source-screen: #c0b0ff;
    --app-source-screen-strong: #5a4aaa;
    --app-source-screen-bg: #1a1a3a;
    --app-source-screen-border: #2a2a5a;

    --app-source-mic: #80d0a8;
    --app-source-mic-strong: #4a8a6a;
    --app-source-mic-bg: #0f2e1f;
    --app-source-mic-border: #1a4a30;

    --app-source-sysaudio: #b0c080;
    --app-source-sysaudio-strong: #6a7a4a;
    --app-source-sysaudio-bg: #2a2010;
    --app-source-sysaudio-border: #4a3a18;

    --app-overlay-bg: rgba(10, 10, 16, 0.78);
    --app-overlay-bg-strong: rgba(10, 10, 16, 0.82);
    --app-overlay-border: rgba(255, 255, 255, 0.06);

    --app-thumb-stage: #101014;
    --app-thumb-stage-fg: #6a6a74;

    --app-input-recess: rgba(0, 0, 0, 0.25);

    --app-ocr-box: rgba(120, 220, 160, 0.45);
    --app-ocr-box-hover: rgba(120, 220, 160, 0.95);
    --app-ocr-box-fill: rgba(120, 220, 160, 0.10);
    --app-ocr-chip-bg: rgba(8, 14, 10, 0.96);
    --app-ocr-chip-text: #eaffef;
    --app-ocr-chip-border: rgba(120, 220, 160, 0.6);
    --app-ocr-hover-shadow: rgba(0, 0, 0, 0.45);
    --app-ocr-hover-inset: rgba(255, 255, 255, 0.04);
    --app-ocr-chip-text-shadow: none;

    --chart-grey-1: #2c2c3a;
    --chart-grey-2: #3e3e50;
    --chart-grey-3: #565669;
    --chart-grey-4: #757589;
    --chart-grey-5: #9a9ab0;

    --cat-creating: #5fe07a;
    --cat-communication: #c0b0ff;
    --cat-meetings: #ff9fd0;
    --cat-research: #60b0ff;
    --cat-learning: #4fd8c8;
    --cat-organizing: #b0c080;
    --cat-personal: #d6a14a;
    --cat-entertainment: #ff7a4d;

    --focus-deep: #3dffa0;
    --focus-mid: #d6a14a;
    --focus-distracted: #ff6b7a;
  }

  :global(html) {
    height: 100%;
    overscroll-behavior: none;
  }

  :global(html.dedicated-surface-window) {
    background: transparent;
  }

  :global(body) {
    min-height: 100%;
    background-color: var(--app-bg);
    color: var(--app-fg);
    /* Warm Paper body face: sans in the warm themes, mono in terminal —
       the token flips per theme, mono stays for machine data everywhere. */
    font-family: var(--app-font-body);
    font-size: var(--text-md);
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
    overscroll-behavior: none;
    /* Native-app selection model: the chrome (icons, buttons, decorative
       glyphs, drag regions) is non-selectable by default like a macOS app,
       and only genuine text-bearing elements opt selection back in below.
       Components can still opt individual nodes in/out explicitly. */
    user-select: none;
    -webkit-user-select: none;
    /* Smooth the chrome flip when the user toggles `appearance`. Kept
       short so the change still feels responsive. */
    transition: background-color 0.18s ease, color 0.18s ease;
  }

  /* Re-enable text selection for content the user reads/copies. Deliberately
     excludes `span`/`div` since those frequently wrap icons; text inside them
     that must stay selectable opts in explicitly (e.g. OCR text). */
  :global(p),
  :global(h1),
  :global(h2),
  :global(h3),
  :global(h4),
  :global(h5),
  :global(h6),
  :global(input),
  :global(textarea),
  :global(code),
  :global(pre),
  :global(label),
  :global(a),
  :global(li),
  :global(td),
  :global(th),
  :global([contenteditable]) {
    user-select: text;
    -webkit-user-select: text;
  }

  /* Themed text selection. Without this WebKit falls back to its default
     highlight, which clashes with the terminal chrome — faint text (e.g. an
     install path) selected against it read as an unreadable wash. A translucent
     accent highlight with forced-strong text stays on-brand and legible in both
     themes. */
  :global(::selection) {
    background: color-mix(in srgb, var(--app-accent) 28%, transparent);
    color: var(--app-text-strong);
  }

  :global(body.dedicated-surface-window) {
    background: transparent;
  }

  :global(a) {
    text-decoration: none;
  }

  /* ── App-wide custom scrollbars ────────────────────────────────
     A single themed baseline for every scrollable surface. Two goals:

     1. Match the theme. The thumb is tinted from the shared `--app-*`
        tokens, so it flips with light/dark like the rest of the chrome
        (quiet border grey at rest → stronger on hover → accent while
        dragging).
     2. Never overlay content. macOS WebKit (and Windows WebView2)
        default to *overlay* scrollbars that float on top of content.
        Defining a `::-webkit-scrollbar` with an explicit width forces
        the classic, gutter-reserving scrollbar instead — so it pushes
        content aside rather than covering it.

     These are `:global` defaults with zero selector specificity, so any
     component that styles its own scrollbar (settings auto-hide, the
     hidden rail history, the thin quick-recall row) still wins. */
  :global(html) {
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
  }
  :global(::-webkit-scrollbar) {
    width: 12px;
    height: 12px;
  }
  :global(::-webkit-scrollbar-track) {
    background: transparent;
  }
  :global(::-webkit-scrollbar-corner) {
    background: transparent;
  }
  :global(::-webkit-scrollbar-thumb) {
    /* The 3px transparent border + padding-box clip insets the visible
       thumb, leaving breathing room on both sides of the gutter. */
    background-color: var(--app-border-strong);
    background-clip: padding-box;
    border: 3px solid transparent;
    border-radius: 999px;
  }
  :global(::-webkit-scrollbar-thumb:hover) {
    background-color: var(--app-border-hover);
    background-clip: padding-box;
  }
  :global(::-webkit-scrollbar-thumb:active) {
    background-color: var(--app-accent-strong);
    background-clip: padding-box;
  }

  /* Custom tooltip — portaled to <body> by the `tip` action
     ($lib/components/tooltip.ts), styled here so it reads the same tokens as
     the app instead of the OS's native `title` bubble. The accent left edge is
     the terminal "prompt" signature. */
  :global(.app-tooltip) {
    position: fixed;
    top: 0;
    left: 0;
    z-index: 9999;
    max-width: 260px;
    padding: 5px 8px 6px;
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    line-height: 1.45;
    letter-spacing: 0.01em;
    color: var(--app-text-strong);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-left: 2px solid var(--app-accent);
    border-radius: 5px;
    box-shadow: var(--app-shadow-popover);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    pointer-events: none;
    opacity: 0;
    transform: translateY(2px);
    transition:
      opacity 90ms ease,
      transform 90ms ease;
  }
  :global(.app-tooltip[data-show="true"]) {
    opacity: 1;
    transform: translateY(0);
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.app-tooltip) {
      transition: none;
      transform: none;
    }
  }

  .app-shell {
    --app-titlebar-height: 36px;
    --app-window-radius: 10px;
    display: flex;
    flex-direction: column;
    min-height: 100vh;
    min-height: 100dvh;
  }

  /* Main window surfaces (Timeline + Insights) own their internal scrolling:
     the shell is pinned to the viewport so a tall surface (e.g. a long Chat
     transcript) scrolls inside its own region instead of growing the shell and
     scrolling the whole window. Without a definite height here the chain is only
     `min-height: 100vh`, so `.insights`'s `height: 100%` can't resolve and the
     surface grows to content height. Dedicated/panel windows pin themselves
     separately; onboarding is not a main-surface route, so it still page-scrolls. */
  .app-shell--bounded {
    height: 100vh;
    height: 100dvh;
    overflow: hidden;
  }

  .app-shell--macos {
    --app-window-radius: 12px;
  }

  .app-shell--windows {
    --app-window-radius: 8px;
  }

  /* ── Title bar ────────────────────────────────────────────────
     Fixed-height custom title bar that sits at the top of every route.
     Tauri's `decorations: false` window means this is the only chrome the
     user sees; the inert filler area carries `data-tauri-drag-region` so
     dragging the empty space moves the window, while the controls on
     either side remain ordinary (clickable) interactive elements. */
  .titlebar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--app-titlebar-height);
    /* Reserve ~72px on the left so our content never collides with the
       macOS native traffic lights drawn by Tauri's overlay title-bar. The
       right side keeps its tighter inset since nothing native sits there. */
    padding: 0 8px 0 78px;
    background: var(--app-titlebar-bg);
    border-bottom: 1px solid var(--app-titlebar-border);
    /* Hard backstop: a tiling WM (e.g. aerospace) can force the window below the
       640px app minimum, and flex items can't shrink past their content width —
       clip rather than let the row spill the right-hand controls off-screen.
       The responsive tiers below shed items progressively so this rarely bites. */
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
    /* Sticky so the title bar stays visible when a route's main content
       scrolls vertically. Uses position: sticky rather than fixed so layout
       below it doesn't need to compensate with extra padding. */
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .surface-titlebar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 40px;
    padding: 0 10px 0 14px;
    background: var(--app-titlebar-bg);
    border-radius: var(--app-window-radius) var(--app-window-radius) 0 0;
    box-shadow: inset 0 -1px 0 var(--app-titlebar-border);
    user-select: none;
    -webkit-user-select: none;
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .surface-titlebar__drag {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
  }

  .surface-titlebar__actions {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex: 0 0 auto;
  }

  .surface-titlebar__close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 72px;
    height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--app-icon-border-hover);
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .surface-titlebar__close:hover {
    background: var(--app-icon-bg-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .surface-titlebar__close:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .surface-titlebar__close:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }

  .titlebar__group {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }

  /* Let the left cluster yield at narrow widths so the deficit is absorbed by
     the privacy-warning's ellipsis (its label already truncates) rather than
     the centered drag region clipping the Search door. */
  .titlebar__group--left {
    flex: 0 1 auto;
    min-width: 0;
  }

  .titlebar__drag {
    flex: 1 1 auto;
    /* Let the centre region collapse to zero so the inert drag slack yields
       first under width pressure and never pushes the surface toggle, search,
       or right-hand controls off-screen. */
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    overflow: hidden;
    cursor: default;
  }

  /* ── Quick Recall door ─────────────────────────────────────────
     A visible, mouse-discoverable entry to Quick Recall; the global ⌥Space
     shortcut alone is undiscoverable for a new user. */
  .titlebar__search {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    padding: 0 8px 0 9px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font: inherit;
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .titlebar__search-icon {
    flex: 0 0 auto;
  }
  .titlebar__search-label {
    letter-spacing: 0.02em;
  }
  .titlebar__search-kbd {
    flex: 0 0 auto;
    padding: 1px 5px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface-raised);
    color: var(--app-text-subtle);
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    line-height: 1.3;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .titlebar__search:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .titlebar__search:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .titlebar__search:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }

  /* ── Responsive title-bar degradation ─────────────────────────
     Three progressive tiers that shed non-essential affordances as the window
     narrows. The app's own minimum is 640px (min_inner_size in
     src-tauri/src/windows.rs), but a tiling WM (e.g. aerospace) ignores that
     and can force the window down to ~400px — so the bar must keep shedding
     well below 640px. Flex items can't shrink past their content width, so
     instead of squeezing we drop whole items, lowest-priority first.

     ALWAYS VISIBLE at every width (never hidden, never clipped):
       • the record pill
       • the raw-timeline clock door
       • the settings gear (`.titlebar__settings`, sans `--help`)
       • notifications bell when present
     Combined with `.titlebar { overflow: hidden }`, the right group can never
     spill off-screen. (The dashboard body's own breakpoint lives in
     +page.svelte.) */

  /* The titlebar is control-dense; the fully-labelled row needs ~820px to fit,
     and the WM can force widths well below the app's 640px minimum, so the row
     sheds progressively. Always-visible at every width: the record pill, the
     timeline door, the settings gear, and notifications-when-present. Combined
     with the record pill's own label ellipsis (left cluster yields first) and
     `.titlebar { overflow: hidden }`, nothing can spill off-screen.
     Breakpoints are tuned to real content widths — the labelled row overflows
     below ~820px, which includes the 800px default window. */

  /* Compact ≤820px: drop the Search word + kbd to an icon-only button, tighten
     the row gap. */
  @media (max-width: 820px) {
    .titlebar {
      gap: 6px;
    }
    .titlebar__search-label,
    .titlebar__search-kbd {
      display: none;
    }
    .titlebar__search {
      gap: 0;
      padding: 0 6px;
    }
  }

  /* Narrow ≤720px: drop the lowest-value right-group items — the help button and
     the theme control (both still reachable from Settings). Gap tightens. */
  @media (max-width: 720px) {
    .titlebar {
      gap: 4px;
    }
    /* `.titlebar`-prefixed to outrank the later base `.titlebar__settings`
       display rule (equal specificity would otherwise lose on source order). */
    .titlebar .titlebar__settings--help {
      display: none;
    }
    .titlebar__theme {
      display: none;
    }
  }

  /* Tight ≤600px (incl. WM-forced sub-minimum widths): drop the source toggles
     — recording sources stay reachable via the tray menu + Settings. */
  @media (max-width: 600px) {
    /* `.titlebar`-prefixed to outrank the later base `.titlebar__source`
       display rule. */
    .titlebar .titlebar__source {
      display: none;
    }
  }

  @keyframes titlebar-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
  }

  @media (prefers-reduced-motion: reduce) {
    .titlebar__source-dot {
      animation: none;
    }
  }

  /* ── Per-source recording pills ───────────────────────────────
     One pill per requested capture source (screen / microphone /
     system audio), rendered after the Record/Stop button. Each pill
     pairs the source's icon with a status icon: a pulsing red dot
     while live, pause bars while inactivity-paused, or a hollow ring
     while the source is still spinning up. Sources not requested for
     the current session aren't rendered. The pill chrome keeps the
     title bar visually coherent alongside the record pill. */
  .titlebar__source {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 8px;
    height: 24px;
    background: var(--app-status-bg);
    border: 1px solid var(--app-status-border);
    border-radius: 4px;
    color: var(--app-status-fg);
  }
  .titlebar__source-icon {
    display: block;
    flex: 0 0 auto;
  }
  .titlebar__source-state {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 8px;
    height: 8px;
    line-height: 1;
    flex: 0 0 auto;
  }
  .titlebar__source-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-status-running-dot);
    box-shadow: 0 0 0 2px var(--app-status-running-dot-glow);
    animation: titlebar-pulse 1.4s ease-in-out infinite;
  }
  .titlebar__source-ring {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    box-sizing: border-box;
    opacity: 0.7;
  }
  .titlebar__source--running {
    color: var(--app-status-running-fg);
    border-color: var(--app-status-running-border);
  }
  .titlebar__source--paused {
    color: var(--app-status-paused-fg);
    border-color: var(--app-status-paused-border);
  }
  .titlebar__source--starting {
    color: var(--app-fg-muted);
  }
  .titlebar__source--off {
    color: var(--app-fg-subtle);
    opacity: 0.55;
  }

  /* ── Toggle mode (idle / not recording) ───────────────────── */
  .titlebar__source--toggle {
    font-family: inherit;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s;
  }
  .titlebar__source--toggle:disabled {
    cursor: progress;
    opacity: var(--app-busy-opacity);
  }
  .titlebar__source--toggle.titlebar__source--selected {
    color: var(--app-text-strong);
    border-color: var(--app-border-strong);
    background: var(--app-surface-raised);
  }
  .titlebar__source--toggle.titlebar__source--unselected {
    /* "Off" must still be legible: --app-fg-subtle at 0.7 opacity rendered the
       glyph near-invisible (well under the 3:1 non-text floor). --app-text-subtle
       (~4.9:1) at near-full opacity reads clearly as a disabled-but-present
       source while staying dimmer than the selected --app-text-strong. */
    color: var(--app-text-subtle);
    border-color: var(--app-status-border);
    background: var(--app-status-bg);
    opacity: 0.9;
  }
  .titlebar__source--toggle:not(:disabled):hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
    opacity: 1;
  }
  .titlebar__source--toggle:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .titlebar__privacy-warning {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: min(380px, 34vw);
    height: 22px;
    padding: 3px 8px;
    border-radius: 4px;
    border: 1px solid var(--app-warn-border);
    background: var(--app-warn-bg);
    color: var(--app-warn);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .titlebar__privacy-warning--restart-required {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg-soft);
    color: var(--app-danger-text);
  }
  .titlebar__privacy-warning-icon {
    flex: 0 0 auto;
  }
  .titlebar__privacy-warning-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .titlebar__privacy-warning-action {
    flex: 0 0 auto;
    height: 16px;
    padding: 0 6px;
    border-radius: 3px;
    border: 1px solid currentColor;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
  }
  .titlebar__privacy-warning-action:hover:not(:disabled) {
    background: color-mix(in srgb, currentColor 14%, transparent);
  }
  .titlebar__privacy-warning-action:disabled {
    cursor: progress;
    opacity: var(--app-busy-opacity);
  }

  /* Diagonal slash glyph used when a source is unselected (idle) or
     forcibly off (live). Drawn as a thin rotated bar so it reads as
     "muted/skipped" without bringing in another SVG. */
  .titlebar__source-slash {
    width: 8px;
    height: 1.5px;
    background: currentColor;
    border-radius: 1px;
    transform: rotate(-45deg);
    opacity: 0.85;
  }

  /* ── Surface actions ──────────────────────────────────────── */
  .titlebar__settings {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    width: 28px;
    height: 28px;
    border-radius: 4px;
    color: var(--app-icon-fg);
    border: 1px solid transparent;
    background: transparent;
    cursor: pointer;
    padding: 0;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .titlebar__settings--labelled {
    gap: 6px;
    width: auto;
    padding: 0 12px 0 10px;
  }
  .titlebar__settings:hover {
    background: var(--app-icon-bg-hover);
    color: var(--app-icon-fg-hover);
    border-color: var(--app-icon-border-hover);
  }
  .titlebar__settings.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .titlebar__settings:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .titlebar__settings:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }
  .titlebar__settings-icon {
    display: block;
    flex: 0 0 auto;
  }
  .titlebar__notifications {
    position: relative;
    display: inline-flex;
  }
  /* Quiet rest state: the bell is always mounted (so neighbours don't shift),
     but when there's nothing to open it recedes to the dim icon tone. */
  .titlebar__notifications-button--quiet {
    color: var(--app-icon-fg);
    opacity: 0.5;
  }
  .titlebar__notifications-button {
    position: relative;
  }
  .titlebar__notification-dot {
    position: absolute;
    top: 2px;
    right: 2px;
    min-width: 12px;
    height: 12px;
    padding: 0 3px;
    border-radius: 999px;
    /* Notification count = "items need attention", not "success" — use the
       info tone, not the green success accent. Warning/error variants below
       escalate it. */
    background: var(--app-info);
    color: var(--app-bg);
    font-size: var(--text-xs);
    font-weight: 800;
    line-height: 12px;
    text-align: center;
  }
  .titlebar__notification-dot--warning {
    background: var(--app-warn);
    color: var(--app-bg);
  }
  .titlebar__notification-dot--error {
    background: var(--app-danger);
    color: var(--app-bg);
  }
  .notification-popover {
    /* Fixed, not absolute: `.titlebar { overflow: hidden }` (the tiling-WM
       spill backstop) clips absolutely-positioned descendants, which clipped
       this popover out of existence. Fixed positioning resolves against the
       viewport and escapes the clip; the titlebar is sticky at the top with a
       fixed height, so anchoring just below it lands in the same spot. */
    position: fixed;
    top: calc(var(--app-titlebar-height) + 8px);
    right: 8px;
    /* Above the sticky titlebar's z-index: 100. */
    z-index: 200;
    width: min(340px, calc(100vw - 24px));
    max-height: 360px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 8px;
    box-shadow: var(--app-shadow-popover);
  }
  .notification-popover__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--app-border);
    font-size: var(--text-sm);
    font-weight: 700;
    color: var(--app-text-strong);
  }
  .notification-popover__clear,
  .notification-item__clear {
    border: 1px solid transparent;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    font: inherit;
    border-radius: 4px;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .notification-popover__clear {
    font-size: var(--text-sm);
    font-weight: 700;
    padding: 4px 7px;
  }
  .notification-popover__clear:hover,
  .notification-item__clear:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
    border-color: var(--app-border);
  }
  .notification-popover__clear:focus-visible,
  .notification-item__clear:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .notification-popover__list {
    overflow-y: auto;
    padding: 6px;
  }
  .notification-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
    padding: 9px 10px;
    border-radius: 6px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
  }
  .notification-item + .notification-item {
    margin-top: 6px;
  }
  .notification-item--warning {
    border-color: var(--app-warn-border);
    background: var(--app-warn-bg);
  }
  .notification-item--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg-soft);
  }
  .notification-item--error .notification-item__title {
    color: var(--app-danger-text);
  }
  .notification-item--info {
    border-color: var(--app-info-border);
    background: var(--app-info-bg);
  }
  .notification-item--info .notification-item__title {
    color: var(--app-info);
  }
  .notification-item__body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .notification-item__title {
    color: var(--app-text-strong);
    font-size: var(--text-sm);
    font-weight: 700;
    line-height: 1.2;
  }
  .notification-item__message {
    color: var(--app-text-muted);
    font-size: var(--text-sm);
    line-height: 1.35;
  }
  .notification-item__time {
    margin-top: 2px;
    color: var(--app-text-faint, var(--app-text-muted));
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
  }
  .notification-popover__error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 6px 6px 0;
    padding: 7px 9px;
    border-radius: 6px;
    border: 1px solid var(--app-danger-border);
    background: var(--app-danger-bg-soft);
    color: var(--app-danger-text);
    font-size: var(--text-sm);
  }
  .notification-popover__error-text {
    min-width: 0;
  }
  .notification-popover__error-dismiss {
    flex: 0 0 auto;
    border: 1px solid currentColor;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--text-xs);
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    padding: 3px 7px;
    border-radius: 4px;
    cursor: pointer;
  }
  .notification-popover__error-dismiss:hover {
    background: color-mix(in srgb, currentColor 14%, transparent);
  }
  .notification-popover__error-dismiss:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .notification-item__action {
    align-self: flex-start;
    margin-top: 4px;
    padding: 4px 7px;
    border-radius: 4px;
    border: 1px solid var(--app-border-strong);
    background: var(--app-surface);
    color: var(--app-text);
    font-size: var(--text-xs);
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .notification-item__action:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }
  .notification-item__action:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .notification-item__clear {
    align-self: start;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
  }
  .titlebar__settings-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    line-height: 1;
    text-transform: uppercase;
    white-space: nowrap;
  }

  /* ── Content ──────────────────────────────────────────────── */
  .app-content {
    flex: 1;
    width: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .app-content--dedicated {
    background: var(--app-bg);
    border-radius: 0 0 var(--app-window-radius) var(--app-window-radius);
    overflow: hidden;
  }

  .app-content--panel {
    padding: 0;
    min-height: 100vh;
    min-height: 100dvh;
    background: transparent;
  }

  /* Settings rendered inside the Main window, below the persistent top nav (the
     Main titlebar). The titlebar already reserves space for the native overlay
     traffic lights, so no top inset is needed here — just a small gap under the
     bar. Full-bleed otherwise (the settings shell owns its own scroll region). */
  .app-content--settings {
    background: var(--app-bg);
    overflow: hidden;
    padding: 8px 20px 0;
  }

  .app-shell--dedicated {
    background: var(--app-bg);
    border-radius: var(--app-window-radius);
    overflow: hidden;
    padding: 0;
    /* Pin the dedicated surface to the viewport so the page header + tab
       strip stay in place and only the scroll region inside the panel area
       moves. Without this the shell grows past the viewport (it inherits
       only `min-height: 100vh` from `.app-shell`) and the entire window
       scrolls instead of just the panel content. */
    height: 100vh;
    height: 100dvh;
  }

  /* The narrow column is opt-in — only routes that explicitly want a
     centered, padded reading column (currently `/settings` and `/debug`)
     request it. Surfaces like the timeline consume the full
     viewport width by default so previews and dense controls aren't
     artificially capped. */
  .app-content--narrow {
    max-width: 860px;
    margin: 0 auto;
    padding: calc(var(--app-titlebar-height) + 14px) 24px 64px;
    gap: 14px;
  }

  .app-content--dedicated.app-content--narrow {
    max-width: none;
    margin: 0;
    padding: 16px 20px 28px;
    gap: 14px;
  }

  /* ── Keyboard shortcuts help ──────────────────────────────── */
  .shortcut-help {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--app-overlay-bg);
    backdrop-filter: blur(10px);
  }

  .shortcut-help__panel {
    width: min(560px, 100%);
    max-height: min(680px, calc(100vh - 48px));
    overflow-y: auto;
    border: 1px solid var(--app-border-strong);
    border-radius: 12px;
    background: var(--app-surface-raised);
    color: var(--app-text);
    box-shadow: var(--app-shadow-popover);
    padding: 18px;
  }

  .shortcut-help__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 16px;
  }

  .shortcut-help__eyebrow {
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    line-height: 1;
    margin-bottom: 6px;
    text-transform: uppercase;
  }

  .shortcut-help h2 {
    color: var(--app-text-strong);
    font-size: var(--text-xl);
    line-height: 1.15;
    letter-spacing: -0.02em;
  }

  .shortcut-help__close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    cursor: pointer;
    font: inherit;
  }

  .shortcut-help__close:hover,
  .shortcut-help__close:focus-visible {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }

  .shortcut-help__groups {
    display: grid;
    gap: 14px;
  }

  .shortcut-help__group {
    display: grid;
    gap: 8px;
  }

  .shortcut-help__group h3 {
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 800;
    letter-spacing: 0.12em;
    line-height: 1;
    text-transform: uppercase;
  }

  .shortcut-help__list {
    display: grid;
    gap: 8px;
  }

  .shortcut-help__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    padding: 9px 10px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-raised);
  }

  .shortcut-help__row dt {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 72px;
  }

  .shortcut-help__row dd {
    color: var(--app-text);
    font-size: var(--text-base);
    line-height: 1.3;
    text-align: right;
  }

  .shortcut-help kbd {
    min-width: 24px;
    padding: 3px 7px 4px;
    border: 1px solid var(--app-border-strong);
    border-bottom-color: var(--app-text-subtle);
    border-radius: 7px;
    background: var(--app-bg);
    color: var(--app-text-strong);
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    font-weight: 700;
    line-height: 1;
    text-align: center;
    box-shadow: inset 0 -1px 0 var(--app-overlay-border);
  }

  .shortcut-help__note {
    margin-top: 14px;
    color: var(--app-text-muted);
    font-size: var(--text-sm);
    line-height: 1.45;
  }
</style>
