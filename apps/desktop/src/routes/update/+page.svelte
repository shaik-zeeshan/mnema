<script lang="ts">
  // The dedicated update window (`AppWindow::Update`, label "update"), opened by
  // the backend once per available version. Deliberately a REAL app window and
  // not a system alert: a parentless `plugin-dialog` message dialog is drawn by
  // macOS's UserNotificationCenter agent, so it survives the update restart and
  // strands on screen advertising a version the user already installed. See
  // `app_updates::prompt_update_available`.
  //
  // Status is backend-owned: this page renders `AppUpdateStatus` and streams
  // changes off `app_update_status_changed`, so installing from the tray (or
  // Settings → About) updates it live instead of leaving it stale.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount, tick } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import { renderMarkdown } from "$lib/markdown";
  import {
    appUpdateProgressPercent,
    appUpdateProgressText,
    appUpdateStatusMessage,
    formatUpdateDate,
    updateChannelLabel,
  } from "$lib/settings/state/about.svelte";
  import type { AppUpdateStatus } from "$lib/types";

  let status = $state<AppUpdateStatus | null>(null);
  let acting = $state(false);
  let actionError = $state<string | null>(null);

  const updateState = $derived(status?.state ?? "idle");
  const version = $derived(status?.update?.version ?? null);
  const currentVersion = $derived(status?.app.version ?? null);
  const releaseDate = $derived(formatUpdateDate(status?.update?.date));
  const busy = $derived(updateState === "downloading" || updateState === "installing");
  // "failed" keeps the install button alive: a failed download is retryable, and
  // the backend still holds the pending update.
  const canInstall = $derived(
    !!status?.update
      && (updateState === "available" || updateState === "failed")
      && !acting
      && !busy,
  );
  const canRestart = $derived(updateState === "restartRequired" && !acting);

  // The channel line stays OUT of the window while Stable is the only channel a
  // user would realistically be on — labelling it "Stable channel" answers a
  // question nobody asked. The code stays: anyone actually on Preview needs to
  // know their updates come from a different feed, so that case still renders.
  const channelLabel = $derived(
    status && status.channel !== "stable" ? `${updateChannelLabel(status.channel)} channel` : null,
  );

  const heading = $derived.by(() => {
    if (!version) return "mnema is up to date";
    switch (updateState) {
      case "downloading": return `Downloading ${version}`;
      case "installing": return `Installing ${version}`;
      case "restartRequired": return `${version} is ready to run`;
      default: return `mnema ${version} is available`;
    }
  });

  // Release notes are Markdown in the feed. `renderMarkdown` escapes raw HTML
  // (`html: false`), which is what makes the {@html} below safe.
  const notesHtml = $derived(
    status?.update?.notes?.trim() ? renderMarkdown(status.update.notes) : null,
  );

  // Release notes carry real links (GitHub's generated notes end in a "Full
  // Changelog" URL). A plain anchor click NAVIGATES this webview to github.com,
  // and the window has no back affordance — so links go to the OS browser
  // instead. Same delegated-click shape as `AnswerProse`.
  function handleNotesClick(event: MouseEvent) {
    const anchor = (event.target as HTMLElement | null)?.closest("a");
    const href = anchor?.getAttribute("href");
    if (!href) return;
    event.preventDefault();
    void openUrl(href);
  }

  // Element handles for the content-fit pass below.
  let notesEl = $state<HTMLElement | null>(null);
  let notesInnerEl = $state<HTMLElement | null>(null);

  // Grow/shrink the window to what the notes actually need. A fixed height
  // leaves a dead void under a two-line changelog and clips a long one; the
  // notes region is the only flexible band, so the delta between the content's
  // natural height and the height it's been given IS the window's error.
  // Clamped so a huge changelog can't grow past a sensible panel.
  const MIN_WINDOW_HEIGHT = 360;
  const MAX_WINDOW_HEIGHT = 640;
  const WINDOW_WIDTH = 480;

  async function fitWindowToContent() {
    if (!notesEl || !notesInnerEl) return;
    // `clientHeight` counts the scroller's bottom padding, so the content's
    // requirement has to count it too — otherwise the fit lands the last line
    // inside the bottom fade and it reads as clipped.
    const padBottom = parseFloat(getComputedStyle(notesEl).paddingBottom) || 0;
    const overflow = notesInnerEl.offsetHeight + padBottom - notesEl.clientHeight;
    if (Math.abs(overflow) < 2) return;
    const target = Math.round(
      Math.min(MAX_WINDOW_HEIGHT, Math.max(MIN_WINDOW_HEIGHT, window.innerHeight + overflow)),
    );
    if (target === Math.round(window.innerHeight)) return;
    try {
      await getCurrentWindow().setSize(new LogicalSize(WINDOW_WIDTH, target));
    } catch {
      // Not in a Tauri window (dev render) — the fixed size is fine there.
    }
  }

  // Re-fit whenever the rendered notes change (cold load, and a status event
  // that swaps the payload).
  $effect(() => {
    notesHtml;
    void tick().then(fitWindowToContent);
  });

  function describe(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) return String(err.message);
    return "Something went wrong.";
  }

  async function install() {
    acting = true;
    actionError = null;
    try {
      status = await invoke<AppUpdateStatus>("install_app_update");
      // The backend lands on `restartRequired`; go straight into the new build
      // rather than making the user click twice.
      if (status?.state === "restartRequired") await restart();
    } catch (err) {
      actionError = describe(err);
    } finally {
      acting = false;
    }
  }

  async function restart() {
    acting = true;
    actionError = null;
    try {
      await invoke("restart_after_app_update");
    } catch (err) {
      actionError = describe(err);
      acting = false;
    }
    // No `finally`: on success the process is on its way out, and clearing the
    // spinner would flash an idle window during the graceful stop.
  }

  onMount(() => {
    void (async () => {
      try {
        status = await invoke<AppUpdateStatus>("get_app_update_status");
      } catch (err) {
        actionError = describe(err);
      }
    })();
    const unlisten = listen<AppUpdateStatus>("app_update_status_changed", (event) => {
      status = event.payload;
    });
    return () => void unlisten.then((off) => off());
  });
