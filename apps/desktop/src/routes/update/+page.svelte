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
  //
  // Layout follows docs/mockups/redesign/update.html: three bands — identity,
  // notes, controls — and only the middle one flexes, so the buttons never move
  // as the state advances (a shifting primary here is a mis-click into a restart).
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onDestroy, onMount, tick } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconExternal from "~icons/lucide/external-link";
  import IconPower from "~icons/lucide/power";
  import IconRefresh from "~icons/lucide/rotate-cw";
  import IconShield from "~icons/lucide/shield-check";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import { isOpenableHref, renderMarkdown } from "$lib/markdown";
  import { RENEWAL_CHECKOUT_URL } from "$lib/licensing";
  import { licenseStatus, refreshLicenseNow } from "$lib/licensing-store.svelte";
  import {
    appUpdateProgressPercent,
    appUpdateProgressText,
    formatUpdateDate,
  } from "$lib/settings/state/about.svelte";
  import {
    appUpdateHeading,
    canInstallUpdate,
    canRestartUpdate,
    fitUpdateWindowHeight,
    formatCheckedClock,
    outOfWindowLicense,
    UPDATE_KICKER,
    updateChannelLine,
    updateViewKey,
    updateWindowLines,
    updateWindowMessage,
  } from "$lib/update-window";
  import type { AppUpdateStatus } from "$lib/types";

  let status = $state<AppUpdateStatus | null>(null);
  let acting = $state(false);
  let actionError = $state<string | null>(null);

  // Every decision below is a pure function in `$lib/update-window`, tested by
  // specs/update-window.test.ts — a Svelte `$derived` is unreachable from the
  // frontend suite, and this window's whole job is deciding what to say and
  // which button to arm.
  const key = $derived(updateViewKey(status));
  const version = $derived(status?.update?.version ?? null);
  const currentVersion = $derived(status?.app.version ?? null);
  const releaseDate = $derived(formatUpdateDate(status?.update?.date));
  const canInstall = $derived(canInstallUpdate(status, acting));
  const canRestart = $derived(canRestartUpdate(status, acting));
  const heading = $derived(appUpdateHeading(status));
  const message = $derived(updateWindowMessage(key));
  const lines = $derived(updateWindowLines(status));
  // The channel stays out of the window on Stable (it answers a question nobody
  // asked); anyone on Preview needs to know their updates come from another feed.
  const channelLabel = $derived(updateChannelLine(status));
  const license = $derived(outOfWindowLicense(licenseStatus.value));
  const busyLabel = $derived(
    key === "installing" ? "Installing…" : key === "downloading" || acting ? "Downloading…" : null,
  );
  const progressKnown = $derived(!!status?.progress?.contentLengthBytes);

  // Release notes are Markdown in the feed. `renderMarkdown` escapes raw HTML
  // (`html: false`), which is what makes the {@html} below safe.
  const notesHtml = $derived(
    status?.update?.notes?.trim() ? renderMarkdown(status.update.notes) : null,
  );
  // Out of window, the notes sit behind "What's in <version>" so the band leads
  // with what the lapse means; every other state with a version shows them.
  let notesOpen = $state(false);
  const showNotes = $derived(
    !!notesHtml && key !== "checking" && key !== "uptodate" && key !== "check-failed" && key !== "incompatible" && (key !== "expired" || notesOpen),
  );

  // Out-of-window feedback under the buttons: Refresh license, then Renew.
  let refreshing = $state(false);
  let renewNote = $state<"checked" | "checkout" | null>(null);

  // Release notes carry real links (GitHub's generated notes end in a "Full
  // Changelog" URL). A plain anchor click NAVIGATES this webview to github.com,
  // and the window has no back affordance — so links go to the OS browser
  // instead. Same delegated-click shape as `AnswerProse`, including the two
  // parts that are easy to miss:
  //   - a MIDDLE click fires `auxclick`, not `click`, so without the twin
  //     handler below the webview honours `target=_blank` natively and the
  //     link escapes the controlled opener. The feed's notes are NOT covered
  //     by the updater signature, so treat them as untrusted content.
  //   - a rendered href is NOT proof of a safe destination: `renderMarkdown`
  //     keeps scheme-less relative hrefs and tags them `data-external` too.
  //     `isOpenableHref` is the open-time gate; see its doc comment.
  function openNoteLink(event: MouseEvent): boolean {
    const anchor = (event.target as HTMLElement | null)?.closest("a");
    const href = anchor?.getAttribute("href");
    if (!href) return false;
    // Always swallow the navigation, even for a link we refuse to open: this
    // webview has no back affordance, so navigating away is unrecoverable.
    event.preventDefault();
    if (!isOpenableHref(href)) return false;
    void openUrl(href).catch((err) => {
      // Not swallowed: a refused open is otherwise a dead click nobody can
      // explain.
      actionError = describe(err);
    });
    return true;
  }

  // `renderMarkdown`'s fence rule always emits the shared code chrome (a
  // language strip plus a `data-copy-code` button) and leaves the button inert
  // for whoever renders it. A release note with a fenced block is rare but a
  // dead button is worse than none, so service it the same way AnswerProse does.
  // One timer, but the button it belongs to is restored eagerly when a second
  // block is copied — a single shared handle otherwise cancelled the FIRST
  // button's revert and left it reading "Copied" for the life of the window.
  let copyRevertTimer: ReturnType<typeof setTimeout> | null = null;
  let copiedBtn: HTMLButtonElement | null = null;

  function restoreCopiedBtn() {
    if (copyRevertTimer !== null) clearTimeout(copyRevertTimer);
    copyRevertTimer = null;
    if (copiedBtn?.isConnected) {
      copiedBtn.textContent = "Copy";
      copiedBtn.setAttribute("aria-label", "Copy code");
    }
    copiedBtn = null;
  }

  function handleNotesClick(event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    const btn = target?.closest("[data-copy-code]") as HTMLButtonElement | null;
    if (btn) {
      const block = btn.closest(".answer-code");
      const codeEl = block?.querySelector("pre") ?? block?.querySelector("code");
      void navigator.clipboard.writeText(codeEl?.textContent ?? "");
      restoreCopiedBtn();
      // aria-label is set by the renderer, so it OVERRIDES textContent for a
      // screen reader — flip both or the confirmation is never announced.
      btn.textContent = "Copied";
      btn.setAttribute("aria-label", "Copied");
      copiedBtn = btn;
      copyRevertTimer = setTimeout(restoreCopiedBtn, 1200);
      return;
    }
    openNoteLink(event);
  }

  // Middle-click fires `auxclick`, not `click`. So does RIGHT-click, which must
  // keep its context menu — hence the explicit middle-button check.
  function handleNotesAuxClick(event: MouseEvent) {
    if (event.button !== 1) return;
    openNoteLink(event);
  }

  // Guard the pending "Copied" revert against teardown — this window is closed
  // by hand and by the restart, both of which can land inside the 1.2s window.
  onDestroy(() => {
    if (copyRevertTimer !== null) clearTimeout(copyRevertTimer);
  });

  // Element handles for the content-fit pass below.
  let bodyEl = $state<HTMLElement | null>(null);
  let bandEl = $state<HTMLElement | null>(null);
  let notesEl = $state<HTMLElement | null>(null);

  // Grow/shrink the window to what the middle band actually needs. A fixed
  // height leaves a dead void under a two-line changelog and clips a long one.
  // The band sits at its natural height inside the flexible body (only the notes
  // shrink and scroll), so its natural height — plus whatever the notes hide —
  // minus the room the body offers IS the window's error. Clamped so a huge
  // changelog can't grow past a sensible panel. Width follows the window, so a
  // user who widened it keeps that width.
  async function fitWindowToContent() {
    if (!bodyEl || !bandEl) return;
    const hidden = notesEl ? notesEl.scrollHeight - notesEl.clientHeight : 0;
    const pad = getComputedStyle(bodyEl);
    const room = bodyEl.clientHeight - parseFloat(pad.paddingTop) - parseFloat(pad.paddingBottom);
    const target = fitUpdateWindowHeight(window.innerHeight, bandEl.offsetHeight + hidden - room);
    if (target === null) return;
    try {
      await getCurrentWindow().setSize(new LogicalSize(window.innerWidth, target));
    } catch {
      // Not in a Tauri window (dev render) — the fixed size is fine there.
    }
  }

  // Re-fit only when the BAND changes (notes arrive, notes open, notes ↔
  // message ↔ out-of-window). Never on a plain state step: available →
  // downloading → ready keeps the same notes, so the window — and the buttons —
  // stay exactly where they are.
  const bandKind = $derived(showNotes ? "notes" : key === "expired" ? "lapsed" : "message");
  $effect(() => {
    notesHtml;
    bandKind;
    notesOpen;
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
      // rather than making the user click twice. `restart()` owns `acting` from
      // here — it deliberately never clears it on success, so returning early
      // keeps this function's `finally` from re-arming the button underneath a
      // process that is already exiting.
      if (status?.state === "restartRequired") {
        await restart();
        return;
      }
    } catch (err) {
      actionError = describe(err);
    }
    acting = false;
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

  // The existing Manual Receipt Refresh. A renewal that lands republishes the
  // license and re-runs the update check, which walks this window to Available;
  // until then the line says what the check found.
  async function refreshLicense() {
    refreshing = true;
    actionError = null;
    try {
      await refreshLicenseNow();
      renewNote = "checked";
    } catch (err) {
      actionError = describe(err);
    }
    refreshing = false;
  }

  async function renew() {
    actionError = null;
    try {
      await openUrl(RENEWAL_CHECKOUT_URL);
      renewNote = "checkout";
    } catch (err) {
      actionError = describe(err);
    }
  }

  function primary() {
    if (key === "ready") void restart();
    else if (key === "expired") void renew();
    else void install();
  }

  const renewLine = $derived(
    renewNote === "checkout"
      ? "Checkout opened in your browser. Come back here when you’re done."
      : renewNote === "checked"
        ? licenseStatus.value?.kind === "licensed" && licenseStatus.value.inWindow
          ? "Renewal found — checking for the update now."
          : `Checked just now — your window still ends ${license?.windowEnd ?? "as before"}.`
        : null,
  );

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
  <title>Mnema · Update</title>
</svelte:head>

<main class="up" data-state={key}>
  <!-- 1 · identity -->
  <header class="up-head">
    <!-- The real bundle icon (static/app-icon.png is a copy of
         src-tauri/icons/128x128@2x.png): a drawn approximation of the mark is
         not the app's identity. It carries its own tile, so no plate behind it. -->
    <img class="up-mark" src="/app-icon.png" alt="" width="44" height="44" />
    <div class="up-head__text">
      <span class="mx-kicker">{UPDATE_KICKER[key]}{#if channelLabel}&ensp;·&ensp;{channelLabel}{/if}</span>
      <h1 class="mx-h2">{heading}</h1>
      <div class="up-ver">
        {#if version && currentVersion && key !== "expired"}
          <span>{currentVersion}</span><i aria-hidden="true">→</i><b>{version}</b>
          {#if releaseDate}<i aria-hidden="true">·</i><span>{releaseDate}</span>{/if}
        {:else if currentVersion}
          <span class="up-ver__words">
            You have <b class="num">{currentVersion}</b>
            {#if key === "expired" && license}· updates covered through <span class="num">{license.windowEnd}</span>
            {:else if key === "uptodate" && formatCheckedClock(status?.lastCheckedAtUnixMs, Date.now())}· checked {formatCheckedClock(status?.lastCheckedAtUnixMs, Date.now())}
            {:else if key === "check-failed" && formatCheckedClock(status?.lastCheckedAtUnixMs, Date.now())}· last checked {formatCheckedClock(status?.lastCheckedAtUnixMs, Date.now())}
            {/if}
          </span>
        {/if}
      </div>
    </div>
  </header>

  <!-- 2 · notes / message / out-of-window explainer — the only flexible band -->
  <div class="up-body" bind:this={bodyEl}>
    <div class="up-band" bind:this={bandEl}>
      {#if key === "expired"}
        <div class="mx-notice" data-tone="ok">
          <span class="mx-notice__icon"><IconShield width="14" height="14" /></span>
          <p class="mx-notice__text">
            <b>Nothing stops working.</b> Mnema <span class="num">{currentVersion}</span> is yours to keep —
            capture, search, Ask AI and every recording stay as they are. Renewing only brings newer builds.
          </p>
        </div>
        {#if license}
          <dl class="mx-spec mx-panel mx-panel--inset up-lapsed">
            <div><dt>Licensed to</dt><dd>{license.licensee}</dd></div>
            <div><dt>Update window</dt><dd>ended <span class="num">{license.windowEnd}</span></dd></div>
            <div><dt>Renewal</dt><dd>another year of updates, same key</dd></div>
          </dl>
        {/if}
        {#if notesHtml && version}
          <button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm up-seenotes"
            aria-expanded={notesOpen}
            onclick={() => (notesOpen = !notesOpen)}
          >What’s in {version}</button>
        {/if}
      {:else if message}
        <p class="mx-prose">{message}</p>
      {/if}
      {#if showNotes}
        <!-- No "What's new" label: GitHub-generated notes open with their own
             `## What's Changed` heading, and two stacked headings read as a mistake. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <section
          class="mx-panel mx-panel--inset up-notes"
          aria-label="Release notes"
          bind:this={notesEl}
          onclick={handleNotesClick}
          onauxclick={handleNotesAuxClick}
        >
          {@html notesHtml}
        </section>
      {/if}
    </div>
  </div>

  <!-- 3 · controls — fixed position across states -->
  <footer class="up-foot">
    {#if key === "downloading"}
      <div aria-live="polite">
        <div class="mx-progress__meta">
          <b>Downloading</b>
          <span>{appUpdateProgressText(status)}{#if progressKnown}&nbsp;· {Math.round(appUpdateProgressPercent(status))}%{/if}</span>
        </div>
        <div
          class="mx-progress"
          class:mx-progress--indeterminate={!progressKnown}
          role="progressbar"
          aria-label="Downloading"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={progressKnown ? Math.round(appUpdateProgressPercent(status)) : undefined}
          style:--v={progressKnown ? appUpdateProgressPercent(status) / 100 : 1}
        ><i></i></div>
      </div>
    {:else if key === "installing"}
      <div aria-live="polite">
        <div class="mx-progress__meta"><b>Installing</b><span>keep Mnema open</span></div>
        <div class="mx-progress mx-progress--indeterminate" role="progressbar" aria-label="Installing"><i></i></div>
      </div>
    {/if}

    <!-- `install_app_update` resolves (never rejects) on a failed install, so
         `status.error` — folded into these lines — is the only report of it;
         `actionError` is what a rejected invoke or a refused open adds. -->
    {#each lines as line, i (i)}
      <span class="mx-inline" data-tone={line.tone} role={line.tone === "danger" || line.tone === "warn" ? "alert" : undefined}>
        {#if line.icon === "power"}<IconPower width="14" height="14" />
        {:else if line.icon === "check"}<IconCheck width="14" height="14" />
        {:else if line.icon === "spin"}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>
        {:else if line.icon === "cloud-off"}<IconCloudOff width="14" height="14" />
        {:else if line.icon === "alert"}<IconAlert width="14" height="14" />{/if}
        {key === "expired" && renewLine ? renewLine : line.text}
      </span>
    {/each}
    {#if actionError}
      <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="14" height="14" />{actionError}</span>
    {/if}

    <div class="up-actions">
      {#if key === "expired"}
        <button
          type="button"
          class="mx-btn mx-btn--ghost mx-btn--sm"
          aria-busy={refreshing}
          disabled={refreshing}
          onclick={() => void refreshLicense()}
        ><IconRefresh width="13" height="13" />Refresh license</button>
      {/if}
      <span class="up-spacer"></span>
      <!-- Enabled while DOWNLOADING: the backend owns the download and the tray
           keeps showing "Updating…". Disabled while INSTALLING: the restart is
           chained here in the page, so closing the window destroys the webview,
           the pending `invoke` never resolves, and the app is left with capture
           stopped and the bundle swapped but never relaunched. -->
      <button
        type="button"
        class="mx-btn mx-btn--ghost"
        onclick={() => void closeCurrentWindow()}
        disabled={key === "installing"}
      >
        {#if key === "downloading"}Continue in background
        {:else if key === "expired"}Not now
        {:else if message}Close
        {:else}Later{/if}
      </button>
      <button
        type="button"
        class="mx-btn mx-btn--primary"
        onclick={primary}
        aria-busy={!!busyLabel || (key === "ready" && acting)}
        disabled={key === "ready" ? !canRestart : key === "expired" ? false : !canInstall}
      >
        {#if key === "ready"}
          {#if acting}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>Restarting…{:else}Restart now{/if}
        {:else if key === "expired"}
          Renew for a year · $29<IconExternal width="13" height="13" />
        {:else if busyLabel}
          <span class="mx-spin mx-spin--sm" aria-hidden="true"></span>{busyLabel}
        {:else}
          {key === "download-failed" || key === "install-failed" ? "Try again" : "Install and restart"}
        {/if}
      </button>
    </div>
  </footer>
</main>

<style>
  /* flex:1 1 auto, never height:100% — WKWebView collapses the latter. */
  .up {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--app-surface);
    color: var(--app-text);
  }

  /* Identity band */
  .up-head {
    display: grid;
    grid-template-columns: 44px minmax(0, 1fr);
    gap: 14px;
    align-items: center;
    padding: var(--s-5) var(--s-5) var(--s-4);
  }
  .up-mark { display: block; width: 44px; height: 44px; }
  .up-head__text { display: grid; gap: 6px; min-width: 0; }
  .up-ver {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    font: 450 var(--text-base)/1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .up-ver b { font-weight: 500; color: var(--app-text-strong); }
  .up-ver i { font-style: normal; color: var(--app-text-faint); }
  .up-ver__words { font: 400 var(--text-base)/1.3 var(--font-sans); }

  /* Middle band: the body takes the slack; the band inside it sits at its
     natural height and only the notes shrink and scroll. */
  .up-body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 0 var(--s-5);
  }
  .up-band {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-height: 0;
    max-height: 100%;
  }
  .up-band > * { flex: none; }
  .up-band > .up-notes { flex: 0 1 auto; }
  .up-lapsed { grid-template-columns: max-content 1fr; padding: 2px var(--s-4); }
  .up-lapsed dd { font-size: var(--text-base); }
  .up-band .mx-notice { grid-template-columns: auto 1fr; align-items: start; }
  .up-band .mx-notice__text b { color: var(--app-text-strong); font-weight: 600; }
  .up-seenotes { align-self: flex-start; }

  .up-notes {
    min-height: 0;
    overflow: auto;
    padding: var(--s-3) var(--s-4) var(--s-4);
  }
  .up-notes :global(h1),
  .up-notes :global(h2),
  .up-notes :global(h3) {
    margin: 4px 0 6px;
    font: 600 var(--text-md)/1.3 var(--font-sans);
    color: var(--app-text-strong);
  }
  .up-notes :global(p) {
    margin: var(--s-2) 0 0;
    font: 400 var(--text-sm)/1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .up-notes :global(ul),
  .up-notes :global(ol) { margin: 0 0 var(--s-2); padding-left: 18px; display: grid; gap: 5px; }
  .up-notes :global(li) { font: 400 var(--text-base)/1.5 var(--font-sans); color: var(--app-text-muted); }
  .up-notes :global(li::marker) { color: var(--app-text-faint); }
  .up-notes :global(strong) { font-weight: 600; color: var(--app-text-strong); }
  /* GitHub's closing "**Full Changelog**: url" is the meta line, not a heading. */
  .up-notes :global(p strong) { font-weight: inherit; color: inherit; }
  .up-notes :global(a) {
    overflow-wrap: anywhere;
    color: var(--app-text);
    text-decoration: underline;
    text-decoration-color: var(--app-border-hover);
    text-underline-offset: 2px;
  }
  /* Not the code inside a fenced `.answer-code` block — without the :not() the
     chip styling boxes every line of a code block (same exclusion as AnswerProse). */
  .up-notes :global(code:not(.answer-code code)) {
    font: 450 var(--text-sm)/1 var(--font-mono);
    padding: 1px 4px;
    border-radius: var(--r-xs);
    background: var(--mx-wash);
    color: var(--app-text);
  }
  .up-notes :global(.answer-code) {
    margin: 0 0 var(--s-2);
    border: 1px solid var(--app-border);
    border-radius: var(--r-sm);
    overflow: hidden;
  }
  .up-notes :global(.answer-code__header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    padding: 4px 8px;
    border-bottom: 1px solid var(--app-border);
    font: 450 var(--text-xs)/1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .up-notes :global(.answer-code__copy) {
    padding: 2px 8px;
    border: 1px solid var(--app-border);
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    cursor: pointer;
  }
  .up-notes :global(.answer-code__copy:hover) { color: var(--app-text-strong); border-color: var(--app-border-strong); }
  .up-notes :global(.answer-code__copy:focus-visible) { outline: 2px solid var(--app-accent); }
  /* Wrap rather than scroll: Copy copies `textContent`, so a line scrolled off
     a 480px window would put content on the clipboard the user never saw. */
  .up-notes :global(.answer-code__pre) {
    margin: 0;
    padding: 8px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    overflow-x: auto;
    font: 450 var(--text-sm)/1.5 var(--font-mono);
  }
  .up-notes :global(.answer-code__pre code) { padding: 0; background: transparent; color: var(--app-text); }

  /* Controls band */
  .up-foot {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4) var(--s-5) var(--s-5);
    margin-top: var(--s-4);
    border-top: 1px solid var(--mx-hairline);
  }
  .up-foot > .mx-inline {
    min-height: 18px;
    flex-wrap: nowrap;
    align-items: flex-start;
    color: var(--tone, var(--app-text-subtle));
  }
  .up-foot > .mx-inline :global(svg),
  .up-foot > .mx-inline .mx-spin { margin-top: 2px; }
  .up-actions { display: flex; align-items: center; gap: var(--s-2); }
  .up-spacer { flex: 1; }
</style>
