<script lang="ts">
  // Timeline — a chronological / time-of-day breakdown of the user's day, shown
  // inline as a Chat answer-chart. Renders a vertical timeline rail: a thin spine
  // with a per-row colour dot (category colour), the time range in a quiet
  // tabular monospace, the label as the primary text, and an optional app chip.
  //
  // The caller passes an already-validated, parsed array of intervals (the Chat
  // answer parser does the validation), so this component is presentation-only
  // and defensive about missing end / app / category.
  //
  // Props:
  //   title?: string | null            — small uppercase muted caption at the top.
  //   intervals: {                      — time-ordered rows.
  //     label: string;
  //     start: string;                  — human time-of-day, e.g. "9:30 AM".
  //     end?: string | null;            — optional human time-of-day.
  //     app?: string | null;            — optional app / window context chip.
  //     category?: string | null;       — optional category key for the dot colour.
  //   }[]

  import { CATEGORY_COLOR } from "$lib/insights/activity-helpers";

  interface TimelineInterval {
    label: string;
    start: string;
    end?: string | null;
    app?: string | null;
    category?: string | null;
  }

  interface Props {
    title?: string | null;
    intervals: TimelineInterval[];
  }

  let { title = null, intervals }: Props = $props();

  // Unknown/missing category falls back to the neutral chart grey — mirrors the
  // UNCATEGORIZED_COLOR used elsewhere in the Insights surfaces.
  const FALLBACK_COLOR = "--chart-grey-3";

  function colorVarFor(category?: string | null): string {
    if (!category) return FALLBACK_COLOR;
    return (CATEGORY_COLOR as Record<string, string>)[category] ?? FALLBACK_COLOR;
  }

  function timeRange(interval: TimelineInterval): string {
    if (interval.end) return `${interval.start} – ${interval.end}`;
    return interval.start;
  }
</script>

{#if intervals.length > 0}
  <div class="timeline">
    {#if title}
      <div class="timeline-title">{title}</div>
    {/if}
    <ol class="rail">
      {#each intervals as interval, i (i)}
        <li class="row">
          <span class="spine" aria-hidden="true">
            <span class="dot" style="background:var({colorVarFor(interval.category)});"></span>
          </span>
          <span class="body">
            <span class="time">{timeRange(interval)}</span>
            <span class="label">
              <span class="label-text">{interval.label}</span>
              {#if interval.app}
                <span class="app-chip">{interval.app}</span>
              {/if}
            </span>
          </span>
        </li>
      {/each}
    </ol>
  </div>
{/if}

<style>
  .timeline {
    display: flex;
    flex-direction: column;
  }
  .timeline-title {
    font-size: 10.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    margin: 0 0 10px;
  }
  .rail {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .row {
    display: grid;
    grid-template-columns: 14px 1fr;
    gap: 9px;
    align-items: stretch;
  }
  /* The spine is the vertical rail: a thin centred line that the per-row dot
     sits on. ::before draws the connector, the dot caps it for that row. */
  .spine {
    position: relative;
    display: block;
    width: 14px;
  }
  .spine::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
    background: var(--app-border);
  }
  /* Top of the first row and bottom of the last row taper the spine so it reads
     as a contained rail rather than running off the edges. */
  .row:first-child .spine::before {
    top: 7px;
  }
  .row:last-child .spine::before {
    bottom: calc(100% - 7px);
  }
  .dot {
    position: absolute;
    top: 6px;
    left: 50%;
    width: 7px;
    height: 7px;
    border-radius: 999px;
    transform: translateX(-50%);
    box-shadow: 0 0 0 2px var(--app-surface-subtle);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    padding-bottom: 12px;
  }
  .row:last-child .body {
    padding-bottom: 0;
  }
  .time {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.01em;
  }
  .label {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: 0;
    flex-wrap: wrap;
  }
  .label-text {
    font-size: 12px;
    color: var(--app-text);
    line-height: 1.4;
    min-width: 0;
  }
  .app-chip {
    flex: 0 0 auto;
    font-size: 9.5px;
    color: var(--app-text-muted);
    padding: 1px 6px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    white-space: nowrap;
  }
</style>
