<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import { trapTabKey } from "$lib/keyboard";
  import Segmented from "$lib/components/Segmented.svelte";

  type PendingCliAccessRequest = {
    requestId: string;
    client: {
      label: string;
      source: string;
    };
    command: string;
    minimumScope: "lastDay" | "allRetained";
    preferredScope: "lastDay" | "allRetained";
    minimumDurationSeconds: number;
    preferredDurationSeconds: number;
    createdAt: string;
  };

  let selectedScope = $state<"lastDay" | "allRetained">("lastDay");
  let selectedDuration = $state<"1h" | "24h" | "7d">("24h");
  let pendingRequest = $state<PendingCliAccessRequest | null>(null);
  // Action errors (approve/deny) surface inline at the foot of the dialog.
  let error = $state<string | null>(null);
  // Load errors are kept separate so the body can show a Retry state instead
  // of the contradictory "nothing pending" empty copy.
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let approving = $state(false);
  let cancelling = $state(false);
  // A resolved Ok from the backend approve means the grant really landed. We
  // capture the consent terms so the body can show a brief positive receipt
  // (naming tool + scope + expiry) before/while the backend tears the window
  // down — and so a delayed teardown is never misreported as a failure.
  let granted = $state<{ tool: string; scopeProse: string; expiryLabel: string } | null>(null);
  // Ticks so the request-age label stays honest while the dialog sits open.
  let now = $state(Date.now());

  // Watchdog so a grant/deny never hangs forever on the spinner if the backend
  // window teardown is delayed or never arrives.
  let actionWatchdog: ReturnType<typeof setTimeout> | null = null;

  // Bound to the dialog container and the Deny button so focus can land on a
  // SAFE anchor on open, and Tab is contained inside the consent dialog.
  let dialogEl = $state<HTMLDivElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);
  // Bound to the granted-receipt Close button so focus follows into the receipt
  // when the grant lands (the Deny anchor it replaced is gone by then).
  let grantedCloseButton = $state<HTMLButtonElement | null>(null);

  const durationSeconds = {
    "1h": 60 * 60,
    "24h": 24 * 60 * 60,
    "7d": 7 * 24 * 60 * 60,
  } as const;

  const scopeOptions = [
    {
      value: "lastDay",
      label: "Last day",
      hint: "Reads only text captured in the last 24 hours.",
    },
    {
      value: "allRetained",
      label: "All retained",
      hint: "Reads your entire retained capture history.",
    },
  ] as const;

  const durationOptions = [
    { value: "1h", label: "1h" },
    { value: "24h", label: "24h" },
    { value: "7d", label: "7d" },
  ] as const;

  // Noun-phrase scope wording for prose sentences (the receipt), where the bare
  // Segmented label ("Last day") would read as "your Last day text".
  const scopeProse = {
    lastDay: "text from the last 24 hours",
    allRetained: "your entire retained capture history",
  } as const;

  const scopeMeta = $derived(
    scopeOptions.find((option) => option.value === selectedScope) ?? scopeOptions[0],
  );
  const isBroadScope = $derived(selectedScope === "allRetained");

  // Plain option arrays for the shared Segmented control (its Option type is
  // {value,label}); the hint copy stays on scopeOptions/durationOptions.
  const scopeSegments = scopeOptions.map((option) => ({
    value: option.value,
    label: option.label,
  }));
  const durationSegments = durationOptions.map((option) => ({
    value: option.value,
    label: option.label,
  }));

  const scopeDisabledValues = $derived(
    scopeOptions.filter((option) => scopeDisabled(option.value)).map((option) => option.value),
  );
  const durationDisabledValues = $derived(
    durationOptions
      .filter((option) => durationDisabled(option.value))
      .map((option) => option.value),
  );

  // "Why is this greyed out" signifiers for the disabled choices.
  const scopeLocked = $derived(pendingRequest?.minimumScope === "allRetained");
  const durationLocked = $derived(durationDisabledValues.length > 0);
  const minDurationLabel = $derived(
    durationLabelForSeconds(pendingRequest?.minimumDurationSeconds ?? 0),
  );

  // Format the expiry timestamp for a grant that starts at `baseMs` and lasts
  // `selectedDuration`. Shared by the live label and the approve-time receipt so
  // both read the same wording off the same clock.
  function formatExpiry(baseMs: number) {
    const endsAt = new Date(baseMs + durationSeconds[selectedDuration] * 1000);
    return endsAt.toLocaleString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  // Reference the `now` ticker so the displayed expiry tracks elapsed dwell time
  // rather than freezing at load (it would otherwise under-report by however long
  // the dialog sat open).
  const expiryLabel = $derived(formatExpiry(now));

  // How long this request has been waiting. The hard timeout is CLI-side (~120s),
  // so this is an honesty cue, not a countdown — if the tool stops waiting the
  // approve/deny call surfaces a specific "no longer valid" message instead.
  const requestAgeLabel = $derived.by(() => {
    if (!pendingRequest) return null;
    const createdMs = new Date(pendingRequest.createdAt).getTime();
    if (Number.isNaN(createdMs)) return null;
    const ageSeconds = Math.max(0, Math.round((now - createdMs) / 1000));
    if (ageSeconds < 5) return "Requested just now";
    if (ageSeconds < 60) return `Requested ${ageSeconds}s ago`;
    const minutes = Math.round(ageSeconds / 60);
    return `Requested ${minutes} min ago`;
  });

  onMount(() => {
    void loadPendingRequest();
    // Land focus on a SAFE anchor — never Allow — so a reflexive keypress can't
    // grant access. Prefer Deny once it renders; fall back to the dialog itself.
    void tick().then(() => {
      (denyButton ?? dialogEl)?.focus();
    });
    const ageTimer = setInterval(() => (now = Date.now()), 15000);
    return () => {
      clearInterval(ageTimer);
      clearActionWatchdog();
    };
  });

  async function loadPendingRequest() {
    error = null;
    loadError = null;
    loading = true;
    try {
      pendingRequest = await invoke<PendingCliAccessRequest | null>("get_pending_cli_access_request");
      if (pendingRequest) {
        selectedScope = pendingRequest.preferredScope;
        selectedDuration = durationLabelForSeconds(pendingRequest.preferredDurationSeconds);
      }
    } catch (err) {
      loadError = friendlyError(err);
    } finally {
      loading = false;
    }
  }

  // On success the backend tears this window down; if that never arrives the
  // watchdog re-enables the action and explains, so it can never hang forever.
  function startActionWatchdog(reset: () => void) {
    clearActionWatchdog();
    actionWatchdog = setTimeout(() => {
      actionWatchdog = null;
      // A grant that already resolved Ok is a success even if the window
      // teardown is slow — never overwrite the receipt with a failure.
      if (granted) return;
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

  // Granted-state dismiss. The grant has already landed, so there is no pending
  // request to cancel — closing the window directly (not via
  // cancel_pending_cli_access_request, which would reject with "no pending
  // request") backs the receipt's "Close" affordance in the rare case the
  // backend teardown is delayed or never arrives. If the backend already closed
  // the window first, this button never renders long enough to be pressed.
  async function closeGrantedWindow() {
    try {
      await getCurrentWindow().close();
    } catch {
      // If the window is already being torn down by the backend, the close is a
      // harmless no-op — nothing more to do here.
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

  async function approveAccess() {
    if (!pendingRequest || approving) return;
    error = null;
    approving = true;
    // Snapshot the consent terms now: the receipt must keep naming them even as
    // the backend clears the pending request and tears the window down. Expiry is
    // computed off a FRESH clock at approve time so the receipt reflects what was
    // actually granted, not a stale load-time estimate.
    const receipt = {
      tool: pendingRequest.client.label,
      scopeProse: scopeProse[selectedScope],
      expiryLabel: formatExpiry(Date.now()),
    };
    startActionWatchdog(() => (approving = false));
    try {
      await invoke("approve_pending_cli_access_request", {
        approval: {
          scope: selectedScope,
          durationSeconds: durationSeconds[selectedDuration],
        },
      });
      // The backend resolves Ok only on a real grant. Show the positive receipt
      // and stand the watchdog down so a slow teardown can't report failure.
      // Clear any prior action error before flipping to granted so a stale/
      // watchdog-set error can never render over the success receipt.
      clearActionWatchdog();
      approving = false;
      error = null;
      granted = receipt;
      // The Deny anchor that held focus is gone; follow focus into the receipt's
      // Close button so keyboard users aren't dropped onto <body>.
      void tick().then(() => grantedCloseButton?.focus());
    } catch (err) {
      clearActionWatchdog();
      error = mapActionError(err);
      approving = false;
    }
  }

  function durationLabelForSeconds(seconds: number): "1h" | "24h" | "7d" {
    if (seconds <= durationSeconds["1h"]) return "1h";
    if (seconds <= durationSeconds["24h"]) return "24h";
    return "7d";
  }

  function scopeDisabled(scope: "lastDay" | "allRetained") {
    return pendingRequest?.minimumScope === "allRetained" && scope === "lastDay";
  }

  function durationDisabled(duration: "1h" | "24h" | "7d") {
    return durationSeconds[duration] < (pendingRequest?.minimumDurationSeconds ?? 0);
  }

  function identitySourceLabel(source: string) {
    switch (source) {
      case "explicit":
        return "Identity declared by the tool";
      case "env":
        return "Identity from an environment variable";
      case "inferred":
        return "Identity inferred from the process";
      default:
        return "No identity provided";
    }
  }

  // Weaker provenances (inferred / none) carry less trust than an explicitly
  // declared identity, so their chip is warn-tinted rather than neutral — the
  // chip then reads its trust at a glance, not just by its text. "explicit"
  // stays neutral (never greened — the identity is still unverifiable).
  function identitySourceWarn(source: string) {
    // Inferred, plus every source that falls through to "No identity provided"
    // (none / unknown). "explicit" and "env" stay neutral.
    return source !== "explicit" && source !== "env";
  }

  function friendlyError(err: unknown) {
    if (typeof err === "string") return err;
    if (err instanceof Error && err.message) return err.message;
    return "Something went wrong. Please try again.";
  }

  // The pending request is cleared once the requesting tool stops waiting (its
  // client-side timeout is ~120s) — approve/deny then rejects with the backend's
  // "no pending CLI Access request". Map that to a specific, non-alarming reason
  // instead of leaking the raw string as a generic action failure.
  function mapActionError(err: unknown) {
    const message = friendlyError(err);
    if (/no pending CLI Access request/i.test(message)) {
      return "This request is no longer valid — the requesting tool stopped waiting. You can close this window.";
    }
    return message;
  }

  // Esc denies the request; Enter is intentionally not bound to approve, so
  // a reflexive keypress can never grant access on a consent screen.
  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !approving && !cancelling) {
      event.preventDefault();
      void closeWindow();
      return;
    }
    // Keep Tab focus contained within the consent dialog.
    trapTabKey(event, dialogEl);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<svelte:head>
  <title>mnema · CLI Access</title>
</svelte:head>

<main class="access-request">
  <div
    class="access-dialog"
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="access-dialog-title"
    aria-describedby="access-dialog-lede"
    aria-busy={loading}
    tabindex="-1"
  >
    <header class="access-dialog__header">
      <span class="access-dialog__icon" aria-hidden="true">
        <!-- Neutral padlock (permission) glyph — NOT a green check-shield, which
             would read as "verified/safe" on a request whose identity can't be
             independently verified. -->
        <svg viewBox="0 0 24 24">
          <rect x="5" y="11" width="14" height="9" rx="2" />
          <path d="M8 11V8a4 4 0 0 1 8 0v3" />
        </svg>
      </span>
      <div class="access-dialog__title">
        <p class="eyebrow">CLI Access</p>
        <h1 id="access-dialog-title">Review command-line tool access</h1>
        <p id="access-dialog-lede" class="lede">
          A command-line tool is requesting time-bounded access to your searchable Mnema text.
        </p>
      </div>
    </header>

    <div class="access-dialog__body">
      {#if granted}
        <div class="receipt" role="status" aria-live="polite">
          <span class="receipt__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M20 6 9 17l-5-5" />
            </svg>
          </span>
          <p class="receipt__title">Access granted</p>
          <p class="receipt__body">
            <strong>{granted.tool}</strong> can read
            <strong>{granted.scopeProse}</strong> until <strong>{granted.expiryLabel}</strong>.
          </p>
          <p class="receipt__hint">
            Manage or revoke this in Settings → Data → Access.
          </p>
        </div>
      {:else if loading}
        <span class="sr-only" role="status" aria-live="polite">Loading access request…</span>
        <div class="skeleton" aria-hidden="true">
          <div class="skeleton__line skeleton__line--lg"></div>
          <div class="skeleton__line"></div>
          <div class="skeleton__line skeleton__line--sm"></div>
        </div>
      {:else if loadError}
        <div class="load-error" role="alert">
          <span class="load-error__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0Z" />
              <path d="M12 9v4" />
              <path d="M12 17h.01" />
            </svg>
          </span>
          <p class="load-error__title">Couldn't load the request</p>
          <p class="load-error__body">{loadError}</p>
          <button class="btn btn--ghost load-error__retry" type="button" onclick={() => loadPendingRequest()}>
            Retry
          </button>
        </div>
      {:else if !pendingRequest}
        <div class="empty">
          <span class="empty__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 7v5l3 2" />
            </svg>
          </span>
          <p class="empty__title">No request waiting</p>
          <p class="empty__body">
            There is no pending CLI Access request right now. You can close this window.
          </p>
        </div>
      {:else}
        <section class="requester" aria-label="Requesting tool">
          <div class="requester__head">
            <span class="requester__label" title={pendingRequest.client.label}
              >{pendingRequest.client.label}</span
            >
            <span
              class="requester__source"
              class:requester__source--warn={identitySourceWarn(pendingRequest.client.source)}
              >{identitySourceLabel(pendingRequest.client.source)}</span
            >
          </div>
          <p class="trust-note">
            <span class="trust-note__icon" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0Z" />
                <path d="M12 9v4" />
                <path d="M12 17h.01" />
              </svg>
            </span>
            <span>This identity is reported by the requesting process and cannot be independently verified.</span>
          </p>
          <p class="requester__trigger">
            Requested via <code>mnema {pendingRequest.command}</code>
          </p>
          {#if requestAgeLabel}
            <p class="requester__age">{requestAgeLabel}</p>
          {/if}
        </section>

        <fieldset class="request-section">
          <legend class="group-label">What it can read</legend>
          <Segmented
            options={scopeSegments}
            value={selectedScope}
            disabledValues={scopeDisabledValues}
            disabled={approving || cancelling}
            ariaLabel="What it can read"
            onValueChange={(v) => (selectedScope = v as "lastDay" | "allRetained")}
          />
          {#if scopeLocked}
            <p class="choice-hint">
              <span>This tool requires access to all retained history.</span>
            </p>
          {/if}
          {#if isBroadScope}
            <p class="choice-hint choice-hint--warn">
              <span class="choice-hint__icon" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0Z" />
                  <path d="M12 9v4" />
                  <path d="M12 17h.01" />
                </svg>
              </span>
              <span><strong>Reads your entire history.</strong> {scopeMeta.hint}</span>
            </p>
          {:else}
            <p class="choice-hint">
              <span>{scopeMeta.hint}</span>
            </p>
          {/if}
        </fieldset>

        <fieldset class="request-section">
          <legend class="group-label">For how long</legend>
          <Segmented
            options={durationSegments}
            value={selectedDuration}
            disabledValues={durationDisabledValues}
            disabled={approving || cancelling}
            ariaLabel="For how long"
            onValueChange={(v) => (selectedDuration = v as "1h" | "24h" | "7d")}
          />
          {#if durationLocked}
            <p class="choice-hint">
              <span>This tool requires at least {minDurationLabel} of access.</span>
            </p>
          {/if}
          <p class="choice-hint">
            <span>Access ends {expiryLabel}.</span>
          </p>
        </fieldset>
      {/if}

      {#if error && !granted}
        <div class="inline-error" role="alert">
          <span class="inline-error__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0Z" />
              <path d="M12 9v4" />
              <path d="M12 17h.01" />
            </svg>
          </span>
          <span class="inline-error__msg">{error}</span>
        </div>
      {/if}
    </div>

    {#if granted}
      <!-- The backend closes this window on a real grant, so this footer is
           normally torn down before it can be used. It exists for the rare
           lingering case (delayed/failed teardown) so the granted state is never
           left with zero in-app affordance backing the "Close" copy. -->
      <footer class="access-dialog__actions">
        <button
          class="btn btn--ghost"
          bind:this={grantedCloseButton}
          type="button"
          onclick={closeGrantedWindow}
        >
          Close
        </button>
      </footer>
    {:else}
      <footer class="access-dialog__actions">
        <button
          class="btn btn--ghost"
          bind:this={denyButton}
          type="button"
          disabled={approving || cancelling}
          onclick={closeWindow}
        >
          {#if cancelling}
            <span class="btn__spinner" aria-hidden="true"></span>
            {pendingRequest ? "Denying…" : "Closing…"}
          {:else}
            {pendingRequest ? "Deny" : "Close"}
          {/if}
        </button>
        {#if pendingRequest}
          <button
            class="btn btn--allow"
            type="button"
            disabled={loading || approving || cancelling}
            onclick={approveAccess}
          >
            {#if approving}
              <span class="btn__spinner" aria-hidden="true"></span>
              Allowing…
            {:else if isBroadScope}
              <!-- Escalate the affirmative label to match the broad-scope warning
                   above: the button names the heavier consent it grants. -->
              Allow full-history access
            {:else}
              Allow access
            {/if}
          </button>
        {/if}
      </footer>
    {/if}
  </div>
</main>

<!-- The Tauri window is already the dialog frame, so this route avoids a nested card. -->
<style>
  .access-request {
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
    display: flex;
    overflow: hidden;
    background: var(--app-surface);
    color: var(--app-text);
  }

  .access-dialog {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: transparent;
  }

  .access-dialog__header {
    display: grid;
    grid-template-columns: 30px minmax(0, 1fr);
    gap: 10px;
    padding: 16px 20px 12px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-raised);
  }

  /* Neutral treatment — green is reserved for an actual granted state, not for
     decorating an as-yet-unverified, ungranted request. */
  .access-dialog__icon {
    display: grid;
    width: 30px;
    height: 30px;
    place-items: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
  }

  .access-dialog__icon svg {
    width: 15px;
    height: 15px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .access-dialog__title {
    min-width: 0;
  }

  .access-dialog__body {
    flex: 1 1 auto;
    display: flex;
    min-height: 0;
    flex-direction: column;
    gap: 12px;
    padding: 14px 20px;
    overflow-y: auto;
  }

  .access-dialog__actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 20px 16px;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-raised);
  }

  .eyebrow {
    margin: 0 0 4px;
    color: var(--app-text-subtle);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    line-height: 1.2;
  }

  h1 {
    margin: 0;
    color: var(--app-text-strong);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: 0.02em;
    line-height: 1.2;
  }

  .lede {
    margin: 4px 0 0;
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  /* Requester is the focal point: who is asking, and what they get. */
  .requester {
    display: grid;
    gap: 6px;
    padding: 12px 14px;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface-subtle);
  }

  .requester__head {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    justify-content: space-between;
  }

  /* The requester identity is the load-bearing "who is asking" line on a consent
     screen — never silently clip it. Wrap a long name across lines (and keep the
     full string in title=) rather than ellipsizing it away. */
  .requester__label {
    min-width: 0;
    color: var(--app-text-strong);
    font-size: var(--text-md);
    font-weight: 700;
    letter-spacing: 0.01em;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }

  .requester__source {
    flex-shrink: 0;
    padding: 2px 8px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  /* Lower-trust provenances (inferred / none) are warn-tinted so the chip
     signals its weaker identity at a glance, not by text alone. "explicit" /
     "env" keep the neutral treatment — never greened, since even a declared
     identity stays unverifiable. */
  .requester__source--warn {
    border-color: var(--app-warn-border);
    background: var(--app-warn-bg);
    color: var(--app-warn);
  }

  .requester__trigger {
    margin: 0;
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.55;
  }

  .requester__age {
    margin: 0;
    color: var(--app-text-subtle);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
  }

  .requester code {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--app-surface-hover);
    color: var(--app-text);
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
  }

  /* Verification caveat sits directly under the identity it qualifies, in a
     warn-tinted container — it must visibly outrank a benign hint, not read as
     the faintest line on the screen. */
  .trust-note {
    display: flex;
    gap: 7px;
    align-items: flex-start;
    margin: 0;
    padding: 7px 9px;
    border: 1px solid var(--app-warn-border);
    border-radius: 6px;
    background: var(--app-warn-bg);
  }

  .trust-note > span:last-child {
    color: var(--app-warn);
    font-size: var(--text-sm);
    font-weight: 600;
    line-height: 1.45;
  }

  .trust-note__icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    margin-top: 1px;
    color: var(--app-warn);
  }

  .trust-note__icon svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .request-section {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 0;
    border: 0;
  }

  .request-section + .request-section {
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
  }

  .group-label {
    padding: 0;
    /* Load-bearing scope/duration legends on a consent screen — must clear the
       contrast floor, so use the legible muted token, not faint subtle. */
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .choice-hint {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0;
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.4;
  }

  /* The broad-scope warning must visibly outrank the benign expiry note:
     warn-tinted container + a bold lead-in, not the same faint hint text. */
  .choice-hint--warn {
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--app-warn-border);
    border-radius: 6px;
    background: var(--app-warn-bg);
    color: var(--app-warn);
    font-size: var(--text-base);
  }

  .choice-hint--warn strong {
    color: var(--app-warn);
    font-weight: 800;
  }

  .choice-hint__icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--app-warn);
  }

  .choice-hint__icon svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .skeleton {
    display: grid;
    gap: 12px;
    padding: 4px 0;
  }

  .skeleton__line {
    height: 12px;
    border-radius: 5px;
    background: var(--app-surface-hover);
    animation: skeleton-pulse 1.4s ease-in-out infinite;
  }

  .skeleton__line--lg {
    height: 18px;
    width: 62%;
  }

  .skeleton__line--sm {
    width: 42%;
  }

  @keyframes skeleton-pulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 0.8;
    }
  }

  .empty {
    display: grid;
    gap: 8px;
    justify-items: center;
    margin: auto 0;
    padding: 24px 16px;
    text-align: center;
  }

  .empty__icon {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    color: var(--app-text-subtle);
  }

  .empty__icon svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .empty__title {
    margin: 0;
    color: var(--app-text);
    font-size: var(--text-md);
    font-weight: 700;
  }

  .empty__body {
    margin: 0;
    max-width: 34ch;
    color: var(--app-text-muted);
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  /* Positive granted receipt — this is the one moment green is earned (an actual
     grant just landed), so it uses the accent rather than the neutral padlock. */
  .receipt {
    display: grid;
    gap: 8px;
    justify-items: center;
    margin: auto 0;
    padding: 24px 16px;
    text-align: center;
  }

  .receipt__icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }

  .receipt__icon svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .receipt__title {
    margin: 0;
    color: var(--app-text-strong);
    font-size: var(--text-md);
    font-weight: 700;
  }

  .receipt__body {
    margin: 0;
    max-width: 38ch;
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.5;
  }

  .receipt__body strong {
    color: var(--app-text);
    font-weight: 700;
  }

  .receipt__hint {
    margin: 0;
    max-width: 36ch;
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  /* Visible only to assistive tech (announces the loading state). */
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

  /* Load failure gets its own recoverable state (Retry) instead of the
     contradictory "nothing pending" empty copy. */
  .load-error {
    display: grid;
    gap: 8px;
    justify-items: center;
    margin: auto 0;
    padding: 24px 16px;
    text-align: center;
  }

  .load-error__icon {
    display: grid;
    place-items: center;
    color: var(--app-danger);
  }

  .load-error__icon svg {
    width: 22px;
    height: 22px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .load-error__title {
    margin: 0;
    color: var(--app-text);
    font-size: var(--text-md);
    font-weight: 700;
  }

  .load-error__body {
    margin: 0;
    max-width: 34ch;
    color: var(--app-text-muted);
    font-size: var(--text-sm);
    line-height: 1.45;
    word-break: break-word;
  }

  .load-error__retry {
    margin-top: 4px;
  }

  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 6px;
  }

  .inline-error__icon {
    display: grid;
    place-items: center;
    color: var(--app-danger);
    flex-shrink: 0;
    margin-top: 1px;
  }

  .inline-error__icon svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .inline-error__msg {
    flex: 1;
    color: var(--app-danger-text);
    font-size: var(--text-sm);
    line-height: 1.5;
    word-break: break-word;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    min-height: 30px;
    padding: 7px 12px;
    border-radius: 4px;
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s, transform 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  /* One focus-ring treatment for BOTH footer buttons (accent border + glow) so
     keyboard focus reads identically as the user tabs Deny ↔ Allow. */
  .btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .btn:active:not(:disabled) {
    transform: translateY(0.5px);
  }

  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  /* Allow GRANTS irreversible access, so it does NOT get a go-green solid fill
     that reads as "the safe path". It's an accent-tinted OUTLINE: clearly the
     affirmative action, but with no more visual pull than Deny encourages a
     deliberate choice rather than a reflexive grant. */
  .btn--allow {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  /* Multi-property hover (subtle bg lift + brighter border) so the affirmative
     action gives the same depth of feedback as Deny, while the resting state
     stays a restrained outline (no go-green solid fill). */
  .btn--allow:not(:disabled):hover {
    background: color-mix(in srgb, var(--app-accent) 10%, var(--app-accent-bg));
    color: var(--app-accent);
    border-color: var(--app-accent);
  }

  /* In-flight spinner reuses the file's keyframes vocabulary (see @keyframes spin). */
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

  /* Respect the OS "reduce motion" preference: hold the skeleton at a static
     opacity and slow the in-flight spinner to a near-still crawl. */
  @media (prefers-reduced-motion: reduce) {
    .skeleton__line {
      animation: none;
      opacity: 0.6;
    }

    .btn__spinner {
      animation-duration: 2.4s;
    }

    .btn:active:not(:disabled) {
      transform: none;
    }
  }

  @media (max-width: 480px) {
    .access-request {
      overflow: hidden;
    }

    .access-dialog {
      width: 100%;
    }

    .access-dialog__actions {
      justify-content: stretch;
    }

    .btn {
      flex: 1 1 100%;
    }
  }
</style>
