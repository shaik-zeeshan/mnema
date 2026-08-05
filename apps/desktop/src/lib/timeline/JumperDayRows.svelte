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
  <!-- NSMenu anatomy: 24px rows, the shortcut column right-aligned, hairline
       separators between the sections. "Now" carries the real L binding; the
       other two have no shortcut and leave the column empty rather than
       inventing one. -->
  <div class="jrows__targets">
    <button type="button" class="jrows__row" onclick={onJumpNow} disabled={busy}>
      <span class="jrows__day">Now</span>
      <span class="jrows__kbd">L</span>
    </button>
    <button
      type="button"
      class="jrows__row"
      onclick={onJumpMorning}
      disabled={busy || !morningEnabled}
    >
      <span class="jrows__day">This morning</span>
    </button>
    <button
      type="button"
      class="jrows__row"
      onclick={onJumpYesterday}
      disabled={busy || !yesterdayEnabled}
    >
      <span class="jrows__day">Yesterday</span>
    </button>
  </div>

  <div class="jrows__sep" aria-hidden="true"></div>

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
  /* TACTILE — NSMenu anatomy. A menu row is 24px tall, highlights as a FULL-ROW
     accent fill (never a border), and keeps its numbers mono + tabular. Depth
     here is the menu's own material; nothing inside it is a box. */
  .jrows {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .jrows__targets,
  .jrows__list {
    display: flex;
    flex-direction: column;
    padding: 0 var(--s-4);
  }

  .jrows__sep {
    height: var(--hairline);
    background: var(--app-border);
    margin: var(--s-4) var(--s-8);
  }

  .jrows__row {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    width: 100%;
    height: 24px;
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: var(--r-sm);
    color: var(--app-text-strong);
    font-size: var(--t-ui);
    line-height: 1;
    padding: 0 var(--s-8);
    cursor: pointer;
  }
  .jrows__row:not(:disabled):hover {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  /* A day with nothing recorded is DISABLED — you can never land on it (G6). */
  .jrows__row:disabled {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }
  .jrows__row:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
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
  /* The ⌘ column: right-aligned, and empty when there is no real binding. */
  .jrows__kbd,
  .jrows__hours {
    margin-left: auto;
    font-family: var(--app-font-mono);
    font-size: var(--t-meta);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .jrows__row:not(:disabled):hover .jrows__kbd,
  .jrows__row:not(:disabled):hover .jrows__hours {
    color: color-mix(in srgb, var(--app-accent-contrast) 75%, transparent);
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
  .jrows__row:not(:disabled):hover .jrows__bar i {
    background: color-mix(in srgb, var(--app-accent-contrast) 40%, transparent);
    opacity: 1;
  }
  .jrows__row:not(:disabled):hover .jrows__bar i.jrows__cell--on {
    background: var(--app-accent-contrast);
  }

  .jrows__msg {
    padding: var(--s-12) var(--s-8);
    color: var(--app-text-subtle);
    font-size: var(--t-meta);
    text-align: center;
  }
</style>
