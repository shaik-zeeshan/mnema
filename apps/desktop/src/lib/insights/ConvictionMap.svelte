<script lang="ts" module>
  export interface MapRow {
    subject: string;
    headline: string;
    count: number;
    /** Lead conclusion's confidence now, 0..1. */
    now: number;
    /** Lead's confidence 14 days ago; null = formed this fortnight. */
    then: number | null;
    /** The other conclusions' confidences, 0..1. */
    others: number[];
    faded: boolean;
  }
</script>

<script lang="ts">
  // Conviction map — the Subjects hero (insights.html `.in-map`). One row per
  // subject on a 0–100% track banded at the shared tier cuts: the dot is the lead
  // conclusion now, the tail runs back to where it sat 14 days ago (from its real
  // Confidence History), and ticks mark the subject's other conclusions.
  import { tip } from "$lib/components/tooltip";
  import IconSearch from "~icons/lucide/search";
  import { DISPLAY_FLOOR, INITIAL_BASE, STRONGLY_HELD } from "$lib/insights/subjectsTiers";

  interface Props {
    rows: MapRow[];
    /** Active filter text — names the no-match empty. */
    query: string;
    onOpen: (subject: string) => void;
    onClear: () => void;
  }
  let { rows, query, onOpen, onClear }: Props = $props();

  const pct = (c: number) => Math.round(Math.max(0, Math.min(1, c)) * 100);
  const [FLOOR, BASE, HELD] = [DISPLAY_FLOOR, INITIAL_BASE, STRONGLY_HELD].map(pct);
  const BANDS = [
    { cls: "fade", from: 0, to: FLOOR, label: "fading" },
    { cls: "", from: FLOOR, to: BASE, label: "taking shape" },
    { cls: "", from: BASE, to: HELD, label: "forming" },
    { cls: "firm", from: HELD, to: 100, label: "strongly held" },
  ];
  const TICKS = [0, FLOOR, BASE, HELD, 100];

  function delta(r: MapRow): { text: string; kind: string; value: number | null } {
    if (r.then === null) return { text: "new", kind: "new", value: null };
    const d = pct(r.now) - pct(r.then);
    return { text: d > 0 ? `+${d}` : d < 0 ? `−${-d}` : "±0", kind: d > 0 ? "up" : d < 0 ? "down" : "", value: d };
  }
</script>

