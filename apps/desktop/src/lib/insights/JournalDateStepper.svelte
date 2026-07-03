<script lang="ts">
  // Journal date stepper — ‹ day › navigation, a calendar popover on the day
  // label (reuses the Timeline jumper's bits-ui calendar pane, so day-jumping
  // looks the same everywhere), and a Today reset. Split out of
  // DayTimeline.svelte to keep it under the 800-line ceiling.
  import { untrack } from "svelte";
  import {
    CalendarDate,
    getLocalTimeZone,
    today,
    type DateValue,
  } from "@internationalized/date";
  import JumperCalendar from "$lib/timeline/JumperCalendar.svelte";
  import { shiftAnchor } from "$lib/insights/activity-helpers";

  interface Props {
    /** The viewed day's anchor — the stepper writes it, the parent derives the range. */
    anchorMs: number;
    /** Local midnight of the viewed day (seeds the calendar). */
    rangeStartMs: number;
    /** True when the viewed day is the current day. */
    atLatest: boolean;
    dayLabel: string;
  }
  let { anchorMs = $bindable(), rangeStartMs, atLatest, dayLabel }: Props = $props();

  let open = $state(false);
  let calValue = $state<DateValue | undefined>(undefined);
  let calPlaceholder = $state<DateValue>(today(getLocalTimeZone()));
  let popEl = $state<HTMLDivElement | null>(null);
  let triggerEl = $state<HTMLButtonElement | null>(null);

  function viewedDate(): CalendarDate {
    const d = new Date(rangeStartMs);
    return new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate());
  }

  function toggle(): void {
    open = !open;
    if (open) {
      // Seed to the viewed day so the popover reflects "you are here".
      const cd = viewedDate();
      calValue = cd;
      calPlaceholder = cd;
    }
  }

  function isFuture(d: DateValue): boolean {
    return d.compare(today(getLocalTimeZone())) > 0;
  }

  // Picking a day commits it and closes. Local noon dodges DST-boundary
  // midnights. The seed write (same day) is a no-op by the compare guard.
  $effect(() => {
    const v = calValue;
    if (!open || !v) return;
    untrack(() => {
      if (v.compare(viewedDate()) === 0) return;
      anchorMs = new Date(v.year, v.month - 1, v.day, 12).getTime();
      open = false;
    });
  });

  function onWindowPointerDown(e: PointerEvent): void {
    if (!open) return;
    const t = e.target as Node | null;
    if (!t || popEl?.contains(t) || triggerEl?.contains(t)) return;
    open = false;
  }

  function onWindowKeydown(e: KeyboardEvent): void {
    if (open && e.key === "Escape") {
      e.preventDefault();
      open = false;
      triggerEl?.focus();
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<div class="date-stepper">
  <button
    class="nav"
    type="button"
    aria-label="Previous day"
    onclick={() => (anchorMs = shiftAnchor(anchorMs, "day", -1))}>‹</button
  >
  <button
    class="range-label"
    type="button"
    bind:this={triggerEl}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label="Jump to date"
    onclick={toggle}>{dayLabel}</button
  >
  <button
    class="nav"
    type="button"
    aria-label="Next day"
    disabled={atLatest}
    onclick={() => (anchorMs = shiftAnchor(anchorMs, "day", 1))}>›</button
  >
  {#if !atLatest}
    <button class="today" type="button" onclick={() => (anchorMs = Date.now())}
      >Today</button
    >
  {/if}

  {#if open}
    <div class="cal-pop" role="dialog" aria-label="Jump to date" bind:this={popEl}>
      <JumperCalendar
        bind:value={calValue}
        bind:placeholder={calPlaceholder}
        isDateDisabled={isFuture}
        isCommittedDate={() => false}
      />
    </div>
  {/if}
</div>

<style>
  .date-stepper {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
  }
  .nav {
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-subtle);
    cursor: pointer;
    font: inherit;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .nav:hover:not(:disabled) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .nav:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .nav:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: default;
  }
  .range-label {
    margin: 0;
    padding: 2px 4px;
    border: 0;
    background: transparent;
    font: inherit;
    color: var(--app-text);
    letter-spacing: 0.02em;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    border-radius: 4px;
    border-bottom: 1px dotted var(--app-border-strong);
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .range-label:hover {
    color: var(--app-accent);
    border-bottom-color: var(--app-accent-border);
  }
  .range-label:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .today {
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-subtle);
    font: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .today:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .today:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .cal-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    width: 300px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    box-shadow: var(--app-shadow-popover);
    overflow: hidden;
  }
  /* The calendar pane ships a border-right for the jumper's two-pane layout;
     standalone it reads as a stray line. Parent-scoped :global outranks the
     child's scoped rule (0-3-0 vs 0-2-0). */
  .cal-pop :global(.timeline__picker-cal) {
    border-right: none;
  }
</style>
