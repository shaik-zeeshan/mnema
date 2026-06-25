<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { tick, type Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { isMainAppRoute, normalizeAppPathname } from "$lib/route-path";
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";
  import { closeCurrentWindow, isDedicatedSurfaceWindow, isQuickRecallWindow, openDebugWindow, openSettings } from "$lib/surface-windows";
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
  import type { AppearanceSetting } from "$lib/types";
  import {
    appNotifications,
    clearAppNotification,
    clearAppNotifications,
    initAppNotifications,
    type AppNotification,
  } from "$lib/notifications.svelte";
  import {
    GLOBAL_SHORTCUTS,
    getEffectiveGlobalShortcut,
    getGlobalShortcutAction,
    type GlobalShortcutId,
  } from "$lib/global-shortcuts";
  import { initKeyboardBindings } from "$lib/keyboard-bindings.svelte";
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
  const isSettings = $derived(normalizedPathname.startsWith("/settings"));
  // Settings renders inside the Main window as the `/settings` route. The Main
  // titlebar (record controls, source pills, surface toggle, gear) stays visible
  // on Settings too — it is the Main window's persistent top nav — and Settings
  // renders its own sidebar shell in the content area below it. Native traffic
  // lights stay (overlay titlebar), reserved for by the titlebar's left inset.
  const isSettingsRoute = $derived(normalizedPathname === "/settings");
  const isDebug = $derived(normalizedPathname.startsWith("/debug"));
  const isPanelSurface = isQuickRecallWindow();
  // The Main window now hosts two top-level Surfaces — Timeline (`/`) and
  // Insights (`/insights`). The shared main titlebar (record controls, source
  // pills, settings, the Timeline⇄Insights surface toggle) renders on both.
  const isMainSurfaceRoute = $derived(isMainRoute || isInsightsRoute);
  const showMainTitlebar = $derived((isMainSurfaceRoute || isSettingsRoute) && !isPanelSurface);
  const showDedicatedTitlebar = isDedicatedSurfaceWindow();
  const transparentSurface = $derived(showDedicatedTitlebar || isPanelSurface);
  const isMainWindow = $derived(!showDedicatedTitlebar && !isPanelSurface);
  const canShowShortcutsHelp = $derived(isMainWindow && isMainRoute);
  let windowPlatform = $state<KeyboardPlatform>(detectKeyboardPlatform());
  let notificationsOpen = $state(false);
  let notificationsOpenedByKeyboard = false;
  let notificationsButtonEl = $state<HTMLButtonElement | null>(null);
  let notificationsPopoverEl = $state<HTMLDivElement | null>(null);
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
      goto("/", { replaceState: true });
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

  $effect(() => {
    if (!hasNotifications) notificationsOpen = false;
  });

  async function runNotificationAction(notification: AppNotification): Promise<void> {
    if (notification.action?.type === "open_settings_tab") {
      await openSettings(notification.action.tab);
      await clearAppNotification(notification.id);
      notificationsOpen = false;
    }
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

  // ── Recording status mirrored from the shared capture-controls seam ────
  const isCapturing = $derived(captureControls.running);
  const captureLoadingStart = $derived(captureControls.loadingStart);
  const captureLoadingStop = $derived(captureControls.loadingStop);
  const captureLoadingPause = $derived(captureControls.loadingPause);
  const captureLoadingSettings = $derived(captureControls.loadingSettings);
  const captureStatusLabel = $derived(captureControls.statusLabel);
  const captureStatusModifier = $derived(captureControls.statusModifier);

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
        ? "recording"
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
      ? `${lane.label}: enabled — click to skip on next recording`
      : `${lane.label}: disabled — click to include in next recording`;
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
          isCapturing ? "Stop recording" : "Start recording",
        ),
      );
    }

    if (isCapturing) {
      rows.push(
        shortcutWithLabel(
          getEffectiveGlobalShortcut("pauseResumeRecording"),
          captureControls.isUserPaused ? "Resume recording" : "Pause recording",
        ),
      );
    }

    rows.push(getEffectiveGlobalShortcut("toggleMainWindow"));
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
    if (captureLoadingStart || captureLoadingStop || !isCapturing) return;
    await stopCapture();
    if (!captureControls.isRunning) {
      await startCapture();
    }
  }

  // ── Main surface toggle (Timeline ⇄ Insights) ─────────────────────────
  // "dashboard" is retired: the Main window hosts two switchable Surfaces.
  // The active segment reflects the current route (the static `/index.html`
  // production entry normalizes to `/` = Timeline).
  function goToSurface(surface: "timeline" | "insights"): void {
    const target = surface === "insights" ? "/insights" : "/";
    if (normalizeAppPathname($page.url.pathname) === target) return;
    void goto(target);
  }

  function openNotifications(openedByKeyboard = false): void {
    if (!hasNotifications) return;
    notificationsOpenedByKeyboard = openedByKeyboard;
    notificationsOpen = true;
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
        notificationsButtonEl?.focus({ preventScroll: true });
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
        <span
          class="titlebar__status titlebar__status--{captureStatusModifier}"
          aria-live="polite"
          title="Recording status"
        >
          <span class="titlebar__status-dot" aria-hidden="true"></span>
          <span class="titlebar__status-label">{captureStatusLabel}</span>
        </span>
        {#if isCapturing}
          <button
            type="button"
            class="titlebar__record titlebar__record--pause"
            class:titlebar__record--resume={captureControls.isUserPaused}
            onclick={captureControls.isUserPaused ? resumeCapture : pauseCapture}
            disabled={captureLoadingPause}
            title={captureControls.isUserPaused ? "Resume recording" : "Pause recording"}
            aria-label={captureControls.isUserPaused ? "Resume recording" : "Pause recording"}
          >
            <span>{captureLoadingPause ? "Working…" : captureControls.isUserPaused ? "Resume" : "Pause"}</span>
          </button>
          <button
            type="button"
            class="titlebar__record titlebar__record--stop"
            onclick={stopCapture}
            disabled={captureLoadingStop}
            title={`Stop recording (${shortcutDisplay("toggleRecording")})`}
            aria-label="Stop recording"
          >
            <span class="titlebar__record-glyph titlebar__record-glyph--square" aria-hidden="true"></span>
            <span>{captureLoadingStop ? "Stopping…" : "Stop"}</span>
          </button>
        {:else}
          <button
            type="button"
            class="titlebar__record titlebar__record--start"
            onclick={startCapture}
            disabled={captureLoadingStart || captureLoadingSettings}
            title={`Start recording (${shortcutDisplay("toggleRecording")})`}
            aria-label="Start recording"
          >
            <span class="titlebar__record-glyph" aria-hidden="true">●</span>
            <span>{captureLoadingStart ? "Starting…" : "Record"}</span>
          </button>
        {/if}
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
              title={liveTitleFor(lane, state)}
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
              title={`${selectTitleFor(lane, state)} (${shortcutDisplay(sourceShortcutIdFor(lane.key))})`}
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
            title={privacyVisualCaptureStatus.detail}
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
                title={privacyVisualCaptureStatus.detail}
                aria-label={privacyVisualCaptureStatus.detail}
                disabled={captureLoadingStart || captureLoadingStop}
                onclick={restartCaptureForPrivacyRecovery}
              >
                Restart
              </button>
            {/if}
          </span>
        {/if}
      {/if}
    </div>

    <!-- Inert centre area carries the drag region + the Timeline⇄Insights
         surface toggle + the (Timeline-only) centered search trigger. -->
    <div class="titlebar__drag" data-tauri-drag-region>
      <!-- Surface toggle — Main hosts Timeline + Insights; "dashboard" retired (#103). -->
      <div class="surface-toggle" role="tablist" aria-label="Main surface">
        <button
          type="button"
          role="tab"
          class:active={isMainRoute}
          aria-selected={isMainRoute}
          aria-current={isMainRoute ? "page" : undefined}
          onclick={() => goToSurface("timeline")}
        >
          Timeline
        </button>
        <button
          type="button"
          role="tab"
          class:active={isInsightsRoute}
          aria-selected={isInsightsRoute}
          aria-current={isInsightsRoute ? "page" : undefined}
          onclick={() => goToSurface("insights")}
        >
          Insights
        </button>
      </div>
    </div>

    <div class="titlebar__group titlebar__group--right">
      {#if showMainTitlebar}
        {#if hasNotifications}
          <div class="titlebar__notifications">
            <button
              bind:this={notificationsButtonEl}
              type="button"
              class="titlebar__settings titlebar__notifications-button"
              aria-label="Open notifications"
              aria-expanded={notificationsOpen}
              aria-controls="notification-popover"
              title="Notifications"
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
              <span class="titlebar__notification-dot" aria-hidden="true">{notificationCount}</span>
            </button>
            {#if notificationsOpen}
              <div
                id="notification-popover"
                class="notification-popover"
                role="dialog"
                aria-label="Notifications"
                bind:this={notificationsPopoverEl}
              >
                <div class="notification-popover__head">
                  <span>Notifications</span>
                  <button type="button" class="notification-popover__clear" onclick={() => void clearAppNotifications()}>
                    Clear all
                  </button>
                </div>
                <div class="notification-popover__list">
                  {#each appNotifications.items as notification (notification.id)}
                    <div class="notification-item notification-item--{notification.severity}">
                      <div class="notification-item__body">
                        <span class="notification-item__title">{notification.title}</span>
                        <span class="notification-item__message">{notification.message}</span>
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
                        x
                      </button>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
        {/if}
        <button
          type="button"
          class="titlebar__settings"
          class:active={isSettings}
          aria-label="Open settings"
          aria-current={isSettings ? "page" : undefined}
          title={`Settings (${shortcutDisplay("openSettings")})`}
          onclick={() => void openSettings()}
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
        <ThemeModeControl
          bind:value={chromeAppearance}
          compact
          disabled={!theme.loaded || savingChromeAppearance}
          onChange={setChromeAppearance}
        />
        {#if devEnabled}
          <button
            type="button"
            class="titlebar__settings"
            aria-label="Open debug"
            title={`Debug (${shortcutDisplay("openDebug")})`}
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
        title="Close"
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
          >×</button>
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
     `:global` styled content — can consume them. Two themes are defined:
     dark (default, mirrors the prior hard-coded chrome exactly so this
     slice is a no-op on first paint) and a bright, high-legibility light
     theme. The active set is selected by `data-theme` on `<html>`, written
     by `$lib/theme.svelte`. We deliberately avoid `prefers-color-scheme`
     media queries here because the runtime owns the decision (the user
     can pin `light`/`dark` explicitly via `appearance`). */
  :global(:root) {
    /* Dark theme — current chrome values, lifted verbatim. */
    --app-bg: #0c0c0e;
    --app-fg: #e2e2e8;
    --app-fg-muted: #8a8aaa;
    --app-fg-subtle: #45455a;

    --app-titlebar-bg: #08080c;
    --app-titlebar-border: #15151f;
    --app-titlebar-title: #45455a;

    --app-status-bg: #0a0a10;
    --app-status-border: #161624;
    --app-status-fg: #555574;
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

    /* Surface / control tokens shared by the dashboard, settings, and the
       shared bits-ui-backed controls (Switch, Select, RadioGroup, Slider).
       Keeping these centralized means each component declares the dark
       palette once via these tokens and the light theme below flips them
       in one place — no per-component palette duplication. */
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
    --app-text-muted: #7a7a9a;
    --app-text-subtle: #44445a;
    --app-text-faint: #33334a;
    --app-accent: #3dffa0;
    --app-accent-strong: #2a8a60;
    --app-accent-bg: #0d1f15;
    --app-accent-border: #1a4a30;
    --app-accent-glow: rgba(61, 255, 160, 0.18);

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

    --app-ocr-box: rgba(120, 220, 160, 0.45);
    --app-ocr-box-hover: rgba(120, 220, 160, 0.95);
    --app-ocr-box-fill: rgba(120, 220, 160, 0.10);
    --app-ocr-chip-bg: rgba(8, 14, 10, 0.96);
    --app-ocr-chip-text: #eaffef;
    --app-ocr-chip-border: rgba(120, 220, 160, 0.6);
    --app-ocr-hover-shadow: rgba(0, 0, 0, 0.45);
    --app-ocr-hover-inset: rgba(255, 255, 255, 0.04);
    --app-ocr-chip-text-shadow: none;

    /* Insights chart tokens (dark). Grayscale "free tier" ramp, the engine
       category palette, and focus heat — consumed by the SVG chart primitives
       in `$lib/insights/charts/`. Flipping `data-theme` reskins them via the
       light overrides below. Values mirror docs/user-context/mockups/tokens.css. */
    --chart-grey-1: #2c2c3a;
    --chart-grey-2: #3e3e50;
    --chart-grey-3: #565669;
    --chart-grey-4: #757589;
    --chart-grey-5: #9a9ab0;

    --cat-creating: #3dffa0;
    --cat-communication: #c0b0ff;
    --cat-meetings: #ff9fd0;
    --cat-research: #60b0ff;
    --cat-learning: #4fd8c8;
    --cat-organizing: #b0c080;
    --cat-personal: #d6a14a;
    --cat-entertainment: #ff6b7a;

    --focus-deep: #3dffa0;
    --focus-mid: #d6a14a;
    --focus-distracted: #ff6b7a;
  }

  /* Light theme — bright, neutral, high contrast. The accent stays in the
     red family to preserve recording-status semantics; backgrounds and
     borders flip to warm-cool greys so legibility on a 13px monospace body
     remains strong. */
  :global([data-theme="light"]) {
    --app-bg: #f6f6f4;
    --app-fg: #14141a;
    --app-fg-muted: #5a5a6a;
    --app-fg-subtle: #8a8a9a;

    --app-titlebar-bg: #ececea;
    --app-titlebar-border: #d4d4d2;
    --app-titlebar-title: #9a9aa8;

    --app-status-bg: #ffffff;
    --app-status-border: #d8d8dc;
    --app-status-fg: #5a5a6a;
    --app-status-dot: #c4c4cc;

    --app-status-running-fg: #c81d2e;
    --app-status-running-border: #f1b9bf;
    --app-status-running-dot: #d62236;
    --app-status-running-dot-glow: rgba(214, 34, 54, 0.22);

    --app-status-paused-fg: #8a5a10;
    --app-status-paused-border: #ecd9b0;
    --app-status-paused-dot: #c08018;
    --app-status-paused-dot-glow: rgba(192, 128, 24, 0.22);

    --app-record-start-bg: #ffffff;
    --app-record-start-fg: #c81d2e;
    --app-record-start-border: #ecbcc2;
    --app-record-start-bg-hover: #fff0f2;
    --app-record-start-fg-hover: #a01624;
    --app-record-start-border-hover: #d68c95;

    --app-record-stop-bg: #c81d2e;
    --app-record-stop-fg: #ffffff;
    --app-record-stop-border: #a01624;
    --app-record-stop-bg-hover: #a01624;
    --app-record-stop-border-hover: #7a1019;

    --app-record-glyph-start: #c81d2e;
    --app-record-glyph-stop: #ffffff;

    --app-icon-fg: #5a5a6a;
    --app-icon-fg-hover: #14141a;
    --app-icon-bg-hover: #e2e2e0;
    --app-icon-border-hover: #c8c8c6;
    --app-icon-bg-active: #dcdcda;
    --app-icon-border-active: #b8b8b6;

    /* Light surface palette mirrors the structural roles of the dark
       palette so any consumer styled against the tokens flips coherently.
       Greys are warmed slightly to match the `#f6f6f4` page background; the
       accent stays in the green family (matching dashboard "OK" and the
       primary save button) but darkens for legibility on white. */
    --app-surface: #ffffff;
    --app-surface-subtle: #f6f6f4;
    --app-surface-raised: #fbfbfa;
    --app-surface-hover: #eeeeec;
    --app-surface-active: #e8f1ea;
    --app-border: #d8d8d4;
    --app-border-strong: #c4c4c0;
    --app-border-hover: #a4a4a0;
    --app-text-strong: #14141a;
    --app-text: #2a2a32;
    --app-text-muted: #5a5a6a;
    --app-text-subtle: #7a7a86;
    --app-text-faint: #9a9aa4;
    --app-accent: #1f7a4a;
    --app-accent-strong: #155a36;
    --app-accent-bg: #e6f4ec;
    --app-accent-border: #9bd3b4;
    --app-accent-glow: rgba(31, 122, 74, 0.16);

    --app-warn: #9a5a12;
    --app-warn-strong: #7f4300;
    --app-warn-bg: #fff1df;
    --app-warn-border: #dfbc8a;

    --app-danger: #c43a48;
    --app-danger-strong: #b42332;
    --app-danger-bg: #fff0f2;
    --app-danger-bg-soft: #fff6f7;
    --app-danger-border: #e4b6be;
    --app-danger-text: #d24a59;

    --app-info: #2b78c5;
    --app-info-strong: #225fa3;
    --app-info-bg: #eef5ff;
    --app-info-border: #bdd3ef;

    --app-neutral-bg: #f2f3f6;
    --app-neutral-border: #d5d7de;
    --app-neutral-text: #636a79;

    --app-source-screen: #6f5ed1;
    --app-source-screen-strong: #5949b8;
    --app-source-screen-bg: #f1edff;
    --app-source-screen-border: #cdc3f2;

    --app-source-mic: #2f8e59;
    --app-source-mic-strong: #287a4a;
    --app-source-mic-bg: #e8f5ec;
    --app-source-mic-border: #afd8bf;

    --app-source-sysaudio: #8b7a2c;
    --app-source-sysaudio-strong: #786821;
    --app-source-sysaudio-bg: #faf4df;
    --app-source-sysaudio-border: #dbc98a;

    --app-overlay-bg: rgba(255, 255, 255, 0.78);
    --app-overlay-bg-strong: rgba(255, 255, 255, 0.86);
    --app-overlay-border: rgba(20, 24, 32, 0.12);

    --app-ocr-box: rgba(31, 122, 74, 0.42);
    --app-ocr-box-hover: rgba(31, 122, 74, 0.88);
    --app-ocr-box-fill: transparent;
    --app-ocr-chip-bg: rgba(255, 255, 255, 0.92);
    --app-ocr-chip-text: #155a36;
    --app-ocr-chip-border: rgba(31, 122, 74, 0.24);
    --app-ocr-hover-shadow: rgba(21, 28, 38, 0.18);
    --app-ocr-hover-inset: transparent;
    --app-ocr-chip-text-shadow: none;

    /* Insights chart tokens (light). The category palette is darkened for
       legibility on white surfaces; the grayscale ramp inverts (light → dark)
       so bars read on the bright background. Mirrors the light-theme values in
       docs/user-context/mockups/tokens.css. */
    --chart-grey-1: #d8d8de;
    --chart-grey-2: #b6b6c0;
    --chart-grey-3: #909099;
    --chart-grey-4: #6a6a74;
    --chart-grey-5: #46464e;

    --cat-creating: #1f7a4a;
    --cat-communication: #5949b8;
    --cat-meetings: #c2407f;
    --cat-research: #2b78c5;
    --cat-learning: #1f8579;
    --cat-organizing: #6f7a2e;
    --cat-personal: #9a5a12;
    --cat-entertainment: #c43a48;

    --focus-deep: #1f7a4a;
    --focus-mid: #9a5a12;
    --focus-distracted: #c43a48;
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
    font-family: "Berkeley Mono", "TX-02", "Monaspace Neon", ui-monospace,
      "Cascadia Code", "Fira Code", monospace;
    font-size: 13px;
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
    font-size: 9px;
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

  .titlebar__group {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }

  .titlebar__drag {
    flex: 1 1 auto;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    overflow: hidden;
    cursor: default;
  }

  /* ── Surface toggle (Timeline ⇄ Insights) ─────────────────────
     The canonical segmented control from the Insights mockups (app.css
     `.surface-toggle`), token-driven. The active segment is signalled by an
     accent fill alone so the segments stay even-width. Shared visual contract
     with the Insights sub-nav switcher. */
  .surface-toggle {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .surface-toggle button {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 13px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .surface-toggle button:hover {
    color: var(--app-text-strong);
  }
  .surface-toggle button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  /* ── Recording status indicator ───────────────────────────── */
  .titlebar__status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    background: var(--app-status-bg);
    border: 1px solid var(--app-status-border);
    border-radius: 4px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-status-fg);
    font-variant-numeric: tabular-nums;
  }

  .titlebar__status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-status-dot);
    flex: 0 0 auto;
  }

  .titlebar__status--running {
    color: var(--app-status-running-fg);
    border-color: var(--app-status-running-border);
  }
  .titlebar__status--running .titlebar__status-dot {
    background: var(--app-status-running-dot);
    box-shadow: 0 0 0 3px var(--app-status-running-dot-glow);
    animation: titlebar-pulse 1.4s ease-in-out infinite;
  }
  .titlebar__status--paused {
    color: var(--app-status-paused-fg);
    border-color: var(--app-status-paused-border);
  }
  .titlebar__status--paused .titlebar__status-dot {
    background: var(--app-status-paused-dot);
    box-shadow: 0 0 0 3px var(--app-status-paused-dot-glow);
  }

  @keyframes titlebar-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
  }

  /* ── Per-source recording pills ───────────────────────────────
     One pill per requested capture source (screen / microphone /
     system audio), rendered after the Record/Stop button. Each pill
     pairs the source's icon with a status icon: a pulsing red dot
     while live, pause bars while inactivity-paused, or a hollow ring
     while the source is still spinning up. Sources not requested for
     the current session aren't rendered. The pill chrome mirrors
     `.titlebar__status` so the title bar stays visually coherent. */
  .titlebar__source {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 7px;
    height: 22px;
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
    opacity: 0.6;
  }
  .titlebar__source--toggle.titlebar__source--selected {
    color: var(--app-text-strong);
    border-color: var(--app-border-strong);
    background: var(--app-surface-raised);
  }
  .titlebar__source--toggle.titlebar__source--unselected {
    color: var(--app-fg-subtle);
    border-color: var(--app-status-border);
    background: var(--app-status-bg);
    opacity: 0.7;
  }
  .titlebar__source--toggle:not(:disabled):hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
    opacity: 1;
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
    font-size: 9px;
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
    font-size: 8px;
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
    opacity: 0.55;
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

  /* ── Record / Stop button ─────────────────────────────────── */
  .titlebar__record {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 4px;
    border: 1px solid transparent;
    font-family: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s, color 0.12s;
  }
  .titlebar__record:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .titlebar__record--pause {
    background: var(--app-surface-raised);
    color: var(--app-text);
    border-color: var(--app-border-strong);
  }
  .titlebar__record--pause:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .titlebar__record--resume {
    background: var(--app-warn-bg);
    color: var(--app-warn);
    border-color: var(--app-warn-border);
  }
  .titlebar__record--resume:not(:disabled):hover {
    background: color-mix(in srgb, var(--app-warn-bg) 74%, var(--app-warn) 26%);
    color: var(--app-text-strong);
    border-color: var(--app-warn-strong);
  }
  .titlebar__record--start {
    background: var(--app-record-start-bg);
    color: var(--app-record-start-fg);
    border-color: var(--app-record-start-border);
  }
  .titlebar__record--start:not(:disabled):hover {
    background: var(--app-record-start-bg-hover);
    color: var(--app-record-start-fg-hover);
    border-color: var(--app-record-start-border-hover);
  }
  .titlebar__record--stop {
    background: var(--app-record-stop-bg);
    color: var(--app-record-stop-fg);
    border-color: var(--app-record-stop-border);
  }
  .titlebar__record--stop:not(:disabled):hover {
    background: var(--app-record-stop-bg-hover);
    border-color: var(--app-record-stop-border-hover);
  }
  .titlebar__record-glyph {
    display: inline-block;
    width: 8px;
    height: 8px;
    line-height: 1;
    text-align: center;
    color: var(--app-record-glyph-start);
    font-size: 12px;
  }
  .titlebar__record--stop .titlebar__record-glyph {
    color: var(--app-record-glyph-stop);
  }
  .titlebar__record-glyph--square {
    background: currentColor;
    border-radius: 1px;
    width: 7px;
    height: 7px;
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
  .titlebar__settings-icon {
    display: block;
    flex: 0 0 auto;
  }
  .titlebar__notifications {
    position: relative;
    display: inline-flex;
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
    background: var(--app-warn);
    color: var(--app-bg);
    font-size: 8px;
    font-weight: 800;
    line-height: 12px;
    text-align: center;
  }
  .notification-popover {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 20;
    width: min(340px, calc(100vw - 24px));
    max-height: 360px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border);
    border-radius: 8px;
    box-shadow: 0 18px 42px rgba(0, 0, 0, 0.35);
  }
  .notification-popover__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--app-border);
    font-size: 11px;
    font-weight: 700;
    color: var(--app-text-strong);
  }
  .notification-popover__clear,
  .notification-item__clear {
    border: 0;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    font: inherit;
  }
  .notification-popover__clear {
    font-size: 10px;
    font-weight: 700;
  }
  .notification-popover__clear:hover,
  .notification-item__clear:hover {
    color: var(--app-text-strong);
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
    background: var(--app-danger-bg);
  }
  .notification-item__body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .notification-item__title {
    color: var(--app-text-strong);
    font-size: 11px;
    font-weight: 700;
    line-height: 1.2;
  }
  .notification-item__message {
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.35;
  }
  .notification-item__action {
    align-self: flex-start;
    margin-top: 4px;
    padding: 4px 7px;
    border-radius: 4px;
    border: 1px solid var(--app-border-strong);
    background: var(--app-surface);
    color: var(--app-text);
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .notification-item__action:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }
  .notification-item__clear {
    align-self: start;
    width: 20px;
    height: 20px;
    font-size: 16px;
    line-height: 18px;
  }
  .titlebar__settings-label {
    display: block;
    font-size: 9px;
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
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(10px);
  }

  .shortcut-help__panel {
    width: min(560px, 100%);
    max-height: min(680px, calc(100vh - 48px));
    overflow-y: auto;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface);
    color: var(--app-text);
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.42);
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
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    line-height: 1;
    margin-bottom: 6px;
    text-transform: uppercase;
  }

  .shortcut-help h2 {
    color: var(--app-text-strong);
    font-size: 18px;
    line-height: 1.15;
  }

  .shortcut-help__close {
    width: 30px;
    height: 30px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    cursor: pointer;
    font: inherit;
    font-size: 20px;
    line-height: 1;
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
    font-size: 10px;
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
    border-radius: 12px;
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
    font-size: 12px;
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
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 700;
    line-height: 1;
    text-align: center;
    box-shadow: inset 0 -1px 0 rgba(255, 255, 255, 0.04);
  }

  .shortcut-help__note {
    margin-top: 14px;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.45;
  }
</style>
