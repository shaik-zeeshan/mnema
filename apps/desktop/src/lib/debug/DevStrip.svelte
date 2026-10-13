<!--
  The developer window's strip: lights + drag region from WindowStrip, title
  "Developer", and the right-aligned actions. F1a wires Pause; pin, mini mode
  and the ⋯ menu join here in F1b.
-->
<script lang="ts">
  import WindowStrip from "$lib/components/WindowStrip.svelte";
  import { tip } from "$lib/components/tooltip";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import { getDebugController } from "./state/controller.svelte";

  const controller = getDebugController();
  const label = $derived(controller.live ? "Pause live updates" : "Resume live updates");
</script>

<WindowStrip title="Developer">
  {#snippet actions()}
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm dv-pause"
      aria-pressed={!controller.live}
      aria-label={label}
      use:tip={label}
      onclick={controller.toggleLive}
    >
      {#if controller.live}
        <IconPause width="12" height="12" aria-hidden="true" /><span class="dv-live"></span>
      {:else}
        <IconPlay width="12" height="12" aria-hidden="true" />
      {/if}
    </button>
  {/snippet}
</WindowStrip>
