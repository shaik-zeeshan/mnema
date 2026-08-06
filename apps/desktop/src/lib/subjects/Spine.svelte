<script lang="ts">
  // The story over time: the selected conclusion's evidence interleaved with the
  // confidence markers between them, newest at top, ending at "formed".
  //
  // Render-only. Every event comes from `buildTimeline` (subjectTimeline.ts) —
  // markers and evidence are interleaved by TIMESTAMP, never by a fabricated
  // causal link between a marker and any one piece of evidence.
  import type { TimelineEvent } from "$lib/insights/subjectTimeline";
  import type { Activity } from "$lib/types/recording";
  import { clockLabel, pct, relativeTime } from "./data";

  interface Props {
    events: TimelineEvent[];
    /** Resolved activities — fills a contradict event's kind chip + category. */
    activities: Map<number, Activity>;
    /** frameId → preview asset URL. */
    previews: Map<number, string>;
    onView: (activityId: number) => void;
  }

  let { events, activities, previews, onView }: Props = $props();
</script>

{#snippet chev()}
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M6 3.5 10.5 8 6 12.5" />
  </svg>
{/snippet}

  <div class="legend">
    <span><i></i>confidence trajectory</span>
    <span><i class="dot"></i>evidence event</span>
    <span class="legend--up">↑ reinforced</span>
    <span class="legend--down">↓ decayed / contradicted</span>
    <span class="legend__foot t-meta">The story over time · most recent at top</span>
  </div>

  <div class="spine">
    {#each events as ev, i (i)}
      <div class="sev">
        <span class="sev__g">
          {#if ev.kind === "evidence" || ev.kind === "contradict"}
            {relativeTime(ev.atMs)}<br />{clockLabel(ev.atMs)}
          {:else}
            {relativeTime(ev.atMs)}
          {/if}
        </span>
        <span class="sev__n" class:sev__n--last={i === events.length - 1}>
          <i class:small={ev.kind === "marker" || ev.kind === "replaced"}></i>
          <u></u>
        </span>
        <div class="sev__b">
          {#if ev.kind === "evidence" || ev.kind === "contradict"}
            <!-- A contradict event carries only its activity id; the same
                 Activity join the evidence rows use fills in its kind chip,
                 category and frame. -->
            {@const act = activities.get(ev.activityId)}
            {@const raw = act?.evidence?.[0]}
            {@const kind = raw?.subjectType === "audio_segment" ? "mic" : raw ? "scr" : null}
            {@const thumb =
              ev.kind === "evidence"
                ? ev.frameId
                : raw?.subjectType === "frame"
                  ? raw.subjectId
                  : null}
            <div class="sev__card">
              <span class="sev__thumb">
                {#if thumb !== null && previews.get(thumb)}
                  <img src={previews.get(thumb)} alt="" />
                {/if}
              </span>
              <span class="sev__x">
                <span class="sev__line">
                  {#if kind}
                    <span class="evsrc evsrc--{kind}">{kind}</span>
                  {/if}
                  {#if act?.category}
                    <span class="t-label sev__cat">{act.category}</span>
                  {/if}
                  <span class="sev__ttl">{ev.title}</span>
                </span>
                <span
                  class="t-meta"
                  class:sev__supports={ev.kind === "evidence" && ev.stance === "support"}
                  class:sev__contradicts={ev.kind === "contradict"}
                >
                  {ev.kind === "contradict" ? "contradicts" : "supports"}
                </span>
              </span>
              <button
                type="button"
                class="btn btn--ghost btn--sm sev__act"
                onclick={() => onView(ev.activityId)}
              >
                {kind === "mic" ? "view in Timeline" : "view frame"}
                {@render chev()}
              </button>
            </div>
          {:else if ev.kind === "marker"}
            <span class="sev__mk is-num">
              confidence {pct(ev.from)} → {pct(ev.to)} · {ev.direction}
            </span>
          {:else if ev.kind === "replaced"}
            <span class="sev__mk">
              Replaced an earlier take — <span class="sev__quote">“{ev.statement}”</span>
            </span>
          {:else}
            <span class="sev__mk">
              <span class="sev__formed">✦ formed</span> — Conclusion first formed. Started
              at {pct(ev.confidence)}%.
            </span>
          {/if}
        </div>
      </div>
    {/each}
    {#if events.length === 0}
      <p class="t-meta spine__empty">Nothing recorded for this conclusion yet.</p>
    {/if}
  </div>

<style>
  .spine__empty {
    padding: var(--s-16) 0;
    color: var(--app-text-muted);
  }
  /* ── the spine ────────────────────────────────────────────────────────── */
  .legend {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--s-12);
    padding: var(--s-6) var(--s-16) var(--s-8) calc(var(--s-16) + 78px);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .legend i {
    width: 14px;
    height: 2px;
    border-radius: 1px;
    background: var(--app-accent);
  }
  .legend i.dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }
  .legend--up {
    color: var(--app-accent);
  }
  .legend--down {
    color: var(--app-info);
  }
  .legend__foot {
    margin-left: auto;
  }

  .spine {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--s-16) var(--s-12);
  }
  .sev {
    display: flex;
    gap: var(--s-8);
  }
  .sev__g {
    flex: 0 0 66px;
    width: 66px;
    text-align: right;
    padding-top: 7px;
    font: var(--w-regular) var(--t-meta) / 1.3 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  .sev__n {
    flex: 0 0 12px;
    position: relative;
  }
  .sev__n i {
    position: absolute;
    left: 3px;
    top: 10px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  .sev__n i.small {
    top: 12px;
    left: 4px;
    width: 5px;
    height: 5px;
    background: var(--app-text-faint);
  }
  .sev__n u {
    position: absolute;
    left: 6px;
    top: 18px;
    bottom: -3px;
    width: var(--hairline);
    background: var(--app-border-strong);
  }
  .sev__n--last u {
    display: none;
  }
  .sev__b {
    flex: 1 1 auto;
    min-width: 0;
    padding: var(--s-4) 0 var(--s-8);
  }
  .sev__card {
    display: flex;
    gap: var(--s-8);
    padding: var(--s-6) var(--s-8);
    border-radius: var(--r-md);
    background: var(--app-surface);
  }
  .sev__thumb {
    flex: 0 0 76px;
    width: 76px;
    height: 48px;
    border-radius: var(--r-sm);
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
  }
  .sev__thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .sev__x {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    justify-content: center;
  }
  .sev__line {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    min-width: 0;
  }
  .sev__cat {
    color: var(--app-text-subtle);
  }
  .sev__ttl {
    font: var(--w-medium) var(--t-ui) / 1.25 var(--app-font-sans);
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sev__supports {
    color: var(--app-accent);
  }
  .sev__contradicts {
    color: var(--app-info);
  }
  .sev__act {
    align-self: center;
    flex: 0 0 auto;
  }
  .sev__act svg {
    width: 11px;
    height: 11px;
  }
  .sev__mk {
    display: inline-block;
    padding: 3px 0;
    font: var(--w-regular) var(--t-meta) / 1.4 var(--app-font-mono);
    color: var(--app-text-muted);
  }
  .sev__quote {
    color: var(--app-text-subtle);
  }
  .sev__formed {
    color: var(--app-accent);
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
</style>
