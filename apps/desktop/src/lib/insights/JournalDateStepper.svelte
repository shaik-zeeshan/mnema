<script lang="ts">
  // The Journal's date control (page 08) — the Timeline's glass capsule idiom:
  // ‹ | calendar + day | ›, one piece of level-5 material riding in the title
  // bar. The day label opens the same bits-ui calendar pane the Timeline jumper
  // uses, so day-jumping looks identical everywhere. State lives in the
  // `journalDate` store (journal-date.svelte.ts) because the surface that loads
  // the day renders in a different tree.
  import { untrack } from "svelte";
  import {
    CalendarDate,
    getLocalTimeZone,
    today,
    type DateValue,
  } from "@internationalized/date";
  import IconCalendar from "~icons/lucide/calendar";
  import IconChevronLeft from "~icons/lucide/chevron-left";
  import IconChevronRight from "~icons/lucide/chevron-right";
  import JumperCalendar from "$lib/timeline/JumperCalendar.svelte";
  import { journalDate } from "$lib/insights/journal-date.svelte";

  let open = $state(false);
  let calValue = $state<DateValue | undefined>(undefined);
  let calPlaceholder = $state<DateValue>(today(getLocalTimeZone()));
  let popEl = $state<HTMLDivElement | null>(null);
  let triggerEl = $state<HTMLButtonElement | null>(null);

  function viewedDate(): CalendarDate {
    const d = new Date(journalDate.range.startMs);
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

  // Picking a day commits it and closes. The seed write (same day) is a no-op
  // by the compare guard.
  $effect(() => {
    const v = calValue;
    if (!open || !v) return;
    untrack(() => {
      if (v.compare(viewedDate()) === 0) return;
      journalDate.setDay(v.year, v.month, v.day);
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

<div class="datecap">
  <button
    class="datecap__nav"
    type="button"
    aria-label="Previous day"
    onclick={() => journalDate.step(-1)}><IconChevronLeft /></button
  >
  <button
    class="datecap__day"
    type="button"
    bind:this={triggerEl}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label="Jump to date"
    onclick={toggle}
  >
    <IconCalendar />
    <span class="is-num">{journalDate.dayLabel}</span>
  </button>
  <button
    class="datecap__nav"
    type="button"
    aria-label="Next day"
    disabled={journalDate.atLatest}
    onclick={() => journalDate.step(1)}><IconChevronRight /></button
  >

  {#if open}
    <div class="cal-pop glass-pop" role="dialog" aria-label="Jump to date" bind:this={popEl}>
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
  /* The capsule is level-5 material (`--glass-pop`): it floats over the chrome
     the way the Timeline's jump capsule floats over the stage. Its own rim is
     the only edge it spends. */
  .datecap {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 26px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .datecap__nav {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .datecap__nav:hover:not(:disabled) {
    background: var(--glass-tint);
    color: var(--app-text-strong);
  }
  .datecap__nav:disabled {
    opacity: var(--opacity-disabled);
    cursor: default;
  }
  .datecap__nav :global(svg) {
    width: 12px;
    height: 12px;
  }
  .datecap__day {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--r-pill);
    background: transparent;
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .datecap__day:hover {
    background: var(--glass-tint);
  }
  .datecap__nav:focus-visible,
  .datecap__day:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .datecap__day :global(svg) {
    width: 12px;
    height: 12px;
    color: var(--app-text-muted);
  }
  .cal-pop {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 40;
    width: 300px;
    border-radius: var(--r-lg);
    overflow: hidden;
  }
  /* The calendar pane ships a border-right for the jumper's two-pane layout;
     standalone it reads as a stray line. Parent-scoped :global outranks the
     child's scoped rule (0-3-0 vs 0-2-0). */
  .cal-pop :global(.timeline__picker-cal) {
    border-right: none;
  }
</style>
