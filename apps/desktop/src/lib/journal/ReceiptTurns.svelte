<script lang="ts">
  // The receipt's transcript tile (mockup 08): the spoken turns that fall inside
  // this activity, each in its speaker's colour, the one under the playhead lit.
  // Selecting a turn plays that turn's real audio — selection and playback are
  // one act (ADR 0049), so these rows are buttons, not decoration.
  import { selectionIndex } from "$lib/insights/receipt-lane";
  import { clockShort } from "$lib/insights/receipt-clock";
  import type { TurnView } from "$lib/insights/receipt-audio";

  let {
    turns,
    activeKey,
    onSelect,
  }: {
    turns: TurnView[];
    activeKey: string | null;
    onSelect: (key: string) => void;
  } = $props();

  let rowEls = $state<(HTMLButtonElement | null)[]>([]);

  // Keep the lit row visible as the playhead moves it (guarded for no match).
  $effect(() => {
    const i = selectionIndex(turns, activeKey);
    if (i < 0) return;
    rowEls[i]?.scrollIntoView({ block: "nearest" });
  });
</script>

<div class="tile tile--h2 tile--static">
  <div class="tile__h">
    <span class="t-label">Transcript</span>
    <span class="tile__more is-num">{turns.length} {turns.length === 1 ? "turn" : "turns"}</span>
  </div>
  <div class="pay pay--rows scroll turns">
    {#each turns as turn, i (turn.key)}
      <button
        type="button"
        class="turn"
        class:on={turn.key === activeKey}
        style="color:var({turn.colorVar})"
        aria-pressed={turn.key === activeKey}
        bind:this={rowEls[i]}
        onclick={() => onSelect(turn.key)}
      >
        <time>{clockShort(turn.startMs)}</time>
        <span class="txt"><b>{turn.speaker}</b>: {turn.text || "—"}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .turns {
    overflow-y: auto;
  }
  .turn {
    display: grid;
    grid-template-columns: 42px 1fr;
    gap: var(--s-8);
    width: 100%;
    padding: 5px var(--tile-pad);
    border: 0;
    background: transparent;
    text-align: left;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
    opacity: 0.7;
    cursor: pointer;
    transition: opacity var(--dur-quick) var(--ease);
  }
  .turn:hover {
    opacity: 0.9;
  }
  .turn:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
  /* The lit row keeps its speaker colour (the inline `color` wins over any rule
     here) — the tint and the edge are that colour at 10%. */
  .turn.on {
    opacity: 1;
    background: color-mix(in srgb, currentColor 10%, transparent);
    box-shadow: inset 2px 0 0 currentColor;
  }
  .turn time {
    font-family: var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }
  .turn b {
    font-weight: var(--w-semi);
  }
  .txt {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  @media (prefers-reduced-motion: reduce) {
    .turn {
      transition: none;
    }
  }
</style>
