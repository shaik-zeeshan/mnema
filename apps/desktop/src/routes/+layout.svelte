<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import type { Snippet } from "svelte";
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";
  import {
    bootstrapCaptureControls,
    captureControls,
    startCapture,
    stopCapture,
  } from "$lib/capture-controls.svelte";
  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  const isSettings = $derived($page.url.pathname.startsWith("/settings"));
  const isDebug = $derived($page.url.pathname.startsWith("/debug"));
  const isMenu = $derived($page.url.pathname.startsWith("/menu"));
  const showTimelineLink = $derived(isSettings || isDebug || isMenu);

  const devEnabled = $derived(developerOptions.value);
  const devLoaded = $derived(developerOptions.loaded);

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

  // Routes that want a centered, padded reading column.
  const isNarrow = $derived(isSettings || isDebug || isMenu);

  // ── Recording status mirrored from the shared capture-controls seam ────
  const isCapturing = $derived(captureControls.running);
  const captureLoadingStart = $derived(captureControls.loadingStart);
  const captureLoadingStop = $derived(captureControls.loadingStop);
  const captureLoadingSettings = $derived(captureControls.loadingSettings);
  const captureStatusLabel = $derived(captureControls.statusLabel);
  const captureStatusModifier = $derived(captureControls.statusModifier);
</script>

