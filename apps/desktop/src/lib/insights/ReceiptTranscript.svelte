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

  // Distinct voices over the span — the rail's one count. Speaker names are
  // already resolved upstream (profiles + fallbacks), so this is a set size.
  const speakerCount = $derived(new Set(turns.map((t) => t.speaker)).size);

  // Keep the active row visible as the selection moves (guarded for no match).
  $effect(() => {
    const i = selectionIndex(turns, selectedKey);
    if (i < 0) return;
    rowEls[i]?.scrollIntoView({ block: "nearest" });
  });
</script>

<div class="plate script" role="group" aria-label="Transcript">
  <div class="script__h">
    <span class="t-label">Transcript</span>
    <span class="t-meta is-num script__n">
      {speakerCount}
      {speakerCount === 1 ? "speaker" : "speakers"}
    </span>
  </div>
  <div class="script__list">
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
      <span class="script__t is-num">{clock(turn.startMs)}</span>
      <span class="script__body"><span class="script__spk">{turn.speaker}:</span> {turn.text || "—"}</span>
    </button>
    {/each}
  </div>
</div>

<style>
  /* Prose, so an opaque plate — never the sheet's material (page 08). The rail
     is a bounded scroller: every turn's FULL text, wrapped, never truncated.
     One rule per line to mirror ActivityReceipt.svelte. */
  .script { display: flex; flex-direction: column; min-height: 0; padding: 10px 0 4px; overflow: hidden; }
  .script__h { flex: 0 0 auto; display: flex; align-items: center; gap: 8px; padding: 0 12px 7px; }
  .script__n { margin-left: auto; color: var(--app-text-subtle); }
  .script__list { flex: 1 1 auto; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 1px; padding: 0 6px; scrollbar-width: thin; scrollbar-color: var(--app-border-hover) transparent; }
  .script__row { display: flex; gap: 8px; align-items: baseline; width: 100%; padding: 5px 6px; border: 0; border-radius: var(--r-sm); background: transparent; font: inherit; text-align: left; color: var(--app-text-muted); cursor: pointer; transition: background-color var(--dur-quick) var(--ease); }
  .script__row:hover { background: var(--app-surface-hover); }
  .script__row:focus-visible { outline: none; box-shadow: var(--ring); }
  .script__row.is-active { background: var(--app-surface-hover); }
  .script__t { flex: 0 0 auto; font: var(--w-regular) var(--t-label) / 1.5 var(--app-font-mono); color: var(--app-text-faint); }
  .script__body { font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans); color: inherit; }
  .script__row.is-active .script__body { color: var(--app-text); }
  .script__spk { font-weight: var(--w-medium); color: var(--_c); }

  @media (prefers-reduced-motion: reduce) {
    .script__row { transition: none; }
  }
</style>
