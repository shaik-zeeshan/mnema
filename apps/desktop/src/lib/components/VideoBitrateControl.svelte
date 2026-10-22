<script lang="ts">
  import SelectMenu from "$lib/components/Select.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import FieldWarning from "$lib/components/FieldWarning.svelte";
  import type { VideoBitrateMode, VideoBitratePreset } from "$lib/types";

  let {
    mode = $bindable<VideoBitrateMode>("preset"),
    preset = $bindable<VideoBitratePreset>("medium"),
    customMbpsRaw = $bindable(""),
    customErrors = [],
  }: {
    mode: VideoBitrateMode;
    preset: VideoBitratePreset;
    customMbpsRaw: string;
    customErrors?: string[];
  } = $props();
</script>

<!-- Kit (settings.html Bitrate row): one select with the presets + "Custom…";
     Custom reveals a Mbps stepper beside it. -->
<div class="bitrate-control">
  <div class="bitrate-control__row">
    <SelectMenu
      value={mode === "custom" ? "custom" : preset}
      onValueChange={(v) => {
        if (v === "custom") {
          mode = "custom";
        } else {
          mode = "preset";
          preset = v as VideoBitratePreset;
        }
      }}
      ariaLabel="Bitrate"
      options={[
        { value: "low", label: "Low · ~3 Mbps" },
        { value: "medium", label: "Medium · ~8 Mbps" },
        { value: "high", label: "High · ~20 Mbps" },
        { value: "custom", label: "Custom…" },
      ]}
    />
    {#if mode === "custom"}
      <div class="bitrate-control__mbps">
        <Stepper id="bitrate-mbps" bind:value={customMbpsRaw} min={1} max={40} unit="Mbps" placeholder="e.g. 12" ariaLabel="Bitrate (Mbps)" invalid={customErrors.length > 0} errorId="bitrate-custom-error" />
      </div>
    {/if}
  </div>
  {#if mode === "custom"}
    <FieldWarning id="bitrate-custom-error" messages={customErrors} />
  {/if}
</div>

<style>
  .bitrate-control {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }

  .bitrate-control__row {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }

  .bitrate-control__row > :global(.select-wrapper) {
    width: 220px;
  }

  .bitrate-control__mbps {
    width: 190px;
  }
</style>
