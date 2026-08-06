<script lang="ts">
  // One time-of-day band = one 4×1 tile (mockup 08). The chrome is the bento's
  // constant 18px header row (mono eyebrow left, meta right); the payload under
  // it is the free composition no other tile in the app uses — a 68px
  // when-column, an 18px spine with a category-tinted node, then the card.
  //
  // Three row shapes, all real states: a full card, a one-line row for an
  // activity under five minutes (`isShortActivity`), and a dashed away-gap row
  // for ≥ 5 minutes with no capture between covered frames. The live edge is a
  // dashed *pending* card — never a fake summary.
  import type { Activity, ActivityFocus } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import { isShortActivity, pendingReasonCopy } from "$lib/insights/journal-view";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    focusHint,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import { clock, clockShort } from "$lib/insights/receipt-clock";
  import { bandRowKey, type JournalBand } from "./bands";

  let {
    band,
    pending,
    onOpen,
  }: {
    band: JournalBand;
    pending: JournalPending;
    onOpen: (activity: Activity) => void;
  } = $props();

  // "3 activities · 9:04 – 11:44"; the live band ends at `now`, not a clock.
  const meta = $derived.by(() => {
    const parts: string[] = [];
    if (band.count > 0) {
      parts.push(`${band.count} ${band.count === 1 ? "activity" : "activities"}`);
    }
    const end = band.endMs === null ? "now" : clockShort(band.endMs);
    parts.push(`${clockShort(band.startMs)} – ${end}`);
    return parts.join(" · ");
  });

  const FOCUS_TOKEN: Record<ActivityFocus, string> = {
    deep: "--focus-deep",
    mixed: "--focus-mid",
    distracted: "--focus-distracted",
  };
  function catVar(category: Activity["category"]): string {
    return category ? `var(${CATEGORY_COLOR[category]})` : `var(${UNCATEGORIZED_COLOR})`;
  }
  // Zero frames is the expired state (retention removes pixels, never cards),
  // never a literal "0 frames".
  function footLabel(frameCount: number, expired: boolean): string {
    if (expired) return "footage expired";
    return `▸ ${frameCount} ${frameCount === 1 ? "frame" : "frames"} · receipt`;
  }
</script>

