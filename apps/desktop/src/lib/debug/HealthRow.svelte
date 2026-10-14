<!-- One `.dv-row`: dot · name · plain reason · metric · chevron (the mockup's `row()`). -->
<script lang="ts">
  import IconChevron from "~icons/lucide/chevron-right";
  import type { Tone } from "./verdict";

  let {
    tone,
    name,
    why,
    metric,
    plain = false,
    onclick,
  }: { tone: Tone; name: string; why: string; metric: string; plain?: boolean; onclick?: () => void } = $props();
</script>

<button type="button" class="dv-row" data-tone={tone} data-plain={plain ? "" : undefined} {onclick}>
  <span class="dv-dot"></span>
  <span class="dv-row__name">{name}</span>
  <span class="dv-row__why">{why}</span>
  <span class="dv-row__metric">{metric}</span>
  <IconChevron width="14" height="14" aria-hidden="true" />
</button>
