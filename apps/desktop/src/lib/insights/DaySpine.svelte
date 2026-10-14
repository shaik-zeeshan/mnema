<script lang="ts">
  // DaySpine — the Journal's day spine (insights.html `.in-spine`): an activity
  // lane (overlaps stacked into sub-lanes), a focus lane and a now-line over
  // the day's real span. Hovering a block lights its card (and vice versa via
  // `hot`); clicking scrolls to it. On a day nothing was written up it draws
  // grey capture runs labelled by app instead — no categories, no focus.
  // Geometry lives in journal-view.ts (spineAxis, stackLanes).
  import type { ActivityFocus } from "$lib/types/recording";
  import type { JournalCardSlot, Span } from "$lib/insights/journal-day";
  import { CATEGORY_COLOR, UNCATEGORIZED_COLOR, categoryLabel, humanizeMs } from "$lib/insights/activity-helpers";
  import { awayLabel, spineAxis, stackLanes } from "$lib/insights/journal-view";
  import { formatClock, formatClockRange } from "$lib/time-format";
  import { tip } from "$lib/components/tooltip";

  interface Props {
    slots: JournalCardSlot[];
    gaps: Span[];
    /** Capture-run day: grey runs (with their top apps) replace activities. */
    runs?: { span: Span; apps: string[] }[] | null;
    /** The live edge still being written up (or paused). */
    pending?: { sinceMs: number; paused: boolean } | null;
    /** Today only: draws the now-line and ends the span at now. */
    nowMs?: number | null;
    hot?: number | null;
    onHot?: (id: number | null) => void;
    onPick?: (id: number) => void;
  }
  let { slots, gaps, runs = null, pending = null, nowMs = null, hot = null, onHot, onPick }: Props = $props();

  const FOCUS: Record<ActivityFocus, [string, string]> = {
    deep: ["--focus-deep", "Deep"],
    mixed: ["--focus-mid", "Mixed"],
    distracted: ["--focus-distracted", "Scattered"],
  };
  const cat = (c: JournalCardSlot["activity"]["category"]) => `var(${c ? CATEGORY_COLOR[c] : UNCATEGORIZED_COLOR})`;

  const spans = $derived(slots.map((s) => ({ startMs: s.activity.startedAtMs, endMs: s.activity.endedAtMs })));
  const all = $derived([
    ...spans,
    ...(runs ?? []).map((r) => r.span),
    ...gaps,
    ...(pending ? [{ startMs: pending.sinceMs, endMs: pending.sinceMs }] : []),
  ]);
  const first = $derived(Math.min(...all.map((s) => s.startMs)));
  const last = $derived(Math.max(...all.map((s) => s.endMs), nowMs ?? -Infinity));
  const axis = $derived(spineAxis(first, last));
  const pct = (ms: number) => ((ms - axis.startMs) / (axis.endMs - axis.startMs)) * 100;
  const w = (s: Span) => pct(s.endMs) - pct(s.startMs);
  const stack = $derived(stackLanes(spans));
  // Capture-run days cut only at gaps: the gaps are what lies between runs.
  const awayGaps = $derived(
    runs ? runs.slice(1).map((r, i) => ({ startMs: runs![i].span.endMs, endMs: r.span.startMs })) : gaps,
  );
</script>

