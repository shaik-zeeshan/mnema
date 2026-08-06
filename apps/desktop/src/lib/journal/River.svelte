<script lang="ts">
  // The day-river — direction 04's Journal, page 08 viewport A. A keyboard list
  // in the same idiom as the settings rows: full-row accent selection, ↑↓ moves,
  // ⏎ opens the receipt. The parent (routes/journal) owns the selection and the
  // keys; this component only renders and reports clicks.
  //
  // The model is the shipping one — `buildJournalDay` + `journal-view.ts` — so
  // every fact drawn here is a fact the backend serves: bands are the model's
  // own Morning/Afternoon/Evening runs, away-gaps are ≥5-minute frameless spans
  // inside the summarized region, "N frames · receipt" is the per-card frame
  // count (zero ⇒ "footage expired", ADR 0029), and the focus chip renders only
  // when `Activity.focus` is present (there is no focus score, so no percentage
  // per card).
  import type { Activity, ActivityFocus } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import type { RiverBand } from "$lib/insights/journal-view";
  import { isShortActivity, pendingReasonCopy, riverRowKey } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    focusHint,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import { bandStats } from "$lib/journal/band-stats";

  interface Props {
    bands: RiverBand[];
    pending: JournalPending;
    showSkeleton: boolean;
    hasCards: boolean;
    showNothingCaptured: boolean;
    showBeingWritten: boolean;
    dayLabel: string;
    selectedId: number | null;
    onSelect: (activity: Activity) => void;
    onOpen: (activity: Activity) => void;
  }

  let {
    bands,
    pending,
    showSkeleton,
    hasCards,
    showNothingCaptured,
    showBeingWritten,
    dayLabel,
    selectedId,
    onSelect,
    onOpen,
  }: Props = $props();

  let riverEl = $state<HTMLElement | null>(null);

  // Keep the ↑↓ selection in view. `nearest` so a click never scrolls.
  $effect(() => {
    const id = selectedId;
    if (id == null) return;
    riverEl?.querySelector(`[data-aid="${id}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    });
  }
  function catVar(category: Activity["category"]): string {
    return category ? `var(${CATEGORY_COLOR[category]})` : `var(${UNCATEGORIZED_COLOR})`;
  }
  const FOCUS_TOKEN: Record<ActivityFocus, string> = {
    deep: "--focus-deep",
    mixed: "--focus-mid",
    distracted: "--focus-distracted",
  };
  function frameLabel(n: number): string {
    return `▸ ${n.toLocaleString()} ${n === 1 ? "frame" : "frames"} · receipt`;
  }
  function take(activity: Activity): void {
    onSelect(activity);
    onOpen(activity);
  }
</script>

{#if showSkeleton}
  <div class="river" aria-busy="true">
    {#each [0, 1, 2, 3] as i (i)}
      <div class="jrow jrow--sk">
        <span class="jrow__t"></span>
        <span class="jrow__n"><i></i><u></u></span>
        <div class="jcard jcard--sk"></div>
      </div>
    {/each}
  </div>
{:else if hasCards}
  <div class="river" bind:this={riverEl} aria-label="The day">
    {#each bands as band (band.label + band.rows[0].atMs)}
      {@const stats = bandStats(band.rows)}
      <div class="band-h">
        <span class="t-label">{band.label}</span>
        <span class="t-meta is-mono is-num band-h__n">
          {stats.count}
          {stats.count === 1 ? "activity" : "activities"} · {humanizeMs(stats.totalMs)}
        </span>
      </div>

      {#each band.rows as row (riverRowKey(row))}
        {#if row.kind === "gap"}
          <div class="jgap">
            <span class="jgap__x">
              {clock(row.gap.startMs)} – {clock(row.gap.endMs)} · away — no capture
            </span>
          </div>
        {:else}
          {@const a = row.slot.activity}
          {@const short = isShortActivity(a)}
          <button
            type="button"
            class="jrow"
            class:jrow--sm={short}
            class:is-key={selectedId === a.id}
            style="--cat: {catVar(a.category)};"
            data-aid={a.id}
            onclick={() => take(a)}
          >
            <span class="jrow__t">
              <span class="big">{clock(a.startedAtMs)}</span>
              <span>{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </span>
            <span class="jrow__n"><i></i><u></u></span>
            <span class="jcard">
              {#if short}
                <span class="jcard__ttl" class:jcard__ttl--uncat={!a.category}>
                  {a.category ? a.title : "Uncategorized"}
                </span>
                {#if !a.category}
                  <span class="t-meta jcard__aside">— {a.title}</span>
                {/if}
                {#if selectedId === a.id}<span class="kbd jcard__k">⏎</span>{/if}
              {:else}
                <span class="jcard__h">
                  <span class="jchip"><i></i>{a.category ? categoryLabel(a.category) : "Uncategorized"}</span>
                  {#if a.focus}
                    <span class="jfocus"><i style="background:var({FOCUS_TOKEN[a.focus]});"></i>{focusHint(a.focus)}</span>
                  {/if}
                  {#if selectedId === a.id}<span class="kbd jcard__k">⏎</span>{/if}
                </span>
                <span class="jcard__ttl">{a.title}</span>
                {#if a.summary}<span class="jcard__sum">{a.summary}</span>{/if}
                <span class="jcard__ft" class:jcard__ft--expired={row.slot.expired}>
                  {row.slot.expired ? "footage expired" : frameLabel(row.slot.frameCount)}
                </span>
              {/if}
            </span>
          </button>
        {/if}
      {/each}
    {/each}

    {#if pending.active && pending.reason}
      <div class="jpend">
        <div class="jpend__b">
          <div class="jpend__t">
            {#if pending.reason.kind === "summarizing"}
              <span class="dotpulse"></span>
              <span class="t-ui strong">Summarizing this window…</span>
            {:else}
              <span class="t-ui strong">{pendingReasonCopy(pending.reason.reason)}</span>
            {/if}
            {#if pending.sinceMs !== null}
              <span class="t-meta is-mono jpend__when">since {clock(pending.sinceMs)}</span>
            {/if}
          </div>
          {#if pending.reason.kind === "summarizing"}
            <span class="t-meta">
              The journal trails live capture by up to 30 minutes — the footage itself is already
              on the Timeline.
            </span>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{:else}
  <div class="river river--empty">
    <div class="emptybox">
      <span class="gl" aria-hidden="true">◇</span>
      {#if showNothingCaptured}
        <span class="t-ui strong">Nothing captured on {dayLabel}</span>
        <span class="t-meta">There's no capture on this day, so there's no journal to show.</span>
        <span class="hint"><span class="kbd">⌘1</span><span>Timeline</span></span>
      {:else if showBeingWritten}
        <span class="t-ui strong">Your day is being written</span>
        <span class="t-meta">
          Capture is landing. The first journal card appears once the first half-hour window has
          been summarized.
        </span>
        {#if pending.active && pending.reason?.kind === "engine_unavailable"}
          <span class="t-meta is-mono emptybox__why">
            {pendingReasonCopy(pending.reason.reason)}
          </span>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .river {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0 var(--s-16) var(--s-12);
  }
  .river--empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--s-24) var(--s-16);
  }

  /* Band header — sticks while its own run scrolls under it. */
  .band-h {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    padding: var(--s-8) 0 var(--s-6) 62px;
    background: linear-gradient(var(--app-bg) 74%, transparent);
  }
  .band-h__n {
    color: var(--app-text-subtle);
  }

  /* One row: time rail | spine node | card. The whole row is the button, the
     same full-row accent selection the settings rows use. */
  .jrow {
    display: flex;
    gap: var(--s-8);
    align-items: stretch;
    width: 100%;
    padding: 2px var(--s-8) 2px 0;
    border: 0;
    border-radius: var(--r-md);
    background: transparent;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: default;
  }
  .jrow:hover .jcard {
    background: var(--app-surface-raised);
  }
  .jrow:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--app-accent);
  }
  .jrow.is-key {
    background: var(--app-accent);
  }
  .jrow__t {
    flex: 0 0 54px;
    width: 54px;
    padding-top: 9px;
    text-align: right;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .jrow__t span {
    font: var(--w-regular) var(--t-meta) / 1.3 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .jrow__t span.big {
    color: var(--app-text-muted);
    font-weight: var(--w-medium);
  }
  .jrow.is-key .jrow__t span {
    color: var(--app-accent-contrast);
    opacity: 0.8;
  }
  .jrow__n {
    flex: 0 0 10px;
    width: 10px;
    position: relative;
  }
  .jrow__n i {
    position: absolute;
    left: 2px;
    top: 12px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--cat);
  }
  .jrow__n u {
    position: absolute;
    left: 5px;
    top: 21px;
    bottom: -4px;
    width: var(--hairline);
    background: var(--app-border-strong);
  }
  .jrow:last-child .jrow__n u {
    display: none;
  }

  .jcard {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--s-6) var(--s-8) var(--s-8);
    border-radius: var(--r-md);
    background: var(--app-surface);
    box-shadow: inset 2px 0 0 var(--cat);
  }
  .jrow.is-key .jcard,
  .jrow.is-key:hover .jcard {
    background: color-mix(in srgb, var(--app-accent-contrast) 14%, transparent);
  }
  .jcard__h {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .jchip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--cat);
  }
  .jchip i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }
  /* Deep / Mixed / Scattered — the three-value `Activity.focus` enum, drawn
     only when it is present. There is no focus score, so no per-card %. */
  .jfocus {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .jfocus i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
  }
  .jcard__ttl {
    font: var(--w-medium) var(--t-ui) / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    overflow-wrap: anywhere;
  }
  .jcard__ttl--uncat {
    color: var(--app-text-muted);
  }
  .jcard__aside {
    color: var(--app-text-subtle);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .jcard__sum {
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
    max-width: 78ch;
    overflow-wrap: anywhere;
  }
  .jcard__ft {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    margin-top: 2px;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .jcard__k {
    margin-left: auto;
  }
  .jrow.is-key .jcard__ttl,
  .jrow.is-key .jchip {
    color: var(--app-accent-contrast);
  }
  .jrow.is-key .jcard__sum,
  .jrow.is-key .jcard__ft,
  .jrow.is-key .jcard__aside,
  .jrow.is-key .jfocus {
    color: var(--app-accent-contrast);
    opacity: 0.82;
  }
  .jrow.is-key .kbd {
    background: color-mix(in srgb, var(--app-accent-contrast) 22%, transparent);
    color: var(--app-accent-contrast);
    box-shadow: none;
  }

  /* Under five minutes: a swatch, a title, nothing else. */
  .jrow--sm .jcard {
    flex-direction: row;
    align-items: center;
    gap: var(--s-8);
    padding: 5px var(--s-8);
    background: transparent;
    box-shadow: none;
  }
  .jrow--sm:hover .jcard {
    background: var(--app-surface);
  }
  .jrow--sm .jrow__n i {
    top: 8px;
  }
  .jrow--sm .jrow__t {
    padding-top: 5px;
  }

  .jgap {
    display: flex;
    gap: var(--s-8);
    padding: 2px 0;
  }
  .jgap__x {
    flex: 1 1 auto;
    margin-left: 62px;
    padding: 5px var(--s-8);
    font: var(--w-regular) var(--t-meta) / 1.3 var(--app-font-sans);
    font-style: italic;
    color: var(--app-text-subtle);
  }

  /* The live edge. Only drawn when the model says there is un-summarized
     capture past the worker watermark — never a decorative "now" slot. */
  .jpend {
    display: flex;
    gap: var(--s-8);
    padding: 2px 0;
  }
  .jpend__b {
    flex: 1 1 auto;
    margin-left: 62px;
    padding: var(--s-8);
    border: var(--hairline) dashed var(--app-border-strong);
    border-radius: var(--r-md);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .jpend__t {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
  }
  .jpend__when {
    margin-left: auto;
    color: var(--app-text-subtle);
  }
  .dotpulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
    animation: journal-pulse 1.6s ease-in-out infinite;
  }
  @keyframes journal-pulse {
    50% {
      opacity: 0.3;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dotpulse {
      animation: none;
    }
  }

  /* Loading — the river's shape, not a spinner. */
  .jrow--sk .jcard--sk {
    height: 54px;
    background: var(--app-surface);
    box-shadow: none;
    animation: journal-pulse 1.4s ease-in-out infinite;
  }
  .jrow--sk .jrow__n i {
    background: var(--app-border-strong);
  }
  .jrow--sk + .jrow--sk {
    margin-top: var(--s-6);
  }

  .emptybox {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-6);
    max-width: 420px;
    padding: var(--s-20) var(--s-12);
    border-radius: var(--r-lg);
    text-align: center;
    background: var(--app-surface);
  }
  .emptybox .gl {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-faint);
  }
  .emptybox__why {
    color: var(--app-text-subtle);
  }
</style>
