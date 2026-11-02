<script lang="ts">
  // Narrow window only (mockup 1-A `.n-pick`): the conclusions rail folds into
  // this bar above the details — the chosen one, "23 of 24", and ‹ › to step.
  // The button opens the rail as a popover (the page owns `open`).
  import type { Conclusion } from "$lib/types/recording";
  import IconDown from "~icons/lucide/chevron-down";
  import IconUp from "~icons/lucide/chevron-up";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";

  interface Props {
    conclusion: Conclusion;
    index: number;
    total: number;
    open: boolean;
    onToggle: () => void;
    onStep: (d: 1 | -1) => void;
  }
  let { conclusion, index, total, open, onToggle, onStep }: Props = $props();
</script>

<div class="sd-pick">
  <button type="button" class="mx-btn sd-pick__btn" data-pick-toggle aria-haspopup="listbox" aria-expanded={open} onclick={onToggle}>
    <span class="num">{Math.round(conclusion.confidence * 100)}%</span>
    <span class="sd-pick__t">{conclusion.statement}</span>
    <small>{index + 1} of {total}</small>
    {#if open}<IconUp width="13" height="13" aria-hidden="true" />{:else}<IconDown width="13" height="13" aria-hidden="true" />{/if}
  </button>
  <button type="button" class="mx-btn mx-btn--ghost sd-step" aria-label="Previous conclusion" disabled={index === 0} onclick={() => onStep(-1)}>
    <IconPrev width="14" height="14" aria-hidden="true" />
  </button>
  <button type="button" class="mx-btn mx-btn--ghost sd-step" aria-label="Next conclusion" disabled={index >= total - 1} onclick={() => onStep(1)}>
    <IconNext width="14" height="14" aria-hidden="true" />
  </button>
</div>

<style>
  .sd-pick { display: none; }
  @container sd (max-width: 620px) {
    .sd-pick { display: flex; gap: 4px; padding: 8px 14px; border-bottom: 1px solid var(--mx-hairline); }
  }
  .sd-pick__btn { flex: 1; min-width: 0; height: var(--h-md); }
  .sd-pick__btn .num { font-family: var(--font-mono); color: var(--app-text-strong); }
  .sd-pick__t { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; text-align: left; }
  .sd-pick__btn small { font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-subtle); white-space: nowrap; }
  .sd-step { width: var(--h-md); height: var(--h-md); padding: 0; justify-content: center; }
</style>
