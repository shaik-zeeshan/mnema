<script lang="ts">
  // The Subjects list — the one scrolling region of the /subjects destination.
  //
  // Tiers at the ENGINE'S own thresholds (`subjectsTiers.ts` holds them; this
  // file never re-derives one), and the sparkline as each row's hero. The header
  // line states COUNTS, never a rolled-up score — there is no aggregate subject
  // score in the data and this page refuses to invent one.
  import IconChevron from "~icons/lucide/chevron-right";
  import {
    DISPLAY_FLOOR,
    INITIAL_BASE,
    STRONGLY_HELD,
    isSparse,
  } from "$lib/insights/subjectsTiers";
  import { ago, conf, trendLabel } from "./format";
  import Trajectory from "./Trajectory.svelte";
  import type { SubjectRow, SubjectsData } from "./subjects-data.svelte";

  interface Props {
    data: SubjectsData;
    onopen: (subject: string) => void;
  }

  let { data, onopen }: Props = $props();

  // The tier's right-hand bound, in the row register. Only the conviction tiers
  // have one — "warming" is a direction, not a threshold.
  const BOUNDS: Record<string, string> = {
    strong: `≥ ${STRONGLY_HELD.toFixed(2)}`,
    forming: `≥ ${INITIAL_BASE.toFixed(2)}`,
    shaping: `< ${INITIAL_BASE.toFixed(2)}`,
    fading: `< ${DISPLAY_FLOOR.toFixed(2)}`,
  };

  // Every clause is dropped when its count is zero — the line shortens rather
  // than claiming a zero (G8).
  const headline = $derived.by<string>(() => {
    const s = data.summary;
    const left: string[] = [];
    if (s.active > 0) left.push(`${s.active} active ${s.active === 1 ? "view" : "views"}`);
    if (s.fading > 0) left.push(`${s.fading} fading`);
    const moves: string[] = [];
    if (s.warming > 0) moves.push(`${s.warming} warming ▲`);
    if (s.steady > 0) moves.push(`${s.steady} steady`);
    if (s.cooling > 0) moves.push(`${s.cooling} cooling ▼`);
    const head = left.join(" · ");
    return moves.length > 0 ? `${head} — ${moves.join(" · ")}` : head;
  });

  const subtitle = $derived(
    data.axis === "conviction"
      ? "Conviction — how firmly the engine holds each view (its confidence)."
      : "Movement — which way each view has been heading lately.",
  );

  // Below the sparse limit, tier headers would be mostly-empty furniture: one
  // flat list instead (the same rule the old surface applies).
  const flat = $derived(data.searching || isSparse(data.displayRows.length));
  const flatRows = $derived(data.searching ? data.searchResults : data.displayRows);

  function onRowKey(event: KeyboardEvent, subject: string): void {
    if (event.key === "Enter") {
      // Cancel the button's own Enter→click, so ⏎ opens rather than re-selects.
      event.preventDefault();
      onopen(subject);
    }
  }
</script>

