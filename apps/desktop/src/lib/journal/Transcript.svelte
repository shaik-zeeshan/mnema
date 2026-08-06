<script lang="ts">
  // The receipt's transcript reader — page 08's `.trn` rows. Every spoken turn
  // over the activity's span with its live-resolved speaker name; the row under
  // the playhead is lit and colour-barred. Clicking a row relives that spoken
  // moment at 1×. Presentational only: the parent owns the selection.
  //
  // There is no speaker LANE — it's named in the code's comments but has never
  // existed as UI, so the mockup doesn't draw one and neither does this.
  import type { TurnView } from "$lib/insights/receipt-audio";
  import { selectionIndex } from "$lib/insights/receipt-lane";
  import { clockShort } from "$lib/insights/receipt-clock";

  interface Props {
    turns: TurnView[];
    activeKey: string | null;
    onSelect: (key: string) => void;
  }
  let { turns, activeKey, onSelect }: Props = $props();

  let rowEls = $state<(HTMLButtonElement | null)[]>([]);

  $effect(() => {
    const i = selectionIndex(turns, activeKey);
    if (i < 0) return;
    rowEls[i]?.scrollIntoView({ block: "nearest" });
  });
</script>

<div class="rcpt__tr" role="group" aria-label="Transcript">
  {#each turns as turn, i (turn.key)}
    <button
      type="button"
      class="trn"
      class:on={turn.key === activeKey}
      style="--spk: var({turn.colorVar});"
      aria-pressed={turn.key === activeKey}
      bind:this={rowEls[i]}
      onclick={() => onSelect(turn.key)}
    >
      <span class="trn__t">{clockShort(turn.startMs)}</span>
      <span class="trn__s">{turn.speaker}</span>
      <span class="trn__x">{turn.text}</span>
    </button>
  {/each}
</div>

<style>
  .rcpt__tr {
    flex: 0 0 auto;
    max-height: 110px;
    overflow-y: auto;
    padding: 0 var(--s-12) var(--s-8);
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
  }
  .trn {
    display: flex;
    gap: var(--s-8);
    align-items: baseline;
    width: 100%;
    padding: 1px var(--s-4) 1px var(--s-6);
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .trn:hover {
    background: var(--app-surface-hover);
  }
  .trn:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--app-accent);
  }
  .trn.on {
    box-shadow: inset 2px 0 0 var(--spk);
  }
  .trn__t {
    flex: 0 0 44px;
    font: var(--w-regular) var(--t-label) / 1.5 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .trn__s {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--spk);
  }
  .trn__x {
    flex: 1 1 auto;
    min-width: 0;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
    overflow-wrap: anywhere;
  }
  .trn.on .trn__x {
    color: var(--app-text-strong);
  }
</style>
