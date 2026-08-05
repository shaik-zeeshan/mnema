<script lang="ts">
  // ── Timeline Jumper — quick targets + seven day rows ──────────────────────
  // The jump menu's fixed head (round-4 decision G6): quick targets
  // Now / This morning / Yesterday, then seven day rows, each carrying its own
  // coverage bar and captured hours. A day with no recording is DISABLED — you
  // can never land on an empty day. There is no type-a-date field (dropped from
  // v1 by G6).
  //
  // Rendering only: coverage comes from the backend's `list_day_coverage` and
  // is shaped by the rune-free helpers in `jumper-coverage.ts`.
  import type { DayRow } from "./jumper-coverage";

  interface Props {
    rows: DayRow[];
    loading: boolean;
    busy: boolean;
    /** Quick targets are disabled when nothing was captured in their window. */
    morningEnabled: boolean;
    yesterdayEnabled: boolean;
    /** The day the timeline is currently parked on ("YYYY-MM-DD"), if any. */
    hereKey: string | null;
    onJumpNow: () => void;
    onJumpMorning: () => void;
    onJumpYesterday: () => void;
    onJumpDay: (row: DayRow) => void;
  }

  let {
    rows,
    loading,
    busy,
    morningEnabled,
    yesterdayEnabled,
    hereKey,
    onJumpNow,
    onJumpMorning,
    onJumpYesterday,
    onJumpDay,
  }: Props = $props();
</script>

<!-- NSMenu anatomy (direction 02): a checkmark gutter on every row so the
     committed day lines up with the quick targets, and the coverage bar in the
     kit's `.ss-daybar` slot on the right. -->
<div class="jrows">
  <div class="jrows__list">
    <button
      type="button"
      class="ss-menu__i jrows__row"
      onclick={onJumpNow}
      disabled={busy}
    >
      <span class="ss-menu__ck" aria-hidden="true"></span>
      <span class="jrows__day">Now</span>
    </button>
    <button
      type="button"
      class="ss-menu__i jrows__row"
      class:is-off={busy || !morningEnabled}
      onclick={onJumpMorning}
      disabled={busy || !morningEnabled}
    >
      <span class="ss-menu__ck" aria-hidden="true"></span>
      <span class="jrows__day">This morning</span>
    </button>
    <button
      type="button"
      class="ss-menu__i jrows__row"
      class:is-off={busy || !yesterdayEnabled}
      onclick={onJumpYesterday}
      disabled={busy || !yesterdayEnabled}
    >
      <span class="ss-menu__ck" aria-hidden="true"></span>
      <span class="jrows__day">Yesterday</span>
    </button>
  </div>

  <div class="ss-menu__sep" aria-hidden="true"></div>
  <div class="ss-menu__hd">Last 7 days</div>

  <div class="jrows__list">
    {#if loading}
      <div class="jrows__msg">loading coverage…</div>
    {:else}
      {#each rows as row (row.key)}
        <button
          type="button"
          class="ss-menu__i jrows__row"
          class:is-off={busy || row.disabled}
          class:jrows__row--here={row.key === hereKey && !row.disabled}
          onclick={() => onJumpDay(row)}
          disabled={busy || row.disabled}
        >
          <span class="ss-menu__ck" aria-hidden="true">
            {#if row.key === hereKey && !row.disabled}
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 12.5l5 5 10-11" /></svg>
            {/if}
          </span>
          <span class="jrows__day">{row.label}</span>
          {#if row.disabled}
            <span class="jrows__none">no capture</span>
          {:else}
            <span class="ss-daybar jrows__bar" aria-hidden="true">
              {#each row.cells as on, i (i)}
                <i class:jrows__cell--on={on}></i>
              {/each}
            </span>
            <span class="jrows__hours">{row.hoursLabel}</span>
          {/if}
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  /* `.ss-menu__i` / `.ss-menu__ck` / `.ss-menu__hd` / `.ss-menu__sep` /
     `.ss-daybar`: direction 02's kit (lib/studio/studio-shell.css). Only the
     button reset and the per-hour coverage cells are local. */
  .jrows {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 4px;
  }

  .jrows__list {
    display: flex;
    flex-direction: column;
  }

  .jrows__row {
    cursor: default;
    /* The kit's menu item is 24px; the day rows carry a bar, so they get the
       28px content-row floor instead. */
    height: var(--h-row, 28px);
  }
  .jrows__row:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  /* "You are here" is the checkmark gutter — the native rule — so the row
     itself only needs the accent tint. */
  .jrows__row--here .jrows__day {
    color: var(--app-accent);
  }
  .jrows__row--here:hover .jrows__day {
    color: inherit;
  }
  .jrows__row .ss-menu__ck {
    color: var(--app-accent);
  }
  .jrows__row:hover:not(.is-off) .ss-menu__ck {
    color: inherit;
  }
  .jrows__row .ss-menu__ck :global(svg) {
    width: 11px;
    height: 11px;
  }

  .jrows__day {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .jrows__hours {
    flex: 0 0 auto;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .jrows__row:hover:not(.is-off) .jrows__hours,
  .jrows__row:hover:not(.is-off) .jrows__none {
    color: inherit;
    opacity: 0.8;
  }
  .jrows__none {
    margin-left: auto;
    font-size: var(--t-meta);
    color: var(--app-text-faint);
    white-space: nowrap;
  }

  /* The kit's `.ss-daybar` is one fill; the app already knows the per-hour
     truth, so the same 46×6 slot carries 24 cells instead of a percentage. */
  .jrows__bar {
    display: flex;
    gap: 1px;
    padding: 0;
  }
  .jrows__bar i {
    flex: 1 1 0;
    background: var(--app-text-faint);
    opacity: 0.35;
  }
  .jrows__bar i.jrows__cell--on {
    background: var(--app-accent);
    opacity: 0.9;
  }

  .jrows__msg {
    padding: 12px 8px;
    color: var(--app-text-subtle);
    font-size: var(--t-meta);
    text-align: center;
  }
</style>
