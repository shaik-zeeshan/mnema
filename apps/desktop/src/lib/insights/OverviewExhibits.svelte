<script lang="ts">
  // The Overview's exhibits row (insights.html `.in-ex`): "Where the <unit>
  // went" — per-slot stacked columns + the category list — beside the Focus
  // split/heat and Time in apps. Capture-only greys the columns to app time and
  // says "needs AI" where categories and focus would be.
  import StackedBar from "$lib/insights/charts/StackedBar.svelte";
  import Heatmap from "$lib/insights/charts/Heatmap.svelte";
  import FocusSplit from "$lib/insights/FocusSplit.svelte";
  import {
    fmtDuration,
    type CategoryRow,
    type Column,
    type FocusCounts,
    type HeatRow,
    type PeriodUnit,
  } from "$lib/insights/lede-stats";
  import IconOff from "~icons/lucide/circle-off";
  import IconInbox from "~icons/lucide/inbox";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconRight from "~icons/lucide/chevron-right";
  import IconRetry from "~icons/lucide/rotate-ccw";

  interface Props {
    unit: PeriodUnit;
    /** "this week" · "Oct 5 – 11". */
    scope: string;
    capOnly: boolean;
    nothing: boolean;
    columns: Column[];
    categories: CategoryRow[];
    focus: FocusCounts;
    heat: HeatRow[];
    apps: { app: string; activeMs: number }[];
    usage: { loading: boolean; failed: string | null; noAppNote: string | null };
    engine: { loading: boolean; failed: string | null; emptyNote: string | null };
    onRetryUsage: () => void;
    onRetryEngine: () => void;
    onOpenCategories: () => void;
    onOpenFocus: () => void;
    onOpenApps: () => void;
  }
  let {
    unit, scope, capOnly, nothing, columns, categories, focus, heat, apps, usage, engine,
    onRetryUsage, onRetryEngine, onOpenCategories, onOpenFocus, onOpenApps,
  }: Props = $props();

  let hl = $state<string | null>(null);
  const chartLoading = $derived(capOnly ? usage.loading : usage.loading || engine.loading);
  const chartError = $derived(capOnly ? usage.failed : engine.failed);
  const per = $derived(
    capOnly
      ? unit === "day" ? "app time per 2 hours" : "app time per day"
      : unit === "day" ? "activity minutes per 2 hours, by category" : "activity hours per day, by category",
  );
  const catTotal = $derived(categories.reduce((t, c) => t + c.ms, 0));
</script>

