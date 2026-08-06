<script lang="ts">
  // ══ ONE SUBJECT, ONE ROW ═══════════════════════════════════════════════════
  //
  // The row's hero is the confidence TRACE in a well: a measured quantity
  // plotted against the engine's printed 0.15 floor. It reads and never turns —
  // the whole row is a link into `/overview/subjects/<subject>`.
  //
  // Every figure is a real read (G8): the conclusion count, the percent, the
  // last-moved stamp. A subject whose trajectory fetch hasn't landed (or has
  // fewer than two snapshots) shows NO trace rather than a straight-line
  // stand-in, and a missing timestamp prints nothing at all.
  import type { ConfidenceSnapshot } from "$lib/types/recording";
  import type { Trend } from "$lib/insights/subjectsTiers";
  import ConfidenceTrace from "$lib/overview/ConfidenceTrace.svelte";
  import { agoLabel, pctLabel, trendLabel } from "$lib/overview/subjects/format";

  interface Props {
    subject: string;
    statement: string;
    conclusionCount: number;
    confidence: number;
    trend: Trend;
    pinned: boolean;
    faded: boolean;
    lastMovedAtMs: number;
    /** The leading conclusion's own history; < 2 points draws nothing. */
    lead: ConfidenceSnapshot[];
    /** The subject's other conclusions, faint behind the lead. */
    others: ConfidenceSnapshot[][];
    /** The list's shared clock, so a six-week trace looks six weeks long. */
    domain?: [number, number];
  }

  let {
    subject,
    statement,
    conclusionCount,
    confidence,
    trend,
    pinned,
    faded,
    lastMovedAtMs,
    lead,
    others,
    domain,
  }: Props = $props();

  const moved = $derived(agoLabel(lastMovedAtMs));
</script>

<a
  class="ti-grow srow"
  class:srow--faded={faded}
  href="/overview/subjects/{encodeURIComponent(subject)}"
>
  <span class="ti-grow__txt">
    <span class="srow__l1">
      <span class="srow__dot" class:srow__dot--faded={faded}></span>
      <span class="srow__nm">{subject}</span>
      {#if pinned}<span class="srow__pin" title="Pinned — protected from decay">★</span>{/if}
      <span class="srow__trend" class:is-up={trend === "up"} class:is-down={trend === "down"}>
        {trendLabel(trend)}
      </span>
      <span class="t-meta srow__count is-num">
        · {conclusionCount} conclusion{conclusionCount === 1 ? "" : "s"}
      </span>
    </span>
    <span class="ti-grow__sub srow__hd">{statement}</span>
  </span>
  <span class="ti-grow__val">
    {#if lead.length >= 2}
      <ConfidenceTrace
        history={lead}
        {others}
        {domain}
        size="row"
        label="Confidence over time for {subject}"
      />
    {/if}
    <span class="srow__n">
      <b class="is-num">{pctLabel(confidence)}</b>
      {#if moved}<span class="is-num">{moved}</span>{/if}
    </span>
    <span class="ti-chev" aria-hidden="true">
      <svg width="8" height="12" viewBox="0 0 8 12" fill="none" stroke="currentColor"
        stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
        <path d="m1.5 1 5 5-5 5" />
      </svg>
    </span>
  </span>
</a>

<style>
  .srow {
    min-height: 52px;
    text-decoration: none;
    color: inherit;
  }
  .srow:hover {
    background: var(--app-surface-hover);
  }
  .srow:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  /* Faded views are kept for history, never deleted — so they stay legible,
     just quieter. */
  .srow--faded {
    opacity: 0.62;
  }
  .srow__l1 {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    min-width: 0;
  }
  .srow__dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--app-accent);
    flex: 0 0 auto;
  }
  .srow__dot--faded {
    background: var(--app-text-faint);
  }
  .srow__nm {
    font: var(--w-medium) var(--t-ui) / 1.2 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .srow__pin {
    color: var(--app-accent);
    flex: 0 0 auto;
  }
  .srow__trend {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    text-transform: uppercase;
    color: var(--app-text-subtle);
    flex: 0 0 auto;
  }
  .srow__trend.is-up {
    color: var(--app-accent);
  }
  .srow__trend.is-down {
    color: var(--app-warn);
  }
  .srow__count {
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .srow__hd {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 52ch;
  }
  .srow__n {
    flex: 0 0 auto;
    width: 62px;
    text-align: right;
  }
  .srow__n b {
    display: block;
    font: var(--w-semi) var(--t-title) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  .srow__n span {
    display: block;
    margin-top: 3px;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
  }
</style>
