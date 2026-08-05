<script lang="ts">
  // ── Timeline Jumper — quick targets + seven day tiles ─────────────────────
  // The jump menu's fixed head (round-4 decision G6): quick targets
  // Now / This morning / Yesterday, then seven days, each carrying its own
  // coverage bar and captured hours. A day with no recording is DISABLED — you
  // can never land on an empty day. There is no type-a-date field (dropped from
  // v1 by G6).
  //
  // Direction 01 draws the seven days as a row of bento day TILES rather than a
  // list, oldest → newest left to right so the newest sits on the right exactly
  // as it does on the rail.
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

  // `rows` arrives newest-first; the tiles read left-to-right in time.
  const tiles = $derived([...rows].reverse());

  const WEEKDAY = new Intl.DateTimeFormat(undefined, { weekday: "short" });
  function weekday(row: DayRow): string {
    return WEEKDAY.format(new Date(row.date.year, row.date.month - 1, row.date.day));
  }
</script>

<div class="jrows">
  <div class="jrows__targets">
    <button
      type="button"
      class="jrows__target"
      onclick={onJumpNow}
      disabled={busy}>Now</button
    >
    <button
      type="button"
      class="jrows__target"
      onclick={onJumpMorning}
      disabled={busy || !morningEnabled}>This morning</button
    >
    <button
      type="button"
      class="jrows__target"
      onclick={onJumpYesterday}
      disabled={busy || !yesterdayEnabled}>Yesterday</button
    >
  </div>

  {#if loading}
    <div class="jrows__msg">loading coverage…</div>
  {:else}
    <div class="jrows__week">
      {#each tiles as row (row.key)}
        <button
          type="button"
          class="jday"
          class:jday--here={row.key === hereKey && !row.disabled}
          onclick={() => onJumpDay(row)}
          disabled={busy || row.disabled}
          aria-label={`${row.label} — ${row.hoursLabel} captured`}
          title={row.label}
        >
          <span class="jday__d">{weekday(row)}</span>
          <span class="jday__n is-num">{row.date.day}</span>
          <span class="jday__cov" aria-hidden="true">
            {#each row.cells as on, i (i)}
              <i class:jday__cell--on={on}></i>
            {/each}
          </span>
          <span class="jday__h is-num">{row.disabled ? "—" : row.hoursLabel}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .jrows {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    min-width: 0;
    padding: var(--s-4) var(--s-8) var(--s-8);
  }

  /* Three quick targets on one row — the panel's only push-bezel controls. */
  .jrows__targets {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--s-6);
  }
  .jrows__target {
    height: var(--h-md);
    min-width: 0;
    padding: 0 var(--s-6);
    border: 0;
    border-radius: var(--r-md);
    background: var(--app-surface-raised) var(--push-grad);
    box-shadow: 0 0.5px 1.5px rgba(0, 0, 0, 0.25);
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .jrows__target:not(:disabled):hover {
    background-color: var(--app-surface-hover);
  }
  .jrows__target:disabled {
    opacity: var(--opacity-disabled);
    cursor: not-allowed;
  }
  .jrows__target:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }

  /* ── The seven day tiles ─────────────────────────────────────────────────
     One tile per day: mono weekday eyebrow, the date, its own coverage bar,
     and the captured hours underneath. Same object, seven times. */
  .jrows__week {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: var(--s-6);
  }
  .jday {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    min-width: 0;
    padding: var(--s-6) 2px var(--s-4);
    border: 0;
    border-radius: var(--r-lg);
    background: var(--tile-fill);
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .jday:not(:disabled):hover {
    background: var(--tile-fill-hover);
  }
  .jday:disabled {
    opacity: 0.34;
    cursor: not-allowed;
  }
  .jday:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }
  .jday__d {
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .jday__n {
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .jday__cov {
    display: flex;
    gap: 1px;
    align-self: stretch;
    margin: 4px 5px 3px;
    height: 3px;
    border-radius: 2px;
    overflow: hidden;
  }
  .jday__cov i {
    flex: 1 1 0;
    background: var(--app-border-strong);
  }
  .jday__cov i.jday__cell--on {
    background: var(--app-accent);
  }
  .jday__h {
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  /* "You are here" — full accent fill, the native selected-row rule. */
  .jday--here {
    background: var(--app-accent);
  }
  .jday--here:not(:disabled):hover {
    background: var(--app-accent);
  }
  .jday--here .jday__d,
  .jday--here .jday__n,
  .jday--here .jday__h {
    color: var(--app-accent-contrast);
  }
  .jday--here .jday__cov i {
    background: color-mix(in srgb, var(--app-accent-contrast) 30%, transparent);
  }
  .jday--here .jday__cov i.jday__cell--on {
    background: var(--app-accent-contrast);
  }

  .jrows__msg {
    padding: var(--s-12) var(--s-8);
    color: var(--app-text-subtle);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    text-align: center;
  }

  @media (prefers-reduced-motion: reduce) {
    .jrows__target,
    .jday {
      transition: none;
    }
  }
</style>
