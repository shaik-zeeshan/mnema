<script lang="ts">
  // The river — the day top to bottom, in the one scrolling region.
  //
  // Four card states, none of them an error (mockup 08): the full card (≥5 min),
  // the compact row (<5 min), the away gap, and "footage expired" — a card whose
  // pixels retention removed, which keeps its summary and reads as a fact. At the
  // live edge the pending slot says what derivation is doing, or which single
  // switch is off, and never blames the user.
  //
  // Pure presentation: the model comes from `buildJournalDay` + `journal-view`,
  // both untouched from the Insights Journal.
  import type { Activity, ActivityFocus } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import type { BandLabel, RiverBand, RiverRow } from "$lib/insights/journal-view";
  import { isShortActivity, pendingReasonCopy, riverRowKey } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    focusHint,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import { openSettings } from "$lib/surface-windows";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  interface Props {
    bands: RiverBand[];
    pending: JournalPending;
    showSkeleton: boolean;
    hasCards: boolean;
    showNothingCaptured: boolean;
    showBeingWritten: boolean;
    dayLabel: string;
    isToday: boolean;
    selectedId: number | null;
    onselect: (activity: Activity) => void;
    onopen: (activity: Activity) => void;
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
    selectedId,
    onselect,
    onopen,
  }: Props = $props();

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
  function rowEndMs(row: RiverRow): number {
    return row.kind === "card" ? row.slot.activity.endedAtMs : row.gap.endMs;
  }
  /** "08:40 – 12:00 · 4 activities" — counted, never estimated. */
  function bandSpan(band: RiverBand, lastBand: boolean): string {
    const rows = band.rows;
    const start = clock(rows[0].atMs);
    const openEnded = lastBand && isToday && pending.active;
    const end = openEnded ? "now" : clock(Math.max(...rows.map(rowEndMs)));
    const n = rows.filter((r) => r.kind === "card").length;
    return `${start} – ${end} · ${n} ${n === 1 ? "activity" : "activities"}`;
  }
  function frameLabel(n: number): string {
    return `${n} ${n === 1 ? "frame" : "frames"} · receipt`;
  }
</script>

