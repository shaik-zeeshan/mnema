<script lang="ts">
  // JournalRiver — the river half of the Journal surface (Slice 3), split out of
  // DayTimeline.svelte to keep both files under the 800-line ceiling. Given the
  // banded river + pending model (built by the parent from `buildJournalDay` +
  // `journal-view.ts`) it renders the `.slot` grid (when | spine | card), the
  // away-gaps, the live-edge pending slot, plus the loading skeleton and the two
  // empty-state panels. Activities under 5 minutes (`isShortActivity`) render as
  // compact one-line rows instead of full cards to keep the river dense. It owns
  // no data loading — pure presentation.
  import { untrack } from "svelte";
  import type { Activity, ActivityFocus } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import type { RiverBand } from "$lib/insights/journal-view";
  import { isShortActivity, pendingReasonCopy } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    focusHint,
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
  const FOCUS_TOKEN: Record<ActivityFocus, string> = {
    deep: "--focus-deep",
    mixed: "--focus-mid",
    distracted: "--focus-distracted",
  };
  function frameLabel(n: number): string {
    return `▸ ${n} ${n === 1 ? "frame" : "frames"} · receipt`;
  }
</script>

{#if showSkeleton}
  <section class="river" aria-busy="true">
    {#each Array.from({ length: 4 }) as _, i (i)}
      <div class="slot">
        <div class="when"><Skeleton variant="text" width="34px" height="11px" /></div>
        <div class="spine"><span class="node node--sk"></span></div>
        <div class="card card--sk">
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
            <div class="when"></div>
            <div class="spine"></div>
            <div class="txt">
              {clock(row.gap.startMs)} – {clock(row.gap.endMs)} · away — no capture
            </div>
          </div>
        {:else if isShortActivity(row.slot.activity)}
          {@const a = row.slot.activity}
          <div class="slot slot--compact" data-at-ms={row.atMs}>
            <div class="when">
              {clock(a.startedAtMs)}
              <span class="dur">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </div>
            <div class="spine">
              <span class="node" style="background:{catVar(a.category)};"></span>
            </div>
            <button type="button" class="row-compact" onclick={() => onOpenActivity(a)}>
              <span class="swatch" style="background:{catVar(a.category)};"></span>
              <span class="row-title">{a.title}</span>
            </button>
          </div>
        {:else}
          {@const a = row.slot.activity}
          <div class="slot" data-at-ms={row.atMs}>
            <div class="when">
              {clock(a.startedAtMs)}
              <span class="dur">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </div>
            <div class="spine">
              <span class="node" style="background:{catVar(a.category)};"></span>
            </div>
            <button
              type="button"
              class="card"
              style="--cat: {catVar(a.category)};"
              onclick={() => onOpenActivity(a)}
            >
              <div class="card-top">
                <span class="chip">
                  <span class="swatch" style="background:{catVar(a.category)};"></span>
                  {a.category ? categoryLabel(a.category) : "Uncategorized"}
                </span>
                {#if a.focus}
                  <span class="focus">
                    <i style="background:var({FOCUS_TOKEN[a.focus]});"></i>
                    {focusHint(a.focus)}
                  </span>
                {/if}
              </div>
              <h3>{a.title}</h3>
              <p>{a.summary}</p>
              <div class="card-foot">
                <span class="receipt">
                  {row.slot.expired ? "footage expired" : frameLabel(row.slot.frameCount)}
                </span>
              </div>
            </button>
          </div>
        {/if}
      {/each}
    {/each}

    {#if pending.active && pending.reason}
      <div class="slot">
        <div class="when">
          {pending.sinceMs !== null ? clock(pending.sinceMs) : ""}
          <span class="dur">now</span>
        </div>
        <div class="spine"><span class="node node--pending"></span></div>
        <div class="card card--pending">
          {#if pending.reason.kind === "summarizing"}
            <div class="pt"><span class="spin"></span>Summarizing this window…</div>
            <div class="sub">
              The journal trails live capture by up to 30 minutes — the footage
              itself is already on the Timeline.
            </div>
          {:else}
            <div class="pt pt--paused">{pendingReasonCopy(pending.reason.reason)}</div>
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
      Capture is landing. The first journal card appears once the first half-hour
      window has been summarized.
    </p>
  </div>
{/if}

<style>
  /* All colours are app tokens (`--app-*`, `--cat-*`, `--focus-*`); the mockup's
     raw hex (docs/mockups/dayflow/01-day-journal.html) is only its self-contained
     copy of these same tokens. */
  .river {
    display: flex;
    flex-direction: column;
    gap: 0;
    width: 100%;
  }
  .day-rule {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: var(--text-xs);
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    /* Sticks to the scrollport top while its band scrolls; part of the old
       margins became padding so the solid bg covers cards passing beneath. */
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--app-bg);
    padding: 4px 0 6px;
    margin: 2px 0 8px;
  }
  .day-rule:not(:first-child) {
    margin-top: 14px;
  }
  .day-rule .rule {
    flex: 1;
    height: 1px;
    background: var(--app-border);
  }
  .slot {
    display: grid;
    grid-template-columns: 64px 20px 1fr;
    gap: 0 10px;
  }
  .slot + .slot,
  .gap-note + .slot,
  .slot + .gap-note {
    margin-top: 12px;
  }
  .slot .when {
    padding-top: 16px;
    text-align: right;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .slot .when .dur {
    display: block;
    font-size: var(--text-xs);
    color: var(--app-text-faint);
  }
  .spine {
    position: relative;
  }
  .spine::before {
    content: "";
    position: absolute;
    left: 50%;
    top: 0;
    bottom: -12px;
    width: 1px;
    background: var(--app-border);
  }
  /* Last slot can't rely on :last-child — the live-edge sentinel (and at times
     the jump pill) now render after it. */
  .river > .slot:not(:has(~ .slot, ~ .gap-note)) .spine::before {
    bottom: 12px;
  }
  .spine .node {
    position: absolute;
    left: 50%;
    top: 19px;
    transform: translate(-50%, 0);
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 2px solid var(--app-bg);
  }
  .spine .node--pending {
    background: var(--app-text-faint);
  }
  .spine .node--sk {
    background: var(--app-border-strong);
  }

  .gap-note {
    display: grid;
    grid-template-columns: 64px 20px 1fr;
    gap: 0 10px;
    margin-top: 12px;
  }
  .gap-note .spine::before {
    bottom: -12px;
    border-left: 1px dashed var(--app-border);
    background: transparent;
    width: 0;
  }
  .gap-note .txt {
    font-size: var(--text-sm);
    color: var(--app-text-faint);
    padding: 4px 0;
    font-style: italic;
  }

  /* Compact row — activities under 5 minutes render as one quiet clickable
     line (swatch + title) instead of a full card; the click opens the receipt.
     Tighter paddings + node top keep the spine node centered on the single
     text line (row lands ~32px tall). */
  .slot--compact .when {
    padding-top: 7px;
  }
  .slot--compact .node {
    top: 9px;
  }
  .row-compact {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
    text-align: left;
    border: 0;
    border-radius: 8px;
    background: transparent;
    padding: 7px 10px;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .row-compact:hover {
    background: var(--app-surface-hover);
  }
  .row-compact:hover .row-title {
    color: var(--app-text-strong);
  }
  .row-compact:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .row-compact .swatch {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .row-compact .row-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    line-height: 1.35;
    color: var(--app-text-muted);
    transition: color 0.12s ease;
  }

  /* Card — a full-width button so the whole card opens the receipt (keyboard
     reachable). Category rides a left edge bar (`--cat`) on a neutral body. */
  .card {
    position: relative;
    display: block;
    width: 100%;
    text-align: left;
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface);
    padding: 13px 16px 12px 19px;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: border-color 0.12s ease;
  }
  .card::before {
    content: "";
    position: absolute;
    left: -1px;
    top: -1px;
    bottom: -1px;
    width: 3px;
    border-radius: 10px 0 0 10px;
    background: var(--cat, transparent);
  }
  .card:hover {
    border-color: var(--app-border-hover);
  }
  .card:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .card:hover .receipt,
  .card:focus-visible .receipt {
    color: var(--app-accent);
    border-bottom-color: var(--app-accent-border);
  }
  .card--sk {
    cursor: default;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card--sk::before {
    display: none;
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .chip .swatch {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .focus {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .focus i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
  }
  .card h3 {
    margin: 0 0 4px;
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    line-height: 1.35;
  }
  .card p {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.65;
    color: var(--app-text-muted);
  }
  .card-foot {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-top: 9px;
    padding-top: 8px;
    border-top: 1px dashed var(--app-border);
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    flex-wrap: wrap;
  }
  .receipt {
    margin-left: auto;
    color: var(--app-text-muted);
    border-bottom: 1px dotted var(--app-border-strong);
    line-height: 1.3;
    white-space: nowrap;
    transition:
      color 0.12s ease,
      border-bottom-color 0.12s ease;
  }

  /* ---- Pending slot at the live edge ---- */
  .card--pending {
    border-style: dashed;
    background: transparent;
    cursor: default;
  }
  .card--pending::before {
    display: none;
  }
  .card--pending:hover {
    border-color: var(--app-border);
  }
  .card--pending .pt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
  }
  .card--pending .pt .spin {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent-strong);
    animation: journal-pulse 1.6s ease-in-out infinite;
  }
  @keyframes journal-pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .card--pending .pt .spin {
      animation: none;
    }
  }
  .card--pending .sub {
    margin-top: 3px;
    font-size: var(--text-xs);
    color: var(--app-text-faint);
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
