<script lang="ts">
  // What a model costs, under its picker — round-4 decision G8.
  //
  // Two parts, both computed from facts this machine actually reports:
  //  • a VERDICT CHIP (direction 04's `.badge`): the download size against this
  //    Mac's physical RAM. `blocked` is handed back through `onFit` so the
  //    caller can disable its own Use/Download button on a red verdict.
  //  • one line of prose: the same size against free disk AND RAM.
  //
  // `byteSize` comes from the model-status DTO, i.e. the crate manifests, which
  // are the corrected registry G8 requires (speakrs is 419,482,724 bytes there,
  // asserted against its 76-file table by a test in `crates/speaker-analysis`).
  // Nothing is re-declared here.
  //
  // Renders nothing when the model is OS-managed (no download) or when neither
  // machine limit could be measured — a missing denominator is the designed
  // outcome, not a placeholder.
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { modelFit, modelFootprint } from "$lib/settings/state/system-facts";
  import { formatBytes } from "$lib/settings/state/format";

  interface Props {
    /** The model's download size in bytes; `null`/0 for OS-managed models. */
    byteSize: number | null | undefined;
    /** Called with the fit verdict so the caller can gate its Use button. */
    onFit?: (blocked: boolean) => void;
  }

  let { byteSize, onFit }: Props = $props();

  void systemFacts.ensureLoaded();
  const line = $derived(modelFootprint(systemFacts.value, byteSize ?? null));
  const fit = $derived(modelFit(systemFacts.value, byteSize ?? null));

  $effect(() => {
    onFit?.(fit?.blocked ?? false);
  });
</script>

{#if fit || line}
  <div class="model-footprint">
    {#if fit}
      <span class="mline">
        <span class="badge">{formatBytes(byteSize ?? 0)}</span>
        <span class="badge badge--{fit.tone}">{fit.label}</span>
      </span>
    {/if}
    {#if line}
      <p class="group-hint">{line}</p>
    {/if}
  </div>
{/if}

<style>
  .model-footprint {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
