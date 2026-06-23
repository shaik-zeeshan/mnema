<script lang="ts">
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import type { ResolutionMode, ResolutionPreset } from "$lib/types";

  let {
    mode = $bindable<ResolutionMode>("original"),
    preset = $bindable<ResolutionPreset>("1080p"),
    widthRaw = $bindable(""),
    heightRaw = $bindable(""),
    disabledValues = [],
    customErrors = [],
  }: {
    mode: ResolutionMode;
    preset: ResolutionPreset;
    widthRaw: string;
    heightRaw: string;
    disabledValues?: ResolutionMode[];
    customErrors?: string[];
  } = $props();
</script>

<RadioGroup
  bind:value={mode}
  {disabledValues}
  options={[
    { value: "original", label: "Original", description: "Capture at the display's native resolution" },
    { value: "preset", label: "Preset", description: "Select a standard output resolution" },
    { value: "custom", label: "Custom", description: "Enter exact width and height in pixels" },
  ]}
/>

{#if mode === "preset"}
  <div class="resolution-preset-grid">
    {#each (["1080p", "720p", "540p"] as const) as candidate}
      {@const meta = { "1080p": { w: 1920, h: 1080 }, "720p": { w: 1280, h: 720 }, "540p": { w: 960, h: 540 } }[candidate]}
      <button
        class="preset-chip"
        class:preset-chip--active={preset === candidate}
        onclick={() => { preset = candidate; }}
        type="button"
      >
        <span class="preset-chip__label">{candidate}</span>
        <span class="preset-chip__dim">{meta.w}x{meta.h}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .resolution-preset-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    margin-top: 8px;
  }

  .preset-chip {
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

  .preset-chip:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text);
  }

  .preset-chip--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .preset-chip:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .preset-chip__label {
    color: var(--app-text);
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .preset-chip--active .preset-chip__label {
    color: var(--app-accent);
  }

  .preset-chip__dim {
    color: var(--app-text-faint);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
  }

  .preset-chip--active .preset-chip__dim {
    color: var(--app-accent-strong);
  }

  .custom-resolution-inputs {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: end;
    gap: 8px;
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

  .custom-res-sep {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 34px;
    color: var(--app-text-faint);
    font-size: 12px;
    font-weight: 800;
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

  @media (max-width: 640px) {
    .resolution-preset-grid {
      grid-template-columns: 1fr;
    }

    .custom-resolution-inputs {
      grid-template-columns: 1fr;
    }

    .custom-res-sep {
      display: none;
    }
  }
</style>

{#if mode === "custom"}
  <div class="custom-resolution-inputs">
    <div class="custom-res-field">
      <label class="custom-res-label" for="res-width">Width (px)</label>
      <Stepper id="res-width" bind:value={widthRaw} min={16} max={8192} placeholder="e.g. 1920" ariaLabel="width" invalid={customErrors.length > 0} />
    </div>
    <span class="custom-res-sep" aria-hidden="true">x</span>
    <div class="custom-res-field">
      <label class="custom-res-label" for="res-height">Height (px)</label>
      <Stepper id="res-height" bind:value={heightRaw} min={16} max={8192} placeholder="e.g. 1080" ariaLabel="height" invalid={customErrors.length > 0} />
    </div>
  </div>

  {#if customErrors.length > 0}
    <div class="inline-validation">
      {#each customErrors as err}
        <p class="inline-validation__item"><span class="inline-validation__icon">!</span>{err}</p>
      {/each}
    </div>
  {/if}
{/if}
