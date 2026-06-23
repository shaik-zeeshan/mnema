<script lang="ts">
  import Stepper from "$lib/components/Stepper.svelte";
  import type { VideoBitrateMode, VideoBitratePreset } from "$lib/types";

  let {
    mode = $bindable<VideoBitrateMode>("preset"),
    preset = $bindable<VideoBitratePreset>("medium"),
    customMbpsRaw = $bindable(""),
    customMbps = null,
    customErrors = [],
  }: {
    mode: VideoBitrateMode;
    preset: VideoBitratePreset;
    customMbpsRaw: string;
    customMbps?: number | null;
    customErrors?: string[];
  } = $props();
</script>

<div class="bitrate-mode-chips">
  {#each (["low", "medium", "high"] as const) as candidate}
    {@const meta = { low: { mbps: "~3", hint: "Lower quality, smallest file" }, medium: { mbps: "~8", hint: "Balanced quality and size" }, high: { mbps: "~20", hint: "High quality, larger file" } }[candidate]}
    <button
      type="button"
      class="bitrate-chip"
      class:bitrate-chip--active={mode === "preset" && preset === candidate}
      onclick={() => { mode = "preset"; preset = candidate; }}
    >
      <span class="bitrate-chip__label">{candidate}</span>
      <span class="bitrate-chip__mbps">{meta.mbps} Mbps</span>
    </button>
  {/each}
  <button
    type="button"
    class="bitrate-chip"
    class:bitrate-chip--active={mode === "custom"}
    onclick={() => { mode = "custom"; }}
  >
    <span class="bitrate-chip__label">Custom</span>
    <span class="bitrate-chip__mbps">1-40 Mbps</span>
  </button>
</div>

{#if mode === "custom"}
  <div class="custom-bitrate-row">
    <div class="custom-res-field">
      <label class="custom-res-label" for="bitrate-mbps">Bitrate (Mbps, whole number)</label>
      <div class="custom-bitrate-input-wrap">
        <Stepper id="bitrate-mbps" bind:value={customMbpsRaw} min={1} max={40} unit="Mbps" placeholder="e.g. 12" ariaLabel="bitrate in Mbps" invalid={customErrors.length > 0} />
      </div>
    </div>
  </div>

  {#if customErrors.length > 0}
    <div class="inline-validation">
      {#each customErrors as err}
        <p class="inline-validation__item"><span class="inline-validation__icon">!</span>{err}</p>
      {/each}
    </div>
  {:else if customMbps !== null}
    <p class="group-hint">Custom bitrate: <strong>{customMbps} Mbps</strong>.</p>
  {/if}
{/if}

<style>
  .bitrate-mode-chips {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
  }

  .bitrate-chip {
    display: flex;
    min-width: 0;
    min-height: 58px;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 3px;
    padding: 10px 12px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font: inherit;
    cursor: pointer;
    text-align: left;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .bitrate-chip:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text);
  }

  .bitrate-chip--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .bitrate-chip:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .bitrate-chip__label {
    color: var(--app-text);
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .bitrate-chip--active .bitrate-chip__label {
    color: var(--app-accent);
  }

  .bitrate-chip__mbps {
    color: var(--app-text-faint);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    line-height: 1.35;
  }

  .bitrate-chip--active .bitrate-chip__mbps {
    color: var(--app-accent-strong);
  }

  .custom-bitrate-row {
    margin-top: 8px;
  }

  .custom-res-field {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 6px;
  }

  .custom-res-label {
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .custom-bitrate-input-wrap {
    max-width: 240px;
  }

  .inline-validation {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 8px 10px;
    margin-top: 8px;
    border: 1px solid var(--app-warn-border);
    border-radius: 4px;
    background: var(--app-warn-bg);
  }

  .inline-validation__item {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    color: var(--app-warn);
    font-size: 10px;
    line-height: 1.45;
  }

  .inline-validation__icon {
    flex: 0 0 auto;
    font-weight: 800;
  }

  .group-hint {
    margin-top: 8px;
    color: var(--app-text-faint);
    font-size: 10px;
    letter-spacing: 0.03em;
    line-height: 1.5;
  }

  @media (max-width: 720px) {
    .bitrate-mode-chips {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  @media (max-width: 480px) {
    .bitrate-mode-chips {
      grid-template-columns: 1fr;
    }

    .custom-bitrate-input-wrap {
      max-width: none;
    }
  }
</style>
