<script lang="ts">
  // The Journal lede — the day's date, the digest that Overview's Today tile
  // already showed (same `UserContextDigest` record; opening the journal never
  // re-runs the model, `↻ re-read` is the only thing that does), and the four
  // counted numbers.
  //
  // Every number here is counted, none estimated (G8): tracked = summed per-app
  // active time, deep focus = the share of focus-carrying activities that came
  // back `deep`, activities = the card count, top category = the argmax over
  // clipped per-category time. A number whose input hasn't loaded is absent
  // rather than zero.
  import type { LedeStats } from "$lib/insights/lede-stats";
  import { humanizeHours } from "$lib/insights/activity-helpers";
  import { relativeAgo } from "$lib/journal/band-stats";

  interface Props {
    dayLabel: string;
    headline: string | null;
    narrative: string | null;
    generatedAtMs: number | null;
    digestError: string | null;
    engineOn: boolean;
    regenerating: boolean;
    canReRead: boolean;
    stats: LedeStats;
    statsReady: boolean;
    usageReady: boolean;
    activityCount: number;
    atLatest: boolean;
    onReRead: () => void;
    onStepDay: (dir: -1 | 1) => void;
    onToday: () => void;
  }

  let {
    dayLabel,
    headline,
    narrative,
    generatedAtMs,
    digestError,
    engineOn,
    regenerating,
    canReRead,
    stats,
    statsReady,
    usageReady,
    activityCount,
    atLatest,
    onReRead,
    onStepDay,
    onToday,
  }: Props = $props();

  const written = $derived(generatedAtMs != null ? relativeAgo(generatedAtMs) : "");
</script>

<div class="lede">
  <div class="lede__main">
    <div class="lede__hd">
      <p class="t-title lede__day">{dayLabel}</p>
      <span class="lede__nav">
        <button type="button" class="btn btn--ghost btn--sm btn--icon" aria-label="Previous day" onclick={() => onStepDay(-1)}>‹</button>
        <button
          type="button"
          class="btn btn--ghost btn--sm btn--icon"
          aria-label="Next day"
          disabled={atLatest}
          onclick={() => onStepDay(1)}>›</button
        >
        {#if !atLatest}
          <button type="button" class="btn btn--ghost btn--sm" onclick={onToday}>Today</button>
        {/if}
      </span>
      {#if written}
        <span class="t-meta is-mono is-num lede__when">read written {written}</span>
      {/if}
      {#if engineOn}
        <button
          type="button"
          class="btn btn--ghost btn--sm lede__reread"
          disabled={!canReRead}
          onclick={onReRead}
        >
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M13.4 7A5.5 5.5 0 0 0 3.6 4.6M2.6 9a5.5 5.5 0 0 0 9.8 2.4" />
            <path d="M13.6 3.6V7h-3.4M2.4 12.4V9h3.4" />
          </svg>
          {regenerating ? "reading…" : "re-read"}
        </button>
      {/if}
    </div>
    {#if narrative}
      <p class="lede__n">
        {#if headline}<b>{headline}</b>{/if}
        {narrative}
      </p>
    {:else if regenerating}
      <p class="lede__n lede__n--quiet">Reading the day…</p>
    {:else if digestError}
      <p class="lede__n lede__n--quiet">{digestError}</p>
    {/if}
  </div>

  <div class="stats">
    {#if usageReady}
      <div class="stat"><b>{humanizeHours(stats.trackedMs)}</b><span class="t-label">tracked</span></div>
    {/if}
    {#if statsReady && stats.deepPct !== null}
      <div class="stat"><b>{stats.deepPct}%</b><span class="t-label">deep focus</span></div>
    {/if}
    {#if statsReady}
      <div class="stat"><b>{activityCount}</b><span class="t-label">activities</span></div>
    {/if}
    {#if statsReady && stats.topCategory}
      <div class="stat">
        <b class="stat__cat">
          <i style="background:var({stats.topCategory.colorVar});"></i>{stats.topCategory.label}
        </b>
        <span class="t-label">top category</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .lede {
    flex: 0 0 auto;
    display: flex;
    gap: var(--s-24);
    align-items: flex-start;
    padding: var(--s-12) var(--s-16) var(--s-8);
  }
  .lede__main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
  }
  .lede__hd {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .lede__day {
    margin: 0;
  }
  .lede__nav {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .lede__when {
    color: var(--app-text-subtle);
  }
  .lede__reread svg {
    width: 11px;
    height: 11px;
  }
  .lede__n {
    margin: 0;
    max-width: 74ch;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
  }
  .lede__n b {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }
  .lede__n--quiet {
    color: var(--app-text-subtle);
  }

  .stats {
    flex: 0 0 auto;
    display: flex;
    gap: var(--s-16);
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 62px;
  }
  .stat b {
    font: var(--w-semi) var(--t-display) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
  }
  /* The category is a name, not a number — sans, at title size, with its dot. */
  .stat__cat {
    display: inline-flex;
    align-items: center;
    gap: var(--s-6);
    font-family: var(--app-font-sans);
    font-size: var(--t-title);
    white-space: nowrap;
  }
  .stat__cat i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
</style>
