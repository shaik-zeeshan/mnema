<script lang="ts">
  // JournalRiver — the river half of the Today surface, split out of
  // DayTimeline.svelte to keep both files under the 800-line ceiling. Given the
  // banded river + pending model (built by the parent from `buildJournalDay` +
  // `journal-view.ts`) it renders LEDGER PROSE (Warm Paper redesign, Slice 3;
  // mockup story-first-v5.html frame 1): borderless entries on a hanging mono
  // time gutter — a tinted small-caps category word leading the title sentence,
  // a summary line, and a quiet mono receipt line. Sub-5-minute activities
  // (`isShortActivity`) are single lines; away-gaps are italic notes; the live
  // edge is a breathing "writing this hour…" entry. It owns no data loading —
  // pure presentation. Clicking an entry opens its receipt via `onOpenActivity`
  // (the parent owns the receipt surface).
  import { untrack } from "svelte";
  import { captureControls } from "$lib/capture-controls.svelte";
  import type { Activity } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import type { RiverBand } from "$lib/insights/journal-view";
  import { isShortActivity, pendingReasonCopy } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import ScrollTimeBubble from "$lib/insights/ScrollTimeBubble.svelte";

  interface Props {
    bands: RiverBand[];
    pending: JournalPending;
    showSkeleton: boolean;
    hasCards: boolean;
    showNothingCaptured: boolean;
    showBeingWritten: boolean;
    dayLabel: string;
    isToday: boolean;
    onOpenActivity: (activity: Activity) => void;
  }

  let {
    bands,
    pending,
    showSkeleton,
    hasCards,
    showNothingCaptured,
    showBeingWritten,
    dayLabel,
    isToday,
    onOpenActivity,
  }: Props = $props();

  // ---- Live edge (today only): every day opens at the top; the "↓ now" pill
  // is the opt-in jump to the most recent activity. ----
  let sentinelEl = $state<HTMLElement | null>(null);
  let liveEdgeVisible = $state(false);

  $effect(() => {
    const el = sentinelEl;
    if (!el) return;
    const io = new IntersectionObserver((entries) => {
      liveEdgeVisible = entries[entries.length - 1].isIntersecting;
    });
    io.observe(el);
    return () => io.disconnect();
  });

  // Follow-bottom: only bands/pending changes retrigger this; visibility is
  // read untracked so the user scrolling back down never forces a jump.
  $effect(() => {
    bands;
    pending;
    untrack(() => {
      if (isToday && liveEdgeVisible) sentinelEl?.scrollIntoView({ block: "end" });
    });
  });

  function jumpToNow() {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    sentinelEl?.scrollIntoView({ block: "end", behavior: reduce ? "auto" : "smooth" });
  }

  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    });
  }
  // Category → CSS colour value (named category token, else the neutral grey).
  function catVar(category: Activity["category"]): string {
    return category ? `var(${CATEGORY_COLOR[category]})` : `var(${UNCATEGORIZED_COLOR})`;
  }
  function frameLabel(n: number): string {
    return `▸ ${n} ${n === 1 ? "frame" : "frames"} · receipt`;
  }

  // ---- Off-the-record live edge (today only): the hole forming in the story,
  // in its own vocabulary (mockup story-first-v5.html frame 2 `.live-off`).
  // State comes straight from the shared capture-controls store — the same
  // seam the titlebar record pill reads.
  const offRecord = $derived(isToday && captureControls.offTheRecord);
  const offRecordDeadlineMs = $derived(captureControls.offRecordDeadlineUnixMs);
</script>

