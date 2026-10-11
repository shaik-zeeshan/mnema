<script lang="ts">
  // CLI Access consent window (`AppWindow::CliAccessRequest`, 520×600, no native
  // chrome). Look: docs/mockups/redesign/access.html. Behaviour: ADR 0059 —
  // focus starts on Deny, Enter is never bound, Esc denies, options below the
  // request's minimum are disabled, and Allow carries no green fill.
  import { tip } from "$lib/components/tooltip";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import { trapTabKey } from "$lib/keyboard";
  import Choice from "$lib/components/Choice.svelte";
  import IconTerminal from "~icons/lucide/square-terminal";
  import IconLock from "~icons/lucide/lock";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconOctagon from "~icons/lucide/octagon-alert";
  import IconClock from "~icons/lucide/clock";
  import IconCheck from "~icons/lucide/check";
  import IconBan from "~icons/lucide/ban";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconRefresh from "~icons/lucide/rotate-cw";
  import {
    allowLabel,
    identitySource,
    isWiden,
    requestAge,
    SCOPE_DESC,
    SCOPE_LABEL,
    SCOPE_PROSE,
    SCOPE_RANK,
    SCOPES,
    scopeDisabled,
    scopeHint,
    scopeTag,
    splitCommand,
    type PendingCliAccessRequest,
    type Scope,
  } from "$lib/access/request-view";

  let selectedScope = $state<Scope>("lastDay");
  let pendingRequest = $state<PendingCliAccessRequest | null>(null);
  // Action errors (approve/deny/block) sit above the footer and keep the form.
  let error = $state<string | null>(null);
  // `gone`: the error says the rendered request can no longer be answered (the
  // tool stopped waiting, or it was blocked in Settings meanwhile) — only Close
  // is left to press.
  let gone = $state(false);
  // Load errors are kept separate so the body can show Retry instead of the
  // contradictory "nothing pending" copy.
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let approving = $state(false);
  let cancelling = $state(false);
  let blocking = $state(false);
  const acting = $derived(approving || cancelling || blocking);
  // A resolved Ok from approve/block means the write really landed. The terms are
  // snapshotted so the receipt keeps naming them while the backend tears the
  // window down — and a delayed teardown is never misreported as a failure.
  // `broad` keeps a full-history grant on the danger ramp after the click.
  let receipt = $state<{
    kind: "granted" | "blocked";
    tool: string;
    scope: Scope;
    current: Scope | null;
    widen: boolean;
  } | null>(null);
  // Ticks so the request-age label stays honest while the dialog sits open.
  let now = $state(Date.now());
  let actionWatchdog: ReturnType<typeof setTimeout> | null = null;

  // Focus lands on a SAFE anchor (Deny / Close) on open, and Tab stays inside.
  let dialogEl = $state<HTMLDivElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);
  let receiptCloseButton = $state<HTMLButtonElement | null>(null);
  let bodyEl = $state<HTMLElement | null>(null);

  const source = $derived(pendingRequest ? identitySource(pendingRequest.client.source) : null);
  const widen = $derived(pendingRequest ? isWiden(pendingRequest) : false);
  const cmd = $derived(pendingRequest ? splitCommand(pendingRequest.command) : null);
  const age = $derived(pendingRequest ? requestAge(pendingRequest.createdAt, now) : null);
  const isBroadScope = $derived(selectedScope === "allRetained");
  const hint = $derived(pendingRequest ? scopeHint(pendingRequest, selectedScope) : "");
  const scopeOptions = $derived(
    SCOPES.map((value) => ({
      value,
      label: SCOPE_LABEL[value],
      description: SCOPE_DESC[value],
      tag: pendingRequest ? scopeTag(pendingRequest, value) : "",
    })),
  );
  const scopeDisabledValues = $derived(
    pendingRequest ? SCOPES.filter((s) => scopeDisabled(pendingRequest!, s)) : [],
  );

  // Allow must never quietly take access away. Approving SETS the permission's
  // scope, and this window pre-selects the minimum, which on some doors sits
  // BELOW what the tool already holds (`mnema access request --scope last-day`
  // against a full-history permission). Name the loss before the click.
  const narrowsExistingAccess = $derived(
    pendingRequest?.currentScope &&
      SCOPE_RANK[pendingRequest.currentScope] > SCOPE_RANK[selectedScope]
      ? SCOPE_PROSE[pendingRequest.currentScope]
      : null,
  );

  // The form is taller than the 600 px the window opens at, and the body's
  // scroll is invisible under macOS overlay scrollbars — a clipped standing-terms
  // line is the whole statement of what access means. Grow the window to fit
  // (never shrink, so the buttons move only when content grows).
  $effect(() => {
    void [pendingRequest, selectedScope, error, gone, bodyEl];
    void tick().then(async () => {
      if (!bodyEl) return;
      const overflow = bodyEl.scrollHeight - bodyEl.clientHeight;
      if (overflow < 2) return;
      const target = Math.min(window.screen.availHeight - 48, window.innerHeight + overflow);
      if (target <= window.innerHeight) return;
      try {
        await getCurrentWindow().setSize(new LogicalSize(window.innerWidth, Math.round(target)));
      } catch {
        // Not in a Tauri window (dev render).
      }
    });
  });

  onMount(() => {
    void loadPendingRequest();
    // Never Allow: a reflexive keypress must not grant access.
    void tick().then(() => (denyButton ?? dialogEl)?.focus());
    const ageTimer = setInterval(() => (now = Date.now()), 15000);
    // This window is REUSED (`open_or_focus_window` focuses an existing label),
    // so a second request can arrive on this same page instance. Re-read whenever
    // the window comes forward; a receipt is left alone so a slow teardown can't
    // wipe the user's confirmation.
    const onWindowFocus = () => {
      if (receipt || acting) return;
      void loadPendingRequest();
    };
    window.addEventListener("focus", onWindowFocus);
    return () => {
      clearInterval(ageTimer);
      window.removeEventListener("focus", onWindowFocus);
      clearActionWatchdog();
    };
  });

  async function loadPendingRequest() {
    error = null;
    gone = false;
    loadError = null;
    loading = true;
    try {
      const previousRequestId = pendingRequest?.requestId;
      pendingRequest = await invoke<PendingCliAccessRequest | null>("get_pending_cli_access_request");
      // Only (re)set the selection for a request not shown before — this runs on
      // every focus, and a user who widened then clicked away must not come back
      // to a silently reverted choice. Pre-select the NARROWEST scope that works,
      // never `preferredScope`: the widen is left to a deliberate click.
      if (pendingRequest && pendingRequest.requestId !== previousRequestId) {
        selectedScope = pendingRequest.minimumScope;
      }
    } catch (err) {
      loadError = friendlyError(err);
    } finally {
      loading = false;
      // The skeleton→content swap drops focus onto <body>. Re-anchor on Deny
      // (the anti-reflex guarantee) unless the user already tabbed inside.
      void tick().then(() =>
        requestAnimationFrame(() => {
          if (dialogEl?.contains(document.activeElement)) return;
          (denyButton ?? dialogEl)?.focus();
        }),
      );
    }
  }

  // A refused click means the slot no longer holds what this window rendered.
  // Swap to the successor if one is waiting; otherwise the request is gone and
  // the "no longer valid" error stands with only Close left.
  async function showSuccessorIfAnyIsWaiting() {
    let successor: PendingCliAccessRequest | null = null;
    try {
      successor = await invoke<PendingCliAccessRequest | null>("get_pending_cli_access_request");
    } catch {
      return;
    }
    if (!successor || successor.requestId === pendingRequest?.requestId) return;
    pendingRequest = successor;
    selectedScope = successor.minimumScope;
    error = null;
    gone = false;
    void tick().then(() => denyButton?.focus());
  }

  // On success the backend tears this window down; if that never arrives the
  // watchdog re-enables the action and explains, so it never hangs.
  function startActionWatchdog(reset: () => void) {
    clearActionWatchdog();
    actionWatchdog = setTimeout(() => {
      actionWatchdog = null;
      if (receipt) return;
      reset();
      error = "Couldn't finish — the request didn't complete. Please try again.";
    }, 6000);
  }

  function clearActionWatchdog() {
    if (actionWatchdog) {
      clearTimeout(actionWatchdog);
      actionWatchdog = null;
    }
  }

  // Receipt dismiss: nothing is pending any more, so close the window directly
  // (cancel_pending_cli_access_request would reject with "no pending request").
  async function closeReceiptWindow() {
    try {
      await getCurrentWindow().close();
    } catch {
      // Already being torn down by the backend.
    }
  }

  async function closeWindow() {
    error = null;
    cancelling = true;
    startActionWatchdog(() => (cancelling = false));
    try {
      await invoke("cancel_pending_cli_access_request");
    } catch (err) {
      clearActionWatchdog();
      error = mapActionError(err);
      cancelling = false;
    }
  }

  async function settle(err: unknown, reset: () => void) {
    clearActionWatchdog();
    error = mapActionError(err);
    reset();
    const message = friendlyError(err);
    if (/blocked in Settings/i.test(message)) gone = true;
    // The requester stopped waiting, OR a different tool's request took the
    // slot since we rendered — re-read and swap to it if so.
    if (/no pending CLI Access request/i.test(message)) {
      gone = true;
      await showSuccessorIfAnyIsWaiting();
    }
    // The clicked button went disabled mid-flight and dropped focus to <body>;
    // land it back on the safe anchor.
    void tick().then(() => denyButton?.focus());
  }

  function landReceipt(kind: "granted" | "blocked", terms: NonNullable<typeof receipt>) {
    clearActionWatchdog();
    approving = false;
    blocking = false;
    error = null;
    receipt = { ...terms, kind };
    // The Deny anchor is gone; follow focus into the receipt's Close.
    void tick().then(() => receiptCloseButton?.focus());
  }

  function snapshotTerms(request: PendingCliAccessRequest) {
    return {
      kind: "granted" as const,
      tool: request.client.label,
      scope: selectedScope,
      current: request.currentScope,
      widen,
    };
  }

  async function approveAccess() {
    if (!pendingRequest || acting || gone) return;
    error = null;
    approving = true;
    const terms = snapshotTerms(pendingRequest);
    startActionWatchdog(() => (approving = false));
    try {
      await invoke("approve_pending_cli_access_request", {
        // requestId binds this click to the request this page RENDERED: the
        // slot can be refilled by another tool between render and click.
        approval: { requestId: pendingRequest.requestId, scope: selectedScope },
      });
      // Ok only on a real grant.
      landReceipt("granted", terms);
    } catch (err) {
      await settle(err, () => (approving = false));
    }
  }

  // A standing rejection: persisted even for a never-granted tool, and the CLI
  // is answered `blocked`. No confirmation — Settings can lift it.
  async function blockClient() {
    if (!pendingRequest || acting || gone) return;
    error = null;
    blocking = true;
    const terms = snapshotTerms(pendingRequest);
    startActionWatchdog(() => (blocking = false));
    try {
      await invoke("block_pending_cli_access_request", { requestId: pendingRequest.requestId });
      landReceipt("blocked", terms);
    } catch (err) {
      await settle(err, () => (blocking = false));
    }
  }

  function friendlyError(err: unknown) {
    if (typeof err === "string") return err;
    if (err instanceof Error && err.message) return err.message;
    return "Something went wrong. Please try again.";
  }

  // The pending request clears once the tool stops waiting (~120 s client-side);
  // the backend then answers "no pending CLI Access request".
  function mapActionError(err: unknown) {
    const message = friendlyError(err);
    if (/no pending CLI Access request/i.test(message)) {
      return "This request is no longer valid — the requesting tool stopped waiting. You can close this window.";
    }
    return message;
  }

  // Esc denies; Enter is intentionally not bound to Allow.
  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !acting) {
      event.preventDefault();
      void closeWindow();
      return;
    }
    trapTabKey(event, dialogEl);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<svelte:head>
  <title>mnema · CLI Access</title>
