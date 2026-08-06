<script lang="ts">
  // The opened belief: its statement at full length, the confidence hero, and
  // the story over time.
  //
  // The spine is `buildTimeline` (tested, in `subjectTimeline.ts`): evidence
  // events, reinforcement and decay markers, a replaced take, and the "formed"
  // row that is always last. Nothing here re-orders or re-derives that stream —
  // no causal claim is made between a marker and any one piece of evidence.
  import IconPin from "~icons/lucide/pin";
  import IconScreen from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import { DISPLAY_FLOOR } from "$lib/insights/subjectsTiers";
  import { ago, conf, pct, shortDate } from "./format";
  import Trajectory from "./Trajectory.svelte";
  import type { SubjectDetailData } from "./subject-detail-data.svelte";

  interface Props {
    data: SubjectDetailData;
  }

  let { data }: Props = $props();

  const c = $derived(data.selected);
  const history = $derived(c ? data.historyOf(c.id) : []);
  const faded = $derived(c?.status === "faded");

  const trend = $derived.by<{ glyph: string; cls: string }>(() => {
    if (history.length >= 2) {
      const delta = history[history.length - 1] - history[0];
      if (delta > 0.04) return { glyph: "▲", cls: "is-up" };
      if (delta < -0.04) return { glyph: "▼", cls: "is-down" };
    }
    return { glyph: "–", cls: "" };
  });

  const busy = $derived(c !== null && data.actionId === c.id);
</script>

