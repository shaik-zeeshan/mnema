<script lang="ts">
  // JournalRiver — the river half of the Journal surface (page 08), split out of
  // DayTimeline.svelte to keep both files under the 800-line ceiling. Given the
  // banded river + pending model (built by the parent from `buildJournalDay` +
  // `journal-view.ts`) it renders the banded rows (when column | card plate),
  // the away-gaps, the live-edge pending slot, plus the loading skeleton and the
  // two empty-state panels. Activities under 5 minutes (`isShortActivity`)
  // collapse to one plain line. It owns no data loading — pure presentation.
  //
  // The direction's one exception lives here: EVERY row is an opaque plate
  // except the away gap, which is a hairline outline with nothing behind it —
  // a hole in the layer stack is exactly what "no capture" means.
  import { untrack } from "svelte";
  import type { Activity, ActivityFocus } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import type { RiverBand } from "$lib/insights/journal-view";
  import {
    isShortActivity,
    pendingPausedDetail,
    riverRowKey,
  } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    focusHint,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import { openSettings } from "$lib/surface-windows";
  import IconPlay from "~icons/lucide/play";
  import IconWarn from "~icons/lucide/triangle-alert";
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
    return `${n} ${n === 1 ? "frame" : "frames"} · receipt`;
  }
</script>