{#if showSkeleton}
  <div class="river" aria-busy="true">
    {#each Array.from({ length: 4 }) as _, i (i)}
      <div class="when"><Skeleton variant="text" width="46px" height="10px" /></div>
      <div class="spine"><span class="node node--sk"></span></div>
      <div class="card card--sk">
        <Skeleton variant="text" width="54%" height="11px" />
        <Skeleton variant="text" width="86%" height="9px" />
      </div>
    {/each}
  </div>
{:else if hasCards}
  {#each bands as band, bi (band.label + band.rows[0].atMs)}
    <div class="band" data-band={band.label as BandLabel}>
      <span class="band__n">{band.label}</span>
      <span class="t-meta is-mono">{bandSpan(band, bi === bands.length - 1)}</span>
    </div>

    <div class="river">
      {#each band.rows as row (riverRowKey(row))}
        {#if row.kind === "gap"}
          <div class="when"></div>
          <div class="spine spine--gap"></div>
          <div class="away">
            {clock(row.gap.startMs)} – {clock(row.gap.endMs)} · away — no capture
          </div>
        {:else if isShortActivity(row.slot.activity)}
          {@const a = row.slot.activity}
          <div class="when when--compact">
            <span class="clock">{clock(a.startedAtMs)}</span>
            <span class="dur">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
          </div>
          <div class="spine">
            <span class="node node--compact" style="background:{catVar(a.category)};"></span>
          </div>
          <div class="compact" class:is-sel={selectedId === a.id}>
            <button type="button" class="compact__hit" onclick={() => onselect(a)}>
              <span class="sw" style="background:{catVar(a.category)};"></span>
              <span class="compact__t">{a.title}</span>
            </button>
            {#if !row.slot.expired}
              <button type="button" class="foot" onclick={() => onopen(a)}>
                ▸ {frameLabel(row.slot.frameCount)}
              </button>
            {:else}
              <span class="foot foot--flat">footage expired</span>
            {/if}
          </div>
        {:else}
          {@const a = row.slot.activity}
          <div class="when">
            <span class="clock">{clock(a.startedAtMs)}</span>
            <span class="dur">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span>
          </div>
          <div class="spine">
            <span class="node" style="background:{catVar(a.category)};"></span>
          </div>
          <div class="card" class:is-sel={selectedId === a.id}>
            <button type="button" class="card__hit" onclick={() => onselect(a)}>
              <span class="meta">
                <span class="chip">
                  <i class="sw" style="background:{catVar(a.category)};"></i>
                  {a.category ? categoryLabel(a.category) : "Uncategorized"}
                </span>
                {#if a.focus}
                  <span class="chip">
                    <i class="sw" style="background:var({FOCUS_TOKEN[a.focus]});"></i>
                    {focusHint(a.focus)}
                  </span>
                {/if}
              </span>
              <span class="title">{a.title}</span>
              <span class="summary">{a.summary}</span>
            </button>
            {#if row.slot.expired}
              <!-- Retention removed the pixels; the card stays. A fact, not a
                   warning (ADR 0029). -->
              <span class="foot foot--flat">footage expired</span>
            {:else}
              <button type="button" class="foot" onclick={() => onopen(a)}>
                ▸ {frameLabel(row.slot.frameCount)}
              </button>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
  {/each}

  {#if pending.active && pending.reason}
    <div class="band" data-band="live"><span class="band__n">Deriving</span></div>
    <div class="river">
      <div class="when">
        <span class="clock">{pending.sinceMs !== null ? clock(pending.sinceMs) : ""}</span>
        <span class="dur">now</span>
      </div>
      <div class="spine"><span class="node node--pending"></span></div>
      <div class="card card--pending">
        {#if pending.reason.kind === "summarizing"}
          <span class="title title--quiet"><i class="pulse"></i>Summarizing this window…</span>
          <span class="summary">
            The journal trails live capture by up to 30 minutes — the footage
            itself is already on the Timeline.
          </span>
        {:else}
          <span class="title title--quiet">{pendingReasonCopy(pending.reason.reason)}</span>
          <button type="button" class="link" onclick={() => void openSettings("intelligence")}>
            Settings › Intelligence →
          </button>
        {/if}
      </div>
    </div>
  {/if}
{:else if showNothingCaptured}
  <div class="empty">
    <h4>Nothing captured on {dayLabel}</h4>
    <p>
      There's no capture on this day, so there's no journal to show. Days with any
      recording at all show whatever was captured.
    </p>
  </div>
{:else if showBeingWritten}
  <div class="empty">
    <h4>Your day is being written</h4>
    <p>
      Capture is landing. The first journal card appears once the first half-hour
      window has been summarized.
    </p>
  </div>
{/if}

<style>
  /* Section rule — the band's name, its span and its count. Sticky, so the
     address stays while the band scrolls (the settings surface's rule). */
  .band {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    height: 24px;
    padding: 0 16px;
    background: var(--app-bg);
    border-bottom: var(--hairline) solid var(--app-border);
  }
  .band__n {
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-strong);
  }

  /* when | spine | card — one grid per band so the spine is continuous. */
  .river {
    display: grid;
    grid-template-columns: 78px 16px 1fr;
    padding: 0 16px;
  }

  .when {
    padding: 10px 8px 0 0;
    text-align: right;
  }
  .when--compact {
    padding-top: 6px;
  }
  .clock {
    display: block;
    font: var(--w-medium) var(--t-meta) / 1.3 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .dur {
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
    width: 1px;
    background: var(--app-border);
    transform: translateX(-50%);
  }
  .spine--gap::before {
    background: transparent;
    border-left: 1px dashed var(--app-border);
  }
  .node {
    position: absolute;
    left: 50%;
    top: 15px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    transform: translateX(-50%);
    box-shadow: 0 0 0 3px var(--app-bg);
  }
  .node--compact {
    top: 11px;
  }
  .node--pending {
    background: var(--app-border-hover);
  }
  .node--sk {
    background: var(--app-border-strong);
  }

  /* Cards are hairline-separated content, not boxes: the only edge a card ever
     grows is the accent rule of the selection. */
  .card {
    padding: 8px 0 12px 10px;
    min-width: 0;
  }
  .card--sk {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
  }
  .card.is-sel,
  .compact.is-sel {
    background: var(--app-surface);
    border-radius: var(--r-md);
    box-shadow: inset 2px 0 0 var(--app-accent);
    padding-left: 12px;
  }
  .card__hit {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  .card__hit:focus-visible {
    outline: none;
    box-shadow: var(--ring);
    border-radius: var(--r-sm);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    margin-bottom: 3px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 17px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  .title {
    display: block;
    font: var(--w-semi) var(--t-ui) / 1.35 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .title--quiet {
    display: flex;
    align-items: center;
    gap: 7px;
    font-weight: var(--w-regular);
    color: var(--app-text-muted);
  }
  .summary {
    display: block;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
    max-width: 70ch;
  }

  /* The receipt affordance — the door behind every card that still has pixels. */
  .foot {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-top: 6px;
    padding: 0;
    border: 0;
    background: transparent;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
    cursor: pointer;
  }
  .foot:hover {
    color: var(--app-accent);
  }
  .foot--flat {
    cursor: default;
    color: var(--app-text-faint);
  }

  .compact {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    min-height: 28px;
    padding: 3px 0 3px 10px;
  }
  .compact__hit {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    flex: 1 1 auto;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  .compact__hit:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .compact__t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--w-regular) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text);
  }
  .compact .foot {
    margin: 0;
    flex: 0 0 auto;
  }

  .away {
    padding: 6px 0 6px 10px;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    font-style: italic;
    color: var(--app-text-faint);
  }

  .card--pending {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
    animation: journal-pulse 1.6s var(--ease) infinite;
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
    .pulse {
      animation: none;
    }
  }
  .link {
    align-self: flex-start;
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: transparent;
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-accent);
    cursor: pointer;
  }

  .empty {
    margin: 24px 16px;
    padding: 28px 20px;
    border: var(--hairline) solid var(--app-border);
    border-radius: var(--r-lg);
    background: var(--app-surface-subtle);
    text-align: center;
  }
  .empty h4 {
    margin: 0 0 6px;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .empty p {
    margin: 0 auto;
    max-width: 46ch;
    font: var(--w-regular) var(--t-meta) / 1.6 var(--app-font-sans);
    color: var(--app-text-muted);
  }
</style>
