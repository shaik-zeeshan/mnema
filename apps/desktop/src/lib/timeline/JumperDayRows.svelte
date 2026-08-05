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

<div class="jrows">
  <!-- Quick targets are menu rows, not a button strip: same 24px row, same
       check gutter, same right-aligned key column as the day rows below. -->
  <div class="jrows__list jrows__list--targets">
    <button type="button" class="jrows__row" onclick={onJumpNow} disabled={busy}>
      <span class="jrows__ck" aria-hidden="true"></span>
      <span class="jrows__day">Now</span>
      <span class="kbd jrows__kbd" aria-hidden="true">L</span>
    </button>
    <button
      type="button"
      class="jrows__row"
      onclick={onJumpMorning}
      disabled={busy || !morningEnabled}
    >
      <span class="jrows__ck" aria-hidden="true"></span>
      <span class="jrows__day">This morning</span>
      {#if !morningEnabled}<span class="jrows__hours">no recording</span>{/if}
    </button>
    <button
      type="button"
      class="jrows__row"
      onclick={onJumpYesterday}
      disabled={busy || !yesterdayEnabled}
    >
      <span class="jrows__ck" aria-hidden="true"></span>
      <span class="jrows__day">Yesterday</span>
      {#if !yesterdayEnabled}<span class="jrows__hours">no recording</span>{/if}
    </button>
  </div>
  <div class="jrows__sep"></div>

  <div class="jrows__list">
    {#if loading}
      <div class="jrows__msg">loading coverage…</div>
    {:else}
      {#each rows as row (row.key)}
        <button
          type="button"
          class="jrows__row"
          class:jrows__row--here={row.key === hereKey && !row.disabled}
          onclick={() => onJumpDay(row)}
          disabled={busy || row.disabled}
        >
          <span class="jrows__ck" aria-hidden="true">
            {#if row.key === hereKey && !row.disabled}✓{/if}
          </span>
          <span class="jrows__day">{row.label}</span>
          <span class="jrows__bar" aria-hidden="true">
            {#each row.cells as on, i (i)}
              <i class:jrows__cell--on={on}></i>
            {/each}
          </span>
          <span class="jrows__hours">{row.hoursLabel}</span>
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  /* `.btn` + `--ghost` / `--sm`: shared primitive (system.css §6). */
  .jrows {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  /* ── Menu anatomy ─────────────────────────────────────────────────────────
     24px rows, a 14px check gutter, the label, then a right-aligned mono key
     column. Highlight is a full-row accent fill (AppKit's), not a tint. A day
     with no recording is drawn as a disabled menu item and cannot be picked. */
  .jrows__list {
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .jrows__sep {
    height: var(--hairline);
    margin: var(--s-4) var(--s-8);
    background: var(--app-border-strong);
  }

  .jrows__row {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    width: 100%;
    height: 24px;
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: var(--r-sm);
    color: var(--app-text-strong);
    font-size: var(--t-ui);
    letter-spacing: var(--ls-ui);
    padding: 0 var(--s-8) 0 var(--s-4);
    cursor: pointer;
    transition: background 0.1s, color 0.1s;
  }
  .jrows__row:not(:disabled):hover {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  .jrows__row:not(:disabled):hover .jrows__hours,
  .jrows__row:not(:disabled):hover .jrows__kbd {
    color: inherit;
    background: transparent;
    box-shadow: none;
    opacity: 0.8;
  }
  .jrows__row:not(:disabled):hover .jrows__bar i {
    background: var(--app-accent-contrast);
  }
  .jrows__row:disabled {
    color: var(--app-text-subtle);
    opacity: 0.45;
    cursor: not-allowed;
  }
  .jrows__row:focus-visible {
    outline: none;
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  /* "You are here" is the menu's checkmark, in the gutter — the same signal
     AppKit uses for the item you are already on. */
  .jrows__ck {
    flex: 0 0 14px;
    width: 14px;
    display: inline-flex;
    justify-content: center;
    font-size: 10px;
    line-height: 1;
  }

  .jrows__day {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .jrows__hours,
  .jrows__kbd {
    margin-left: auto;
    padding-left: var(--s-16);
  }
  .jrows__hours {
    font-family: var(--app-font-mono);
    font-size: var(--t-meta);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .jrows__kbd {
    padding-left: 4px;
    margin-left: auto;
  }
  .jrows__row:disabled .jrows__hours {
    color: inherit;
  }

  .jrows__bar {
    display: flex;
    gap: 1px;
    flex: 0 0 auto;
    width: 72px;
    height: 6px;
    border-radius: 2px;
    overflow: hidden;
  }
  .jrows__bar i {
    flex: 1 1 0;
    background: var(--app-text-subtle);
    opacity: 0.28;
  }
  .jrows__bar i.jrows__cell--on {
    background: var(--app-accent);
    opacity: 0.85;
  }

  .jrows__msg {
    padding: 12px 8px;
    color: var(--app-text-subtle);
    font-size: var(--t-meta);
    text-align: center;
  }
</style>