<div class="app-shell">
  <!--
    Custom desktop title bar. The Tauri window uses macOS's overlay title-bar
    style, so the OS still draws native traffic lights in the top-left; this
    bar reserves space for them via `.titlebar` left padding. The drag region
    is restricted to the inert filler area (`data-tauri-drag-region`); every
    interactive control sits outside that region so clicks/taps reach the
    button.
  -->
  <header class="titlebar">
    <div class="titlebar__group titlebar__group--left">
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
          class="titlebar__record titlebar__record--stop"
          onclick={stopCapture}
          disabled={captureLoadingStop}
          title="Stop recording"
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
          title="Start recording"
          aria-label="Start recording"
        >
          <span class="titlebar__record-glyph" aria-hidden="true">●</span>
          <span>{captureLoadingStart ? "Starting…" : "Record"}</span>
        </button>
      {/if}
    </div>

    <!-- Inert centre area carries the drag region. Title is decorative. -->
    <div class="titlebar__drag" data-tauri-drag-region>
      <span class="titlebar__title" data-tauri-drag-region>z</span>
    </div>

    <div class="titlebar__group titlebar__group--right">
      {#if showTimelineLink}
        <a
          class="titlebar__settings titlebar__settings--labelled"
          href="/"
          aria-label="Return to timeline"
          title="Back to timeline"
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
            <circle cx="12" cy="12" r="7" />
            <path d="M12 8.5v4.25l2.75 1.75" />
          </svg>
          <span class="titlebar__settings-label">Back to Timeline</span>
        </a>
      {/if}
      <a
        class="titlebar__settings"
        class:titlebar__settings--active={isMenu}
        href="/menu"
        aria-label="Open menu"
        title="Menu"
      >
        <!-- Inline gear icon. Larger and more recognisable than the prior
             ⚙ glyph, sized to match the 28px title-bar control footprint. -->
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
      </a>
    </div>
  </header>

  <main class="app-content" class:app-content--narrow={isNarrow}>
    {#if showChildren}
      {@render children()}
    {/if}
  </main>
</div>

<style>
  :global(*, *::before, *::after) {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  :global(html) {
    height: 100%;
    overscroll-behavior: none;
  }

  :global(body) {
    min-height: 100%;
    background-color: #0c0c0e;
    color: #e2e2e8;
    font-family: "Berkeley Mono", "TX-02", "Monaspace Neon", ui-monospace,
      "Cascadia Code", "Fira Code", monospace;
    font-size: 13px;
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
    overscroll-behavior: none;
  }

  :global(a) {
    text-decoration: none;
  }

  .app-shell {
    --app-titlebar-height: 36px;
    display: flex;
    flex-direction: column;
    min-height: 100vh;
    min-height: 100dvh;
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
    background: #08080c;
    border-bottom: 1px solid #15151f;
    user-select: none;
    -webkit-user-select: none;
    /* Sticky so the title bar stays visible when a route's main content
       scrolls vertically. Uses position: sticky rather than fixed so layout
       below it doesn't need to compensate with extra padding. */
    position: sticky;
    top: 0;
    z-index: 100;
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
    overflow: hidden;
    /* Ensure the drag area stays an explicit drop target for the cursor —
       even when empty, the height of the row catches mousedown for window
       drag. */
    cursor: default;
  }

  .titlebar__title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: #45455a;
  }

  /* ── Recording status indicator ───────────────────────────── */
  .titlebar__status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    background: #0a0a10;
    border: 1px solid #161624;
    border-radius: 4px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #555574;
    font-variant-numeric: tabular-nums;
  }

  .titlebar__status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #2a2a3a;
    flex: 0 0 auto;
  }

  .titlebar__status--running {
    color: #ff5d6c;
    border-color: #3a1820;
  }
  .titlebar__status--running .titlebar__status-dot {
    background: #ff3148;
    box-shadow: 0 0 0 3px rgba(255, 49, 72, 0.18);
    animation: titlebar-pulse 1.4s ease-in-out infinite;
  }
  .titlebar__status--paused {
    color: #d6a14a;
    border-color: #3a2818;
  }
  .titlebar__status--paused .titlebar__status-dot {
    background: #d6a14a;
    box-shadow: 0 0 0 3px rgba(214, 161, 74, 0.16);
  }

  @keyframes titlebar-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
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
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
  }
  .titlebar__record:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .titlebar__record--start {
    background: #1a0f12;
    color: #ff8a96;
    border-color: #3a1820;
  }
  .titlebar__record--start:not(:disabled):hover {
    background: #2a1218;
    color: #ffb0b9;
    border-color: #5a2030;
  }
  .titlebar__record--stop {
    background: #170d0f;
    color: #f0f0f5;
    border-color: #4a1c26;
  }
  .titlebar__record--stop:not(:disabled):hover {
    background: #2a1218;
    border-color: #6a2434;
  }
  .titlebar__record-glyph {
    display: inline-block;
    width: 8px;
    height: 8px;
    line-height: 1;
    text-align: center;
    color: #ff3148;
    font-size: 12px;
  }
  .titlebar__record--stop .titlebar__record-glyph {
    color: #ff8a96;
  }
  .titlebar__record-glyph--square {
    background: currentColor;
    border-radius: 1px;
    width: 7px;
    height: 7px;
  }

  /* ── Settings link ────────────────────────────────────────── */
  /* Sized noticeably larger than the previous ⚙ glyph so the "open
     menu/settings" affordance is unambiguous on every page. */
  .titlebar__settings {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    width: 28px;
    height: 28px;
    border-radius: 4px;
    color: #8a8aaa;
    border: 1px solid transparent;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .titlebar__settings--labelled {
    gap: 6px;
    width: auto;
    padding: 0 12px 0 10px;
  }
  .titlebar__settings:hover {
    background: #1a1a2a;
    color: #e2e2e8;
    border-color: #2a2a3a;
  }
  .titlebar__settings--active {
    background: #14141f;
    color: #e2e2e8;
    border-color: #2a2a3a;
  }
  .titlebar__settings-icon {
    display: block;
    flex: 0 0 auto;
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

  /* The narrow column is opt-in — only routes that explicitly want a
     centered, padded reading column (currently `/settings`, `/debug`, and
     `/menu`) request it. Surfaces like the timeline consume the full
     viewport width by default so previews and dense controls aren't
     artificially capped. */
  .app-content--narrow {
    max-width: 860px;
    margin: 0 auto;
    padding: 0 24px 64px;
    gap: 14px;
  }
</style>
