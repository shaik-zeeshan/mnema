<script lang="ts">
  // The lede — the digest's own prose at the head of the river, and the four
  // figures under it. The headline is the ONE `--t-display` line on this surface.
  //
  // **G8**: each stat renders only when it is a measured fact. Tracked hours wait
  // for the usage read, deep-focus % and the top category wait for the day's
  // activities, and any of them can be absent — the row simply shortens.
  import type { UserContextDigest } from "$lib/types/recording";
  import type { LedeStats } from "$lib/insights/lede-stats";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  interface Props {
    dayLabel: string;
    digest: UserContextDigest | null;
    loading: boolean;
    regenerating: boolean;
    error: string | null;
    /** "22 min ago" — when the read was written. Empty when unknown. */
    writtenAgo: string;
    stats: LedeStats;
    trackedLabel: string;
    activityCount: number;
    usageLoaded: boolean;
    rangeLoaded: boolean;
  }

  let {
    dayLabel,
    digest,
    loading,
    regenerating,
    error,
    writtenAgo,
    stats,
    trackedLabel,
    activityCount,
    usageLoaded,
    rangeLoaded,
  }: Props = $props();
</script>

<div class="lede" aria-busy={(!digest && loading) || regenerating}>
  <div class="eyebrow">
    <span class="mark">◆ The read</span>
    <span class="t-meta">{dayLabel}</span>
    {#if writtenAgo}
      <span class="ss-sstrip__dot"></span>
      <span class="t-meta is-mono">{writtenAgo}</span>
    {/if}
  </div>

  {#if digest}
    {#key digest.generatedAtMs}
      <div class="body">
        {#if digest.headline}<h1 class="head">{digest.headline}</h1>{/if}
        <p class="prose">{digest.narrative}</p>
      </div>
    {/key}
  {:else if loading || regenerating}
    <div class="sk"><Skeleton variant="text" width="62%" height="16px" /></div>
    <div class="sk"><Skeleton variant="text" width="92%" height="11px" /></div>
    <div class="sk"><Skeleton variant="text" width="74%" height="11px" /></div>
  {:else if error}
    <p class="prose prose--quiet">{error}</p>
  {:else}
    <p class="prose prose--quiet">No read has been written for this day yet.</p>
  {/if}
</div>

<div class="stats">
  {#if usageLoaded && stats.trackedMs > 0}
    <div class="stat"><b>{trackedLabel}</b><span>tracked</span></div>
  {/if}
  {#if rangeLoaded && stats.deepPct !== null}
    <div class="stat"><b>{stats.deepPct}%</b><span>deep focus</span></div>
  {/if}
  {#if rangeLoaded && stats.topCategory}
    <div class="stat">
      <b>
        <i class="sw" style="background:var({stats.topCategory.colorVar});"></i>
        {stats.topCategory.label}
      </b>
      <span>top category</span>
    </div>
  {/if}
  {#if rangeLoaded && activityCount > 0}
    <div class="stat"><b>{activityCount}</b><span>activities</span></div>
  {/if}
</div>

<style>
  .lede {
    padding: 12px 16px 0;
  }
  .eyebrow {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-height: 18px;
  }
  .mark {
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-accent);
  }
  .head {
    margin: 6px 0 4px;
    font: var(--w-semi) var(--t-display) / var(--lh-display) var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
    max-width: 46ch;
  }
  .prose {
    margin: 0;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
    max-width: 74ch;
  }
  .prose--quiet {
    color: var(--app-text-subtle);
    font-size: var(--t-ui);
  }
  .sk {
    padding: 4px 0;
  }
  .body {
    animation: lede-reveal 0.22s var(--ease);
  }
  @keyframes lede-reveal {
    from {
      opacity: 0;
      transform: translateY(3px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .body {
      animation: none;
    }
  }

  /* The four figures — hairline over them, never a card. */
  .stats {
    display: flex;
    gap: var(--s-24);
    margin-top: 12px;
    padding: 10px 16px 12px;
    border-top: var(--hairline) solid var(--app-border);
    flex-wrap: wrap;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .stat b {
    display: flex;
    align-items: center;
    gap: 5px;
    font: var(--w-semi) var(--t-title) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .stat span {
    font: var(--w-medium) var(--t-label) / 1.4 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .sw {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
</style>
