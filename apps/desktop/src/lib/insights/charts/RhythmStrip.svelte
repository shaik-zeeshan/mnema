<script lang="ts">
  // RhythmStrip — the range's shape at a glance: one column per time bucket,
  // each column a stacked bar of that bucket's category mix. Column height is
  // the bucket total relative to the busiest bucket; an empty bucket renders a
  // faint baseline tick so the time axis reads continuously. A glance graphic,
  // not a control — no interactivity.
  // Props:
  //   columns: { label: string; title: string; segments: { colorVar: string; value: number }[] }[]
  //            label is the sparse axis text under the column ("" = none);
  //            title is the full hover/a11y readout (e.g. "Tuesday · 6h 20m");
  //            segments carry raw durations with CSS custom-property NAMES,
  //            e.g. "--cat-coding" / "--chart-grey-3", stacked bottom-up in
  //            the order given.

  interface StripSegment {
    colorVar: string;
    value: number;
  }
  interface StripColumn {
    label: string;
    title: string;
    segments: StripSegment[];
  }
  interface Props {
    columns: StripColumn[];
  }

  let { columns }: Props = $props();

  function totalOf(col: StripColumn): number {
    return col.segments.reduce((acc, s) => acc + Math.max(0, s.value), 0);
  }

  const max = $derived(columns.reduce((acc, c) => Math.max(acc, totalOf(c)), 0));

  function heightFor(col: StripColumn): number {
    if (max <= 0) return 0;
    return Math.max(0, Math.min(100, (totalOf(col) / max) * 100));
  }

  function segmentPct(col: StripColumn, value: number): number {
    const total = totalOf(col);
    if (total <= 0) return 0;
    return (Math.max(0, value) / total) * 100;
  }
</script>

<div class="rhythm" role="img" aria-label={columns.map((c) => c.title).join(", ")}>
  {#each columns as col, i (i)}
    <div class="col" title={col.title}>
      <div class="bars">
        {#if totalOf(col) > 0}
          <!-- column-reverse stacks segments bottom-up in caller order. -->
          <div class="fill" style="height:{heightFor(col)}%;">
            {#each col.segments as seg, j (j)}
              <span
                style="height:{segmentPct(col, seg.value)}%; background:var({seg.colorVar});"
              ></span>
            {/each}
          </div>
        {:else}
          <span class="tick" aria-hidden="true"></span>
        {/if}
      </div>
      <span class="lbl">{col.label}</span>
    </div>
  {/each}
</div>

<style>
  .rhythm {
    display: flex;
    align-items: flex-end;
    gap: 2px;
  }
  .col {
    flex: 1 1 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bars {
    height: 40px;
    display: flex;
    align-items: flex-end;
  }
  .fill {
    width: 100%;
    display: flex;
    flex-direction: column-reverse;
    /* A busy month can make quiet days sub-pixel; keep them visible. */
    min-height: 2px;
  }
  .fill > span {
    display: block;
    width: 100%;
  }
  /* With column-reverse, the visually topmost segment is the last child. */
  .fill > span:last-child {
    border-radius: 2px 2px 0 0;
  }
  .tick {
    display: block;
    width: 100%;
    height: 2px;
    background: var(--app-border);
  }
  .lbl {
    /* Fixed height keeps the axis line steady when most labels are empty. */
    height: 10px;
    font-size: 9.5px;
    line-height: 1;
    color: var(--app-text-faint);
    text-align: center;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
  }
</style>
