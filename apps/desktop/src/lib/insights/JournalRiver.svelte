<script lang="ts">
  // JournalRiver — the river half of the Journal surface. Given the banded
  // river + pending model (built by the parent from `buildJournalDay` +
  // `journal-view.ts`) it renders the three-column grid — time in the gutter, a
  // spine with one node per activity, the row itself — plus MORNING/AFTERNOON/
  // EVENING band rules, away-gaps, the live-edge pending slot, the loading
  // skeleton and the two empty-state panels. Activities under five minutes
  // (`isShortActivity`) collapse to a single line. It owns no data loading —
  // pure presentation.
  //
  // Direction 05 "Tactile Instruments" skin: a card is a FILL on the window
  // background (never a bordered box — depth is a surface step), category is a
  // mono eyebrow with a square swatch, focus is a mono chip, and the receipt
  // disclosure states a real frame count with a chevron. Rendered by BOTH the
  // Journal destination (`/overview/journal`) and the older Insights Journal
  // tab, so the two never drift.
  import { untrack } from "svelte";
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
</script>

{#snippet chevron()}
  <svg
    class="chev"
    width="8"
    height="12"
    viewBox="0 0 8 12"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d="m1.5 1 5 5-5 5" />
  </svg>
{/snippet}

{#if showSkeleton}
  <section class="river" aria-busy="true">
    {#each Array.from({ length: 4 }) as _, i (i)}
      <div class="when"><b class="sk sk--when"></b></div>
      <div class="spine"><i class="node node--sk"></i></div>
      <div class="jrow">
        <div class="jcard jcard--sk">
          <span class="sk" style="width:42%"></span>
          <span class="sk" style="width:78%"></span>
        </div>
      </div>
    {/each}
  </section>
{:else if hasCards}
  <section class="river" aria-label="Activity journal">
    <ScrollTimeBubble />
    {#each bands as band (band.label + band.rows[0].atMs)}
      <div class="bandrule"><span class="t-label">{band.label}</span></div>
      {#each band.rows as row (riverRowKey(row))}
        {#if row.kind === "gap"}
          <div class="when"></div>
          <div class="spine spine--gap"></div>
          <div class="jrow" data-at-ms={row.atMs}>
            <div class="jgap">
              {clock(row.gap.startMs)} – {clock(row.gap.endMs)} — away — no capture
            </div>
          </div>
        {:else if isShortActivity(row.slot.activity)}
          {@const a = row.slot.activity}
          <div class="when when--compact">
            <b class="is-num">{clock(a.startedAtMs)}</b>
            <span class="is-num">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
          </div>
          <div class="spine spine--compact">
            <i class="node" style="background:{catVar(a.category)};"></i>
          </div>
          <div class="jrow" data-at-ms={row.atMs}>
            <button type="button" class="jcard jcompact" onclick={() => onOpenActivity(a)}>
              <i class="cdot" style="background:{catVar(a.category)};"></i>
              <span class="jcompact__t">{a.title}</span>
              {@render chevron()}
            </button>
          </div>
        {:else}
          {@const a = row.slot.activity}
          <div class="when">
            <b class="is-num">{clock(a.startedAtMs)}</b>
            <span class="is-num">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
          </div>
          <div class="spine">
            <i class="node" style="background:{catVar(a.category)};"></i>
          </div>
          <div class="jrow" data-at-ms={row.atMs}>
            <button type="button" class="jcard" onclick={() => onOpenActivity(a)}>
              <div class="jcard__h">
                <span class="catchip">
                  <i class="cdot" style="background:{catVar(a.category)};"></i>
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
              {#if a.summary}<p>{a.summary}</p>{/if}
              <!-- The receipt disclosure. G8: the count is the frames the day
                   read actually found over this span — a card whose footage
                   aged out under Retention says so instead of printing 0. -->
              <div class="jcard__f">
                {#if row.slot.expired}
                  <span class="jcard__gone">footage expired</span>
                {:else}
                  {@render chevron()}
                  <b class="is-num">{row.slot.frameCount.toLocaleString()}</b>
                  {row.slot.frameCount === 1 ? "frame" : "frames"} · receipt
                {/if}
              </div>
            </button>
          </div>
        {/if}
      {/each}
    {/each}

    {#if pending.active && pending.reason}
      <div class="when">
        <b class="is-num">{pending.sinceMs !== null ? clock(pending.sinceMs) : ""}</b>
        <span>now</span>
      </div>
      <div class="spine"><i class="node node--live"></i></div>
      <div class="jrow">
        <div class="jcard jcard--pending">
          {#if pending.reason.kind === "summarizing"}
            <div class="jcard__h">
              <span class="pulse"></span>
              <span class="t-ui pending__t">Summarizing this window…</span>
            </div>
            <p class="t-meta pending__sub">
              The journal trails live capture by up to 30 minutes — the footage
              itself is already on the Timeline.
            </p>
          {:else}
            <div class="jcard__h">
              <span class="pulse pulse--off"></span>
              <span class="t-ui pending__t">{pendingReasonCopy(pending.reason.reason)}</span>
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
  <div class="estate">
    <svg
      class="estate__g"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      stroke-width="1.3"
      aria-hidden="true"
    >
      <rect x="1.5" y="2.5" width="11" height="10" rx="1.5" />
      <path d="M1.5 5.5h11M4.5 1v3M9.5 1v3" stroke-linecap="round" />
    </svg>
    <span class="t-ui estate__t">Nothing captured on {dayLabel}</span>
    <span class="t-meta">
      There's no capture on this day, so there's no journal to show. Days with any
      recording at all show whatever was captured.
    </span>
  </div>
{:else if showBeingWritten}
  <div class="estate">
    <svg class="estate__g" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true">
      <path d="M7 1.5 8.4 5.6 12.5 7 8.4 8.4 7 12.5 5.6 8.4 1.5 7 5.6 5.6z" />
    </svg>
    <span class="t-ui estate__t">Your day is being written</span>
    <span class="t-meta">
      Capture is landing. The first journal card appears once the first half-hour
      window has been summarized.
    </span>
  </div>
{/if}

<style>
  /* Three columns: the gutter clock, the spine, the row. Every child below is a
     direct grid item — a band rule spans all three. */
  .river {
    display: grid;
    grid-template-columns: 76px 20px minmax(0, 1fr);
    width: 100%;
  }
  .bandrule {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: var(--s-16) 0 var(--s-8);
    /* Pins to the scrollport top while its band scrolls; the fill has to be
       opaque so cards pass beneath it, not through it. */
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--app-bg);
  }
  .bandrule::after {
    content: "";
    flex: 1 1 auto;
    height: var(--hairline);
    background: var(--app-border);
  }

  .when {
    padding: var(--s-12) var(--s-12) 0 0;
    text-align: right;
  }
  .when--compact {
    padding-top: var(--s-6);
  }
  .when b {
    display: block;
    white-space: nowrap;
    font: var(--w-medium) var(--t-meta) / 1.3 var(--app-font-mono);
    color: var(--app-text-strong);
  }
  .when span {
    display: block;
    font: var(--w-regular) var(--t-label) / 1.5 var(--app-font-mono);
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
    bottom: 0;
    width: var(--hairline);
    background: var(--app-border);
    transform: translateX(-0.5px);
  }
  .spine--gap::before {
    border-left: var(--hairline) dashed var(--app-border);
    background: transparent;
    width: 0;
  }
  .node {
    position: absolute;
    left: 50%;
    top: 17px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    transform: translateX(-50%);
    background: var(--app-text-faint);
    /* The ring is the page fill, so the node reads as a bead ON the spine. */
    box-shadow: 0 0 0 3px var(--app-bg);
  }
  .spine--compact .node {
    top: 11px;
  }
  .node--sk {
    background: var(--ti-empty);
  }
  .node--live {
    background: var(--app-accent);
  }

  .jrow {
    padding-bottom: var(--s-12);
  }

  /* A card is a FILL, never a bordered box — the window ring is this surface's
     one border. The whole card is the button so the receipt is one click (and
     keyboard-reachable). */
  .jcard {
    display: block;
    width: 100%;
    text-align: left;
    border: 0;
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
    padding: var(--s-12);
    font: inherit;
    color: inherit;
    cursor: default;
    transition: background var(--dur-quick) var(--ease);
  }
  button.jcard:hover {
    background: var(--app-surface-hover);
  }
  button.jcard:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .jcard--sk {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
  }
  .sk {
    display: block;
    height: 10px;
    border-radius: var(--r-sm);
    background: var(--ti-empty);
  }
  .sk--when {
    width: 46px;
    margin-left: auto;
  }

  .jcard__h {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-bottom: var(--s-6);
  }
  .jcard h3 {
    margin: 0;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .jcard p {
    margin: 3px 0 0;
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
  }
  .jcard__f {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    margin-top: var(--s-8);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .jcard__f b {
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .jcard__gone {
    color: var(--app-text-faint);
  }
  button.jcard:hover .jcard__f,
  button.jcard:hover .jcard__f b,
  button.jcard:focus-visible .jcard__f,
  button.jcard:focus-visible .jcard__f b {
    color: var(--app-accent);
  }
  .chev {
    flex: 0 0 auto;
    color: var(--app-text-faint);
  }
  button.jcard:hover .chev {
    color: var(--app-accent);
  }

  /* An activity under five minutes is one quiet line, not a card. */
  .jcompact {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-height: 30px;
    padding: var(--s-4) var(--s-12);
  }
  .jcompact__t {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--w-medium) var(--t-ui) / 1.25 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }

  /* A stretch with no frames is NAMED, not skipped. */
  .jgap {
    display: flex;
    align-items: center;
    min-height: 26px;
    padding: 0 var(--s-12);
    font: var(--w-regular) var(--t-meta) / 1.35 var(--app-font-sans);
    font-style: italic;
    color: var(--app-text-faint);
  }

  .catchip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .cdot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  .focus {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .focus i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: 0 0 auto;
  }

  /* ---- The live edge: the pending slot names the lag ---- */
  .jcard--pending {
    background: transparent;
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
  }
  .pending__t {
    color: var(--app-text-strong);
  }
  .pending__sub {
    margin: 0;
    max-width: 58ch;
  }
  .pulse {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
    animation: journal-pulse 1.6s ease-in-out infinite;
  }
  .pulse--off {
    background: var(--app-warn);
    box-shadow: none;
    animation: none;
  }
  @keyframes journal-pulse {
    0%,
    100% {
      opacity: 0.4;
    }
    50% {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }

  .live-edge {
    grid-column: 1 / -1;
    height: 0;
  }
  /* Bottom-sticky: pinned at the scrollport bottom while its natural spot (the
     end of the river) is below the fold; unmounts at the live edge. */
  .jump-now {
    grid-column: 1 / -1;
    position: sticky;
    bottom: 18px;
    z-index: 2;
    justify-self: center;
    margin-top: var(--s-8);
    height: var(--h-sm);
    padding: 0 12px;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--app-surface-raised);
    box-shadow:
      0 0 0 var(--hairline) var(--app-border-strong),
      var(--shadow-popover);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    cursor: default;
  }
  .jump-now:hover {
    color: var(--app-accent);
  }
  .jump-now:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  /* ---- Empty-state panels: a fill with a glyph, never a dashed box ---- */
  .estate {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    max-width: 420px;
    margin-top: var(--s-16);
    padding: var(--s-16);
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
  }
  .estate__g {
    width: 22px;
    height: 22px;
    color: var(--app-text-faint);
  }
  .estate__t {
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }
</style>
