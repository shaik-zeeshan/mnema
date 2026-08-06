<script lang="ts">
  // One conclusion in the drill-in's sortable strip. The card states four real
  // things: how many snapshots stand behind it, the belief, its confidence as a
  // bar and as `NN%`, and where that number came from ("54% → 86%"). A faded
  // one dims but never disappears — below the floor is kept for history.
  //
  // Selecting a card re-points the hero trace and the story below it.
  import type { Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import { deriveTrend } from "$lib/insights/subjectsTiers";
  import { agoLabel, pct, pctLabel } from "$lib/overview/subjects/format";

  interface Props {
    conclusion: Conclusion;
    history: ConfidenceSnapshot[];
    selected: boolean;
    onSelect: () => void;
  }

  let { conclusion, history, selected, onSelect }: Props = $props();

  const faded = $derived(conclusion.status === "faded");
  // Same ±0.04 dead-band the tier list uses — one helper, no second threshold.
  const trend = $derived(
    deriveTrend(
      [{ id: conclusion.id, status: conclusion.status }],
      new Map([[conclusion.id, history.map((h) => h.confidence)]]),
    ),
  );
  const arrow = $derived(trend === "up" ? "↑" : trend === "down" ? "↓" : "–");
  // "54% → 86%" when it actually moved; "steady near 51%" when it did not.
  const move = $derived.by(() => {
    if (history.length < 2) return null;
    const first = pct(history[0].confidence);
    const last = pct(history[history.length - 1].confidence);
    if (trend === "steady") return `steady near ${last}%`;
    return `${first}% → ${last}%`;
  });
  const moved = $derived(agoLabel(conclusion.lastSupportedAtMs));
</script>

<button
  type="button"
  class="ccard"
  class:is-on={selected}
  class:is-faded={faded}
  aria-pressed={selected}
  onclick={onSelect}
>
  <span class="ccard__f">
    {#if conclusion.pinned}<span class="acc">★</span>{/if}
    {#if faded}<span>faded ·</span>{/if}
    {#if history.length > 0}<span class="is-num">{history.length} SNAP</span>{/if}
  </span>
  <span class="ccard__s">{conclusion.statement}</span>
  <span class="ti-well cbar">
    <i class:w={faded} style:width="{pct(conclusion.confidence)}%"></i>
  </span>
  <span class="ccard__f">
    <span class="ccard__pct is-num">{pctLabel(conclusion.confidence)}</span>
    <span class:acc={trend === "up"} class:warn={trend === "down"}>{arrow}</span>
    {#if move}<span class="ccard__move is-num">{move}{moved ? ` · ${moved}` : ""}</span>{/if}
  </span>
</button>

<style>
  .ccard {
    flex: 0 0 232px;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: var(--s-12);
    border: 0;
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
    text-align: left;
    cursor: pointer;
  }
  .ccard.is-on {
    box-shadow:
      0 0 0 var(--hairline) var(--app-accent-border),
      0 0 0 3px var(--app-accent-glow);
  }
  .ccard.is-faded {
    opacity: 0.6;
  }
  .ccard:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .ccard__s {
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-strong);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ccard__f {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
  }
  .ccard__pct {
    color: var(--app-text-strong);
  }
  .ccard__move {
    margin-left: auto;
  }
  .acc {
    color: var(--app-accent);
  }
  .warn {
    color: var(--app-warn);
  }
  /* The confidence bar is the trace's flat sibling: a reading in a well. */
  .cbar {
    height: 5px;
    border-radius: 3px;
    overflow: hidden;
    display: block;
  }
  .cbar i {
    display: block;
    height: 100%;
    background: var(--app-accent);
  }
  .cbar i.w {
    background: var(--app-warn);
  }
</style>
