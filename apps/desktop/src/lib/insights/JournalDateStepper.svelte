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
  <span class="datestep">
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
  </span>
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
  /* Direction 05 "Tactile Instruments": one recessed segmented pill (‹ day ›),
     a pill's own outline being the one border kind this direction allows. The
     day label is still the calendar trigger — G6 drops the type-a-date FIELD,
     not the ability to jump to a day. */
  .date-stepper {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--s-8);
    color: var(--app-text-muted);
  }
  .datestep {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
  }
  .nav {
    height: 20px;
    min-width: 20px;
    padding: 0 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--app-text-muted);
    cursor: default;
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    transition: background var(--dur-quick) var(--ease);
  }
  .nav:hover:not(:disabled) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .nav:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .nav:disabled {
    opacity: var(--opacity-disabled);
  }
  .range-label {
    height: 20px;
    margin: 0;
    padding: 0 10px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    cursor: default;
    transition: background var(--dur-quick) var(--ease);
  }
  .range-label:hover {
    background: var(--app-surface-hover);
  }
  .range-label:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .today {
    height: var(--h-sm);
    padding: 0 var(--s-8);
    border: 0;
    border-radius: var(--r-md);
    background: transparent;
    color: var(--app-accent);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    cursor: default;
    transition: background var(--dur-quick) var(--ease);
  }
  .today:hover {
    background: var(--app-surface-hover);
  }
  .today:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .cal-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    width: 300px;
    background: var(--app-surface-raised);
    border: 0;
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-popover), 0 0 0 var(--hairline) var(--app-border-strong);
    overflow: hidden;
  }
  /* The calendar pane ships a border-right for the jumper's two-pane layout;
     standalone it reads as a stray line. Parent-scoped :global outranks the
     child's scoped rule (0-3-0 vs 0-2-0). */
  .cal-pop :global(.timeline__picker-cal) {
    border-right: none;
  }
</style>
