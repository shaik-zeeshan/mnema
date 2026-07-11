<script lang="ts">
  import Segmented from "$lib/components/Segmented.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import FieldWarning from "$lib/components/FieldWarning.svelte";
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

<!-- A single column so the segmented control and the custom input stack
     vertically regardless of how the parent lays its children out. -->
<div class="bitrate-control">
  <Segmented
    value={mode === "custom" ? "custom" : preset}
    onValueChange={(v) => {
      if (v === "custom") {
        mode = "custom";
      } else {
        mode = "preset";
        preset = v as VideoBitratePreset;
      }
    }}
    ariaLabel="Video bitrate mode"
    options={[
      { value: "low", label: "Low · ~3 Mbps" },
      { value: "medium", label: "Medium · ~8 Mbps" },
      { value: "high", label: "High · ~20 Mbps" },
      { value: "custom", label: "Custom" },
    ]}
  />

  {#if mode === "custom"}
    <div class="custom-bitrate-row">
      <div class="custom-res-field">
        <label class="custom-res-label" for="bitrate-mbps">Bitrate (Mbps)</label>
        <div class="custom-bitrate-input-wrap">
          <Stepper id="bitrate-mbps" bind:value={customMbpsRaw} min={1} max={40} unit="Mbps" placeholder="e.g. 12" ariaLabel="Bitrate (Mbps)" invalid={customErrors.length > 0} errorId="bitrate-custom-error" />
        </div>
      </div>
    </div>
    <FieldWarning id="bitrate-custom-error" messages={customErrors} />

    {#if customErrors.length === 0 && customMbps !== null}
      <p class="group-hint">Custom bitrate: <strong>{customMbps} Mbps</strong>.</p>
    {/if}
  {/if}
</div>

<style>
  .bitrate-control {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }

  .custom-bitrate-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    min-width: 0;
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

  .group-hint {
    margin: 0;
    color: var(--app-text-faint);
    font-size: 10px;
    letter-spacing: 0.03em;
    line-height: 1.5;
  }

  @media (max-width: 480px) {
    .custom-bitrate-input-wrap {
      max-width: none;
    }
  }
</style>
