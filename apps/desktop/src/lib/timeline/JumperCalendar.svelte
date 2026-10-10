<script lang="ts">
  // ── Timeline Jumper — calendar pane ───────────────────────────────────────
  // bits-ui's Calendar (keeps its a11y roving grid + disabled-date predicate)
  // wearing the kit's `.mx-cal` skin. Two signifiers (spec §12.3):
  //   - previewed/selected day → accent FILL (bits-ui `[data-selected]`)
  //   - committed "you are here" day → soft accent tint (`[data-here]`), kept
  //     while previewing a different day so the timeline anchor is never lost.
  // Density dots (`<i data-h=0..3>`) are frames per day relative to the month.
  import { Calendar } from "bits-ui";
  import type { DateValue } from "@internationalized/date";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";

  interface Props {
    /** The previewed day (preview-on-select; does NOT move the timeline). */
    value?: DateValue;
    /** Viewed-month placeholder. */
    placeholder: DateValue;
    isDateDisabled: (d: DateValue) => boolean;
    /** Marks the cell that carries the committed-moment "you are here" tint. */
    isCommittedDate: (d: DateValue) => boolean;
    /** Capture density for a day, 0..3 (none when omitted). */
    heatOf?: (d: DateValue) => number;
  }

  let {
    value = $bindable(),
    placeholder = $bindable(),
    isDateDisabled,
    isCommittedDate,
    heatOf = () => 0,
  }: Props = $props();
</script>

<Calendar.Root
  type="single"
  bind:value
  bind:placeholder
  {isDateDisabled}
  weekdayFormat="narrow"
  weekStartsOn={1}
  class="mx-cal jumper-cal"
>
  {#snippet children({ months, weekdays })}
    <header class="mx-cal__head">
      <Calendar.Heading class="mx-cal__month" />
      <Calendar.PrevButton class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Previous month"><IconPrev width="14" height="14" /></Calendar.PrevButton>
      <Calendar.NextButton class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Next month"><IconNext width="14" height="14" /></Calendar.NextButton>
    </header>
    {#each months as month (month.value)}
      <Calendar.Grid class="jumper-cal__grid">
        <Calendar.GridHead>
          <Calendar.GridRow class="jumper-cal__row">
            {#each weekdays as wd, i (i)}
              <Calendar.HeadCell class="mx-cal__wd">{wd}</Calendar.HeadCell>
            {/each}
          </Calendar.GridRow>
        </Calendar.GridHead>
        <Calendar.GridBody>
          {#each month.weeks as weekDates, weekIdx (weekIdx)}
            <Calendar.GridRow class="jumper-cal__row">
              {#each weekDates as date (date.toString())}
                <Calendar.Cell {date} month={month.value} class="jumper-cal__cell">
                  <Calendar.Day class="mx-cal__day" data-here={isCommittedDate(date) || undefined}>
                    {#snippet children({ day })}<span>{day}</span><i data-h={heatOf(date)}></i>{/snippet}
                  </Calendar.Day>
                </Calendar.Cell>
              {/each}
            </Calendar.GridRow>
          {/each}
        </Calendar.GridBody>
      </Calendar.Grid>
    {/each}
    <div class="mx-cal__foot"><span class="mx-cal__legend"><i></i><i></i><i></i>captured</span></div>
  {/snippet}
</Calendar.Root>

<style>
  /* bits-ui renders a <table>; rows become the kit's 7 × 34px grid. Its state
     attributes (data-selected / -outside-month / -disabled) map onto the kit's
     day skin (data-sel / data-out / :disabled) here. */
  :global(.jumper-cal) {
    padding: 10px 10px 8px;
  }
  :global(.jumper-cal__grid) {
    border-collapse: collapse;
  }
  :global(.jumper-cal__row) {
    display: grid;
    grid-template-columns: repeat(7, 34px);
    margin-bottom: 2px;
  }
  :global(.jumper-cal__cell) {
    padding: 0;
  }
  :global(.jumper-cal .mx-cal__day[data-outside-month]) {
    color: var(--app-text-subtle);
  }
  :global(.jumper-cal .mx-cal__day[data-disabled]) {
    color: var(--app-text-faint);
    cursor: default;
    background: none;
  }
  :global(.jumper-cal .mx-cal__day[data-here]:not([data-selected])) {
    background: color-mix(in srgb, var(--app-accent) 11%, transparent);
    color: var(--app-text-strong);
  }
  :global(.jumper-cal .mx-cal__day[data-selected]) {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
    font-weight: 600;
  }
  :global(.jumper-cal .mx-cal__day[data-selected] i) {
    background: var(--app-accent-contrast);
    opacity: 0.7;
  }
  :global(.jumper-cal .mx-cal__day[data-selected] span) {
    box-shadow: none;
  }
  :global(.jumper-cal .mx-cal__day:focus-visible) {
    outline: 2px solid var(--app-accent);
    outline-offset: -2px;
  }
</style>
