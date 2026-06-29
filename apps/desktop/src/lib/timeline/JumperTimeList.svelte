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
  // hour shows the accent LEFT BAR (§12.3), echoing the playhead.
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

  function densityFraction(count: number): number {
    if (count <= 0) return 0;
    return count / Math.max(1, maxCount);
  }
</script>

<div class="timeline__picker-time">
  <div class="timeline__picker-time-head">
    <span class="timeline__picker-day">{dayLabel || "—"}</span>
    {#if hasSelection && dayHasFrames && !loading}
      <span class="timeline__picker-day-count"
        >{dayHourCount} hr</span
      >
    {/if}
  </div>

  <div class="timeline__picker-scroll">
    {#if !hasSelection}
      <div class="timeline__picker-msg">
        <span class="timeline__picker-msg-ico" aria-hidden="true">◴</span>
        Select a day to see hours
      </div>
    {:else if loading}
      <div class="timeline__picker-msg">
        <span class="timeline__picker-spinner" aria-hidden="true"></span>
        loading month…
      </div>
    {:else if !dayHasFrames}
      <div class="timeline__picker-msg">
        <span class="timeline__picker-msg-ico" aria-hidden="true">∅</span>
        No frames on this day
      </div>
    {:else}
      {#each buckets as t (t.hour)}
        <button
          type="button"
          class="timeline__picker-hour"
          class:timeline__picker-hour--here={isHereHour(t.hour) && !t.disabled}
          onclick={() => onCommitHour(t.hour)}
          disabled={busy || t.disabled}
        >
          {#if !t.disabled && t.count > 0}
            <span
              class="timeline__picker-hour-density"
              style="--density:{densityFraction(t.count)}"
              aria-hidden="true"
            ></span>
          {/if}
          <span class="timeline__picker-hour-tick" aria-hidden="true"></span>
          <span class="timeline__picker-hour-label">{t.label}</span>
          <span class="timeline__picker-hour-count"
            >{t.disabled ? "·" : t.count}</span
          >
        </button>
      {/each}
    {/if}
  </div>

  <div class="timeline__picker-time-foot">
    <button
      class="btn btn--ghost btn--sm timeline__picker-day-latest"
      onclick={onCommitDayLatest}
      disabled={busy || !hasSelection || !dayHasFrames}
    >
      <span class="timeline__picker-glyph" aria-hidden="true">⤓</span>
      latest of day
    </button>
  </div>
</div>

<style>
  .timeline__picker-time {
    display: flex;
    flex-direction: column;
    min-width: 0;
    /* min-height:0 lets the inner scroll pane bound itself against the grid
       row instead of forcing the whole column to content height. */
    min-height: 0;
    overflow: hidden;
  }

  .timeline__picker-time-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px 6px;
    border-bottom: 1px solid var(--app-border);
  }
  .timeline__picker-day {
    font-size: var(--text-base);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .timeline__picker-day-count {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .timeline__picker-scroll {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 6px 8px;
    min-height: 160px;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
  }
  .timeline__picker-scroll::-webkit-scrollbar {
    width: 8px;
  }
  .timeline__picker-scroll::-webkit-scrollbar-thumb {
    background: var(--app-border-strong);
    border-radius: 4px;
    border: 2px solid var(--app-surface);
  }

  .timeline__picker-hour {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 3px;
    color: var(--app-text);
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    padding: 4px 8px;
    margin-bottom: 1px;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  /* Neutral density fill (NEVER accent) — pre-attentive "where was I busy". */
  .timeline__picker-hour-density {
    position: absolute;
    inset: 0;
    border-radius: 3px;
    background: var(--app-text);
    opacity: calc(var(--density, 0) * 0.06);
    pointer-events: none;
  }
  .timeline__picker-hour-tick {
    position: relative;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--app-accent-strong);
    flex: none;
    opacity: 0.85;
  }
  .timeline__picker-hour-label {
    position: relative;
  }
  .timeline__picker-hour-count {
    position: relative;
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    letter-spacing: 0.02em;
  }
  .timeline__picker-hour:not(:disabled):hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .timeline__picker-hour:disabled {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }
  .timeline__picker-hour:disabled .timeline__picker-hour-tick {
    background: var(--app-text-faint);
    opacity: 0.5;
  }
  .timeline__picker-hour:disabled .timeline__picker-hour-count {
    color: var(--app-text-faint);
  }
  /* "You are here" — accent LEFT BAR (no fill), echoing the playhead. */
  .timeline__picker-hour--here {
    color: var(--app-accent);
    box-shadow: inset 2px 0 0 0 var(--app-accent);
  }
  .timeline__picker-hour--here .timeline__picker-hour-tick {
    background: var(--app-accent);
    opacity: 1;
  }
  .timeline__picker-hour--here .timeline__picker-hour-count {
    color: var(--app-accent);
  }
  .timeline__picker-hour:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-color: var(--app-accent-border);
  }

  .timeline__picker-msg {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 16px;
    text-align: center;
    color: var(--app-text-subtle);
    font-size: var(--text-sm);
    min-height: 140px;
  }
  .timeline__picker-msg-ico {
    font-size: var(--text-xl);
    opacity: 0.5;
  }
  .timeline__picker-spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--app-border-strong);
    border-top-color: var(--app-accent);
    border-radius: 50%;
    animation: timeline-jumper-spin 0.7s linear infinite;
  }
  @keyframes timeline-jumper-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .timeline__picker-spinner {
      animation-duration: 1.4s;
    }
  }

  .timeline__picker-time-foot {
    padding: 8px 12px;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }
  .timeline__picker-day-latest {
    width: 100%;
    justify-content: center;
    gap: 7px;
  }
  .timeline__picker-glyph {
    font-size: var(--text-md);
    line-height: 1;
  }

  /* Shared button system (local copy — `.btn` is defined per-surface in this
     app, not in a global sheet). */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }
  .btn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }
  .btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .btn:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }
  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
  }
  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }
  .btn--sm {
    padding: 3px 8px;
    font-size: var(--text-sm);
  }
</style>
