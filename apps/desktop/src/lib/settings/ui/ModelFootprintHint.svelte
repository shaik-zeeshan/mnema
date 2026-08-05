<script lang="ts">
  // Custom input 3 of 5 — the model row's machine facts.
  //
  // A model is not chosen by its parameter count; it is chosen by whether it
  // FITS ON THIS MACHINE. So this renders two things under every model picker
  // (OCR, transcription, speakers, semantic search):
  //   • the mono meta line — download size against free disk and physical RAM,
  //   • the fit verdict for those two numbers, as a verdict chip.
  //
  // Both come from `get_system_facts` and the crate manifests, which ARE the
  // corrected registry G8 requires (speakrs is 419,482,724 bytes there,
  // asserted against its 76-file table by a test in `crates/speaker-analysis`).
  // Nothing is re-declared here, and nothing is claimed that isn't measured —
  // no speed, no battery cost, no temperature.
  //
  // Renders nothing when the model is OS-managed / cloud (no download) or when
  // neither machine limit could be measured.
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { modelFitVerdict, modelFootprint } from "$lib/settings/state/system-facts";

  interface Props {
    /** The model's download size in bytes; `null`/0 for OS-managed models. */
    byteSize: number | null | undefined;
  }

  let { byteSize }: Props = $props();

  void systemFacts.ensureLoaded();
  const line = $derived(modelFootprint(systemFacts.value, byteSize ?? null));
  const verdict = $derived(modelFitVerdict(systemFacts.value, byteSize));
</script>

{#if line || verdict}
  <div class="model-footprint">
    {#if line}<span class="mrow__meta">{line}</span>{/if}
    {#if verdict}
      <span class="chip chip--verdict chip--{verdict.tone}">{verdict.label}</span>
    {/if}
  </div>
{/if}

<style>
  /* The meta line and its verdict share one baseline: the numbers on the left,
     the judgement on the right — the only place three semantic colours appear
     in Settings, and they are text, not decoration. */
  .model-footprint {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    flex-wrap: wrap;
    min-width: 0;
  }

  .model-footprint .mrow__meta {
    min-width: 0;
  }
</style>
