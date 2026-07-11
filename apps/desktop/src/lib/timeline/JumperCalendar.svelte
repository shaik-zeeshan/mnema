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
  .timeline__picker-cal {
    padding: 12px;
    border-right: 1px solid var(--app-border);
    /* Top-align the calendar so its nav row sits level with the time pane
       header — no dead space above it. The grid's square cells fill the width. */
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
  }
  @media (max-width: 640px) {
    .timeline__picker-cal {
      border-right: none;
      border-bottom: 1px solid var(--app-border);
    }
  }

  :global(.cal) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--app-text);
    /* Fill the calendar pane so the week rows can stretch to its full height. */
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 2px 4px;
  }
  :global(.cal__nav) {
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    border-radius: 3px;
    color: var(--app-text-muted);
    cursor: pointer;
    font-size: var(--text-md);
    line-height: 1;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  :global(.cal__nav:hover:not([data-disabled])) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  :global(.cal__nav[data-disabled]) {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }
  :global(.cal__nav:focus-visible) {
    outline: none;
    box-shadow: var(--app-ring);
    border-color: var(--app-accent-border);
  }
  :global(.cal__heading) {
    font-size: var(--text-md);
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--app-text-strong);
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
  }
  /* Week rows (body only — not the weekday header) stretch to fill. */
  :global(.cal__grid tbody .cal__row) {
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(.cal__weekday) {
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    text-align: center;
    padding: 4px 0;
  }
  :global(.cal__cell) {
    padding: 1px;
  }
  :global(.cal__day) {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 3px;
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  :global(.cal__day:hover:not([data-disabled])) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global(.cal__day[data-disabled]),
  :global(.cal__day[data-outside-month]) {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }
  /* Previewing / selected — accent FILL recipe (matches shipped calendar). */
  :global(.cal__day[data-selected]) {
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--app-accent) 50%, transparent);
    color: var(--app-accent);
  }
  :global(.cal__day[data-today]:not([data-selected])) {
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
  }
  /* "You are here" — accent LEFT BAR; layers atop the fill on the committed
     day and stays visible when previewing a different day. */
  :global(.cal__day--here) {
    box-shadow: inset 2px 0 0 0 var(--app-accent);
  }
  :global(.cal__day--here:not([data-selected])) {
    color: var(--app-accent);
  }
  :global(.cal__day:focus-visible) {
    outline: none;
    box-shadow: var(--app-ring);
    border-color: var(--app-accent-border);
  }
</style>
