<script lang="ts">
  import "$lib/styles/kit.css";
  import "$lib/styles/shell.css";
  import { tip } from "$lib/components/tooltip";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { type Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { isMainAppRoute, normalizeAppPathname } from "$lib/route-path";
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";
  import { closeCurrentWindow, currentWindowLabel, isDedicatedSurfaceWindow, isQuickRecallWindow, openDebugWindow, openSettings } from "$lib/surface-windows";
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
  } from "$lib/capture-controls.svelte";
  import { initRenderIdle } from "$lib/render-idle.svelte";
  import { initTheme } from "$lib/theme.svelte";
  import { initAppNotifications } from "$lib/notifications.svelte";
  import { initLicenseStatus } from "$lib/licensing-store.svelte";
  import LicenseBanner from "$lib/LicenseBanner.svelte";
  import LicenseDeepLinkModal from "$lib/LicenseDeepLinkModal.svelte";
  import Titlebar from "$lib/shell/Titlebar.svelte";
  import StatusBar from "$lib/shell/StatusBar.svelte";
  import { getGlobalShortcutAction, type SourceShortcutKey } from "$lib/global-shortcuts";
  import { initKeyboardBindings } from "$lib/keyboard-bindings.svelte";
  import { askAiClock } from "$lib/askAiClock";
  import { detectKeyboardPlatform, isShortcutSuppressedTarget, type KeyboardPlatform } from "$lib/keyboard";
  import { keyboardHelp } from "$lib/keyboard-help.svelte";
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
  const isChatRoute = $derived(normalizedPathname.startsWith("/chat"));
  const isSettings = $derived(normalizedPathname.startsWith("/settings"));
  // Settings renders inside the Main window as the `/settings` route. The Main
  // titlebar stays visible on Settings too — it is the Main window's persistent
  // top nav — and Settings renders its own sidebar shell below it. Native
  // traffic lights stay (overlay titlebar), reserved for by the titlebar's inset.
  const isSettingsRoute = $derived(normalizedPathname === "/settings");
  const isDebug = $derived(normalizedPathname.startsWith("/debug"));
  const isUpdateWindow = currentWindowLabel() === "update";
  const isPanelSurface = isQuickRecallWindow();
  // The Main window hosts the top-level Surfaces — Timeline (`/`) and Insights
  // (`/insights`) — under the shared titlebar, with the recording status bar
  // at the bottom. Settings keeps the titlebar but has no status bar.
  const isMainSurfaceRoute = $derived(isMainRoute || isInsightsRoute || isChatRoute);
  const showMainTitlebar = $derived((isMainSurfaceRoute || isSettingsRoute) && !isPanelSurface);
  const showDedicatedTitlebar = isDedicatedSurfaceWindow();
  const transparentSurface = $derived(showDedicatedTitlebar || isPanelSurface);
  const isMainWindow = $derived(!showDedicatedTitlebar && !isPanelSurface);
  const showStatusBar = $derived(isMainSurfaceRoute && isMainWindow);
  const canShowShortcutsHelp = $derived(showStatusBar);
  const windowPlatform: KeyboardPlatform = detectKeyboardPlatform();

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
  // Render-idle gating for periodic DOM updaters + offscreen-window clamp.
  // Dedicated surface windows (quick recall, onboarding, debug) manage their
  // own placement, so only the main window self-clamps onto a live monitor.
  initRenderIdle({ clampWindow: !isDedicatedSurfaceWindow() && !isQuickRecallWindow() });
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

  // Bootstrap shared capture state once for the whole app — the status bar
  // and the record/stop shortcuts depend on it. The route pages
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

    // Quick Recall → Chat handoff (issue #111, ADR 0031): open the handed-off
    // conversation on the Chat surface (`/chat?c=` selects it; a cold window's
    // queue is drained by the Chat page on mount).
    listen<{ conversationId: string }>("insights_open_conversation", (event) => {
      if (isMainWindow) {
        void goto(`/chat?c=${encodeURIComponent(event.payload.conversationId)}`);
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
    // before the listener above attached. Peek the queue on mount and, if a
    // handoff is pending, route to `/chat`, whose on-mount drain consumes it.
    if (isMainWindow && !isChatRoute) {
      // Snapshot the route at peek time. The peek is async, so the user may
      // navigate during the drain window; if the route changed underneath us we
      // bail rather than yanking them back to /chat (self-healing, but the
      // bounce is jarring). Comparing the captured pathname keeps the re-route
      // intent tied to the route this peek was started for.
      const peekPathname = normalizeAppPathname($page.url.pathname);
      void invoke<boolean>("has_pending_insights_open_conversations")
        .then((pending) => {
          const routeUnchanged =
            normalizeAppPathname($page.url.pathname) === peekPathname;
          if (!destroyed && pending && routeUnchanged && !isChatRoute) {
            void goto("/chat");
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

  // Routes that want a centered, padded reading column (only `/debug`).
  const isNarrow = $derived(isDebug);

  // ── Global shortcuts (recording, sources, help) ─────────────────────────
  const isCapturing = $derived(captureControls.running);
  const canUseGlobalShortcuts = $derived(isMainWindow && isMainRoute);
  const canToggleSourcesByShortcut = $derived(
    canUseGlobalShortcuts && !isCapturing && !captureControls.loadingSettings,
  );
  const canToggleRecordingByShortcut = $derived(
    isCapturing
      ? !captureControls.loadingStop
      : !captureControls.loadingStart && !captureControls.loadingSettings,
  );

  async function toggleRecordingShortcut(): Promise<void> {
    if (!canToggleRecordingByShortcut) return;
    if (isCapturing) {
      await stopCapture();
      return;
    }
    await startCapture();
  }

  async function toggleSourceShortcut(key: SourceShortcutKey): Promise<void> {
    if (!canToggleSourcesByShortcut || sourceSelection.isSaving(key)) return;
    await toggleSourceSelected(key);
  }

  async function pauseResumeRecordingShortcut(): Promise<void> {
    const c = captureControls;
    if (!isCapturing || c.loadingPause || c.loadingStop || c.loadingStart) return;
    if (c.isUserPaused) {
      await resumeCapture();
    } else {
      await pauseCapture();
    }
  }

  function toggleShortcutsHelp(): void {
    if (!canShowShortcutsHelp) return;
    keyboardHelp.open = !keyboardHelp.open;
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
    // Keyed on the WINDOW, not the route, so it cannot drift again: a window
    // that gets the dedicated titlebar's Close chip should honour Escape too.
    // The update window is the one surface the app opens unsolicited, and it
    // shipped as the only dismissible window Escape could not close (⌘W is
    // inert as well — `decorations: false` strips NSWindowStyleMask::Closable).
    if (!showDedicatedTitlebar || (!isSettings && !isDebug && !isUpdateWindow)) return false;
    if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return false;
    if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
    if (isDedicatedWindowCloseSuppressedTarget(event.target)) return false;

    event.preventDefault();
    event.stopPropagation();
    void closeCurrentWindow();
    return true;
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
      shortcutsHelpOpen: keyboardHelp.open,
    }, windowPlatform);
    if (!action) return;

    event.preventDefault();

    if (action.type === "closeShortcutsHelp") {
      event.stopPropagation();
      keyboardHelp.open = false;
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
    if (keyboardHelp.open && !canShowShortcutsHelp) {
      keyboardHelp.open = false;
    }
  });
</script>

<svelte:window onkeydown={handleGlobalShortcutKeydown} />
<svelte:body class:dedicated-surface-window={transparentSurface} />

<div
  class="app-shell"
  class:app-shell--bounded={isMainSurfaceRoute || isSettingsRoute}
  class:app-shell--dedicated={showDedicatedTitlebar}
  class:app-shell--macos={showDedicatedTitlebar && windowPlatform === "macos"}
  class:app-shell--windows={showDedicatedTitlebar && windowPlatform === "windows"}
>
  {#if showMainTitlebar}
    <Titlebar platform={windowPlatform} {devEnabled} />
  {/if}

  {#if showDedicatedTitlebar}
  <header class="surface-titlebar">
    <div class="surface-titlebar__drag" data-tauri-drag-region></div>
    <div class="surface-titlebar__actions">
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
    <!-- App-wide banner (licensing, then low disk): one at a time, Main window
         only — Quick Recall / onboarding / dedicated surfaces stay clean. -->
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

  {#if showStatusBar}
    <StatusBar
      platform={windowPlatform}
      {devEnabled}
      canToggleRecording={canToggleRecordingByShortcut}
      canToggleSources={canToggleSourcesByShortcut}
    />
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

    /* Legacy alias of the kit's record face (`--font-mono`, lib/styles/kit.css)
       so the rules still reading `var(--app-font-mono)` keep working until each
       surface moves its sentences to sans. Mode-independent. */
    --app-font-mono: var(--font-mono);

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

    /* Type scale (mode-independent). 6 integer steps consumed app-wide. */
    --text-xs: 10px;
    --text-sm: 11px;
    --text-base: 12px;
    --text-md: 13px;
    --text-lg: 16px;
    --text-xl: 20px;

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
    font-family: var(--font-sans);
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

  .app-shell {
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
    padding: calc(var(--mx-titlebar-h) + 14px) 24px 64px;
    gap: 14px;
  }

  .app-content--dedicated.app-content--narrow {
    max-width: none;
    margin: 0;
    padding: 16px 20px 28px;
    gap: 14px;
  }

</style>
