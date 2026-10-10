<script lang="ts" module>
  // `HourBucket` now lives with the rune-free time helpers so it can be shared
  // with the unit-tested bucket builder; re-exported here to keep this pane's
  // public type surface stable for existing importers.
  export type { HourBucket } from "./jumper-time";
</script>

<script lang="ts">
  // ── Timeline Jumper — time-list pane ──────────────────────────────────────
  // Grouped AM/PM hourly list for the previewed day. Each row carries a muted
  // frame count + a NEUTRAL density fill scaled to volume (spec §12.5 — never
  // accent, so it never collides with preview/here/active/hover). The committed
  // hour carries the kit's selected fill (no left bar).
  import IconLatest from "~icons/lucide/arrow-right-to-line";
  import IconCalendar from "~icons/lucide/calendar";
  import type { HourBucket } from "./jumper-time";

  interface Props {
    hasSelection: boolean;
    loading: boolean;
    dayLabel: string;
    buckets: HourBucket[];
    maxCount: number;
    busy: boolean;
    isHereHour: (hour: number) => boolean;
    onCommitHour: (hour: number) => void;
    onCommitDayLatest: () => void;
  }

  let {
    hasSelection,
    loading,
    dayLabel,
    buckets,
    maxCount,
    busy,
    isHereHour,
    onCommitHour,
    onCommitDayLatest,
  }: Props = $props();

  const dayHasFrames = $derived(buckets.some((b) => !b.disabled));
  const dayHourCount = $derived(buckets.filter((b) => b.count > 0).length);

  // Land the list on "you are here" (else the latest captured hour), like the playhead.
  let scrollEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void [dayLabel, loading]; // once per previewed day / load, not on every bucket refresh
    const row =
      scrollEl?.querySelector<HTMLElement>(".jumper-hour--here") ??
      Array.from(scrollEl?.querySelectorAll<HTMLElement>(".jumper-hour:not(:disabled)") ?? []).at(-1);
    if (scrollEl && row) scrollEl.scrollTop = row.offsetTop - (scrollEl.clientHeight - row.offsetHeight) / 2;
  });

  function densityFraction(count: number): number {
    if (count <= 0) return 0;
    return count / Math.max(1, maxCount);
  }
</script>

<div class="jumper-times">
  <div class="jumper-times__head">
    <span class="jumper-times__day">{dayLabel || "—"}</span>
    {#if hasSelection && dayHasFrames && !loading}
      <span class="mx-label num">{dayHourCount} hr</span>
    {/if}
  </div>

  <div class="jumper-times__scroll" bind:this={scrollEl}>
    {#if !hasSelection}
      <p class="jumper-times__msg mx-body-sm">Pick a day to see its hours.</p>
    {:else if loading}
      <div class="jumper-times__msg" aria-busy="true">
        <span class="mx-sr">Loading month</span>
        {#each { length: 7 } as _, i (i)}<div class="mx-skel"></div>{/each}
      </div>
    {:else if !dayHasFrames}
      <div class="jumper-times__msg">
        <div class="mx-empty mx-empty--compact">
          <span class="mx-empty__glyph"><IconCalendar width="14" height="14" /></span>
          <b class="mx-empty__title">No frames on this day</b>
          <p class="mx-empty__text">Mnema wasn’t recording.</p>
        </div>
      </div>
    {:else}
      {#each buckets as t (t.hour)}
        <button
          type="button"
          class="jumper-hour"
          class:jumper-hour--here={isHereHour(t.hour) && !t.disabled}
          onclick={() => onCommitHour(t.hour)}
          disabled={busy || t.disabled}
        >
          {#if !t.disabled && t.count > 0}
            <span class="jumper-hour__density" style="--d:{densityFraction(t.count)}" aria-hidden="true"></span>
          {/if}
          <span class="jumper-hour__label">{t.label}</span>
          <span class="jumper-hour__count">{t.disabled ? "·" : t.count}</span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="jumper-times__foot">
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--sm"
      onclick={onCommitDayLatest}
      disabled={busy || !hasSelection || !dayHasFrames}
    ><IconLatest width="13" height="13" />latest of day</button>
  </div>
</div>

<style>
  .jumper-times {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    min-width: 0;
    min-height: 0;
    border-left: 1px solid var(--mx-hairline);
  }
  .jumper-times__head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-2);
    padding: var(--s-3) var(--s-3) var(--s-2);
  }
  .jumper-times__day {
    font: 600 var(--text-base)/1 var(--font-sans);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .jumper-times__scroll {
    position: relative;
    overflow-y: auto;
    padding: 0 6px;
    scrollbar-width: thin;
  }
  .jumper-times__msg {
    margin: 0;
    padding: var(--s-3) var(--s-2);
  }
  .jumper-times__msg .mx-skel {
    height: 22px;
    margin: 0 4px 6px;
  }
  .jumper-times__msg .mx-empty {
    padding: 0;
  }
  .jumper-hour {
    position: relative;
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    width: 100%;
    height: 28px;
    padding: 0 10px 0 14px;
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    cursor: pointer;
    overflow: hidden;
    font: 500 var(--text-sm)/1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
    text-align: left;
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .jumper-hour:hover:not(:disabled) {
    background: var(--mx-wash-strong);
    color: var(--app-text-strong);
  }
  .jumper-hour:disabled {
    color: var(--app-text-faint);
    cursor: default;
  }
  .jumper-hour:focus-visible {
    outline: 2px solid var(--app-accent);
    outline-offset: -2px;
  }
  .jumper-hour--here {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .jumper-hour__density {
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    width: calc(var(--d) * 100%);
    border-radius: var(--r-xs);
    background: color-mix(in srgb, var(--chart-grey-3) 22%, transparent);
    pointer-events: none;
  }
  .jumper-hour__label,
  .jumper-hour__count {
    position: relative;
  }
  .jumper-hour__count {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
  }
  .jumper-times__foot {
    padding: var(--s-2) var(--s-3);
    border-top: 1px solid var(--mx-hairline);
  }
</style>