{#if showSkeleton}
  <section class="river" aria-busy="true">
    {#each Array.from({ length: 4 }) as _, i (i)}
      <div class="entry entry--sk">
        <div class="ewhen"><Skeleton variant="text" width="42px" height="11px" /></div>
        <div class="ebody">
          <Skeleton variant="text" width="52%" height="12px" />
          <Skeleton variant="text" width="84%" height="10px" />
        </div>
      </div>
    {/each}
  </section>
{:else if hasCards}
  <section class="river" aria-label="Activity journal">
    <ScrollTimeBubble />
    {#each bands as band (band.label + band.rows[0].atMs)}
      <div class="day-rule"><span>{band.label}</span><span class="rule"></span></div>
      {#each band.rows as row (row.kind + row.atMs)}
        {#if row.kind === "gap"}
          <div class="gap-note" data-at-ms={row.atMs}>
            <div class="ewhen" aria-hidden="true"></div>
            <div class="txt">
              — away <span class="m">{clock(row.gap.startMs)} – {clock(row.gap.endMs)}</span> · no capture —
            </div>
          </div>
        {:else if isShortActivity(row.slot.activity)}
          {@const a = row.slot.activity}
          <button
            type="button"
            class="entry entry--minor"
            data-at-ms={row.atMs}
            onclick={() => onOpenActivity(a)}
          >
            <div class="ewhen">{clock(a.startedAtMs)}</div>
            <div class="ebody">
              <div class="line">
                {#if a.category}<span class="cw" style="color:{catVar(a.category)};">{categoryLabel(a.category)}</span>{/if}{a.title}
              </div>
            </div>
          </button>
        {:else}
          {@const a = row.slot.activity}
          <button
            type="button"
            class="entry"
            data-at-ms={row.atMs}
            onclick={() => onOpenActivity(a)}
          >
            <div class="ewhen">
              {clock(a.startedAtMs)}
              <span class="dur">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </div>
            <div class="ebody">
              <div>
                {#if a.category}<span class="cw" style="color:{catVar(a.category)};">{categoryLabel(a.category)}</span>{/if}<h3>{a.title}</h3>
              </div>
              <p>{a.summary}</p>
              <div class="efoot">
                <span class="receipt">
                  {row.slot.expired ? "footage expired" : frameLabel(row.slot.frameCount)}
                </span>
              </div>
            </div>
          </button>
        {/if}
      {/each}
    {/each}

    <!-- Live edge — the record's state told in the story's vocabulary
         (mockup .live-on / .live-off). Off the record wins over the pending
         entry: the user sees the hole forming in their story. -->
    {#if offRecord}
      <div class="live-off" class:live-off--muted={offRecordDeadlineMs === null}>
        <div class="ewhen" aria-hidden="true"></div>
        <div class="txt">
          {#if offRecordDeadlineMs !== null}
            <span>— off the record · resumes <span class="m">{clock(offRecordDeadlineMs)}</span> —</span>
          {:else}
            <span>— off the record · until you turn it back on —</span>
          {/if}
          <span class="dash" aria-hidden="true"></span>
        </div>
      </div>
    {:else if pending.active && pending.reason}
      <div class="entry entry--live">
        <div class="ewhen">
          {pending.sinceMs !== null ? clock(pending.sinceMs) : ""}
          <span class="dur">→</span>
        </div>
        <div class="ebody">
          {#if pending.reason.kind === "summarizing"}
            <div class="live-txt">
              <span class="breath" aria-hidden="true"></span>On the record — writing this hour…<span class="m">the journal trails live capture by up to 30 minutes</span>
            </div>
          {:else}
            <div class="live-txt live-txt--paused">
              {pendingReasonCopy(pending.reason.reason)}
            </div>
          {/if}
        </div>
      </div>
    {/if}

    {#if isToday && !liveEdgeVisible}
      <button type="button" class="jump-now" aria-label="Jump to now" onclick={jumpToNow}>
        ↓ now
      </button>
    {/if}
    <div class="live-edge" bind:this={sentinelEl} aria-hidden="true"></div>
  </section>
{:else if showNothingCaptured}
  <div class="empty">
    <div class="glyph" aria-hidden="true">◇</div>
    <h4>Nothing captured on {dayLabel}</h4>
    <p>
      There's no capture on this day, so there's no journal to show. Days with any
      recording at all show whatever was captured.
    </p>
  </div>
{:else if showBeingWritten}
  <div class="empty">
    <div class="glyph" aria-hidden="true">◇</div>
    <h4>Your day is being written</h4>
    <p>
      Capture is landing. The first journal entry appears once the first half-hour
      window has been summarized.
    </p>
  </div>
{/if}

<style>
  /* Ledger prose — borderless entries on a hanging mono time gutter (mockup
     story-first-v5.html frame 1 `.river`). All colours are app tokens
     (`--app-*`, `--cat-*`); the mockup's raw hex is its self-contained copy of
     the same tokens, and the Warm Paper retheme is a token swap. */
  .river {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding-bottom: 48px;
  }
  .day-rule {
    display: flex;
    align-items: center;
    gap: 9px;
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    /* Sticks to the scrollport top while its band scrolls; solid bg covers
       entries passing beneath. */
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--app-bg);
    padding: 4px 0 6px;
    margin: 2px 0 4px;
  }
  .day-rule:not(:first-child) {
    margin-top: 20px;
  }
  .day-rule .rule {
    flex: 1;
    height: 1px;
    background: var(--app-border);
  }

  /* One ledger entry — a full-width button (whole entry opens the receipt,
     keyboard reachable) laid out as [hanging time gutter | prose body]. */
  .entry {
    display: grid;
    grid-template-columns: 76px 1fr;
    gap: 0 20px;
    padding: 9px 10px 9px 0;
    margin: 0 -10px 0 0;
    border: 0;
    border-radius: 8px;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .entry + .entry,
  .gap-note + .entry,
  .entry + .gap-note {
    margin-top: 3px;
  }
  .entry:hover {
    background: var(--app-surface-hover);
  }
  .entry:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .entry:hover .receipt,
  .entry:focus-visible .receipt {
    color: var(--app-accent);
  }
  .ewhen {
    text-align: right;
    padding-top: 2.5px;
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .ewhen .dur {
    display: block;
    font-size: var(--text-xs);
    color: var(--app-text-faint);
    margin-top: 2px;
  }
  .ebody {
    min-width: 0;
  }
  /* Tinted small-caps category word leading the title sentence. */
  .cw {
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    margin-right: 7px;
    vertical-align: 2px;
  }
  .ebody h3 {
    display: inline;
    margin: 0;
    font-family: var(--app-font-narrative);
    font-size: 15px;
    font-weight: 500;
    letter-spacing: -0.003em;
    color: var(--app-text-strong);
  }
  .ebody p {
    margin: 3px 0 0;
    font-family: var(--app-font-narrative);
    font-size: var(--text-md);
    line-height: 1.5;
    color: var(--app-text-muted);
  }
  .efoot {
    margin-top: 5px;
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .receipt {
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.05em;
    color: var(--app-text-faint);
    transition: color 0.12s ease;
  }

  /* Sub-5-minute activities — one quiet line. */
  .entry--minor {
    padding-top: 5px;
    padding-bottom: 5px;
  }
  .entry--minor .ewhen {
    padding-top: 1px;
  }
  .entry--minor .line {
    font-family: var(--app-font-narrative);
    font-size: var(--text-md);
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.12s ease;
  }
  .entry--minor:hover .line {
    color: var(--app-text-strong);
  }

  /* Away-gap — an italic aside in the ledger. */
  .gap-note {
    display: grid;
    grid-template-columns: 76px 1fr;
    gap: 0 20px;
    margin-top: 3px;
  }
  .gap-note .txt {
    font-family: var(--app-font-narrative);
    font-style: italic;
    font-size: var(--text-base);
    color: var(--app-text-faint);
    padding: 5px 0;
  }
  .gap-note .txt .m {
    font-family: var(--app-font-mono);
    font-style: normal;
    font-size: var(--text-sm);
  }

  /* Skeleton rows share the entry grid. */
  .entry--sk {
    cursor: default;
  }
  .entry--sk:hover {
    background: transparent;
  }
  .entry--sk .ebody {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* ---- Live edge — breathing "writing this hour…" entry ---- */
  .entry--live {
    cursor: default;
  }
  .entry--live:hover {
    background: transparent;
  }
  .live-txt {
    font-family: var(--app-font-narrative);
    font-style: italic;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    padding: 3px 0;
  }
  .live-txt .breath {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    /* The record's own colour (mockup .breath = var(--rec)), not the accent:
       the breathing dot IS the on-the-record signal, same as the pill's. */
    background: var(--app-status-running-dot);
    margin-right: 9px;
    vertical-align: -0.5px;
    animation: journal-breathe 2.8s ease-in-out infinite;
  }
  @keyframes journal-breathe {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-txt .breath {
      animation: none;
    }
  }
  .live-txt .m {
    font-family: var(--app-font-mono);
    font-style: normal;
    font-size: var(--text-xs);
    color: var(--app-text-faint);
    margin-left: 7px;
  }
  .live-txt--paused {
    color: var(--app-text-subtle);
  }

  /* ---- Live edge — off the record: the hole forming (mockup .live-off).
     Amber while a timed window counts down; muted when indefinite. ---- */
  .live-off {
    display: grid;
    grid-template-columns: 76px 1fr;
    gap: 0 20px;
    margin-top: 8px;
  }
  .live-off .txt {
    display: flex;
    align-items: center;
    gap: 12px;
    font-family: var(--app-font-narrative);
    font-style: italic;
    font-size: var(--text-base);
    color: var(--app-warn);
    padding: 6px 0;
  }
  .live-off .txt .m {
    font-family: var(--app-font-mono);
    font-style: normal;
    font-size: var(--text-xs);
  }
  .live-off .dash {
    flex: 1;
    border-top: 1px dashed var(--app-warn-border);
  }
  .live-off--muted .txt {
    color: var(--app-text-subtle);
  }
  .live-off--muted .dash {
    border-top-color: var(--app-border);
  }

  /* ---- Live edge: sentinel + "jump to now" pill ---- */
  .live-edge {
    height: 0;
  }
  /* Bottom-sticky: floats pinned at the scrollport bottom while its natural
     spot (end of the river) is below the fold; unmounts at the live edge. */
  .jump-now {
    position: sticky;
    bottom: 18px;
    z-index: 2;
    align-self: center;
    margin-top: 14px;
    padding: 5px 13px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.08em;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .jump-now:hover {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }
  .jump-now:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* ---- Empty-state panels ---- */
  .empty {
    text-align: center;
    padding: 44px 24px 40px;
    border: 1px solid var(--app-border);
    border-radius: 12px;
    background: var(--app-surface-subtle);
  }
  .empty .glyph {
    font-size: 20px;
    color: var(--app-text-faint);
    margin-bottom: 10px;
  }
  .empty h4 {
    margin: 0 0 6px;
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .empty p {
    margin: 0 auto;
    max-width: 380px;
    font-size: var(--text-sm);
    line-height: 1.7;
    color: var(--app-text-muted);
  }
</style>
