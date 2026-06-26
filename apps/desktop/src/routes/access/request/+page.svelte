<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
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
  let error = $state<string | null>(null);
  let loading = $state(true);
  let approving = $state(false);
  let cancelling = $state(false);

  // Bound to the dialog container and the Deny button so focus can land on a
  // SAFE anchor on open, and Tab is contained inside the consent dialog.
  let dialogEl = $state<HTMLDivElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);

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

  const expiryLabel = $derived.by(() => {
    const endsAt = new Date(Date.now() + durationSeconds[selectedDuration] * 1000);
    return endsAt.toLocaleString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  });

  onMount(() => {
    void loadPendingRequest();
    // Land focus on a SAFE anchor — never Allow — so a reflexive keypress can't
    // grant access. Prefer Deny once it renders; fall back to the dialog itself.
    void tick().then(() => {
      (denyButton ?? dialogEl)?.focus();
    });
  });

  async function loadPendingRequest() {
    error = null;
    loading = true;
    try {
      pendingRequest = await invoke<PendingCliAccessRequest | null>("get_pending_cli_access_request");
      if (pendingRequest) {
        selectedScope = pendingRequest.preferredScope;
        selectedDuration = durationLabelForSeconds(pendingRequest.preferredDurationSeconds);
      }
    } catch (err) {
      error = friendlyError(err);
    } finally {
      loading = false;
    }
  }

  async function closeWindow() {
    error = null;
    cancelling = true;
    try {
      await invoke("cancel_pending_cli_access_request");
    } catch (err) {
      error = friendlyError(err);
      cancelling = false;
    }
  }

  async function approveAccess() {
    if (!pendingRequest) return;
    error = null;
    approving = true;
    try {
      await invoke("approve_pending_cli_access_request", {
        approval: {
          scope: selectedScope,
          durationSeconds: durationSeconds[selectedDuration],
        },
      });
    } catch (err) {
      error = friendlyError(err);
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

  function friendlyError(err: unknown) {
    if (typeof err === "string") return err;
    if (err instanceof Error && err.message) return err.message;
    return "Something went wrong. Please try again.";
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
    tabindex="-1"
  >
    <header class="access-dialog__header">
      <span class="access-dialog__icon" aria-hidden="true">
        <svg viewBox="0 0 24 24">
          <path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6Z" />
          <path d="M9.5 12.5 11.2 14 15 10" />
        </svg>
      </span>
      <div class="access-dialog__title">
        <p class="eyebrow">CLI Access</p>
        <h1 id="access-dialog-title">Review local tool access</h1>
        <p id="access-dialog-lede" class="lede">
          A local tool is requesting time-bounded access to your searchable Mnema text.
        </p>
      </div>
    </header>

    <div class="access-dialog__body">
      {#if loading}
        <div class="skeleton" aria-hidden="true">
          <div class="skeleton__line skeleton__line--lg"></div>
          <div class="skeleton__line"></div>
          <div class="skeleton__line skeleton__line--sm"></div>
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
            <span class="requester__label">{pendingRequest.client.label}</span>
            <span class="requester__source">{identitySourceLabel(pendingRequest.client.source)}</span>
          </div>
          <p class="requester__trigger">
            Requested via <code>mnema {pendingRequest.command}</code>
          </p>
        </section>

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

      {#if error}
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
          {:else}
            Allow access
          {/if}
        </button>
      {/if}
    </footer>
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

  .access-dialog__icon {
    display: grid;
    width: 30px;
    height: 30px;
    place-items: center;
    border: 1px solid var(--app-accent-border);
    border-radius: 8px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
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
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    line-height: 1.2;
  }

  h1 {
    margin: 0;
    color: var(--app-text-strong);
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.02em;
    line-height: 1.2;
  }

  .lede {
    margin: 4px 0 0;
    color: var(--app-text-muted);
    font-size: 11px;
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
    align-items: center;
    gap: 10px;
    justify-content: space-between;
  }

  .requester__label {
    min-width: 0;
    overflow: hidden;
    color: var(--app-text-strong);
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.01em;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .requester__trigger {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.55;
  }

  .requester code {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--app-surface-hover);
    color: var(--app-text);
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
  }

  .trust-note {
    display: flex;
    gap: 7px;
    align-items: flex-start;
    margin: 0;
  }

  .trust-note > span:last-child {
    color: var(--app-text-muted);
    font-size: var(--text-sm);
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
    font-size: 10px;
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
    font-size: var(--text-sm);
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
    font-size: 11px;
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
    font-size: 13px;
    font-weight: 700;
  }

  .empty__body {
    margin: 0;
    max-width: 34ch;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.45;
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
    font-size: 11px;
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
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s, transform 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 2px;
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

  .btn--allow:not(:disabled):hover {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent);
  }

  .btn--allow:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
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