</svelte:head>

<main class="ac">
  <div
    class="ac-dlg"
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-label="CLI Access request"
    aria-busy={loading}
    tabindex="-1"
  >
    {#if receipt}
      {@const broad = receipt.kind === "granted" && receipt.scope === "allRetained"}
      <div
        class="ac-receipt"
        data-tone={receipt.kind === "blocked" || broad ? "danger" : "ok"}
        role="status"
        aria-live="polite"
      >
        <span class="ac-receipt__icon" aria-hidden="true">
          {#if receipt.kind === "blocked"}<IconBan width="18" height="18" />
          {:else if broad}<IconOctagon width="18" height="18" />
          {:else}<IconCheck width="18" height="18" />{/if}
        </span>
        {#if receipt.kind === "blocked"}
          <h1 class="mx-h2">{receipt.tool} is blocked</h1>
          <p class="mx-prose">
            Its requests are refused right away — this window won’t open for it again.{#if receipt.current}
              Its {SCOPE_LABEL[receipt.current]} access stops too.{/if} Unblock it any time.
          </p>
        {:else if receipt.widen}
          <h1 class="mx-h2">{receipt.tool} can now read {SCOPE_PROSE[receipt.scope]}</h1>
          <p class="mx-prose">Same permission, widened in place. Results it already has keep working.</p>
        {:else}
          <h1 class="mx-h2">Access granted</h1>
          <p class="mx-prose">
            <b>{receipt.tool}</b> can read <b>{SCOPE_PROSE[receipt.scope]}</b> until you block it. Unused for
            <span class="num">30</span> days, it lapses.
          </p>
        {/if}
        <p class="mx-body-sm">Manage it in Settings → Data → CLI Access.</p>
      </div>
    {:else if loading}
      <div class="ac-body" aria-busy="true">
        <span class="mx-sr" role="status">Loading access request…</span>
        <div class="mx-skel mx-skel--title"></div>
        <div class="mx-panel mx-panel--inset ac-who">
          <div class="mx-skel-row">
            <div class="mx-skel mx-skel--circle"></div>
            <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
            <div class="mx-skel"></div>
          </div>
        </div>
        <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
      </div>
    {:else if loadError}
      <div class="ac-fill">
        <div class="mx-empty" data-tone="danger" role="alert">
          <span class="mx-empty__glyph"><IconAlert width="18" height="18" /></span>
          <h1 class="mx-empty__title">Couldn’t load the request</h1>
          <p class="mx-empty__text" use:tip={loadError}>
            The access service didn’t answer. The tool is still waiting — try again.
          </p>
          <div class="mx-empty__acts">
            <button class="mx-btn mx-btn--sm" type="button" onclick={() => loadPendingRequest()}>
              <IconRefresh width="12" height="12" />Retry
            </button>
          </div>
        </div>
      </div>
    {:else if !pendingRequest}
      <div class="ac-fill">
        <div class="mx-empty">
          <span class="mx-empty__glyph"><IconClock width="18" height="18" /></span>
          <h1 class="mx-empty__title">No request waiting</h1>
          <p class="mx-empty__text">There’s no pending CLI Access request right now. You can close this window.</p>
        </div>
      </div>
    {:else}
      {@const r = pendingRequest}
      <div class="ac-body" bind:this={bodyEl}>
        <header class="ac-head">
          <span class="mx-kicker">{widen ? "More history needed" : "Access request"}</span>
          <h1 class="mx-h2">
            {widen ? `${r.client.label} needs more of your history` : `${r.client.label} wants to read your Mnema history`}
          </h1>
          <p class="mx-body-sm">
            {#if widen && r.currentScope}
              Its command reaches back further than {SCOPE_PROSE[r.currentScope]}. Allowing updates the permission it
              already has — results it’s holding keep working.
            {:else}
              A command-line tool is asking for standing access to your searchable Mnema text.
            {/if}
          </p>
        </header>

        <section class="mx-panel mx-panel--inset ac-who" aria-label="Requesting tool">
          <div class="ac-who__top">
            <span class="ac-glyph" aria-hidden="true"><IconTerminal width="16" height="16" /></span>
            <div class="ac-who__name">
              <span class="mx-h3">{r.client.label}</span>
              {#if source}
                <span class="mx-chip" class:mx-chip--warn={source.warn} use:tip={source.detail}>
                  <span aria-hidden="true">{source.chip}</span><span class="mx-sr">{source.detail}</span>
                </span>
              {/if}
            </div>
            {#if age}
              <time class="ac-who__age" use:tip={"Waiting — the tool gives up after 2 min"}>{age}</time>
            {/if}
          </div>
          {#if cmd}
            <code class="ac-cmd"><b>{cmd.head}</b>{cmd.rest ? ` ${cmd.rest}` : ""}</code>
          {/if}
          <div class="mx-notice" data-tone="warn">
            <span class="mx-notice__icon"><IconAlert width="13" height="13" /></span>
            <p class="mx-notice__text">
              This name is reported by the tool and can’t be verified. Any program that can run <b>mnema</b> can use it.
            </p>
          </div>
        </section>

        <fieldset class="ac-scope">
          <legend class="mx-label">What it can read</legend>
          {#if widen && r.currentScope}
            <div class="ac-was">
              Already allowed <span class="mx-chip">{SCOPE_LABEL[r.currentScope]}</span>
              <IconArrow width="13" height="13" aria-hidden="true" />
              now asks for <span class="mx-chip mx-chip--accent">{SCOPE_LABEL[r.preferredScope]}</span>
            </div>
          {/if}
          <Choice
            list
            options={scopeOptions}
            value={selectedScope}
            disabledValues={scopeDisabledValues}
            disabled={acting || gone}
            ariaLabel="What it can read"
            onValueChange={(v) => (selectedScope = v as Scope)}
          />
          {#if narrowsExistingAccess}
            <!-- Allowing REPLACES the scope, so a selection under what the tool
                 already holds is a revocation wearing an Allow button. -->
            <div class="mx-notice" data-tone="danger">
              <span class="mx-notice__icon"><IconAlert width="13" height="13" /></span>
              <p class="mx-notice__text">
                <b>This reduces access it already has.</b>
                {r.client.label} can currently read {narrowsExistingAccess}; allowing at this setting cuts it back to
                {SCOPE_PROSE[selectedScope]}.
              </p>
            </div>
          {/if}
          {#if isBroadScope}
            <div class="mx-notice" data-tone="danger">
              <span class="mx-notice__icon"><IconOctagon width="13" height="13" /></span>
              <p class="mx-notice__text">
                <b>Reads your entire history.</b> Everything Mnema has retained — every app, every window, every transcript.
              </p>
            </div>
          {:else if hint}
            <p class="ac-hint">{hint}</p>
          {/if}
        </fieldset>

        <div class="mx-notice">
          <span class="mx-notice__icon"><IconClock width="13" height="13" /></span>
          <p class="mx-notice__text">
            <b>Standing until idle.</b> {r.client.label} keeps this until you block it. If it goes unused for
            <span class="num">30</span> days, the permission lapses and the next request asks again.
          </p>
        </div>
      </div>
    {/if}

    {#if error && !receipt}
      <div class="ac-err" role="alert">
        <span class="mx-inline" data-tone="danger"><IconAlert width="13" height="13" />{error}</span>
      </div>
    {/if}

    <footer class="ac-foot">
      {#if receipt}
        <span class="mx-spacer"></span>
        <button class="mx-btn" bind:this={receiptCloseButton} type="button" onclick={closeReceiptWindow}>Close</button>
      {:else if pendingRequest && !loading && !loadError}
        <button
          class="mx-btn mx-btn--ghost mx-btn--danger"
          type="button"
          disabled={acting || gone}
          aria-busy={blocking}
          use:tip={"Refuse this tool now and every time — no window, no asking"}
          onclick={blockClient}
        >
          {#if blocking}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>Blocking…
          {:else}<IconBan width="14" height="14" />Block{/if}
        </button>
        <span class="mx-spacer"></span>
        <button class="mx-btn" bind:this={denyButton} type="button" disabled={acting} onclick={closeWindow}>
          {#if cancelling}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>{gone ? "Closing…" : "Denying…"}
          {:else if gone}Close
          {:else}Deny<kbd>Esc</kbd>{/if}
        </button>
        <!-- Deliberately NOT --primary: a consent click gets no green pull. -->
        <button class="mx-btn" type="button" disabled={acting || gone} aria-busy={approving} onclick={approveAccess}>
          {#if approving}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>Allowing…
          {:else}<IconLock class="mx-btn__accent" width="14" height="14" />{allowLabel(pendingRequest, selectedScope)}{/if}
        </button>
      {:else}
        <span class="mx-spacer"></span>
        <button class="mx-btn" bind:this={denyButton} type="button" disabled={acting} onclick={closeWindow}>
          {#if cancelling}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>Closing…{:else}Close{/if}
        </button>
      {/if}
    </footer>
  </div>
</main>

<style>
  /* flex:1 1 auto, never height:100% — WKWebView collapses the latter. */
  .ac {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    overflow: hidden;
    background: var(--app-surface);
    color: var(--app-text);
  }
  .ac-dlg {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
  }

  .ac-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: grid;
    align-content: start;
    gap: var(--s-4);
    padding: var(--s-5) var(--s-5) var(--s-4);
  }
  .ac-fill { flex: 1 1 auto; min-height: 0; }
  .ac-fill .mx-empty { padding: var(--s-6) var(--s-5); }
  .ac-head { display: grid; gap: 6px; }
  .ac-head .mx-h2 { text-wrap: balance; overflow-wrap: anywhere; }
  .ac-head .mx-body-sm { margin: 0; }

  /* Who is asking: the focal block. Neutral — never green for an unverified identity. */
  .ac-who { display: grid; gap: var(--s-3); padding: 14px var(--s-4); }
  .ac-who .mx-skel-row { padding: 0; }
  .ac-who__top { display: flex; align-items: center; gap: var(--s-3); }
  .ac-glyph {
    flex: none;
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
    background: var(--mx-wash);
    color: var(--app-text-muted);
    box-shadow: inset 0 0 0 1px var(--mx-hairline);
  }
  /* The "who is asking" line is load-bearing — wrap, never clip. */
  .ac-who__name { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; min-width: 0; }
  .ac-who__name .mx-h3 { overflow-wrap: anywhere; }
  .ac-who__age { margin-left: auto; font: 450 var(--text-sm)/1 var(--font-mono); color: var(--app-text-subtle); white-space: nowrap; }
  .ac-cmd {
    display: block;
    padding: 7px 10px;
    border-radius: var(--r-sm);
    background: var(--app-surface-subtle);
    border: 1px solid var(--mx-hairline);
    font: 450 var(--text-sm)/1.45 var(--font-mono);
    color: var(--app-text);
    overflow-wrap: anywhere;
  }
  .ac-cmd b { font-weight: 500; color: var(--app-text-strong); }

  .ac-body .mx-notice { padding: 8px 10px; grid-template-columns: auto 1fr; }
  .ac-body .mx-notice__icon { width: 24px; height: 24px; }
  .ac-body .mx-notice__text b { color: var(--app-text-strong); font-weight: 600; }

  .ac-scope { display: grid; gap: var(--s-2); min-width: 0; margin: 0; padding: 0; border: 0; }
  .ac-scope legend { padding: 0; margin-bottom: var(--s-2); color: var(--app-text-muted); }
  .ac-scope :global(.mx-choice__opt) { padding-top: 9px; padding-bottom: 9px; }
  .ac-scope :global(.mx-choice__desc) { font-size: var(--text-sm); }
  .ac-was { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; font: 400 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-muted); }
  .ac-was { margin-bottom: var(--s-2); }
  .ac-scope .mx-notice { margin-top: var(--s-2); }
  .ac-hint { margin: var(--s-2) 0 0; font-size: var(--text-sm); color: var(--app-text-subtle); }

  .ac-err { flex: none; padding: 0 var(--s-5) var(--s-3); }
  .ac-err .mx-inline { flex-wrap: nowrap; align-items: flex-start; }
  .ac-err .mx-inline :global(svg) { margin-top: 3px; }

  .ac-foot {
    flex: none;
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-5) var(--s-4);
    border-top: 1px solid var(--mx-hairline);
  }

  /* Receipt after the click. A full-history grant and a block stay on the danger ramp. */
  .ac-receipt { flex: 1 1 auto; display: grid; align-content: start; justify-items: start; gap: var(--s-2); padding: var(--s-6) var(--s-5) var(--s-5); }
  .ac-receipt .mx-h2 { overflow-wrap: anywhere; }
  .ac-receipt .mx-body-sm { margin: 0; }
  .ac-receipt__icon {
    width: 36px;
    height: 36px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    margin-bottom: var(--s-1);
    color: var(--tone);
    background: var(--tone-bg);
    box-shadow: inset 0 0 0 1px var(--tone-bd);
  }
  .ac-receipt[data-tone="danger"] .ac-receipt__icon { color: var(--app-danger-strong); }
  .ac-receipt .mx-prose { font-size: var(--text-md); }
  .ac-receipt .mx-prose b { color: var(--app-text-strong); font-weight: 600; }
</style>
