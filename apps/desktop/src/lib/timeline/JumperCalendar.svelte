<script lang="ts">
  // ── Timeline Jumper — calendar pane ───────────────────────────────────────
  // Thin restyle wrapper around bits-ui's Calendar (keeps its a11y roving-grid
  // + disabled-date predicate). Two-signifier split (spec §12.3):
  //   - previewed/selected day → accent FILL (bits-ui `[data-selected]`)
  //   - committed "you are here" day → accent LEFT BAR (`.cal__day--here`),
  //     which stays visible even while previewing a different day so the
  //     timeline anchor is never lost.
  import { Calendar } from "bits-ui";
  import type { DateValue } from "@internationalized/date";

  interface Props {
    /** The previewed day (preview-on-select; does NOT move the timeline). */
    value?: DateValue;
    /** Viewed-month placeholder. */
    placeholder: DateValue;
    isDateDisabled: (d: DateValue) => boolean;
    /** Marks the cell that carries the committed-moment "you are here" bar. */
    isCommittedDate: (d: DateValue) => boolean;
  }

  let {
    value = $bindable(),
    placeholder = $bindable(),
    isDateDisabled,
    isCommittedDate,
  }: Props = $props();
</script>

<div class="timeline__picker-cal">
  <Calendar.Root
    type="single"
    bind:value
    bind:placeholder
    {isDateDisabled}
    weekdayFormat="short"
    class="cal"
  >
    {#snippet children({ months, weekdays })}
      <header class="cal__header">
        <Calendar.PrevButton class="cal__nav">‹</Calendar.PrevButton>
        <Calendar.Heading class="cal__heading" />
        <Calendar.NextButton class="cal__nav">›</Calendar.NextButton>
      </header>
      {#each months as month (month.value)}
        <Calendar.Grid class="cal__grid">
          <Calendar.GridHead>
            <Calendar.GridRow class="cal__row">
              {#each weekdays as wd (wd)}
                <Calendar.HeadCell class="cal__weekday">{wd}</Calendar.HeadCell>
              {/each}
            </Calendar.GridRow>
          </Calendar.GridHead>
          <Calendar.GridBody>
            {#each month.weeks as weekDates, weekIdx (weekIdx)}
              <Calendar.GridRow class="cal__row">
                {#each weekDates as date (date.toString())}
                  <Calendar.Cell {date} month={month.value} class="cal__cell">
                    <Calendar.Day
                      class={isCommittedDate(date)
                        ? "cal__day cal__day--here"
                        : "cal__day"}
                    />
                  </Calendar.Cell>
                {/each}
              </Calendar.GridRow>
            {/each}
          </Calendar.GridBody>
        </Calendar.Grid>
      {/each}
    {/snippet}
  </Calendar.Root>
</div>

<style>
  /* Direction 01: the month grid is a bare bento grid — no cell borders, a
     full accent fill on the previewed day, and a 3px accent dot under any day
     that actually holds recording. Days without coverage are disabled by the
     predicate above, so the grid can never land you on an empty day (G6). */
  .timeline__picker-cal {
    padding: var(--s-4) var(--s-8) var(--s-8);
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
  }
  :global(.cal) {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    color: var(--app-text);
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-8);
    height: var(--tile-hd);
  }
  /* The month stepper — the direction's `.step` pair, borderless on a tinted
     well, exactly like the rail's frame stepper. */
  :global(.cal__nav) {
    width: 22px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--r-sm);
    background: color-mix(in srgb, var(--app-text-strong) 7%, transparent);
    color: var(--app-text-muted);
    cursor: pointer;
    font-size: var(--t-ui);
    line-height: 1;
    transition: background-color var(--dur-quick) var(--ease);
  }
  :global(.cal__nav:hover:not([data-disabled])) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  :global(.cal__nav[data-disabled]) {
    opacity: var(--opacity-disabled);
    cursor: not-allowed;
  }
  :global(.cal__nav:focus-visible) {
    outline: none;
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }
  :global(.cal__heading) {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  /* Override the bits-ui <table> layout into a flex column so the grid body
     fills the remaining height and each week row shares it equally. */
  :global(.cal__grid) {
    border-collapse: collapse;
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__grid thead) {
    display: block;
    flex: 0 0 auto;
  }
  :global(.cal__grid tbody) {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__row) {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 2px;
  }
  /* Week rows (body only — not the weekday header) stretch to fill. */
  :global(.cal__grid tbody .cal__row) {
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__weekday) {
    height: 18px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    text-align: center;
    vertical-align: middle;
  }
  :global(.cal__cell) {
    padding: 0;
  }
  :global(.cal__day) {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  /* A day that holds recording carries an accent dot; a day that does not is
     disabled and faint — the one signal, drawn once. */
  :global(.cal__day:not([data-disabled]):not([data-outside-month])::after) {
    content: "";
    position: absolute;
    bottom: 3px;
    left: 50%;
    transform: translateX(-50%);
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  :global(.cal__day:hover:not([data-disabled])) {
    background: var(--app-surface-hover);
  }
  :global(.cal__day[data-disabled]),
  :global(.cal__day[data-outside-month]) {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }
  /* Previewing / selected — full accent fill (the native selected-row rule). */
  :global(.cal__day[data-selected]) {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }
  :global(.cal__day[data-selected]::after) {
    background: var(--app-accent-contrast);
  }
  :global(.cal__day[data-today]:not([data-selected])) {
    color: var(--app-text-strong);
    font-weight: var(--w-semi);
  }
  /* "You are here" — accent left bar, echoing the playhead; layers atop the
     fill on the committed day and survives previewing a different day. */
  :global(.cal__day--here) {
    box-shadow: inset 2px 0 0 0 var(--app-accent);
  }
  :global(.cal__day--here:not([data-selected])) {
    color: var(--app-accent);
  }
  :global(.cal__day:focus-visible) {
    outline: none;
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.cal__day),
    :global(.cal__nav) {
      transition: none;
    }
  }
</style>