<div class="mx-panel in-spine">
  <div class="in-sec">
    <span class="mx-kicker">Day spine</span>
    <span class="mx-label num">{nowMs != null ? `${formatClock(first)} – now` : formatClockRange(first, last, { sep: " – " })} captured</span>
    <span class="in-spacer"></span>
    <span class="in-sp__legend">
      {#if runs}
        <span><span class="in-sw" style="--c: var(--chart-grey-3)"></span>capture runs</span>
        <span>no activities · categories and focus need AI</span>
      {:else}
        <span>focus strip</span>
        {#each Object.values(FOCUS) as [token, label] (token)}
          <span><span class="in-sw" style="--c: var({token})"></span>{label.toLowerCase()}</span>
        {/each}
      {/if}
    </span>
  </div>
  <div class="in-sp" class:in-sp--hot={hot != null}>
    <div class="in-sp__grid" aria-hidden="true">
      {#each axis.ticks as t (t)}<span style="left:{pct(t)}%"><b>{formatClock(t)}</b></span>{/each}
    </div>
    <div class="in-sp__lane" class:in-sp__lane--multi={stack.lanes > 1} style="--lanes:{runs ? 1 : stack.lanes}">
      {#each awayGaps as g (g.startMs)}
        <span
          class="in-sp__gap"
          style="left:{pct(g.startMs)}%;width:{w(g)}%"
          use:tip={`${awayLabel(g)} · ${formatClockRange(g.startMs, g.endMs)} · no capture`}>{w(g) > 4 ? "away" : ""}</span
        >
      {/each}
      {#if runs}
        {#each runs as r, i (r.span.startMs)}
          <span
            class="in-sp__run"
            style="left:{pct(r.span.startMs)}%;width:calc({w(r.span)}% - 2px);--d:{i}"
            use:tip={`Captured · ${humanizeMs(r.span.endMs - r.span.startMs)} · ${formatClockRange(r.span.startMs, r.span.endMs)}${r.apps.length ? ` · ${r.apps.join(", ")}` : ""}`}
          >
            {#if w(r.span) > 6}<b>{r.apps.join(" · ") || "Captured"}</b><small>{humanizeMs(r.span.endMs - r.span.startMs)} captured</small>{/if}
          </span>
        {/each}
      {:else}
        {#each slots as s, i (s.activity.id)}
          {@const a = s.activity}
          <button
            type="button"
            class="in-sp__a"
            data-hot={hot === a.id ? "" : undefined}
            style="left:{pct(a.startedAtMs)}%;width:calc({w(spans[i])}% - 2px);--l:{stack.lane[i]};--c:{cat(a.category)};--d:{i}"
            use:tip={`${a.title} · ${formatClockRange(a.startedAtMs, a.endedAtMs)} · ${humanizeMs(a.endedAtMs - a.startedAtMs)}${a.category ? ` · ${categoryLabel(a.category)}` : ""}`}
            onmouseenter={() => onHot?.(a.id)}
            onmouseleave={() => onHot?.(null)}
            onclick={() => onPick?.(a.id)}
          >
            {#if w(spans[i]) > 6}<b>{a.title}</b>{#if stack.lanes === 1}<small>{humanizeMs(a.endedAtMs - a.startedAtMs)}</small>{/if}{/if}
          </button>
        {/each}
      {/if}
      {#if pending && nowMs != null && nowMs > pending.sinceMs}
        {@const pw = pct(nowMs) - pct(pending.sinceMs)}
        <span
          class="in-sp__pend"
          style="left:{pct(pending.sinceMs)}%;width:{pw}%"
          use:tip={`${pending.paused ? "Paused" : "Writing up"} · ${formatClock(pending.sinceMs)} → now`}
        >{pw > 6 ? (pending.paused ? "paused" : "writing up") : ""}</span>
      {/if}
    </div>
    {#if !runs}
      <div class="in-sp__focus">
        {#each slots as s, i (s.activity.id)}
          {#if s.activity.focus}
            {@const f = FOCUS[s.activity.focus]}
            <i style="left:{pct(s.activity.startedAtMs)}%;width:calc({w(spans[i])}% - 2px);--c:var({f[0]})" use:tip={`${f[1]} · ${formatClockRange(s.activity.startedAtMs, s.activity.endedAtMs)}`}></i>
          {/if}
        {/each}
      </div>
    {/if}
    {#if nowMs != null}<span class="in-sp__now" style="left:{pct(nowMs)}%"></span>{/if}
  </div>
</div>

<style>
  /* insights.html `.in-spine` / `.in-sp*` (kit tokens only). */
  .in-spine { padding: var(--s-5); margin-bottom: var(--s-7); }
  .in-sec { display: flex; align-items: baseline; gap: var(--s-3); }
  .in-spacer { flex: 1; }
  .in-sw { display: inline-block; width: 8px; height: 8px; border-radius: 2px; background: var(--c); flex: none; }
  .in-sp__legend { display: flex; gap: var(--s-4); font: 500 var(--text-sm)/1 var(--font-sans); color: var(--app-text-subtle); }
  .in-sp__legend > span { display: inline-flex; align-items: center; gap: 6px; }
  .in-sp { position: relative; margin-top: var(--s-5); padding-bottom: 22px; }
  .in-sp__grid { position: absolute; inset: -6px 0 0; pointer-events: none; }
  .in-sp__grid span { position: absolute; top: 0; bottom: 18px; border-left: 1px solid var(--mx-hairline); }
  .in-sp__grid b { position: absolute; bottom: -18px; left: 0; translate: -50% 0; font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-faint); white-space: nowrap; }
  .in-sp__lane { position: relative; --lh: 64px; height: calc(var(--lanes, 1) * var(--lh) + (var(--lanes, 1) - 1) * 4px); }
  .in-sp__lane--multi { --lh: 40px; }
  .in-sp__a, .in-sp__run { position: absolute; height: var(--lh); display: grid; align-content: start; gap: 4px; padding: 9px 9px 0; overflow: hidden; text-align: left; border-radius: 3px 3px var(--r-xs) var(--r-xs); transform-origin: left; animation: in-grow-x var(--t-slow) var(--ease-expo) both; animation-delay: calc(80ms + var(--d) * 35ms); }
  .in-sp__a { top: calc(var(--l, 0) * (var(--lh) + 4px)); cursor: pointer; border: 0; border-top: 2px solid var(--c); background: color-mix(in srgb, var(--c) 16%, var(--app-surface)); font: inherit; color: var(--app-text-strong); transition: background-color var(--t-fast) var(--ease-quart), opacity var(--t-fast) var(--ease-quart); }
  .in-sp__a:focus-visible { outline: 0; box-shadow: var(--app-ring); }
  .in-sp__a b { font: 600 var(--text-sm)/1.25 var(--font-sans); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .in-sp__run { top: 0; border-top: 2px solid var(--chart-grey-3); background: color-mix(in srgb, var(--app-fg) 5%, var(--app-surface)); color: var(--app-text); }
  .in-sp__run b { font: 500 var(--text-sm)/1.25 var(--font-sans); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .in-sp__a small, .in-sp__run small { font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); white-space: nowrap; }
  .in-sp--hot .in-sp__a:not([data-hot]) { opacity: 0.4; }
  .in-sp__a[data-hot] { background: color-mix(in srgb, var(--c) 28%, var(--app-surface)); }
  .in-sp__gap { position: absolute; top: 10px; bottom: 10px; display: grid; place-items: center; border: 1px dashed var(--app-border); border-radius: var(--r-sm); font: 500 var(--text-sm)/1 var(--font-sans); color: var(--app-text-faint); overflow: hidden; white-space: nowrap; }
  .in-sp__pend { position: absolute; top: 0; bottom: 0; display: grid; place-items: center; border-radius: var(--r-xs); border: 1px dashed var(--app-accent-border); background: repeating-linear-gradient(135deg, color-mix(in srgb, var(--app-accent) 12%, transparent) 0 1.5px, transparent 1.5px 6px); font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-accent); white-space: nowrap; overflow: hidden; }
  .in-sp__focus { position: relative; height: 6px; margin-top: 6px; border-radius: 3px; background: color-mix(in srgb, var(--app-fg) 4%, transparent); }
  .in-sp__focus i { position: absolute; top: 0; bottom: 0; border-radius: 3px; background: var(--c); animation: in-fade var(--t-slow) var(--ease-quart) both; animation-delay: 300ms; }
  .in-sp__now { position: absolute; top: -10px; bottom: 16px; width: 2px; margin-left: -1px; background: var(--app-accent); border-radius: 1px; box-shadow: 0 0 0 3px var(--app-accent-glow); pointer-events: none; }
  .in-sp__now::before { content: "now"; position: absolute; white-space: nowrap; top: -14px; left: 50%; translate: -50% 0; font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-accent); }
  @keyframes in-grow-x { from { transform: scaleX(0); } }
  @keyframes in-fade { from { opacity: 0; } }
  @media (prefers-reduced-motion: reduce) { .in-sp__a, .in-sp__run, .in-sp__focus i { animation: none; } }
</style>