<div class="ex">
  <div class="mx-panel week mx-reveal" style="--i:3">
    <div class="sec">
      <span class="mx-kicker">Where the {unit} went</span><span class="mx-spacer"></span><span class="mx-label">{per}</span>
    </div>
    {#if chartLoading}
      <div aria-busy="true">
        <span class="mx-sr">Loading the chart</span>
        <div class="mx-skel-chart" style="height:210px;margin-top:var(--s-4)">
          {#each Array.from({ length: 7 }) as _, i (i)}<div class="mx-skel"></div>{/each}
        </div>
      </div>
    {:else if chartError}
      <div class="mx-empty" data-tone="danger">
        <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">Couldn't load {capOnly ? "your usage charts" : "your activities"}.</h4>
        <p class="mx-empty__text">{chartError}</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={capOnly ? onRetryUsage : onRetryEngine}
            ><IconRetry width="13" height="13" aria-hidden="true" />Try again</button
          >
        </div>
      </div>
    {:else}
      <StackedBar {columns} {unit} highlight={hl} />
      <div class="cats">
        {#if nothing}
          {@render compact(IconInbox, `Nothing captured ${scope}`, "Columns fill in as you record.")}
        {:else if capOnly}
          {@render compact(IconOff, "Categories need AI", "Activities are what carry a category; usage charts only know apps.")}
        {:else if categories.length === 0}
          {@render compact(IconInbox, "No categories yet", engine.emptyNote ?? "Categories appear once activities are written up.")}
        {:else}
          {#each categories as c, i (c.key)}
            <button
              type="button"
              class="cat"
              onclick={onOpenCategories}
              onmouseenter={() => (hl = c.key)}
              onmouseleave={() => (hl = null)}
              onfocus={() => (hl = c.key)}
              onblur={() => (hl = null)}
            >
              <span class="sw" style="--c:var({c.colorVar})"></span><span>{c.label}</span>
              <span class="bar"><i style="--c:var({c.colorVar});--w:{(c.ms / categories[0].ms) * 100}%;--d:{i}"></i></span>
              <span class="num">{fmtDuration(c.ms)}</span><span class="num">{Math.round((c.ms / catTotal) * 100)}%</span>
              <IconRight width="13" height="13" aria-hidden="true" />
            </button>
          {/each}
        {/if}
      </div>
    {/if}
  </div>

  <div class="side">
    <div class="mx-reveal" style="--i:4">
      <div class="sec">
        <span class="mx-kicker">Focus</span><span class="mx-spacer"></span>
        {#if !capOnly && !engine.loading && !engine.failed && focus.n}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onOpenFocus}
            >Focus detail<IconArrow width="13" height="13" aria-hidden="true" /></button
          >
        {/if}
      </div>
      {#if capOnly}
        {@render compact(IconOff, "Focus needs AI", "Deep / mixed / scattered is judged per activity.")}
      {:else if engine.loading}
        <div class="mx-skel-lines" aria-busy="true"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
      {:else if engine.failed}
        {@render inlineError(onRetryEngine)}
      {:else if !focus.n}
        {@render compact(IconInbox, "No focus signal yet", engine.emptyNote ?? "Appears once activities are written up.")}
      {:else}
        <FocusSplit counts={focus} />
        <Heatmap rows={heat} />
      {/if}
    </div>
    <div class="mx-reveal" style="--i:5">
      <div class="sec">
        <span class="mx-kicker">Time in apps</span><span class="mx-spacer"></span>
        {#if !usage.loading && !usage.failed && apps.length}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onOpenApps}
            >All {apps.length} {apps.length === 1 ? "app" : "apps"}<IconArrow width="13" height="13" aria-hidden="true" /></button
          >
        {/if}
      </div>
      {#if usage.loading}
        <div class="mx-skel-lines" aria-busy="true"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
      {:else if usage.failed}
        {@render inlineError(onRetryUsage)}
      {:else if usage.noAppNote}
        {@render compact(IconInbox, "No app time", usage.noAppNote)}
      {:else if !apps.length}
        {@render compact(IconInbox, "No tracked app time in this range", "")}
      {:else}
        <dl class="mx-spec apps">
          {#each apps.slice(0, 6) as a, i (a.app)}
            <div>
              <dt>{a.app}</dt>
              <dd><span class="bar"><i style="--w:{(a.activeMs / apps[0].activeMs) * 100}%;--d:{i + 4}"></i></span></dd>
              <dd class="mx-spec__note num">{fmtDuration(a.activeMs)}</dd>
            </div>
          {/each}
        </dl>
      {/if}
    </div>
  </div>
</div>

{#snippet compact(Icon: typeof IconInbox, title: string, text: string)}
  <div class="mx-empty mx-empty--compact">
    <span class="mx-empty__glyph"><Icon width="14" height="14" aria-hidden="true" /></span>
    <b class="mx-empty__title">{title}</b>
    {#if text}<p class="mx-empty__text">{text}</p>{/if}
  </div>
{/snippet}

{#snippet inlineError(retry: () => void)}
  <p>
    <span class="mx-inline" data-tone="danger"
      ><IconAlert width="13" height="13" aria-hidden="true" />Couldn't load.<button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        onclick={retry}>Retry</button
      ></span
    >
  </p>
{/snippet}

<style>
  .ex {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr);
    gap: var(--s-6);
    margin-bottom: var(--s-7);
  }
  .side {
    display: grid;
    gap: var(--s-6);
    align-content: start;
  }
  .week {
    padding: var(--s-5);
  }
  .sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin: 0 0 var(--s-3);
  }
  .sec .mx-spacer {
    flex: 1;
  }
  .sec > .mx-btn--ghost:last-child {
    margin: -6px -9px -6px 0;
    align-self: center;
  }
  .ex .mx-empty {
    max-width: none;
    padding-inline: 0;
  }
  .cats {
    display: grid;
    margin-top: var(--s-5);
    border-top: 1px solid var(--mx-hairline);
  }
  .cat {
    display: grid;
    grid-template-columns: 10px 112px 1fr 64px 40px 14px;
    align-items: center;
    gap: var(--s-3);
    padding: 8px 6px;
    margin: 0 -6px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    cursor: pointer;
    text-align: left;
    color: var(--app-text);
    font: 500 var(--text-base) / 1 var(--font-sans);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .cat:hover {
    background: var(--app-surface-hover);
  }
  .cat:focus-visible {
    outline: 2px solid var(--app-accent);
  }
  .cat .num {
    font-size: var(--text-sm);
    color: var(--app-text-strong);
    text-align: right;
  }
  .cat .num + .num {
    color: var(--app-text-subtle);
  }
  .cat :global(svg) {
    color: var(--app-text-faint);
    transition:
      transform var(--t-med) var(--ease-expo),
      color var(--t-fast);
  }
  .cat:hover :global(svg) {
    transform: translateX(2px);
    color: var(--app-text-strong);
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--app-fg) 6%, transparent);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    width: var(--w);
    background: var(--c);
    border-radius: 3px;
    transform-origin: left;
    animation: grow-x var(--t-slow) var(--ease-expo) both;
    animation-delay: calc(200ms + var(--d) * 40ms);
  }
  .apps {
    grid-template-columns: 82px 1fr auto;
  }
  .apps dt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .apps dd:not(.mx-spec__note) {
    display: flex;
    align-items: center;
  }
  .apps .bar {
    flex: 1;
    height: 4px;
  }
  .apps .bar i {
    background: var(--chart-grey-3);
  }
  @keyframes grow-x {
    from {
      transform: scaleX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar i {
      animation: none;
    }
  }
  @container mx-window (max-width: 1180px) {
    .ex {
      grid-template-columns: 1fr;
    }
  }
</style>
