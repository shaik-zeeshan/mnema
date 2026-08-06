<script lang="ts">
  // THE STORY OVER TIME — the selected belief's own history, newest first.
  // Three row kinds, one grammar: a confidence step names the move and its
  // direction, an evidence event shows the activity that fed it (with a real
  // frame thumb when one resolved), and a "replaced an earlier take" row is the
  // audit trail ADR 0046 writes — never a causal claim about the step above it.
  //
  // The stream itself is built by the shared, tested `buildTimeline`; this file
  // only renders it.
  import type { ActivityCategory } from "$lib/types/recording";
  import type { TimelineEvent } from "$lib/insights/subjectTimeline";
  import { CATEGORY_COLOR, categoryLabel } from "$lib/insights/activity-helpers";
  import { clock } from "$lib/overview/overview-format";
  import { ago, conf2, pct } from "./subjects-format";

  interface Props {
    events: TimelineEvent[];
    thumbnails: Map<number, string>;
    onOpenEvidence: (activityId: number, frameId: number | null) => void;
  }
  let { events, thumbnails, onOpenEvidence }: Props = $props();

  function catVar(c: string | null): string {
    return c && c in CATEGORY_COLOR
      ? `var(${CATEGORY_COLOR[c as ActivityCategory]})`
      : "var(--app-text-subtle)";
  }
  function catText(c: string | null): string {
    return c ? categoryLabel(c as ActivityCategory) : "Activity";
  }
</script>

<div class="tile tile--w2 tile--static">
  <div class="tile__h">
    <span class="t-label">The story over time</span>
    <span class="tile__more">newest first</span>
  </div>
  <p class="lede t-meta">
    Most recent at top. The line is this belief's confidence journey — evidence
    events feed it, quiet stretches let it decay.
  </p>
  <div class="legend">
    <span><i style="background:var(--app-accent)"></i>confidence trajectory</span>
    <span><i style="background:var(--app-text-faint)"></i>evidence event</span>
    <span class="up">↑ reinforced</span>
    <span class="down">↓ decayed / contradicted</span>
  </div>

  <div class="pay pay--rows track scroll">
    {#each events as ev, i (i)}
      {#if ev.kind === "marker"}
        <div class="row row--static ev ev--mark">
          <time>{ago(ev.atMs)}</time>
          <span class="ev__b">
            <s class={ev.direction === "reinforced" ? "up" : "down"}>
              {ev.direction === "reinforced" ? "↑" : "↓"}
            </s>
            confidence {conf2(ev.from)} → {conf2(ev.to)} · {ev.direction}
          </span>
        </div>
      {:else if ev.kind === "evidence" || ev.kind === "contradict"}
        {@const frameId = ev.kind === "evidence" ? ev.frameId : null}
        {@const thumb = frameId != null ? thumbnails.get(frameId) : undefined}
        <button
          type="button"
          class="row ev"
          onclick={() => onOpenEvidence(ev.activityId, frameId)}
        >
          <time>
            {ago(ev.atMs ?? 0)}
            {#if ev.atMs}<br /><span class="clock">{clock(ev.atMs)}</span>{/if}
          </time>
          <span class="ev__b">
            <span class="ev__t">
              {#if thumb}<img src={thumb} alt="" loading="lazy" />{/if}
            </span>
            <span class="ev__x">
              <span class="ev__c" style="color:{catVar(ev.kind === 'evidence' ? ev.category : null)}">
                <i class="sdot"></i>{catText(ev.kind === "evidence" ? ev.category : null)}
              </span>
              <span class="ev__ttl">{ev.title}</span>
              <span class="ev__f">
                {#if ev.kind === "contradict"}
                  <b class="no">contradicts</b>
                {:else}
                  <b>supports</b>
                {/if}
                <em>{frameId != null ? "view frame →" : "view in Timeline →"}</em>
              </span>
            </span>
          </span>
        </button>
      {:else if ev.kind === "replaced"}
        <div class="row row--static ev">
          <time>{ago(ev.atMs)}</time>
          <span class="ev__b">
            <span class="ev__x">
              <span class="ev__c dim">Replaced an earlier take</span>
              <span class="ev__ttl quote">“{ev.statement}”</span>
            </span>
          </span>
        </div>
      {:else}
        <div class="row row--static ev">
          <time>{ago(ev.atMs)}</time>
          <span class="ev__b">
            <span class="ev__x">
              <span class="ev__c acc">✦ formed</span>
              <span class="ev__ttl">Conclusion first formed</span>
              <span class="ev__f">Started at {pct(ev.confidence)}%.</span>
            </span>
          </span>
        </div>
      {/if}
    {:else}
      <div class="row row--static none">
        <span class="t-meta">Nothing recorded for this belief yet.</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .lede {
    margin: 0 0 var(--s-4);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-12);
    margin-top: var(--s-6);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
  }
  .legend i {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 4px;
    border-radius: 2px;
  }
  .up {
    color: var(--app-accent);
  }
  .down {
    color: var(--app-danger);
  }

  .track {
    margin-top: var(--s-8);
    overflow-y: auto;
  }

  .ev {
    display: grid;
    grid-template-columns: 58px 1fr;
    gap: var(--s-8);
    width: 100%;
    min-height: 0;
    padding: var(--s-8) var(--tile-pad);
    border: 0;
    background: transparent;
    text-align: left;
  }
  button.ev {
    cursor: pointer;
  }
  button.ev:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
  .ev time {
    text-align: right;
    font: var(--w-regular) var(--t-label) / 1.5 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
    white-space: nowrap;
  }
  .ev__b {
    display: flex;
    gap: var(--s-8);
    min-width: 0;
  }
  .ev--mark .ev__b {
    align-items: center;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .ev--mark s {
    margin-right: 4px;
    text-decoration: none;
  }
  .ev__t {
    position: relative;
    flex: 0 0 auto;
    width: 46px;
    height: 28px;
    border-radius: var(--tile-r-in);
    background: var(--media-void);
    overflow: hidden;
  }
  .ev__t img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .ev__x {
    min-width: 0;
  }
  .ev__c {
    display: inline-flex;
    align-items: center;
    gap: var(--s-4);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .ev__c.dim {
    color: var(--app-text-subtle);
  }
  .ev__c.acc {
    color: var(--app-accent);
  }
  .sdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    flex: 0 0 auto;
  }
  .ev__ttl {
    display: block;
    margin-top: 3px;
    font: var(--w-regular) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .ev__ttl.quote {
    color: var(--app-text-muted);
  }
  .ev__f {
    display: block;
    margin-top: 3px;
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
  }
  .ev__f b {
    font-weight: var(--w-medium);
    color: var(--app-accent);
  }
  .ev__f b.no {
    color: var(--app-danger);
  }
  .ev__f em {
    margin-left: var(--s-8);
    font-style: normal;
    color: var(--app-accent);
  }
  .none {
    min-height: 0;
    padding: var(--s-8) var(--tile-pad);
  }
</style>
