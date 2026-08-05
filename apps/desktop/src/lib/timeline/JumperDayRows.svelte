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
  <div class="jrows__targets">
    <button
      type="button"
      class="btn btn--ghost btn--sm jrows__target"
      onclick={onJumpNow}
      disabled={busy}>now</button
    >
    <button
      type="button"
      class="btn btn--ghost btn--sm jrows__target"
      onclick={onJumpMorning}
      disabled={busy || !morningEnabled}>this morning</button
    >
    <button
      type="button"
      class="btn btn--ghost btn--sm jrows__target"
      onclick={onJumpYesterday}
      disabled={busy || !yesterdayEnabled}>yesterday</button
    >
  </div>

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

  .jrows__targets {
    display: flex;
    gap: 4px;
    padding: 8px 8px 6px;
    border-bottom: 1px solid var(--app-border);
  }
  .jrows__target {
    flex: 1 1 0;
    min-width: 0;
    justify-content: center;
  }

  .jrows__list {
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .jrows__row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: var(--app-text);
    font-size: var(--t-meta);
    padding: 4px 8px;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .jrows__row:not(:disabled):hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .jrows__row:disabled {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }
  .jrows__row:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-color: var(--app-accent-border);
  }
  /* "You are here" — the same accent LEFT BAR the calendar and hour list use. */
  .jrows__row--here {
    color: var(--app-accent);
    box-shadow: inset 2px 0 0 0 var(--app-accent);
  }

  .jrows__day {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .jrows__hours {
    margin-left: auto;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .jrows__row:disabled .jrows__hours {
    color: var(--app-text-faint);
  }

  .jrows__bar {
    display: flex;
    gap: 1px;
    flex: 0 0 auto;
    width: 72px;
    height: 8px;
    border-radius: 2px;
    overflow: hidden;
  }
  .jrows__bar i {
    flex: 1 1 0;
    background: var(--app-text-faint);
    opacity: 0.5;
  }
  .jrows__bar i.jrows__cell--on {
    background: var(--app-accent);
    opacity: 1;
  }
  .jrows__row--here .jrows__bar i.jrows__cell--on {
    background: var(--app-accent);
  }

  .jrows__msg {
    padding: 12px 8px;
    color: var(--app-text-subtle);
    font-size: var(--t-meta);
    text-align: center;
  }
</style>