</script>

<svelte:head>
  <title>mnema · Update</title>
</svelte:head>

<main class="update">
  <header class="head">
    <!-- The real bundle icon (static/app-icon.png is a copy of
         src-tauri/icons/128x128@2x.png). A hand-drawn SVG approximation of the
         mark is not the app's identity. -->
    <img class="mark" src="/app-icon.png" alt="" width="44" height="44" />
    <div class="head__text">
      {#if channelLabel}<p class="head__channel">{channelLabel}</p>{/if}
      <h1>{heading}</h1>
      <p class="head__meta">
        {#if version && currentVersion}
          <span class="from">{currentVersion}</span>
          <span class="arrow" aria-hidden="true">→</span>
          <span class="to">{version}</span>
          {#if releaseDate}<span class="dot" aria-hidden="true">·</span>{releaseDate}{/if}
        {:else}
          {appUpdateStatusMessage(status)}
        {/if}
      </p>
    </div>
  </header>

  <div class="body">
    {#if notesHtml}
      <!-- No "What's new" label above the notes: GitHub-generated notes open
           with their own `## What's Changed` heading, and two headings stacked
           read as a mistake. The notes carry their own structure. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <section class="notes" aria-label="Release notes" bind:this={notesEl} onclick={handleNotesClick}>
        <div bind:this={notesInnerEl}>{@html notesHtml}</div>
      </section>
    {:else}
      <div class="body__empty">
        <p>{appUpdateStatusMessage(status)}</p>
        {#if version}
          <p class="body__empty-note">No release notes were published for this version.</p>
        {/if}
      </div>
    {/if}
  </div>

  <footer class="foot">
    {#if status?.progress}
      <div class="progress" aria-live="polite">
        <div class="progress__meta">
          <span>{appUpdateProgressText(status)}</span>
          <span class="progress__pct">{Math.round(appUpdateProgressPercent(status))}%</span>
        </div>
        <div class="progress__bar">
          <span style={`width: ${appUpdateProgressPercent(status)}%`}></span>
        </div>
      </div>
    {:else if canInstall}
      <p class="consequence">
        <svg class="consequence__icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
          <path d="M8 2.2 V7.6" />
          <path d="M4.4 4.3a5 5 0 1 0 7.2 0" />
        </svg>
        Installing stops recording and relaunches mnema.
      </p>
    {/if}

    {#if actionError}
      <p class="foot__error" role="alert">{actionError}</p>
    {/if}

    <div class="actions">
      <button type="button" class="btn" onclick={() => void closeCurrentWindow()} disabled={busy}>
        Later
      </button>
      {#if updateState === "restartRequired"}
        <button type="button" class="btn btn--primary" onclick={() => void restart()} disabled={!canRestart}>
          {#if acting}<span class="btn__spinner" aria-hidden="true"></span>Restarting{:else}Restart now{/if}
        </button>
      {:else}
        <button type="button" class="btn btn--primary" onclick={() => void install()} disabled={!canInstall}>
          {#if acting || busy}
            <span class="btn__spinner" aria-hidden="true"></span>
            {updateState === "installing" ? "Installing" : "Downloading"}
          {:else}
            Install and restart
          {/if}
        </button>
      {/if}
    </div>
  </footer>
</main>

<style>
  /* Three bands — identity / notes / controls — with only the middle one
     flexible, so the buttons never move as the state advances (this is the one
     window where a shifting primary means a mis-click into an app restart).
     flex:1 1 auto, never height:100% — WKWebView collapses the latter. */
  .update {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--app-surface);
    color: var(--app-text);
  }

  .head {
    display: grid;
    grid-template-columns: 44px minmax(0, 1fr);
    gap: 14px;
    align-items: center;
    padding: 16px 20px 18px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-raised);
  }

  /* The bundle icon already carries its own rounded tile and background, so it
     needs no border or plate behind it — one would read as a second tile. */
  .mark {
    display: block;
    width: 44px;
    height: 44px;
  }

  .head__text {
    min-width: 0;
  }

  .head__channel {
    margin: 0 0 3px;
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
  }

  .head h1 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 650;
    line-height: 1.25;
    letter-spacing: -0.015em;
    color: var(--app-text-strong);
  }

  .head__meta {
    margin: 6px 0 0;
    color: var(--app-text-subtle);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .head__meta .from {
    color: var(--app-text-muted);
  }

  .head__meta .to {
    color: var(--app-text);
    font-weight: 650;
  }

  .head__meta .arrow,
  .head__meta .dot {
    margin: 0 5px;
    color: var(--app-text-faint);
  }

  .body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 16px 20px 0;
  }

  .notes {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    /* Must exceed the 24px mask fade below, or the last line renders half-faded
       and reads as clipped even when nothing is scrolled away. */
    padding-bottom: 26px;
    font-size: var(--text-md);
    line-height: 1.65;
    color: var(--app-text);
    /* Fade the last line into the control band so a long changelog reads as
       continuing rather than clipped. */
    mask-image: linear-gradient(to bottom, #000 calc(100% - 24px), transparent);
  }

  .body__empty {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-md);
    line-height: 1.6;
  }

  .body__empty p {
    margin: 0;
  }

  .body__empty-note {
    color: var(--app-text-subtle);
  }

  .notes :global(h1),
  .notes :global(h2),
  .notes :global(h3) {
    margin: 16px 0 6px;
    font-size: var(--text-md);
    font-weight: 650;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }

  .notes :global(h1:first-child),
  .notes :global(h2:first-child),
  .notes :global(h3:first-child) {
    margin-top: 0;
  }

  .notes :global(p) {
    margin: 0 0 9px;
  }

  .notes :global(ul),
  .notes :global(ol) {
    margin: 0 0 9px;
    padding-left: 16px;
    list-style: none;
  }

  /* The marker is absolutely positioned, NOT a grid/flex item: making the `li`
     a grid container blockifies every inline child, so `<strong>` and `<a>`
     each become their own row and the bullet shatters one word per line. */
  .notes :global(li) {
    position: relative;
    margin: 4px 0;
  }

  .notes :global(ul > li)::before {
    content: "–";
    position: absolute;
    left: -16px;
    color: var(--app-text-faint);
  }

  .notes :global(strong) {
    font-weight: 650;
    color: var(--app-text-strong);
  }

  /* Blue, not accent green: GitHub-generated notes are half URL by volume, and
     greening them floods the window with the app's one loud color. */
  .notes :global(a) {
    color: var(--app-info);
    text-decoration: underline;
    text-decoration-color: color-mix(in srgb, var(--app-info) 45%, transparent);
    text-underline-offset: 2px;
  }

  .notes :global(a:hover) {
    text-decoration-color: var(--app-info);
  }

  .notes :global(code) {
    padding: 1px 5px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface-subtle);
    font-size: var(--text-base);
  }

  .foot {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 14px 20px 16px;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-raised);
  }

  /* Installing kills an active recording. That belongs in the resting layout
     above the button, not in a dialog discovered after clicking. */
  .consequence {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    color: var(--app-text-subtle);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .consequence__icon {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
    /* Optical alignment to the first line's cap height, not its box top. */
    margin-top: 2px;
  }

  .progress__meta {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 7px;
    color: var(--app-text-subtle);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
  }

  .progress__pct {
    color: var(--app-accent);
    font-weight: 650;
  }

  /* Decile ticks across trough and fill alike, so the bar reads as a scale
     rather than a blob. */
  .progress__bar {
    height: 6px;
    border-radius: 2px;
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 1px var(--app-border);
  }

  .progress__bar span {
    display: block;
    height: 100%;
    background:
      repeating-linear-gradient(
        to right,
        transparent 0 calc(10% - 1px),
        var(--app-surface-raised) calc(10% - 1px) 10%
      ),
      var(--app-accent);
    transition: width 160ms ease-out;
  }

  .foot__error {
    margin: 0;
    color: var(--app-danger-text);
    font-size: var(--text-base);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  /* Sentence case on purpose: `.btn` elsewhere in the app is uppercase+tracked,
     which flattens a window that is mostly prose. Weight and fill carry the
     hierarchy here instead of letterforms. */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    min-height: 32px;
    padding: 7px 14px;
    border-radius: 6px;
    font-family: inherit;
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.005em;
    cursor: pointer;
    border: 1px solid var(--app-border-strong);
    background: transparent;
    color: var(--app-text-muted);
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  .btn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .btn:focus-visible {
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .btn--primary {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  .btn--primary:not(:disabled):hover {
    background: color-mix(in srgb, var(--app-accent) 12%, var(--app-accent-bg));
    border-color: var(--app-accent);
  }

  /* Busy, not unavailable — the in-flight primary dims less than a disabled one. */
  .btn--primary:disabled {
    opacity: var(--app-busy-opacity);
  }

  .btn__spinner {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-top-color: transparent;
    opacity: 0.85;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .btn__spinner {
      animation-duration: 2.4s;
    }

    .progress__bar span {
      transition: none;
    }
  }
</style>
