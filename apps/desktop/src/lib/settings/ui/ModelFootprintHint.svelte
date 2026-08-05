<script lang="ts">
  // One line under a model picker: what the download costs against the two real
  // limits on this machine (free disk, physical RAM) — round-4 decision G8.
  //
  // `byteSize` comes from the model-status DTO, i.e. the crate manifests, which
  // are the corrected registry G8 requires (speakrs is 419,482,724 bytes there,
  // asserted against its 76-file table by a test in `crates/speaker-analysis`).
  // Nothing is re-declared here.
  //
  // Renders nothing when the model is OS-managed (no download) or when neither
  // machine limit could be measured.
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { modelFootprint } from "$lib/settings/state/system-facts";

  interface Props {
    /** The model's download size in bytes; `null`/0 for OS-managed models. */
    byteSize: number | null | undefined;
  }

  let { byteSize }: Props = $props();

  void systemFacts.ensureLoaded();
  const line = $derived(modelFootprint(systemFacts.value, byteSize ?? null));
</script>

{#if line}
  <p class="group-hint">{line}</p>
{/if}
