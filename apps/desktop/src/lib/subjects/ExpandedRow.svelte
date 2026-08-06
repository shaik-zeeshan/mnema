<script lang="ts">
  // What a subject row opens into: its conclusions ranked by confidence, each
  // with the two corrections that actually exist (Pin, Dismiss), then what they
  // are grounded in.
  //
  // The evidence cap is FIVE CHIPS PLUS AN EXPLICIT "+N more" — never a silent
  // cap, which would read as "that is all the evidence there is", the one thing
  // a receipt must not imply.
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";
  import type { Activity, Conclusion } from "$lib/types/recording";
  import { chipFor, evidenceIds, pct, relativeTime, type SubjectRow } from "./data";

  interface Props {
    row: SubjectRow;
    /** Resolved evidence activities, shared across expanded rows. */
    activities: Map<number, Activity>;
    /** frameId → preview asset URL. */
    previews: Map<number, string>;
    /** Conclusion id of the in-flight pin/dismiss, or null. */
    acting: number | null;
    onPin: (c: Conclusion) => void;
    onDismiss: (c: Conclusion) => void;
    onViewFrame: () => void;
  }

  let { row, activities, previews, acting, onPin, onDismiss, onViewFrame }: Props = $props();

  const CAP = 5;

  const ids = $derived(evidenceIds(row.conclusions));
  const chips = $derived(
    ids
      .map((id) => activities.get(id))
      .filter((a): a is Activity => a !== undefined)
      .map(chipFor)
      .sort((a, b) => (b.atMs ?? 0) - (a.atMs ?? 0)),
  );
</script>

<div class="exp">
  <p class="t-label exp__hd">Conclusions · ranked by confidence</p>
  {#each row.conclusions as c, ci (c.id)}
    {@const faded = c.status === "faded"}
    <div class="cline">
      <span class="cline__x" class:is-faded={faded}>
        {#if c.pinned}<span class="pinstar" aria-hidden="true">★</span>{/if}{c.statement}
      </span>
      <span class="cbar">
        <i style="width:{pct(c.confidence)}%" class:is-faded={faded}></i>
        <u style="left:{pct(DISPLAY_FLOOR)}%"></u>
      </span>
      <span class="cline__p is-mono is-num" class:is-faded={faded}>{pct(c.confidence)}%</span>
      <span class="chip" class:chip--on={!faded}>{faded ? "faded" : "active"}</span>
      <button
        type="button"
        class="btn btn--ghost btn--sm"
        disabled={acting !== null}
        onclick={() => onPin(c)}
      >
        {c.pinned ? "Pinned ◆" : "Pin"}
        {#if ci === 0}<span class="kbd">⌘P</span>{/if}
      </button>
      <button
        type="button"
        class="btn btn--ghost btn--sm"
        disabled={acting !== null}
        onclick={() => onDismiss(c)}
      >
        Dismiss
        {#if ci === 0}<span class="kbd">⌘⌫</span>{/if}
      </button>
    </div>
  {/each}

  <p class="t-label exp__hd">Grounded in</p>
  <div class="evchips">
    {#each chips.slice(0, CAP) as chip (chip.activityId)}
      <span class="evchip">
        <i>
          {#if chip.frameId !== null && previews.get(chip.frameId)}
            <img src={previews.get(chip.frameId)} alt="" />
          {/if}
        </i>
        <span class="evsrc evsrc--{chip.kind}">{chip.kind}</span>
        {relativeTime(chip.atMs)}
      </span>
    {/each}
    {#if ids.length > CAP}
      <span class="evmore t-meta">+{ids.length - CAP} more</span>
    {:else if chips.length === 0}
      <span class="evmore t-meta">
        {ids.length === 0 ? "No grounding evidence linked." : "Resolving evidence…"}
      </span>
    {/if}
    {#if chips.length > 0}
      <button type="button" class="btn btn--sm evview" onclick={onViewFrame}>
        View frame
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"
          stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M6 3.5 10.5 8 6 12.5" />
        </svg>
      </button>
    {/if}
  </div>
</div>

<style>
  .exp {
    margin: 0 var(--s-8) var(--s-8);
    padding: var(--s-8) var(--s-12) var(--s-12);
    border-radius: 0 0 var(--r-md) var(--r-md);
    background: var(--app-surface);
  }
  .exp__hd {
    margin: 0;
    padding-bottom: 2px;
  }
  .exp__hd + .cline {
    box-shadow: none;
  }
  .cline {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: var(--s-6) 0;
  }
  .cline + .cline {
    box-shadow: inset 0 var(--hairline) 0 var(--app-border);
  }
  .cline__x {
    flex: 1 1 auto;
    min-width: 0;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-sans);
    color: var(--app-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cline__x.is-faded,
  .cline__p.is-faded {
    color: var(--app-text-muted);
  }
  .cline__p {
    flex: 0 0 34px;
    text-align: right;
    font-size: var(--t-meta);
    color: var(--app-text-strong);
  }
  .pinstar {
    color: var(--app-warn);
    font-size: 11px;
    line-height: 1;
    margin-right: 3px;
  }

  .cbar {
    flex: 0 0 96px;
    position: relative;
    height: 5px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    overflow: hidden;
  }
  .cbar i {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: var(--app-accent);
  }
  .cbar i.is-faded {
    opacity: 0.5;
  }
  /* The display floor, drawn on the bar too — the same 15% the sparkline draws. */
  .cbar u {
    position: absolute;
    top: 0;
    bottom: 0;
    width: var(--hairline);
    background: var(--app-text-faint);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--r-pill);
    background: var(--app-surface-hover);
    color: var(--app-text-subtle);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-sans);
  }
  .chip--on {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }

  .evchips {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    flex-wrap: wrap;
    margin-top: var(--s-6);
  }
  .evchip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 7px 0 3px;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .evchip > i {
    display: block;
    width: 26px;
    height: 17px;
    border-radius: 2px;
    overflow: hidden;
    background: var(--app-surface-subtle);
  }
  .evchip img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .evsrc {
    padding: 2px 3px;
    border-radius: 2px;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.04em;
  }
  .evsrc--scr {
    background: var(--app-source-screen-bg);
    color: var(--app-source-screen);
  }
  .evsrc--mic {
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }
  .evmore {
    color: var(--app-text-subtle);
  }
  .evview {
    margin-left: auto;
  }
  .evview svg {
    width: 11px;
    height: 11px;
  }
</style>
