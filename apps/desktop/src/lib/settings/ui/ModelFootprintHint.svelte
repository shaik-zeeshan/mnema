<script lang="ts">
  // One line under a model picker: what the download costs against the two real
  // limits on this machine (free disk, physical RAM) — round-4 decision G8 —
  // plus the direction's fit VERDICT, which is the actual output of that
  // comparison. "The verdict is the output; the gigabytes are the evidence."
  //
  // `byteSize` comes from the model-status DTO, i.e. the crate manifests, which
  // are the corrected registry G8 requires (speakrs is 419,482,724 bytes there,
  // asserted against its 76-file table by a test in `crates/speaker-analysis`).
  // Nothing is re-declared here.
  //
  // Renders nothing when the model is OS-managed (no download) or when neither
  // machine limit could be measured — no denominator, no claim.
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { modelFitVerdict, modelFootprint } from "$lib/settings/state/system-facts";

  interface Props {
    /** The model's download size in bytes; `null`/0 for OS-managed models. */
    byteSize: number | null | undefined;
  }

  let { byteSize }: Props = $props();

  void systemFacts.ensureLoaded();
  const line = $derived(modelFootprint(systemFacts.value, byteSize ?? null));
  const verdict = $derived(modelFitVerdict(systemFacts.value, byteSize ?? null));
</script>

{#if line || verdict}
  <p class="footprint">
    {#if verdict}
      <!-- The verdict chips are the only semantic colour on this surface. -->
      <span class="ss-chip ss-chip--{verdict.tone}">{verdict.label}</span>
    {/if}
    {#if line}<span class="group-hint">{line}</span>{/if}
  </p>
{/if}

<style>
  .footprint {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
</style>