{#if showSkeleton}
  <section class="river" aria-busy="true">
    {#each Array.from({ length: 4 }) as _, i (i)}
      <div class="rrow">
        <div class="rwhen"><Skeleton variant="text" width="46px" height="11px" /></div>
        <div class="plate jcard jcard--sk">
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
      <p class="t-label band">{band.label}</p>
      {#each band.rows as row (riverRowKey(row))}
        {#if row.kind === "gap"}
          <!-- THE GAP: the one row in the app with no plate and no shadow. -->
          <div class="rrow" data-at-ms={row.atMs}>
            <div class="rwhen"></div>
            <div class="jgap">
              <span class="t-meta is-num">{clock(row.gap.startMs)} – {clock(row.gap.endMs)}</span>
              <span class="t-meta">· away — no capture</span>
            </div>
          </div>
        {:else if isShortActivity(row.slot.activity)}
          {@const a = row.slot.activity}
          <div class="rrow" data-at-ms={row.atMs}>
            <div class="rwhen">
              <span class="rwhen__c is-num">{clock(a.startedAtMs)}</span>
              <span class="rwhen__d is-num">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </div>
            <button type="button" class="plate jcompact" onclick={() => onOpenActivity(a)}>
              <em style="background:{catVar(a.category)};"></em>
              <span class="jcompact__t">{a.title}</span>
            </button>
          </div>
        {:else}
          {@const a = row.slot.activity}
          <div class="rrow" data-at-ms={row.atMs}>
            <div class="rwhen">
              <span class="rwhen__c is-num">{clock(a.startedAtMs)}</span>
              <span class="rwhen__d is-num">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
            </div>
            <button type="button" class="plate jcard" onclick={() => onOpenActivity(a)}>
              <!-- No app icon, no window title, no URL: an Activity carries
                   none of those (they exist per FRAME, inside the receipt), and
                   focus is a three-value enum rendered as a WORD — there is no
                   number behind it to show. -->
              <span class="jchips">
                <span class="cchip">
                  <em style="background:{catVar(a.category)};"></em>
                  {a.category ? categoryLabel(a.category) : "Uncategorized"}
                </span>
                {#if a.focus}
                  <span class="fchip">
                    <em style="background:var({FOCUS_TOKEN[a.focus]});"></em>
                    {focusHint(a.focus)}
                  </span>
                {/if}
              </span>
              <h3>{a.title}</h3>
              {#if a.summary}<p>{a.summary}</p>{/if}
              {#if row.slot.expired}
                <span class="rfoot rfoot--gone">footage expired</span>
              {:else}
                <span class="rfoot"><IconPlay />{frameLabel(row.slot.frameCount)}</span>
              {/if}
            </button>
          </div>
        {/if}
      {/each}
    {/each}

    {#if pending.active && pending.reason}
      <div class="rrow">
        <div class="rwhen">
          {#if pending.sinceMs !== null}
            <span class="rwhen__c is-num">{clock(pending.sinceMs)}</span>
          {/if}
          <span class="rwhen__d">now</span>
        </div>
        <div class="plate pend">
          {#if pending.reason.kind === "summarizing"}
            <div class="pend__l">
              <span class="spin" aria-hidden="true"></span>
              <span class="t-ui pend__t">Summarizing this window…</span>
            </div>
            <p>
              The journal trails live capture by up to 30 minutes — the footage
              itself is already on the Timeline.
            </p>
          {:else}
            <div class="pend__l">
              <IconWarn />
              <span class="t-ui pend__t">Summaries are paused</span>
            </div>
            <p>
              {pendingPausedDetail(pending.reason.reason)}
              <button type="button" class="pend__link" onclick={() => void openSettings("intelligence")}>
                Open engine settings
              </button>
            </p>
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
  <div class="plate empty">
    <div class="empty__glyph" aria-hidden="true">◇</div>
    <h4>Nothing captured on {dayLabel}</h4>
    <p>
      There's no capture on this day, so there's no journal to show. Days with any
      recording at all show whatever was captured.
    </p>
  </div>
{:else if showBeingWritten}
  <div class="plate empty">
    <div class="empty__glyph" aria-hidden="true">◇</div>
    <h4>Your day is being written</h4>
    <p>
      Capture is landing. The first journal card appears once the first half-hour
      window has been summarized.
    </p>
  </div>
{/if}

<style>
  /* Every row is an opaque plate; the away gap is the sole exception (see
     `.jgap`). Colours are app tokens (`--app-*`, `--cat-*`, `--focus-*`). */
  .river {
    display: flex;
    flex-direction: column;
    width: 100%;
  }

  /* Band headers — the river's only chrome. NOT sticky: the pane scrolls under
     a glass title bar, and an opaque strip pinned beneath it reads as a second
     bar. */
  .band {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 18px 0 8px;
    color: var(--app-text-subtle);
  }
  .band:first-of-type {
    margin-top: 0;
  }
  .band::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--app-border);
  }

  .rrow {
    display: grid;
    grid-template-columns: 78px 1fr;
    gap: 12px;
    margin-bottom: 8px;
    align-items: start;
  }
  .rwhen {
    padding-top: 11px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    text-align: right;
  }
  .rwhen__c {
    font: var(--w-medium) var(--t-ui) / 1.2 var(--app-font-mono);
    color: var(--app-text-strong);
  }
  .rwhen__d {
    font: var(--w-regular) var(--t-meta) / 1.3 var(--app-font-mono);
    color: var(--app-text-faint);
  }

  /* ---- The activity card ---- */
  .jcard {
    display: flex;
    flex-direction: column;
    gap: 5px;
    align-items: flex-start;
    width: 100%;
    padding: 11px 13px 10px;
    border: 0;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .jcard:hover {
    background: var(--app-surface-raised);
  }
  .jcard:focus-visible {
    outline: none;
    box-shadow: var(--sh-tile), var(--ring);
  }
  .jcard--sk {
    gap: 8px;
    cursor: default;
  }
  .jcard h3 {
    margin: 1px 0 0;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .jcard p {
    margin: 0;
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-muted);
    max-width: 68ch;
  }
  .jchips {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .cchip,
  .fchip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 19px;
    padding: 0 8px;
    border-radius: var(--r-pill);
    background: var(--glass-tint);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .cchip em {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    display: block;
    font-style: normal;
  }
  .fchip em {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    display: block;
    font-style: normal;
  }
  .rfoot {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-accent);
  }
  .rfoot :global(svg) {
    width: 11px;
    height: 11px;
  }
  .rfoot--gone {
    color: var(--app-text-faint);
    font-weight: var(--w-regular);
  }

  /* ---- A short activity (< 5 min) is one line, still on a plate ---- */
  .jcompact {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 34px;
    padding: 0 13px;
    border: 0;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }
  .jcompact:hover {
    background: var(--app-surface-raised);
  }
  .jcompact:focus-visible {
    outline: none;
    box-shadow: var(--sh-tile), var(--ring);
  }
  .jcompact em {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 2px;
    font-style: normal;
  }
  .jcompact__t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  /* ---- THE GAP: no plate, no shadow — a hairline outline with nothing behind
     it, because a hole in the layer stack IS what "no capture" means. ---- */
  .jgap {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 30px;
    padding: 0 13px;
    border-radius: var(--r-lg);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    color: var(--app-text-subtle);
  }
  .jgap::before {
    content: "";
    flex: 0 0 auto;
    width: 26px;
    height: 1px;
    background: repeating-linear-gradient(
      90deg,
      var(--app-border-hover) 0 3px,
      transparent 3px 6px
    );
  }
  .jgap :global(.t-meta) {
    color: var(--app-text-subtle);
  }

  /* ---- The live edge: the journal trails capture ---- */
  .pend {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 11px 13px;
  }
  .pend__l {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .pend__l :global(svg) {
    width: 13px;
    height: 13px;
    color: var(--app-warn);
  }
  .pend__t {
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .pend p {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text-subtle);
    max-width: 64ch;
  }
  .pend__link {
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: var(--app-accent);
    cursor: pointer;
  }
  .pend__link:hover {
    text-decoration: underline;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex: 0 0 auto;
    display: block;
    border: 1.5px solid var(--app-border-hover);
    border-top-color: var(--app-accent);
    animation: journal-spin 1s linear infinite;
  }
  @keyframes journal-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
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
    border: 0;
    border-radius: var(--r-pill);
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    cursor: pointer;
    transition: color var(--dur-quick) var(--ease);
  }
  .jump-now:hover {
    color: var(--app-text-strong);
  }
  .jump-now:focus-visible {
    outline: none;
    box-shadow: var(--ring), var(--sh-float);
  }

  /* ---- Empty-state panels ---- */
  .empty {
    text-align: center;
    padding: 44px 24px 40px;
  }
  .empty__glyph {
    font-size: 20px;
    color: var(--app-text-faint);
    margin-bottom: 10px;
  }
  .empty h4 {
    margin: 0 0 6px;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .empty p {
    margin: 0 auto;
    max-width: 420px;
    font: var(--w-regular) var(--t-meta) / 1.6 var(--app-font-sans);
    color: var(--app-text-muted);
  }
</style>
