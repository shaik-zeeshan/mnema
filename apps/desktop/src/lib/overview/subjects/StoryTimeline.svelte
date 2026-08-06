<script lang="ts">
  // ══ THE STORY OVER TIME ════════════════════════════════════════════════════
  //
  // The selected conclusion's whole life, newest first: every confidence move,
  // every piece of evidence that could have caused one, the take this belief
  // replaced (ADR 0046), and the moment it first formed. `buildTimeline` in
  // subjectTimeline.ts merges and orders it — pure and unit-tested; this file
  // only draws.
  //
  // Markers and evidence are interleaved by TIME ONLY. The surface never claims
  // a specific activity caused a specific move, because the store does not
  // record that link.
  import type { Activity, ActivityCategory } from "$lib/types/recording";
  import type { TimelineEvent } from "$lib/insights/subjectTimeline";
  import { CATEGORY_COLOR, categoryLabel } from "$lib/insights/activity-helpers";
  import { clockLabel } from "$lib/overview/day-math";
  import { agoLabel, pct } from "$lib/overview/subjects/format";

  interface Props {
    events: TimelineEvent[];
    activities: ReadonlyMap<number, Activity>;
    onOpen: (activityId: number) => void;
  }

  let { events, activities, onOpen }: Props = $props();

  function categoryColor(category: string | null): string {
    const known = category as ActivityCategory | null;
    return known && known in CATEGORY_COLOR
      ? `var(${CATEGORY_COLOR[known]})`
      : "var(--app-text-faint)";
  }
  function categoryText(category: string | null): string | null {
    const known = category as ActivityCategory | null;
    return known && known in CATEGORY_COLOR ? categoryLabel(known) : null;
  }
  // "view frame →" for a screen moment, "view in Timeline →" otherwise — the
  // wording states what the hand-off will actually open.
  function openLabel(sourceType: "screen" | "audio" | null): string {
    return sourceType === "screen" ? "view frame →" : "view in Timeline →";
  }
</script>

<div class="tl2">
  {#each events as event, i (i)}
    <div class="tl2__t is-num">
      {#if event.atMs}
        {agoLabel(event.atMs)}
        {#if event.kind === "evidence" || event.kind === "contradict"}
          <br />{clockLabel(event.atMs)}
        {/if}
      {/if}
    </div>
    <div class="tl2__row">
      {#if event.kind === "marker"}
        <div class="mkrow">
          <span class:acc={event.direction === "reinforced"} class:warn={event.direction === "decayed"}>
            {event.direction === "reinforced" ? "↑" : "↓"}
          </span>
          confidence <b class="is-num">{pct(event.from)}% → {pct(event.to)}%</b> ·
          {event.direction}
        </div>
      {:else if event.kind === "formed"}
        <div class="mkrow">
          <span class="acc">✦</span>
          <span class="strong">formed</span> · conclusion first formed · started at
          <b class="is-num">{pct(event.confidence)}%</b>
        </div>
      {:else if event.kind === "replaced"}
        <div class="erow erow--replaced">
          <span class="warn" aria-hidden="true">↺</span>
          <span class="t-ui strong erow__lead">Replaced an earlier take</span>
          <span class="t-meta erow__quote">“{event.statement}”</span>
        </div>
      {:else}
        <div class="erow">
          {#if event.kind === "evidence" && event.sourceType}
            <span class="srcbadge">{event.sourceType === "audio" ? "mic" : "scr"}</span>
          {/if}
          {#if event.kind === "evidence" && categoryText(event.category)}
            <span class="catchip t-meta">
              <i style:background={categoryColor(event.category)}></i>
              {categoryText(event.category)}
            </span>
          {/if}
          <span class="t-ui erow__ttl">{event.title}</span>
          {#if event.kind === "contradict"}
            <span class="ti-chip ti-chip--danger">contradicts</span>
          {:else}
            <span class="ti-chip ti-chip--acc">supports</span>
          {/if}
          {#if activities.has(event.activityId)}
            <button
              type="button"
              class="btn btn--ghost btn--sm"
              onclick={() => onOpen(event.activityId)}
            >
              {openLabel(event.kind === "evidence" ? event.sourceType : null)}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/each}
</div>

<div class="ti-legend legend2">
  <span><i class="lg-acc"></i>confidence trajectory</span>
  <span><i class="lg-acc lg-dot"></i>evidence event</span>
  <span class="acc">↑ reinforced</span>
  <span class="warn">↓ decayed or contradicted</span>
</div>

<style>
  .tl2 {
    display: grid;
    grid-template-columns: 84px 1fr;
    margin-top: var(--s-8);
  }
  .tl2__t {
    padding: 10px var(--s-12) 0 0;
    text-align: right;
    font: var(--w-regular) var(--t-label) / 1.4 var(--app-font-mono);
    color: var(--app-text-faint);
  }
  .tl2__row {
    padding-bottom: var(--s-8);
    min-width: 0;
  }

  /* A confidence move is a line of type, not a card: the engine moved a number
     and said why in one word. */
  .mkrow {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    padding: var(--s-4) var(--s-12);
    font: var(--w-regular) var(--t-meta) / 1.6 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .mkrow b {
    font-family: var(--app-font-mono);
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }

  /* Evidence IS a thing that happened, so it gets a fill. */
  .erow {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-width: 0;
    padding: var(--s-6) var(--s-12);
    border-radius: var(--r-md);
    background: var(--ti-grp-fill);
  }
  .erow--replaced {
    background: var(--app-warn-bg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-warn-border);
  }
  .erow__lead {
    flex: 0 0 auto;
    color: var(--app-text-strong);
  }
  .erow__ttl,
  .erow__quote {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .srcbadge {
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    text-transform: uppercase;
    letter-spacing: var(--ls-label);
    color: var(--app-text-subtle);
  }
  .catchip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    flex: 0 0 auto;
  }
  .catchip i {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    display: inline-block;
  }

  .legend2 {
    margin-top: var(--s-8);
    padding-left: 84px;
  }
  .legend2 .lg-acc {
    background: var(--app-accent);
  }
  .legend2 .lg-dot {
    border-radius: 50%;
  }

  .acc {
    color: var(--app-accent);
  }
  .warn {
    color: var(--app-warn);
  }
  .strong {
    color: var(--app-text-strong);
  }
</style>
