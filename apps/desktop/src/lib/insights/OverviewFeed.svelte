<script lang="ts">
  import { formatClock } from "$lib/time-format";
  // Overview's feed (insights.html `.in-feed`): "What changed" in the dossier
  // this period — conclusions formed, strengthened or started fading — beside
  // "Needs attention", the uncategorized activities one tap can tag. Standing
  // beliefs without a change live on Subjects (pin / dismiss live there too).
  import type { Activity, ActivityCategory, Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import { loadSubjectHistories } from "$lib/insights/subject-histories";
  import { CATEGORY_COLOR, CATEGORY_ORDER, categoryLabel } from "$lib/insights/activity-helpers";
  import { fmtDuration } from "$lib/insights/lede-stats";
  import IconInbox from "~icons/lucide/inbox";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";

  interface Props {
    unit: "day" | "week" | "month";
    scope: string;
    startMs: number;
    endMs: number;
    conclusions: Conclusion[];
    activities: Activity[];
    loading: boolean;
    error: string | null;
    correcting: Set<number>;
    onRetry: () => void;
    onCorrectCategory: (a: Activity, c: ActivityCategory) => void;
    onOpenSubject: (subject: string) => void;
    onOpenSubjects: () => void;
  }
  let {
    unit, scope, startMs, endMs, conclusions, activities, loading, error, correcting,
    onRetry, onCorrectCategory, onOpenSubject, onOpenSubjects,
  }: Props = $props();

  type Kind = "formed" | "strengthened" | "fading";
  const CAP = 10;
  const ATTN_CAP = 5;

  // Each live conclusion lands in at most one group; pinned first, then confidence.
  const deltas = $derived.by(() => {
    const inRange = (ms: number) => ms >= startMs && ms < endMs;
    const out: Record<Kind, Conclusion[]> = { formed: [], strengthened: [], fading: [] };
    for (const c of conclusions) {
      if (c.status === "dismissed") continue;
      if (inRange(c.formedAtMs)) out.formed.push(c);
      else if (c.status === "visible" && inRange(c.lastSupportedAtMs)) out.strengthened.push(c);
      // `updatedAtMs` is the best available proxy for when the fade happened.
      else if (c.status === "faded" && inRange(c.updatedAtMs)) out.fading.push(c);
    }
    const order = (a: Conclusion, b: Conclusion) => Number(b.pinned) - Number(a.pinned) || b.confidence - a.confidence;
    return (["formed", "strengthened", "fading"] as Kind[]).map((kind) => ({ kind, rows: out[kind].sort(order) }));
  });
  const total = $derived(deltas.reduce((t, g) => t + g.rows.length, 0));
  // One cap across all groups; group heads keep the full counts.
  const shown = $derived.by(() => {
    let left = CAP;
    return deltas
      .map((g) => {
        const rows = g.rows.slice(0, Math.max(0, left));
        left -= rows.length;
        return { ...g, all: g.rows.length, rows };
      })
      .filter((g) => g.rows.length);
  });

  // Each shown row's trend line (same history Subjects draws), re-read whenever
  // the rows change; the generation token drops a load a newer one overtook.
  let histories = $state(new Map<string, Map<number, ConfidenceSnapshot[]>>());
  let historiesGen = 0;
  $effect(() => {
    const gen = ++historiesGen;
    void loadSubjectHistories(shown.flatMap((g) => g.rows.map((c) => c.subject))).then((next) => {
      if (gen === historiesGen) histories = next;
    });
  });
  // A polyline needs two points: flatten a missing/one-point history.
  function sparkPoints(c: Conclusion): number[] {
    const p = histories.get(c.subject)?.get(c.id)?.map((s) => s.confidence);
    return p && p.length >= 2 ? p : [p?.[0] ?? c.confidence, p?.[0] ?? c.confidence];
  }

  const attention = $derived(
    activities.filter((a) => a.category == null).sort((a, b) => b.startedAtMs - a.startedAtMs),
  );
  let attnOpen = $state(false);

  const day = (ms: number) =>
    new Date(ms).toLocaleDateString("en-US", unit === "month" ? { month: "short", day: "numeric" } : { weekday: "short" });
  const when = (a: Activity) =>
    `${new Date(a.startedAtMs).toLocaleDateString("en-US", { weekday: "short" })} ${formatClock(a.startedAtMs)} · ${fmtDuration(a.endedAtMs - a.startedAtMs)}`;
  const HEAD = $derived<Record<Kind, string>>({ formed: `Formed this ${unit}`, strengthened: "Strengthened", fading: "Started fading" });
</script>

<div class="feed">
  <div class="mx-reveal" style="--i:6">
    <div class="sec"><h2 class="mx-h2">What changed</h2><span class="mx-label">in your dossier {scope}</span></div>
    {#if loading}
      <div aria-busy="true">
        {#each Array.from({ length: 4 }) as _, i (i)}
          <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
        {/each}
      </div>
    {:else if error}
      <div class="mx-empty" data-tone="danger">
        <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">Couldn't load your engine data.</h4>
        <p class="mx-empty__text">{error} Nothing is lost.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={onRetry}
            ><IconRetry width="13" height="13" aria-hidden="true" />Try again</button
          >
        </div>
      </div>
    {:else if !total}
      <div class="mx-empty">
        <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">Nothing changed {scope}.</h4>
        <p class="mx-empty__text">
          No conclusions formed, strengthened or faded in this range. Your standing dossier lives on Subjects.
        </p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={onOpenSubjects}>Open Subjects</button>
        </div>
      </div>
    {:else}
      {#each shown as g (g.kind)}
        <p class="group">{HEAD[g.kind]} · {g.all}</p>
        {#each g.rows as c (c.id)}
          <button type="button" class="concl" class:faded={g.kind === "fading"} onclick={() => onOpenSubject(c.subject)}>
            <span class="body">
              <span class="stmt">{c.statement}</span>
              <span class="meta"
                ><span class="mx-chip">{c.subject}</span>{#if g.kind === "formed"}<span class="new">● new</span>{/if}{#if g.kind === "fading"}below
                  the display floor — kept for history{/if}</span
              >
            </span>
            <span class="spark">
              <Sparkline
                series={[{ colorVar: "--app-text-strong", faded: g.kind === "fading", points: sparkPoints(c) }]}
                label={`${c.subject} — confidence over time`}
              />
            </span>
            <span class="num pct"
              >{Math.round(c.confidence * 100)}%<small class:up={g.kind !== "fading"} class:down={g.kind === "fading"}
                >{g.kind === "formed"
                  ? `formed ${day(c.formedAtMs)}`
                  : g.kind === "strengthened"
                    ? `supported ${day(c.lastSupportedAtMs)}`
                    : "fading"}</small
              ></span
            >
          </button>
        {/each}
      {/each}
      {#if total > CAP}
        <p class="more">
          <span class="mx-label num">showing {CAP} of {total}</span>
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onOpenSubjects}>View all in Subjects</button>
        </p>
      {/if}
    {/if}
  </div>

  <div class="mx-panel attn mx-reveal" style="--i:7">
    <div class="sec">
      <span class="mx-kicker">Needs attention</span><span class="mx-spacer"></span>
      <span class="mx-label num">{!loading && !error && attention.length ? `${attention.length} untagged` : ""}</span>
    </div>
    {#if loading}
      <div aria-busy="true">
        {#each Array.from({ length: 3 }) as _, i (i)}
          <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
        {/each}
      </div>
    {:else if error}
      <p>
        <span class="mx-inline" data-tone="danger"
          ><IconAlert width="13" height="13" aria-hidden="true" />Couldn't load.<button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            onclick={onRetry}>Retry</button
          ></span
        >
      </p>
    {:else if !attention.length}
      <div class="mx-empty mx-empty--compact" data-tone="ok">
        <span class="mx-empty__glyph"><IconCheck width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">All caught up</b>
        <p class="mx-empty__text">Every activity in this range has a category.</p>
      </div>
    {:else}
      <p class="mx-prose lead">Mnema couldn't place these. One tap teaches it.</p>
      {#each attnOpen ? attention : attention.slice(0, ATTN_CAP) as a (a.id)}
        <div class="row" aria-busy={correcting.has(a.id) || undefined}>
          <div><b>{a.title}</b><div class="mx-label num">{when(a)}</div></div>
          <div class="pick">
            {#each CATEGORY_ORDER as k (k)}
              <button
                type="button"
                class="mx-btn mx-btn--sm"
                aria-pressed="false"
                disabled={correcting.has(a.id)}
                onclick={() => onCorrectCategory(a, k)}
                ><span class="sw" style="--c:var({CATEGORY_COLOR[k]})"></span>{categoryLabel(k)}</button
              >
            {/each}
          </div>
        </div>
      {/each}
      {#if attention.length > ATTN_CAP}
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => (attnOpen = !attnOpen)}>
          {attnOpen ? "Show fewer" : `+${attention.length - ATTN_CAP} more`}
        </button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .feed {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr);
    gap: var(--s-6);
    align-items: start;
  }
  .sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin: 0 0 var(--s-3);
  }
  .sec .mx-spacer {
    flex: 1;
  }
  .group {
    margin: var(--s-4) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .group::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--mx-hairline);
  }
  .concl {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 96px 96px;
    gap: var(--s-4);
    align-items: center;
    width: 100%;
    padding: 12px 0;
    border: 0;
    border-bottom: 1px solid var(--mx-hairline);
    background: none;
    text-align: left;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .concl:last-of-type {
    border-bottom: 0;
  }
  .concl:focus-visible {
    outline: 2px solid var(--app-accent);
    outline-offset: 2px;
  }
  .body {
    display: grid;
    gap: 6px;
    min-width: 0;
  }
  .stmt {
    color: var(--app-text-strong);
    font-size: var(--text-md);
    line-height: 1.4;
    transition: color var(--t-fast);
  }
  .concl:hover .stmt {
    color: var(--app-accent);
  }
  .faded .stmt {
    color: var(--app-text-muted);
  }
  .meta {
    display: flex;
    gap: var(--s-2);
    align-items: center;
    font: 400 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .new {
    color: var(--app-accent);
  }
  .spark {
    display: block;
    width: 96px;
    height: 28px;
  }
  .spark :global(svg) {
    width: 100%;
    height: 100%;
  }
  .pct {
    font-size: var(--text-md);
    color: var(--app-text-strong);
    text-align: right;
  }
  .pct small {
    display: block;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    margin-top: 3px;
  }
  .pct small.up {
    color: var(--focus-deep);
  }
  .pct small.down {
    color: var(--focus-distracted);
  }
  .more {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    margin: var(--s-3) 0 0;
  }
  .attn {
    padding: var(--s-4);
  }
  .attn .mx-empty {
    max-width: none;
    padding-inline: 0;
  }
  .lead {
    font-size: var(--text-base);
    margin-bottom: var(--s-2);
  }
  .row {
    padding: 12px 0;
    border-top: 1px solid var(--mx-hairline);
    display: grid;
    gap: 8px;
  }
  .lead + .row {
    border-top: 0;
  }
  .row b {
    font-weight: 500;
    color: var(--app-text-strong);
  }
  .row .mx-label {
    font-size: var(--text-xs);
  }
  .row[aria-busy] {
    opacity: 0.5;
  }
  .pick {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .sw {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
  }
  @container mx-window (max-width: 1180px) {
    .feed {
      grid-template-columns: 1fr;
    }
  }
</style>
