<script lang="ts">
  // ReceiptTranscript — the synced "karaoke" transcript reader (Receipt
  // redesign, slice 2). A bounded, scrolling list of every turn's FULL text
  // (wraps, never truncated). The active row (its key === selectedKey) is
  // emphasized + speaker-colored; the rest are dimmed for context. Selecting a
  // row reports via onSelect(key) — the same "bus" the Speaker-Turn Lane feeds,
  // so a lane block and its transcript row are two views of one selection.
  // Purely presentational; the parent owns selectedKey. A $effect scrolls the
  // active row into view when the selection changes.

  import type { TurnView } from "./receipt-audio";
  import { selectionIndex } from "./receipt-lane";

  interface Props {
    turns: TurnView[];
    selectedKey: string | null;
    onSelect: (key: string) => void;
    /** Wall-clock formatter, passed from the parent. */
    clock: (ms: number) => string;
  }

  let { turns, selectedKey, onSelect, clock }: Props = $props();

  let rowEls = $state<(HTMLButtonElement | null)[]>([]);

  // Keep the active row visible as the selection moves (guarded for no match).
  $effect(() => {
    const i = selectionIndex(turns, selectedKey);
    if (i < 0) return;
    rowEls[i]?.scrollIntoView({ block: "nearest" });
  });
</script>

<div class="script" role="group" aria-label="Transcript">
  {#each turns as turn, i (turn.key)}
    <button
      type="button"
      class="script__row"
      class:is-active={turn.key === selectedKey}
      style="--_c: var({turn.colorVar});"
      aria-pressed={turn.key === selectedKey}
      bind:this={rowEls[i]}
      onclick={() => onSelect(turn.key)}
    >
      <span class="script__t">{clock(turn.startMs)}</span>
      <span class="script__body"><span class="script__spk">{turn.speaker}</span>: {turn.text || "—"}</span>
    </button>
  {/each}
</div>

<style>
  /* One rule per line to mirror ActivityReceipt.svelte + the 04-timelapse mockup.
     Dimmed rows sit at 0.7 (raised from the mockup's 0.55) for legibility. Body
     text is the app default font; time + speaker are mono w/ tabular numerals. */
  .script { max-height: 110px; overflow-y: auto; margin-top: 8px; padding: 4px; border: 1px solid var(--app-border); border-radius: 7px; background: var(--app-surface-subtle); }
  .script__row { display: grid; grid-template-columns: 44px 1fr; column-gap: 8px; align-items: baseline; width: 100%; padding: 4px 8px; border: 0; border-left: 2px solid transparent; border-radius: 5px; background: transparent; font: inherit; text-align: left; color: var(--app-text-muted); opacity: 0.7; cursor: pointer; transition: opacity 0.12s ease, background 0.12s ease; }
  .script__row:hover { opacity: 0.9; background: var(--app-surface-hover); }
  .script__row:focus-visible { outline: 2px solid var(--app-accent); outline-offset: -2px; opacity: 1; }
  .script__row.is-active { opacity: 1; border-left-color: var(--_c); background: color-mix(in srgb, var(--_c) 10%, transparent); }
  .script__t { font-family: var(--app-font-mono); font-size: 10px; font-variant-numeric: tabular-nums; color: var(--app-text-subtle); }
  .script__row.is-active .script__t { color: var(--app-text-muted); }
  .script__body { font-size: 12px; line-height: 1.5; color: inherit; }
  .script__row.is-active .script__body { color: var(--app-text); }
  .script__spk { font-family: var(--app-font-mono); font-weight: 700; font-variant-numeric: tabular-nums; color: var(--_c); }

  @media (prefers-reduced-motion: reduce) {
    .script__row { transition: none; }
  }
</style>
