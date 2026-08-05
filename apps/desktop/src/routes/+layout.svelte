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
    toggleSourceSelected,
    type SourceKey,
  } from "$lib/capture-controls.svelte";
  import RecordPill from "$lib/components/RecordPill.svelte";
  import SurfaceSwitcher from "$lib/components/SurfaceSwitcher.svelte";
  import { getMainSurfaceSetting, surfaceRoute, type MainSurface } from "$lib/main-surface";
  import { initTheme } from "$lib/theme.svelte";
  import { theme, persistAppearance } from "$lib/theme.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
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
  import ToastCard from "$lib/components/Toast.svelte";
  import {
    clearToastArchive,
    dismissToast,
    removeArchivedToast,
    toastStore,
  } from "$lib/toast.svelte";
  import type { Toast } from "$lib/toast";
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
  let settingsButtonEl = $state<HTMLButtonElement | null>(null);
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

    // Default surface (`ui.main_surface`, frame 07): a cold main window that
    // opened at the root switches to Overview when that is the saved default.
    // Root-only on purpose — a deep link (/settings, an insights handoff, …)
    // must never be navigated away from. Harmless alongside the insights peek
    // below: the only route this can move to is `/insights`, which is also the
    // only route the peek moves to, so they can't fight.
    if (isMainWindow && normalizeAppPathname($page.url.pathname) === "/") {
      void getMainSurfaceSetting()
        .then((surface) => {
          if (
            !destroyed &&
            surface === "overview" &&
            normalizeAppPathname($page.url.pathname) === "/"
          ) {
            void goto("/insights", { replaceState: true });
          }
        })
        .catch(() => {
          // Best-effort: unknown setting opens on Timeline (the default).
        });
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
  // The bell archives every toast (frame 14): session-local toast records ride
  // the same popover + count as the backend-persisted notifications.
  const notificationCount = $derived(appNotifications.count + toastStore.archive.length);
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
    appNotifications.items.some((n) => n.severity === "error") ||
      toastStore.archive.some((t) => t.kind === "danger"),
  );
  const hasWarningNotification = $derived(
    appNotifications.items.some((n) => n.severity === "warning"),
  );
  // One merged, newest-first bell list: backend notifications + archived
  // toasts (frame 14 orders the bell by time, newest on top).
  type BellRow =
    | { key: string; at: number; source: "backend"; notification: AppNotification }
    | { key: string; at: number; source: "toast"; toast: Toast };
  const bellRows = $derived.by<BellRow[]>(() => {
    const rows: BellRow[] = [
      ...appNotifications.items.map((notification) => ({
        key: `n:${notification.id}`,
        at: notification.createdAtUnixMs,
        source: "backend" as const,
        notification,
      })),
      ...toastStore.archive.map((toast) => ({
        key: `t:${toast.id}`,
        at: toast.createdAtUnixMs,
        source: "toast" as const,
        toast,
      })),
    ];
    rows.sort((a, b) => b.at - a.at);
    return rows;
  });
  // Archived toast kinds map onto the popover's severity styling; success/info
  // both read as informational rows.
  function toastRowSeverity(toast: Toast): "error" | "info" {
    return toast.kind === "danger" ? "error" : "info";
  }
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

  // ── Recording status mirrored from the shared capture-controls seam ────
  // (The visual chrome itself lives in RecordPill.svelte; the layout keeps
  // only what the global keyboard shortcuts below need.)
  const isCapturing = $derived(captureControls.running);
  const captureLoadingStart = $derived(captureControls.loadingStart);
  const captureLoadingStop = $derived(captureControls.loadingStop);
  const captureLoadingPause = $derived(captureControls.loadingPause);
  const captureLoadingSettings = $derived(captureControls.loadingSettings);

  const canUseGlobalShortcuts = $derived(isMainWindow && isMainRoute);
  const canToggleSourcesByShortcut = $derived(
    canUseGlobalShortcuts && !isCapturing && !captureLoadingSettings,
  );
  const canToggleRecordingByShortcut = $derived(
    isCapturing ? !captureLoadingStop : !captureLoadingStart && !captureLoadingSettings,
  );

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
    rows.push(getEffectiveGlobalShortcut("toggleQuickRecall"));
    rows.push(getEffectiveGlobalShortcut("openSettings"));
    rows.push(getEffectiveGlobalShortcut("surfaceTimeline"));
    rows.push(getEffectiveGlobalShortcut("surfaceOverview"));

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

  async function toggleSourceShortcut(key: SourceKey): Promise<void> {
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

  // ── Main surface navigation (Timeline ⇄ Overview) ─────────────────────
  // "dashboard" is retired: the Main window hosts two switchable Surfaces.
  // The switcher control itself is `SurfaceSwitcher.svelte` (frame 07); this
  // helper backs the ⌘1/⌘2 shortcut actions and the gear toggle below.
  function goToSurface(surface: MainSurface): void {
    const target = surfaceRoute(surface);
    if (normalizeAppPathname($page.url.pathname) === target) return;
    void goto(target);
  }

  // The gear is a real toggle: opening Settings from a surface, then clicking
  // the gear again returns to the surface it was opened from (Timeline or
  // Overview) instead of being a no-op with no obvious exit.
  function onSettingsButtonClick(): void {
    if (isSettings) {
      const returnsToOverview = normalizeAppPathname(getLastMainSurface()).startsWith("/insights");
      goToSurface(returnsToOverview ? "overview" : "timeline");
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
      isSurfaceRoute: isMainSurfaceRoute || isSettingsRoute,
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

    if (action.type === "goToSurface") {
      goToSurface(action.surface);
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
  class:app-shell--under={isInsightsRoute}
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
    <!-- Inert centre area carries the drag region + the Timeline⇄Insights
         surface toggle + the Quick Recall (Search) door. -->
    <div class="titlebar__drag" data-tauri-drag-region>
      <!-- Surface switcher (frame 07) — Timeline ⌘1 | Overview ⌘2, with the
           right-click "Open Mnema on" default-surface menu. -->
      <SurfaceSwitcher />
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
                  <button
                    type="button"
                    class="notification-popover__clear"
                    onclick={() => {
                      clearToastArchive();
                      void clearAppNotifications();
                    }}
                  >
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
                {#each bellRows as row (row.key)}
                  {#if row.source === "backend"}
                    {@const notification = row.notification}
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
                  {:else}
                    {@const toast = row.toast}
                    <div class="notification-item notification-item--{toastRowSeverity(toast)}">
                      <div class="notification-item__body">
                        <span class="notification-item__title">
                          {toast.message}{#if toast.count > 1}&nbsp;<span class="notification-item__count">×{toast.count}</span>{/if}
                        </span>
                        {#if toast.detail}
                          <span class="notification-item__message">{toast.detail}</span>
                        {/if}
                        <time
                          class="notification-item__time"
                          datetime={new Date(toast.createdAtUnixMs).toISOString()}
                          use:tip={formatNotificationTimestamp(toast.createdAtUnixMs)}
                        >{formatNotificationAge(toast.createdAtUnixMs)}</time>
                      </div>
                      <button
                        type="button"
                        class="notification-item__clear"
                        aria-label="Clear notification"
                        onclick={() => removeArchivedToast(toast.id)}
                      >
                        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
                          <path d="M2.5 2.5 9.5 9.5" />
                          <path d="M9.5 2.5 2.5 9.5" />
                        </svg>
                      </button>
                    </div>
                  {/if}
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
      <!-- Recording chrome: the state pill (design frame 11), rightmost. -->
      <RecordPill />
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

  <!-- Toast stack (design frame 14): fixed bottom-right, overlays everything,
       never reflows content. Newest toast sits nearest the corner; overflow
       beyond three collapses into a "+N more in the bell" chip that opens the
       bell archive. -->
  {#if toastStore.visible.length > 0}
    <div class="toast-stack app-toast-stack">
      {#if toastStore.overflow > 0}
        <button
          type="button"
          class="toast toast--more"
          onclick={() => openNotifications()}
        >+{toastStore.overflow} more in the bell</button>
      {/if}
      {#each toastStore.visible as toast (toast.id)}
        <ToastCard {toast} onDismiss={dismissToast} />
      {/each}
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
    /* Material title bar over scroll-under content (Overview only): the
       titlebar colour at ~76% so backdrop-filter samples the app's own
       scrolled content through it (redesign frame 04; materials are CSS). */
    --app-mat-toolbar: rgba(8, 8, 12, 0.76);
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

    /* Recording red is a STATE, never an error (system.css §1) — the state
       pill, capture indicator, and the timeline's active tick wear these. */
    --app-record: #ff3148;
    --app-record-fg: #ff8a96;
    --app-record-bg: #1a0f12;
    --app-record-border: #3a1820;

    /* Capture-source identity tints (system.css §1; popover source glyphs). */
    --app-src-screen: #c0b0ff;
    --app-src-mic: #80d0a8;
    --app-src-sys: #b0c080;

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
    /* Dark ladder retuned 2026-08-04 (docs/redesign/system.css §1): steps
       ~11 sRGB units per level so regions separate by tone. */
    --app-surface: #17171f;
    --app-surface-subtle: #1c1c26;
    --app-surface-raised: #22222e;
    --app-surface-hover: #2e2e40;
    --app-surface-active: #282836;
    --app-border: #1e1e2e;
    --app-border-strong: #2a2a3a;
    --app-border-hover: #3a3a5a;
    --app-text-strong: #e2e2e8;
    --app-text: #c0c0d0;
    /* Secondary conveyed text — brightened to sit comfortably above the AA
       4.5:1 floor on the dark surface (#9696ae ≈ 6.6:1, was #7a7a9a ≈ 4.6:1). */
    --app-text-muted: #9696ae;
    /* Tertiary conveyed text / structural labels — was #44445a (~2:1, FAIL);
       #7e7e98 ≈ 4.9:1 clears AA while staying clearly dimmer than muted. */
    --app-text-subtle: #7e7e98;
    /* Placeholder / decorative ONLY (intentionally sub-AA). Never use for text
       a user must read. */
    --app-text-faint: #33334a;
    --app-accent: #3dffa0;
    --app-accent-strong: #2a8a60;
    --app-accent-bg: #0d1f15;
    --app-accent-border: #1a4a30;
    --app-accent-glow: rgba(61, 255, 160, 0.18);
    /* Dark ink for text placed ON the bright-green accent fill — stays dark
       in both modes because the accent fill it sits on is bright in both. */
    --app-accent-contrast: #07120c;

    /* Font stacks (docs/redesign/system.css §2). Sans is the product voice
       and the default; mono is the machine voice, opt-in via `.is-mono`. */
    --app-font-sans: "Hanken Grotesk", -apple-system, BlinkMacSystemFont,
      "SF Pro Text", "Segoe UI Variable", system-ui, sans-serif;
    --app-font-mono: "Spline Sans Mono", "Berkeley Mono", ui-monospace,
      "SF Mono", Menlo, monospace;

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
    --app-shadow-popover: 0 8px 24px rgba(0, 0, 0, 0.22);

    /* ── Type ramp (docs/redesign/system.css §2, mode-independent) ──
       Six roles, each with one job: a size is not a choice, it is a
       consequence of what the text IS. label=machine labels · meta=secondary
       metadata · ui=THE DEFAULT · read=prose only · title=screen/section
       titles · display=at most one per screen (hero number, readout clock). */
    --t-label: 10px;
    --lh-label: 1.4;
    --ls-label: 0.02em;
    --t-meta: 11px;
    --lh-meta: 1.35;
    --ls-meta: 0.01em;
    --t-ui: 13px;
    --lh-ui: 1.25;
    --ls-ui: -0.006em;
    --t-read: 14px;
    --lh-read: 1.55;
    --ls-read: -0.008em;
    --t-title: 17px;
    --lh-title: 1.3;
    --ls-title: -0.016em;
    --t-display: 22px;
    --lh-display: 1.2;
    --ls-display: -0.02em;

    /* Three weights: regular, medium, semibold. No 300, no 700. */
    --w-regular: 400;
    --w-medium: 510;
    --w-semi: 590;

    /* ── Spacing ramp + named layout constants (system.css §3) ── */
    --s-1: 1px;
    --s-2: 2px;
    --s-3: 3px;
    --s-4: 4px;
    --s-6: 6px;
    --s-8: 8px;
    --s-12: 12px;
    --s-16: 16px;
    --s-20: 20px;
    --s-24: 24px;
    --s-32: 32px;
    --s-40: 40px;
    --s-48: 48px;
    --gap-inline: var(--s-6);
    --gap-label: var(--s-4);
    --gap-row: var(--s-8);
    --gap-group: var(--s-16);
    --gap-section: var(--s-24);
    --pad-window: var(--s-16);
    --pad-control: var(--s-8);
    --pad-panel: var(--s-16);
    --grid-inset: var(--s-16);
    --grid-gutter: var(--s-16);

    /* ── Control metrics, radii, elevation, motion (system.css §4) ── */
    --h-sm: 24px;
    --h-md: 28px;
    --h-lg: 32px;
    --h-row: 28px;
    --h-titlebar: 38px;
    --hit-min: 28px;
    --h-record: 44px;
    --o-badge: 20px;
    --o-icon-sm: 14px;
    --o-icon: 20px;
    --o-icon-lg: 24px;
    --o-setting-row: 40px;
    --o-row-prose: 40px;
    --r-sm: 4px;
    --r-md: 6px;
    --r-lg: 8px;
    --r-xl: 12px;
    --r-pill: 999px;
    --shadow-popover: 0 8px 24px rgba(0, 0, 0, 0.32);
    --shadow-modal: 0 24px 64px rgba(0, 0, 0, 0.48);
    --ring: 0 0 0 2px var(--app-accent-glow);
    --ring-danger: 0 0 0 2px
      color-mix(in srgb, var(--app-danger) 30%, transparent);
    --opacity-disabled: 0.4;
    --opacity-busy: 0.6;
    --dur-quick: 100ms;
    --dur-regular: 250ms;
    --dur-out: 150ms;
    --dur-in: 0ms;
    --ease: cubic-bezier(0.4, 0, 0.2, 1);
    --ease-out: cubic-bezier(0, 0, 0.2, 1);

    /* ── Hairline (system.css §5; halved on retina below) ── */
    --hairline: 1px;

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

    /* Recessed inner shadow for form-control insets (Input/Select/Combobox/
       Stepper). Softens in the light theme below so near-white fields don't
       carry a hard 25%-black inner shadow. */
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

    /* Insights chart tokens (dark). Grayscale "free tier" ramp, the engine
       category palette, and focus heat — consumed by the SVG chart primitives
       in `$lib/insights/charts/`. Flipping `data-theme` reskins them via the
       light overrides below. Values mirror docs/user-context/mockups/tokens.css. */
    --chart-grey-1: #2c2c3a;
    --chart-grey-2: #3e3e50;
    --chart-grey-3: #565669;
    --chart-grey-4: #757589;
    --chart-grey-5: #9a9ab0;

    /* "Creating" and "Entertainment" are rotated off the exact --app-accent /
       --app-danger values so a category color never reads as the semantic
       accent/error signal (grass-green vs the neon accent; coral-orange vs the
       rose danger red). */
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
    --app-mat-toolbar: rgba(236, 236, 234, 0.78);
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

    /* Recording-state tokens, light values (system.css §1). */
    --app-record: #d62236;
    --app-record-fg: #c81d2e;
    --app-record-bg: #ffffff;
    --app-record-border: #ecbcc2;

    --app-src-screen: #6f5ed1;
    --app-src-mic: #2f8e59;
    --app-src-sys: #8b7a2c;

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
    /* Secondary conveyed text — already ~6:1 on the light surface, unchanged. */
    --app-text-muted: #5a5a6a;
    /* Tertiary conveyed text / structural labels — was #7a7a86 (~3.8:1,
       borderline); #5e5e6a ≈ 6.2:1 clears AA. */
    --app-text-subtle: #5e5e6a;
    /* Placeholder / decorative ONLY (intentionally sub-AA). */
    --app-text-faint: #9a9aa4;
    --app-accent: #1f7a4a;
    --app-accent-strong: #155a36;
    --app-accent-bg: #e6f4ec;
    --app-accent-border: #9bd3b4;
    --app-accent-glow: rgba(31, 122, 74, 0.16);
    /* Light ink, because the light theme's accent is DARK (#1f7a4a), not bright.
       The dark theme's near-black works there because its accent (#3dffa0) is
       bright; reusing it here painted #07120c on #1f7a4a at 3.58:1, under the
       4.5:1 floor. White on #1f7a4a is 5.33:1. Matches the design of record
       (`docs/onboarding/mockups/revision-2.html`, `.app.light`). */
    --app-accent-contrast: #ffffff;

    /* Floating-layer shadows soften on light (system.css §4). */
    --shadow-popover: 0 8px 24px rgba(21, 28, 38, 0.14);
    --shadow-modal: 0 24px 64px rgba(21, 28, 38, 0.22);

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

    /* Softer inset recess on near-white fields (0.25 → 0.08). */
    --app-input-recess: rgba(0, 0, 0, 0.08);

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

    /* "Creating"/"Entertainment" rotated off the exact --app-accent /
       --app-danger values (see dark block) so categories never read semantic. */
    --cat-creating: #2f8a3f;
    --cat-communication: #5949b8;
    --cat-meetings: #c2407f;
    --cat-research: #2b78c5;
    --cat-learning: #1f8579;
    --cat-organizing: #6f7a2e;
    --cat-personal: #9a5a12;
    --cat-entertainment: #c2542b;

    --focus-deep: #1f7a4a;
    --focus-mid: #9a5a12;
    --focus-distracted: #c43a48;
  }

  @media (min-resolution: 2dppx) {
    :global(:root) {
      --hairline: 0.5px;
    }
  }

  /* ── Type role classes (system.css §2) ─────────────────────────
     Apply a role with one class. These are the only text classes that
     exist; `.is-mono`/`.is-num` are the machine-voice modifiers. */
  :global(.t-label) {
    font: var(--w-medium) var(--t-label) / var(--lh-label) var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  :global(.t-meta) {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-text-muted);
  }
  :global(.t-ui) {
    font: var(--w-regular) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text);
  }
  :global(.t-read) {
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
    max-width: 70ch;
  }
  :global(.t-title) {
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  :global(.t-display) {
    font:
      var(--w-semi) var(--t-display) / var(--lh-display) var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
  }
  :global(.is-mono) {
    font-family: var(--app-font-mono);
  }
  :global(.is-num) {
    font-variant-numeric: tabular-nums;
  }

  /* ── Shared primitives (system.css §6) ─────────────────────────
     The one `.btn` definition (call sites migrate in slice 3), the field/
     input pair, the toast stack, keycaps, and the recording state pill. */
  :global(.btn) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--gap-inline);
    height: var(--h-md);
    padding: 0 var(--pad-control);
    border: var(--hairline) solid transparent;
    border-radius: var(--r-md);
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    white-space: nowrap;
    background: var(--app-surface-raised);
    color: var(--app-text-strong);
    border-color: var(--app-border-strong);
    transition: background-color var(--dur-quick) var(--ease);
  }
  :global(.btn:hover) {
    background: var(--app-surface-hover);
  }
  :global(.btn:active) {
    background: var(--app-surface-active);
  }
  :global(.btn:focus-visible) {
    outline: none;
    box-shadow: var(--ring);
  }
  :global(.btn[disabled]),
  :global(.btn[aria-disabled="true"]) {
    opacity: var(--opacity-disabled);
    pointer-events: none;
  }
  :global(.btn[aria-busy="true"]) {
    opacity: var(--opacity-busy);
    cursor: progress;
  }
  :global(.btn--primary) {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
    border-color: transparent;
  }
  :global(.btn--primary:hover) {
    background: var(--app-accent);
    filter: brightness(1.08);
  }
  :global(.btn--primary:active) {
    filter: brightness(0.94);
  }
  :global(.btn--ghost) {
    background: transparent;
    border-color: transparent;
    color: var(--app-text-muted);
  }
  :global(.btn--ghost:hover) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  :global(.btn--danger) {
    background: var(--app-danger-bg);
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }
  :global(.btn--danger:focus-visible) {
    box-shadow: var(--ring-danger);
  }
  :global(.btn--sm) {
    height: var(--h-sm);
    padding: 0 var(--s-6);
  }
  :global(.btn--lg) {
    height: var(--h-lg);
    padding: 0 var(--s-12);
  }
  :global(.btn--icon) {
    width: var(--h-md);
    padding: 0;
  }
  :global(.btn--icon.btn--sm) {
    width: var(--h-sm);
  }

  :global(.field) {
    display: flex;
    flex-direction: column;
    gap: var(--gap-label);
  }
  :global(.input) {
    height: var(--h-md);
    padding: 0 var(--pad-control);
    border: var(--hairline) solid var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
  }
  :global(.input:focus-visible) {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: var(--ring);
  }
  :global(.input[aria-invalid="true"]) {
    border-color: var(--app-danger-border);
  }
  /* The helper row EXISTS AT REST and fills with the error message; an
     error must never insert a row. */
  :global(.field__help) {
    min-height: calc(var(--t-meta) * var(--lh-meta));
  }
  :global(.field__help--error) {
    color: var(--app-danger);
  }

  /* Toast: the ONE non-blocking error placement. Bottom-right, stacked
     max 3, overlays content, never reflows. Errors never auto-dismiss. */
  :global(.toast) {
    display: grid;
    gap: var(--gap-label);
    width: 344px;
    padding: var(--pad-panel);
    border-radius: var(--r-xl);
    background: var(--app-surface-raised);
    box-shadow: var(--shadow-popover);
  }
  :global(.toast-stack) {
    position: absolute;
    right: var(--pad-window);
    bottom: var(--pad-window);
    display: grid;
    gap: var(--gap-row);
    justify-items: end;
  }
  /* The app's one live stack host: window-fixed so it overlays every surface
     (content, popovers, dialogs) without reflowing anything. Below the
     tooltip layer (9999) so the dismiss button can still explain itself. */
  .app-toast-stack {
    position: fixed;
    z-index: 9000;
  }
  .toast--more {
    width: auto;
    padding: var(--s-6) var(--s-12);
    border: 0;
    cursor: pointer;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .toast--more:focus-visible {
    outline: none;
    box-shadow: var(--shadow-popover), var(--ring);
  }

  :global(.kbd) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 var(--s-4);
    border-radius: var(--r-sm);
    background: var(--app-surface-raised);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-muted);
  }
  :global(.kbd--mod) {
    min-width: 48px;
    justify-content: flex-start;
    padding-left: var(--s-6);
  }

  /* Recording state pill (FEEDBACK-3): one capsule — dot + elapsed + cost.
     Recording red is a state, never an error. */
  :global(.pill) {
    display: inline-flex;
    align-items: center;
    gap: var(--gap-inline);
    height: var(--h-sm);
    padding: 0 var(--s-8);
    border-radius: var(--r-pill);
    border: 0;
    background: var(--app-record-bg);
    box-shadow: 0 0 0 var(--hairline) var(--app-record-border);
  }
  :global(.pill__t) {
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
  }
  :global(.pill__gb) {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  :global(.pill__w) {
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-record-fg);
    white-space: nowrap;
  }
  :global(.pill--quiet) {
    background: var(--app-surface-hover);
    box-shadow: none;
  }
  :global(.pill--quiet .pill__w) {
    color: var(--app-text-muted);
  }
  :global(.pill--warn) {
    background: var(--app-warn-bg);
    box-shadow: 0 0 0 var(--hairline) var(--app-warn-border);
  }
  :global(.pill--warn .pill__w) {
    color: var(--app-warn);
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
    /* system.css §7: sans is the default voice; mono is opt-in via
       `.is-mono`. UI line-height 1.25 (macOS Body), never web's 1.5. */
    font-family: var(--app-font-sans);
    font-size: var(--t-ui);
    line-height: var(--lh-ui);
    letter-spacing: var(--ls-ui);
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

  /* system.css §7: one focus ring everywhere. Component-scoped
     :focus-visible rules keep winning on specificity where they exist. */
  :global(:focus-visible) {
    outline: none;
    box-shadow: var(--ring);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(*),
    :global(*::before),
    :global(*::after) {
      animation-duration: 1ms !important;
      transition-duration: 1ms !important;
    }
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
    font-size: var(--t-meta);
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
    /* Named container for the record pill's frame-11 degradation ladder
       (@container queries in RecordPill.svelte). Safe here: the bar's width
       is set by the window, never by its content. */
    container-type: inline-size;
    container-name: titlebar;
  }

  /* ── Scroll-under material chrome (Overview route only) ──────────────
     On `/insights` the title bar floats as an absolute material overlay:
     the bento scrolls edge-to-edge beneath it, sampled by the CSS
     backdrop-filter (frame 04; the frozen Timeline stays opaque). */
  .app-shell--under .titlebar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    background: var(--app-mat-toolbar);
    -webkit-backdrop-filter: blur(20px) saturate(1.4);
    backdrop-filter: blur(20px) saturate(1.4);
  }

  .app-shell--under .app-content {
    height: 100vh;
    height: 100dvh;
  }

  /* With the title bar out of flow, the (rare) license banner becomes the
     first in-flow element — keep it below the floating bar. */
  .app-shell--under :global(.license-banner) {
    margin-top: var(--app-titlebar-height);
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
    font-size: var(--t-label);
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

  /* ── Surface switcher ─────────────────────────────────────────
     The Timeline⇄Overview segmented control (frame 07) lives in
     `SurfaceSwitcher.svelte`, styles included. */

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
    font-size: var(--t-ui);
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
    font-size: var(--t-label);
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
       • the record pill (its own frame-11 ladder sheds text, never the dot)
       • the Timeline⇄Overview surface switcher (`SurfaceSwitcher.svelte`)
       • the settings gear (`.titlebar__settings`, sans `--help`)
       • notifications bell when present
     Combined with `.titlebar { overflow: hidden }`, the right group can never
     spill off-screen. (The dashboard body's own breakpoint lives in
     +page.svelte.) */

  /* The titlebar is control-dense; the WM can force widths well below the
     app's 640px minimum, so the row sheds progressively. Always-visible at
     every width: the record pill, the surface toggle, the settings gear, and
     notifications-when-present. Combined with `.titlebar { overflow: hidden }`,
     nothing can spill off-screen. */

  /* Compact ≤820px: drop the Search word + kbd to an icon-only button, hide the
     status text (the colored dot still conveys state), tighten the row gap. */
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

  /* Tight ≤600px: the surface switcher sheds its own kbd hints + padding via
     `@container titlebar` rules in SurfaceSwitcher.svelte (the pill sheds its
     text via the frame-11 ladder). */

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
    font-size: var(--t-label);
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
    font-size: var(--t-meta);
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
    font-size: var(--t-meta);
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
    font-size: var(--t-meta);
    font-weight: 700;
    line-height: 1.2;
  }
  .notification-item__message {
    color: var(--app-text-muted);
    font-size: var(--t-meta);
    line-height: 1.35;
  }
  /* Coalesced-repeat badge on archived toast rows ("×3"). */
  .notification-item__count {
    color: var(--app-text-subtle);
    font-weight: 400;
  }
  .notification-item__time {
    margin-top: 2px;
    color: var(--app-text-faint, var(--app-text-muted));
    font-size: var(--t-label);
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
    font-size: var(--t-meta);
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
    font-size: var(--t-label);
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
    font-size: var(--t-label);
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
    font-size: var(--t-label);
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
    font-size: var(--t-label);
    font-weight: 700;
    letter-spacing: 0.14em;
    line-height: 1;
    margin-bottom: 6px;
    text-transform: uppercase;
  }

  .shortcut-help h2 {
    color: var(--app-text-strong);
    font-size: var(--t-title);
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
    font-size: var(--t-label);
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
    font-size: var(--t-ui);
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
    font-size: var(--t-meta);
    font-weight: 700;
    line-height: 1;
    text-align: center;
    box-shadow: inset 0 -1px 0 var(--app-overlay-border);
  }

  .shortcut-help__note {
    margin-top: 14px;
    color: var(--app-text-muted);
    font-size: var(--t-meta);
    line-height: 1.45;
  }
</style>