<div class="tile tile--w4 tile--static">
  <div class="tile__h">
    <span class="t-label">{band.label}</span>
    <span class="tile__more is-num">{meta}</span>
  </div>
  <div class="pay pay--rows">
    {#each band.rows as row (bandRowKey(row))}
      {#if row.kind === "gap"}
        <div class="row row--static jrow jrow--gap">
          <span class="jwhen"></span>
          <span class="jspine jspine--dash"></span>
          <span class="jgap">
            {clock(row.gap.startMs)} – {clock(row.gap.endMs)} · away — no capture
          </span>
        </div>
      {:else if row.kind === "pending"}
        <div class="row row--static jrow jrow--pend">
          <span class="jwhen"><b>{clock(row.atMs)}</b><i>now</i></span>
          <span class="jspine"><u class="u--pend"></u></span>
          <span class="jcard">
            {#if pending.reason?.kind === "engine_unavailable"}
              <span class="jpend jpend--paused">{pendingReasonCopy(pending.reason.reason)}</span>
            {:else}
              <span class="jpend"><i></i>Summarizing this window…</span>
              <span class="jsum">
                The journal trails live capture by up to 30 minutes — the footage itself is
                already on the Timeline.
              </span>
            {/if}
          </span>
        </div>
      {:else if isShortActivity(row.slot.activity)}
        {@const a = row.slot.activity}
        <button
          type="button"
          class="row jrow jrow--min"
          style="color:{catVar(a.category)}"
          onclick={() => onOpen(a)}
        >
          <span class="jwhen"><b>{clock(a.startedAtMs)}</b><i>{humanizeMs(a.endedAtMs - a.startedAtMs)}</i></span>
          <span class="jspine"><u></u></span>
          <span class="jcard"><span class="jmin">{a.title}</span></span>
        </button>
      {:else}
        {@const a = row.slot.activity}
        <button
          type="button"
          class="row jrow"
          style="color:{catVar(a.category)}"
          onclick={() => onOpen(a)}
        >
          <span class="jwhen"><b>{clock(a.startedAtMs)}</b><i>{humanizeMs(a.endedAtMs - a.startedAtMs)}</i></span>
          <span class="jspine"><u></u></span>
          <span class="jcard">
            <span class="jtop">
              <span class="jcat"><i></i>{a.category ? categoryLabel(a.category) : "Uncategorized"}</span>
              {#if a.focus}
                <span class="jfoc" style="color:var({FOCUS_TOKEN[a.focus]})">
                  <i></i>{focusHint(a.focus)}
                </span>
              {/if}
            </span>
            <span class="jttl">{a.title}</span>
            <span class="jsum">{a.summary}</span>
            <span class="jfoot" class:is-gone={row.slot.expired}>
              {footLabel(row.slot.frameCount, row.slot.expired)}
            </span>
          </span>
        </button>
      {/if}
    {/each}
  </div>
</div>

<style>
  /* The river row: when · spine · card. Same `.row` hover + inset separator as
     every other tile list; only the payload shape is new. */
  .jrow {
    display: grid;
    grid-template-columns: 68px 18px 1fr;
    gap: 0 10px;
    align-items: start;
    width: 100%;
    min-height: 0;
    padding: var(--s-8) var(--tile-pad);
    border: 0;
    background: transparent;
    font: inherit;
    text-align: left;
    /* AI-written titles carry long unbreakable tokens (paths, URLs); without
       this they blow out the 1fr track and x-scroll the page. */
    overflow-wrap: anywhere;
  }
  button.jrow {
    cursor: pointer;
  }
  button.jrow:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }

  .jwhen {
    padding-top: 2px;
    text-align: right;
  }
  .jwhen b {
    display: block;
    font: var(--w-regular) var(--t-meta) / 1.2 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text);
  }
  .jwhen i {
    display: block;
    margin-top: 2px;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-style: normal;
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }

  .jspine {
    position: relative;
    align-self: stretch;
  }
  .jspine::before {
    content: "";
    position: absolute;
    left: 50%;
    top: -10px;
    bottom: -10px;
    width: var(--hairline);
    background: var(--tile-sep);
    transform: translateX(-50%);
  }
  .jspine--dash::before {
    background: repeating-linear-gradient(
      to bottom,
      var(--app-border-strong) 0 3px,
      transparent 3px 7px
    );
  }
  .jspine u {
    position: absolute;
    left: 50%;
    top: 5px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    transform: translateX(-50%);
    background: currentColor;
    box-shadow: 0 0 0 2px var(--tile-fill);
  }
  .jspine u.u--pend {
    background: var(--app-border-strong);
  }

  .jcard {
    min-width: 0;
    padding-left: var(--s-8);
    border-left: 3px solid currentColor;
  }
  .jtop {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .jcat {
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .jcat i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
  }
  .jfoc {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .jfoc i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
  }
  .jttl {
    display: block;
    margin-top: 5px;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .jsum {
    display: block;
    margin-top: 3px;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .jfoot {
    display: block;
    margin-top: var(--s-8);
    padding-top: var(--s-6);
    border-top: var(--hairline) dashed var(--tile-sep);
    text-align: right;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .jfoot.is-gone {
    color: var(--app-text-faint);
  }

  /* Under five minutes: one line, no summary and no footer. */
  .jrow--min .jcard {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    padding-left: 0;
    border-left: 0;
  }
  .jrow--min .jspine u {
    top: 1px;
  }
  .jmin {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font: var(--w-regular) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text);
  }

  /* A gap in covered capture is its own row, never a silent hole. */
  .jrow--gap {
    min-height: 0;
  }
  .jgap {
    grid-column: 3;
    padding: var(--s-4) 0;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    font-style: italic;
    color: var(--app-text-faint);
  }

  /* The live edge: dashed, and it says what it is doing. */
  .jrow--pend .jcard {
    border-left: 3px dashed var(--app-border-strong);
  }
  .jpend {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .jpend--paused {
    font-weight: var(--w-regular);
  }
  .jpend i {
    flex: 0 0 auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
    animation: jpulse 2s var(--ease) infinite;
  }
  @keyframes jpulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .jpend i {
      animation: none;
    }
  }
</style>