{#if c}
  <div class="col">
    <div class="hero">
      <div class="chips">
        {#if c.pinned}<span class="ss-chip ss-chip--ok">★ pinned</span>{/if}
        <span class="ss-chip">{faded ? "below floor" : "visible"}</span>
      </div>

      <p class="stmt">{c.statement}</p>

      <div class="figures">
        <div class="big">
          <span class="big__n"
            >{pct(c.confidence)}%<span class="big__t {trend.cls}">{trend.glyph}</span></span
          >
          <span class="t-label">confidence</span>
        </div>
        <Trajectory
          lines={[{ points: history.length > 0 ? history : [c.confidence, c.confidence], lead: true, faded }]}
          floor={DISPLAY_FLOOR}
          width={220}
          height={40}
          label="This belief's confidence across its recorded snapshots"
        />
        <span class="ss-tstrip__spacer"></span>
        <button
          type="button"
          class="btn btn--sm"
          class:is-on={c.pinned}
          disabled={data.actionId !== null}
          aria-pressed={c.pinned}
          onclick={() => void data.togglePin(c)}
        >
          <span class="ic" aria-hidden="true"><IconPin /></span>
          {#if busy && data.actionKind === "pin"}
            Saving…
          {:else if c.pinned}
            Pinned — protected from decay
          {:else}
            Pin — hold it still
          {/if}
        </button>
        <button
          type="button"
          class="btn btn--sm btn--ghost"
          disabled={data.actionId !== null}
          onclick={() => void data.dismiss(c)}
        >
          {busy && data.actionKind === "dismiss" ? "Dismissing…" : "Dismiss"}
        </button>
      </div>
    </div>

    <div class="story">
      <div class="story__h">
        <span class="t-ui strong">The story over time</span>
        <span class="t-meta">Most recent at top</span>
      </div>
      <p class="t-meta sub story__note">
        The accent line is this belief's confidence journey — evidence events feed it,
        quiet stretches let it decay.
      </p>
      <div class="legend">
        <span><i class="key key--line"></i>confidence trajectory</span>
        <span><i class="key key--dot"></i>evidence event</span>
        <span>↑ reinforced</span>
        <span>↓ decayed / contradicted</span>
      </div>

      {#each data.events as ev, i (i)}
        <div class="ev">
          <div class="spine">
            <span
              class="node"
              class:is-quiet={ev.kind !== "evidence"}
              class:is-warn={ev.kind === "marker" && ev.direction === "decayed"}
            ></span>
          </div>

          {#if ev.kind === "evidence" || ev.kind === "contradict"}
            <div class="ev__b">
              <span class="thumb" class:thumb--audio={ev.kind === "evidence" && ev.sourceType === "audio"}>
                <span class="ic" aria-hidden="true">
                  {#if ev.kind === "evidence" && ev.sourceType === "audio"}<IconMic />{:else}<IconScreen
                    />{/if}
                </span>
                <span class="thumb__tag"
                  >{ev.kind === "evidence" && ev.sourceType === "audio" ? "mic" : "scr"}</span
                >
              </span>
              <div class="ev__txt">
                <div class="ev__l1">
                  {#if ev.kind === "evidence" && ev.category}
                    <span class="cat">{ev.category}</span>
                  {/if}
                  {#if ev.atMs !== null}<span class="t-meta is-mono sub">{ago(ev.atMs)}</span>{/if}
                </div>
                <p class="ev__t">{ev.title}</p>
                <p class="ev__f">
                  <b class:is-bad={ev.kind === "contradict"}
                    >{ev.kind === "contradict" ? "contradicts" : "supports"}</b
                  >
                  <span class="sep">·</span>
                  <button
                    type="button"
                    class="link"
                    onclick={() => void data.openActivity(ev.activityId)}
                    >{ev.kind === "evidence" && ev.sourceType === "audio"
                      ? "view in Timeline"
                      : "view frame"} →</button
                  >
                </p>
              </div>
            </div>
          {:else if ev.kind === "marker"}
            <div class="mk">
              <span class="mk__a" class:is-down={ev.direction === "decayed"}
                >{ev.direction === "decayed" ? "↓" : "↑"}</span
              >
              <span class="t-meta"
                >confidence <b class="is-mono">{conf(ev.from)} → {conf(ev.to)}</b> ·
                {ev.direction}</span
              >
              <span class="t-meta sub">{ago(ev.atMs)}</span>
            </div>
          {:else if ev.kind === "replaced"}
            <div class="mk mk--stack">
              <span class="t-meta strong">Replaced an earlier take</span>
              <span class="t-meta sub struck">{ev.statement}</span>
            </div>
          {:else}
            <div class="mk">
              <span class="mk__a">✦</span>
              <span class="t-meta"
                >formed <b class="is-mono">{shortDate(ev.atMs)}</b> · at
                {pct(ev.confidence)}%</span
              >
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .col {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* ── Hero ──────────────────────────────────────────────────────────────── */
  .hero {
    flex: 0 0 auto;
    padding: var(--s-12) var(--s-16) var(--s-10);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .chips {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    margin-bottom: var(--s-6);
  }

  .stmt {
    margin: 0 0 var(--s-8);
    max-width: 66ch;
    font: var(--w-regular) 15px / 1.5 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-strong);
  }

  /* The hero wraps rather than clipping: at a narrow pane the two controls drop
     to their own line instead of running under the inspector's edge. */
  .figures {
    display: flex;
    align-items: flex-end;
    flex-wrap: wrap;
    gap: var(--s-8) var(--s-16);
  }

  .figures :global(.traj) {
    flex: 0 1 auto;
  }

  .big {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .big__n {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    font: var(--w-semi) var(--t-display) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-display);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }

  .big__t {
    font: var(--w-medium) 11px / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }

  .big__t.is-up {
    color: var(--app-accent-strong);
  }

  .big__t.is-down {
    color: var(--app-warn);
  }

  .ic {
    display: flex;
    font-size: 11px;
  }

  .btn.is-on {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  /* ── The story ─────────────────────────────────────────────────────────── */
  .story {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-10) var(--s-16) var(--s-16);
  }

  .story__h {
    display: flex;
    align-items: baseline;
    gap: var(--s-10);
    margin-bottom: 2px;
  }

  .story__note {
    margin: 0 0 var(--s-6);
    max-width: 74ch;
  }

  .legend {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    flex-wrap: wrap;
    margin-bottom: var(--s-8);
  }

  .legend span {
    display: inline-flex;
    align-items: center;
    gap: var(--gap-label);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-subtle);
  }

  .key {
    display: inline-block;
  }

  .key--line {
    width: 12px;
    height: 2px;
    background: var(--app-accent);
  }

  .key--dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }

  /* One event row: the spine column, then the content. */
  .ev {
    display: grid;
    grid-template-columns: 72px 1fr;
    column-gap: var(--s-10);
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

  .node {
    position: absolute;
    left: 50%;
    top: 14px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    transform: translateX(-50%);
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-bg);
  }

  .node.is-quiet {
    top: 11px;
    background: var(--app-text-faint);
  }

  .node.is-warn {
    background: var(--app-warn);
  }

  .ev__b {
    display: flex;
    gap: var(--s-8);
    padding: 7px 0;
    align-items: flex-start;
    min-width: 0;
  }

  .thumb {
    position: relative;
    width: 58px;
    height: 36px;
    flex: 0 0 auto;
    border-radius: 3px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--app-src-screen-bg);
    color: var(--app-src-screen);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
  }

  .thumb--audio {
    background: var(--app-src-mic-bg);
    color: var(--app-src-mic);
  }

  .thumb .ic {
    font-size: 14px;
  }

  .thumb__tag {
    position: absolute;
    left: 3px;
    bottom: 3px;
    padding: 0 3px;
    border-radius: 2px;
    background: rgb(0 0 0 / 62%);
    color: #fff;
    font: var(--w-medium) 8px / 1.5 var(--app-font-mono);
  }

  .ev__txt {
    min-width: 0;
  }

  .ev__l1 {
    display: flex;
    align-items: center;
    gap: var(--s-6);
  }

  .cat {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
    text-transform: capitalize;
  }

  .ev__t {
    margin: 2px 0 1px;
    font: var(--w-regular) var(--t-ui) / 1.3 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .ev__f {
    margin: 0;
    display: flex;
    align-items: baseline;
    gap: 5px;
    font: var(--w-regular) var(--t-meta) / 1.35 var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .ev__f b {
    font-weight: var(--w-medium);
    color: var(--app-accent-strong);
  }

  .ev__f b.is-bad {
    color: var(--app-warn);
  }

  .link {
    border: 0;
    padding: 0;
    background: transparent;
    font: inherit;
    color: var(--app-text-muted);
    cursor: default;
    text-decoration: underline;
    text-decoration-color: var(--app-border-hover);
    text-underline-offset: 2px;
  }

  .link:hover {
    color: var(--app-text-strong);
  }

  .mk {
    display: flex;
    align-items: baseline;
    gap: var(--s-6);
    padding: 5px 0;
  }

  .mk--stack {
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
  }

  .mk__a {
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-accent);
  }

  .mk__a.is-down {
    color: var(--app-warn);
  }

  .struck {
    text-decoration: line-through;
  }

  .sep {
    color: var(--app-text-faint);
  }
</style>