{#snippet row(r: SubjectRow)}
  <button
    type="button"
    class="srow"
    class:is-sel={data.selected === r.subject}
    class:is-faded={r.faded}
    aria-pressed={data.selected === r.subject}
    onclick={() => data.select(r.subject)}
    ondblclick={() => onopen(r.subject)}
    onkeydown={(e) => onRowKey(e, r.subject)}
  >
    <span class="sdot" class:is-up={r.trend === "up"} class:is-down={r.trend === "down"}></span>
    <span class="stxt">
      <span class="sline1">
        <span class="sname">{r.subject}</span>
        {#if r.pinned}<span class="trendpill" title="Pinned">★</span>{/if}
        <span
          class="trendpill"
          class:is-up={r.trend === "up"}
          class:is-down={r.trend === "down"}>{trendLabel(r.trend)}</span
        >
        <span class="t-meta"
          >· {r.conclusionCount}
          {r.conclusionCount === 1 ? "conclusion" : "conclusions"}</span
        >
      </span>
      <span class="shead">{r.headline}</span>
    </span>
    <Trajectory
      lines={r.spark}
      floor={DISPLAY_FLOOR}
      label={`${r.subject} — confidence across its recorded snapshots`}
    />
    <span class="snum">
      <span class="snum__c">{conf(r.topConfidence)}</span>
      <span class="snum__t">{ago(r.lastMovedAtMs)}</span>
    </span>
    <span class="ss-chev" aria-hidden="true"><IconChevron /></span>
  </button>
{/snippet}

<div class="pane">
  {#if data.loadError && !data.conclusions?.length}
    <div class="state">
      <p class="state__t">Couldn't read what the engine has concluded.</p>
      <p class="t-meta">{data.loadError}</p>
      <button type="button" class="btn btn--sm" onclick={() => void data.load()}>Try again</button>
    </div>
  {:else if data.loading && !data.conclusions}
    <p class="quiet t-meta">Reading the dossier…</p>
  {:else if data.displayRows.length === 0}
    <div class="state">
      <p class="state__t">No views yet.</p>
      <p class="t-meta">
        {#if data.engineOn === false}
          The Reasoning Engine is off, so nothing is being concluded. Turn it on in
          Settings › Intelligence.
        {:else}
          Views form as the engine finds repeated evidence in what you do. Nothing has
          formed yet.
        {/if}
      </p>
    </div>
  {:else}
    <div class="lede">
      {#if data.searching}
        <p class="t-meta">
          {flatRows.length}
          {flatRows.length === 1 ? "match" : "matches"} for “{data.appliedQuery}”
        </p>
      {:else}
        <p class="t-meta">{headline}</p>
      {/if}
      <p class="t-meta sub">{subtitle}</p>
    </div>

    {#if flat}
      {#if flatRows.length === 0}
        <p class="quiet t-meta">No subject matches that.</p>
      {:else}
        <div class="ss-grp grp">
          {#each flatRows as r (r.subject)}{@render row(r)}{/each}
        </div>
      {/if}
    {:else}
      {#each data.tiers as tier (tier.id)}
        {@const shown = data.shownCount(tier.id)}
        {@const hidden = tier.items.length - shown}
        <div class="tierhd">
          <span class="tierhd__n">{tier.title}</span>
          <span class="t-meta">{tier.note}</span>
          <span class="tierhd__c"
            >{BOUNDS[tier.id]
              ? `${tier.items.length} · ${BOUNDS[tier.id]}`
              : tier.items.length}</span
          >
        </div>
        <div class="ss-grp grp" class:is-faded-tier={tier.faded}>
          {#each tier.items.slice(0, shown) as r (r.subject)}{@render row(r)}{/each}
          {#if hidden > 0}
            <button type="button" class="more" onclick={() => data.showMore(tier.id)}>
              Show {hidden} more
            </button>
          {/if}
        </div>
      {/each}
    {/if}

    <p class="foot t-meta sub">
      Confidence is recency-weighted — views warm with fresh evidence and cool on their
      own. Faded views are kept for history, never deleted.
    </p>
  {/if}
</div>

<style>
  .pane {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 0 var(--s-12);
  }

  .lede {
    padding: 10px var(--s-16) 0;
  }

  .lede p {
    margin: 0;
  }

  .lede p + p {
    margin-top: 4px;
  }

  .quiet,
  .state {
    padding: var(--s-16);
  }

  .state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-6);
  }

  .state__t {
    margin: 0;
    font: var(--w-medium) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .state :global(p.t-meta) {
    margin: 0;
    max-width: 56ch;
  }

  /* ── Tier header — the sentence the list reads top to bottom ───────────── */
  .tierhd {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    height: 26px;
    padding: 0 var(--s-16);
    margin-top: var(--s-12);
    background: var(--app-bg);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .tierhd__n {
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }

  .tierhd__c {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .grp {
    margin: var(--s-8) var(--s-8) 0;
  }

  /* ── The row ───────────────────────────────────────────────────────────── */
  .srow {
    display: flex;
    align-items: center;
    gap: var(--s-10);
    width: 100%;
    min-height: 44px;
    padding: 6px var(--s-10);
    border: 0;
    border-top: var(--hairline) solid var(--app-border);
    border-radius: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    cursor: default;
  }

  .srow:first-child {
    border-top: 0;
  }

  .srow:hover {
    background: var(--app-surface-hover);
  }

  .srow:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  .srow.is-faded,
  .is-faded-tier .srow {
    opacity: 0.55;
  }

  /* Full-row accent selection — the native rule, and what fills the inspector. */
  .srow.is-sel,
  .srow.is-sel:hover {
    background: var(--app-accent);
    border-radius: 5px;
  }

  .srow.is-sel .sname,
  .srow.is-sel .snum__c {
    color: var(--app-accent-contrast);
  }

  .srow.is-sel .shead,
  .srow.is-sel .snum__t,
  .srow.is-sel .trendpill,
  .srow.is-sel .t-meta,
  .srow.is-sel :global(.ss-chev) {
    color: var(--app-accent-contrast);
    opacity: 0.82;
  }

  .srow.is-sel .sdot {
    background: var(--app-accent-contrast);
  }

  /* The sparkline inverts with the row (its strokes are class-driven, so no
     inline style has to be fought here). */
  .srow.is-sel :global(.line) {
    stroke: var(--app-accent-contrast);
    opacity: 0.45;
  }

  .srow.is-sel :global(.line.is-lead) {
    opacity: 1;
  }

  .srow.is-sel :global(.floor) {
    stroke: var(--app-accent-contrast);
    opacity: 0.35;
  }

  .sdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-text-faint);
  }

  .sdot.is-up {
    background: var(--app-accent);
  }

  .sdot.is-down {
    background: var(--app-warn);
  }

  .stxt {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .sline1 {
    display: flex;
    align-items: baseline;
    gap: var(--s-6);
    min-width: 0;
  }

  .sname {
    font: var(--w-medium) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .shead {
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .trendpill {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  .trendpill.is-up {
    color: var(--app-accent-strong);
  }

  .trendpill.is-down {
    color: var(--app-warn);
  }

  .sline1 .t-meta {
    flex: 0 0 auto;
    white-space: nowrap;
  }

  .snum {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 1px;
    flex: 0 0 auto;
    width: 62px;
  }

  .snum__c {
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }

  .snum__t {
    font: var(--w-regular) var(--t-label) / 1.4 var(--app-font-mono);
    color: var(--app-text-faint);
  }

  .more {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    min-height: 28px;
    border: 0;
    border-top: var(--hairline) solid var(--app-border);
    background: transparent;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
    cursor: default;
  }

  .more:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  .foot {
    margin: var(--s-16) var(--s-16) 0;
    max-width: 74ch;
  }
</style>
