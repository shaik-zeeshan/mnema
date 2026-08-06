<script lang="ts">
  // The model row's real output: a verdict computed against THIS Mac, in the
  // direction's four tones (green / amber / red / cloud-blue). The size and the
  // machine denominators are stated separately by `ModelFootprintHint` (G8);
  // this says where it runs and whether it is usable here.
  //
  // Props mirror the status DTO fields verbatim — there is no place here to
  // invent a fact; `state/model-verdict.ts` maps them to the phrase.
  import IconOk from "~icons/lucide/check";
  import IconWarn from "~icons/lucide/triangle-alert";
  import IconCloud from "~icons/lucide/globe";
  import { modelVerdict, type ModelVerdictInput } from "$lib/settings/state/model-verdict";

  let { provider, available, status, osManaged = false, downloadBytes = null }: ModelVerdictInput =
    $props();

  const verdict = $derived(
    modelVerdict({ provider, available, status, osManaged, downloadBytes }),
  );
</script>

<span class="verdict verdict--{verdict.tone}">
  {#if verdict.tone === "cloud"}
    <IconCloud aria-hidden="true" />
  {:else if verdict.tone === "ok"}
    <IconOk aria-hidden="true" />
  {:else}
    <IconWarn aria-hidden="true" />
  {/if}
  {verdict.text}
</span>