<div class="mx-panel in-map mx-reveal" style="--i:3">
  <div class="in-sec">
    <span class="mx-kicker">Conviction map</span>
    <span class="mx-spacer"></span>
    <span class="in-map__legend">
      <span><i class="d"></i>now</span><span><i class="o"></i>14 days ago</span><span><i class="t"></i>other conclusions</span>
    </span>
  </div>
  <div class="in-map__plot">
    {#if rows.length}
      <div class="in-map__bands" aria-hidden="true">
        {#each BANDS as b (b.label)}
          <span class={b.cls} style="left:{b.from}%;width:{b.to - b.from}%"><b>{b.label}</b></span>
        {/each}
      </div>
      <div class="in-map__axis" aria-hidden="true">
        <span></span>
        <div>{#each TICKS as v (v)}<span style="left:{v}%">{v}%</span>{/each}</div>
        <span></span>
      </div>
      {#each rows as r, i (r.subject)}
        {@const d = delta(r)}
        {@const c = pct(r.now)}
        <button
          type="button"
          class="in-map__row"
          class:in-map__row--faded={r.faded}
          style="--d:{i}"
          use:tip={`${r.subject} · ${c}% — ${r.headline} · ${r.count} conclusion${r.count === 1 ? "" : "s"} · ${r.then === null ? "formed this fortnight" : `was ${pct(r.then)}% 14 days ago`}`}
          onclick={() => onOpen(r.subject)}
        >
          <span class="in-map__name">{r.subject}<small>{r.count}</small></span>
          <span class="in-map__track">
            {#each r.others as o, k (k)}<i class="in-map__tick" style="left:{pct(o)}%"></i>{/each}
            {#if r.then !== null && d.value}
              <i
                class="in-map__tail"
                style="left:{Math.min(pct(r.then), c)}%;width:{Math.abs(d.value)}%;--c:{d.value > 0 ? 'var(--focus-deep)' : 'var(--focus-distracted)'};--o:{d.value > 0 ? 'left' : 'right'}"
              ></i>
              <i class="in-map__then" style="left:{pct(r.then)}%"></i>
            {/if}
            <i class="in-map__now" class:in-map__now--new={r.then === null} style="left:{c}%"></i>
          </span>
          <span class="in-map__val">{c}%<small class={d.kind}>{d.text}</small></span>
        </button>
      {/each}
    {:else}
      <div class="mx-empty">
        <span class="mx-empty__glyph"><IconSearch width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">No subjects match “{query}”.</h4>
        <p class="mx-empty__text">Filter looks at subject names and the wording of each belief. Try a shorter term, or clear it.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={onClear}>Clear filter</button>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  /* insights.html `.in-map`. */
  .in-sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
  }
  .in-map {
    padding: var(--s-5);
    margin-bottom: var(--s-7);
  }
  .in-map__plot {
    position: relative;
    margin-top: var(--s-4);
  }
  .in-map__bands {
    position: absolute;
    left: 168px;
    right: 104px;
    top: 0;
    bottom: 0;
    pointer-events: none;
  }
  .in-map__bands span {
    position: absolute;
    top: 0;
    bottom: 0;
    border-left: 1px dashed var(--mx-hairline);
  }
  .in-map__bands span b {
    position: absolute;
    top: 0;
    left: 8px;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    white-space: nowrap;
  }
  .in-map__bands .firm {
    background: color-mix(in srgb, var(--app-fg) 2.5%, transparent);
  }
  .in-map__bands .fade {
    background: repeating-linear-gradient(135deg, color-mix(in srgb, var(--app-fg) 3%, transparent) 0 1px, transparent 1px 7px);
  }
  .in-map__axis,
  .in-map__row {
    display: grid;
    grid-template-columns: 168px minmax(0, 1fr) 104px;
    align-items: center;
  }
  .in-map__axis {
    height: 44px;
  }
  .in-map__axis div {
    position: relative;
    height: 100%;
  }
  .in-map__axis div span {
    position: absolute;
    bottom: 4px;
    translate: -50% 0;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-faint);
  }
  .in-map__row {
    width: 100%;
    height: 38px;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--r-sm);
    position: relative;
    transition: background-color var(--t-fast) var(--ease-quart);
    animation: mx-rise var(--t-slow) var(--ease-expo) both;
    animation-delay: calc(180ms + var(--d) * 35ms);
  }
  .in-map__row:hover {
    background: color-mix(in srgb, var(--app-fg) 4%, transparent);
  }
  .in-map__row:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .in-map__name {
    padding-left: var(--s-3);
    font: 600 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .in-map__name small {
    margin-left: 6px;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-faint);
  }
  .in-map__track {
    position: relative;
    height: 100%;
  }
  .in-map__track::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    background: var(--mx-hairline);
  }
  .in-map__tick {
    position: absolute;
    top: calc(50% - 5px);
    width: 2px;
    height: 10px;
    margin-left: -1px;
    border-radius: 1px;
    background: var(--chart-grey-3);
  }
  .in-map__tail {
    position: absolute;
    top: calc(50% - 1.5px);
    height: 3px;
    border-radius: 2px;
    background: var(--c);
    opacity: 0.75;
    transform-origin: var(--o, left);
    animation: grow-x var(--t-slow) var(--ease-expo) both;
    animation-delay: calc(320ms + var(--d) * 35ms);
  }
  .in-map__then {
    position: absolute;
    top: calc(50% - 4px);
    width: 8px;
    height: 8px;
    margin-left: -4px;
    border-radius: 50%;
    background: var(--app-surface);
    border: 1.5px solid var(--chart-grey-4);
  }
  .in-map__now {
    position: absolute;
    top: calc(50% - 6px);
    width: 12px;
    height: 12px;
    margin-left: -6px;
    border-radius: 50%;
    background: var(--app-text-strong);
    box-shadow: 0 0 0 3px var(--app-surface);
  }
  .in-map__now--new {
    background: var(--app-accent);
    box-shadow:
      0 0 0 3px var(--app-surface),
      0 0 0 5px var(--app-accent-glow);
  }
  .in-map__row--faded .in-map__now {
    background: var(--chart-grey-4);
  }
  .in-map__row--faded .in-map__name {
    color: var(--app-text-muted);
  }
  .in-map__val {
    display: flex;
    justify-content: flex-end;
    align-items: baseline;
    gap: 8px;
    padding-right: var(--s-3);
    font: 500 var(--text-md) / 1 var(--font-mono);
    color: var(--app-text-strong);
  }
  .in-map__val small {
    width: 34px;
    text-align: right;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }
  .in-map__val small.up {
    color: var(--focus-deep);
  }
  .in-map__val small.down {
    color: var(--focus-distracted);
  }
  .in-map__val small.new {
    color: var(--app-accent);
  }
  .in-map__legend {
    display: flex;
    gap: var(--s-4);
    font: 500 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .in-map__legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .in-map__legend i {
    display: inline-block;
  }
  .in-map__legend .d {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--app-text-strong);
  }
  .in-map__legend .o {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--chart-grey-4);
  }
  .in-map__legend .t {
    width: 2px;
    height: 10px;
    border-radius: 1px;
    background: var(--chart-grey-3);
  }
  @keyframes grow-x {
    from {
      transform: scaleX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .in-map__row,
    .in-map__tail {
      animation: none;
    }
  }
</style>
