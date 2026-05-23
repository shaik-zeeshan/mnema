<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

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
  let approving = $state(false);
  let cancelling = $state(false);

  const durationSeconds = {
    "1h": 60 * 60,
    "24h": 24 * 60 * 60,
    "7d": 7 * 24 * 60 * 60,
  } as const;

  const scopeOptions = [
    {
      value: "lastDay",
      label: "Last day",
    },
    {
      value: "allRetained",
      label: "All retained",
    },
  ] as const;

  const durationOptions = [
    {
      value: "1h",
      label: "1h",
    },
    {
      value: "24h",
      label: "24h",
    },
    {
      value: "7d",
      label: "7d",
    },
  ] as const;

  onMount(() => {
    void loadPendingRequest();
  });

  async function loadPendingRequest() {
    error = null;
    try {
      pendingRequest = await invoke<PendingCliAccessRequest | null>("get_pending_cli_access_request");
      if (pendingRequest) {
        selectedScope = pendingRequest.preferredScope;
        selectedDuration = durationLabelForSeconds(pendingRequest.preferredDurationSeconds);
      }
    } catch (err) {
      error = typeof err === "string" ? err : JSON.stringify(err, null, 2);
    }
  }

  async function closeWindow() {
    error = null;
    cancelling = true;
    try {
      await invoke("cancel_pending_cli_access_request");
    } catch (err) {
      error = typeof err === "string" ? err : JSON.stringify(err, null, 2);
      cancelling = false;
    }
  }

  async function approveAccess() {
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
      error = typeof err === "string" ? err : JSON.stringify(err, null, 2);
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

  function commandLabel(command: string) {
    if (command === "show-text") return "Show text";
    if (command === "access request") return "Access request";
    return command.slice(0, 1).toUpperCase() + command.slice(1);
  }
</script>

<svelte:head>
  <title>mnema · CLI Access</title>
</svelte:head>

<main class="access-request">
  <div class="access-dialog" role="dialog" aria-modal="false" aria-labelledby="access-dialog-title">
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
        <p class="lede">Approve a time-bounded grant for searchable Mnema text.</p>
      </div>
    </header>

    <div class="access-dialog__body">
      <div class="request-summary">
        {#if pendingRequest}
          <p><strong>{pendingRequest.client.label}</strong> wants access for {commandLabel(pendingRequest.command)}.</p>
          <p>Identity source: {pendingRequest.client.source}. Local client identity is provided by the requesting process and cannot be independently verified.</p>
        {:else}
          <p>No pending CLI Access request is waiting.</p>
        {/if}
      </div>

      <fieldset class="request-section">
        <legend class="group-label">Scope</legend>
        <div class="choice-row choice-row--scope" role="radiogroup" aria-label="Access scope">
          {#each scopeOptions as option}
            {@const disabled = scopeDisabled(option.value) || !pendingRequest || approving || cancelling}
            <button
              class="choice"
              class:choice--active={selectedScope === option.value}
              type="button"
              role="radio"
              aria-checked={selectedScope === option.value}
              disabled={disabled}
              onclick={() => { if (!disabled) selectedScope = option.value; }}
            >
              <span class="choice__label">{option.label}</span>
            </button>
          {/each}
        </div>
      </fieldset>

      <fieldset class="request-section">
        <legend class="group-label">Duration</legend>
        <div class="choice-row choice-row--duration" role="radiogroup" aria-label="Access duration">
          {#each durationOptions as option}
            {@const disabled = durationDisabled(option.value) || !pendingRequest || approving || cancelling}
            <button
              class="choice"
              class:choice--active={selectedDuration === option.value}
              type="button"
              role="radio"
              aria-checked={selectedDuration === option.value}
              disabled={disabled}
              onclick={() => { if (!disabled) selectedDuration = option.value; }}
            >
              <span class="choice__label">{option.label}</span>
            </button>
          {/each}
        </div>
      </fieldset>

      {#if error}
        <div class="inline-error" role="alert">
          <span class="inline-error__icon">!</span>
          <span class="inline-error__msg">{error}</span>
        </div>
      {/if}
    </div>

    <footer class="access-dialog__actions">
      <button class="btn btn--ghost" type="button" disabled={approving || cancelling} onclick={closeWindow}>
        {cancelling ? "Cancelling" : "Cancel"}
      </button>
      <button class="btn btn--primary" type="button" disabled={!pendingRequest || approving || cancelling} onclick={approveAccess}>
        {approving ? "Allowing" : `Allow ${selectedScope === "lastDay" ? "last day" : "all retained"} for ${selectedDuration}`}
      </button>
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
    overflow: hidden;
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

  .request-summary {
    display: grid;
    gap: 4px;
    padding: 8px 0 0;
    border: 1px solid var(--app-warn-border);
    border-width: 1px 0 0;
    background: transparent;
  }

  .request-summary p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.42;
  }

  .request-summary strong {
    color: var(--app-text);
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
    color: var(--app-text-subtle);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .choice-row {
    display: grid;
    gap: 6px;
  }

  .choice-row--scope {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .choice-row--duration {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .choice {
    min-width: 0;
    min-height: 34px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 7px 8px;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s;
  }

  .choice:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text);
  }

  .choice:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 2px;
  }

  .choice:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .choice--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .choice__label {
    overflow: hidden;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 1.2;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
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
    color: var(--app-danger);
    font-size: 11px;
    flex-shrink: 0;
    margin-top: 1px;
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
    transition: background 0.12s, border-color 0.12s, color 0.12s, opacity 0.12s;
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

  .btn--primary {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }

  .btn--primary:not(:disabled):hover {
    border-color: var(--app-accent);
    color: var(--app-text-strong);
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
