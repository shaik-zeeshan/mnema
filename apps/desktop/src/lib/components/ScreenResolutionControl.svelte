<script lang="ts">
  import Segmented from "$lib/components/Segmented.svelte";
  import Input from "$lib/components/Input.svelte";
  import FieldWarning from "$lib/components/FieldWarning.svelte";
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
    disabledValues?: string[];
    customErrors?: string[];
  } = $props();
</script>

<!-- A single column so the segmented control and the custom inputs stack
     vertically regardless of how the parent lays its children out. -->
<div class="resolution-control">
  <!-- Original, the scaled presets, and Custom collapse into one segmented
       control. Selecting a preset writes both mode and preset; "Custom" flips
       mode to custom and reveals the width/height inputs below. -->
  <Segmented
    value={mode === "custom" ? "custom" : mode === "original" ? "original" : preset}
    onValueChange={(v) => {
      if (v === "custom") {
        mode = "custom";
      } else if (v === "original") {
        mode = "original";
      } else {
        mode = "preset";
        preset = v as ResolutionPreset;
      }
    }}
    ariaLabel="Screen resolution"
    {disabledValues}
    options={[
      { value: "original", label: "Original" },
      { value: "1080p", label: "1080p" },
      { value: "720p", label: "720p" },
      { value: "540p", label: "540p" },
      { value: "custom", label: "Custom" },
    ]}
  />

  {#if mode === "custom"}
    <div class="custom-resolution-inputs">
      <div class="custom-res-field">
        <label class="custom-res-label" for="res-width">Width (px)</label>
        <Input id="res-width" bind:value={widthRaw} inputmode="numeric" placeholder="e.g. 1920" invalid={customErrors.length > 0} errorId="res-custom-error" />
      </div>
      <span class="custom-res-sep" aria-hidden="true">×</span>
      <div class="custom-res-field">
        <label class="custom-res-label" for="res-height">Height (px)</label>
        <Input id="res-height" bind:value={heightRaw} inputmode="numeric" placeholder="e.g. 1080" invalid={customErrors.length > 0} errorId="res-custom-error" />
      </div>
    </div>
    <FieldWarning id="res-custom-error" messages={customErrors} />
  {/if}
</div>

<style>
  .resolution-control {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }

  .custom-resolution-inputs {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    min-width: 0;
    flex-wrap: wrap;
  }

  .custom-res-field {
    display: flex;
    min-width: 0;
    flex: 1 1 120px;
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
</style>
