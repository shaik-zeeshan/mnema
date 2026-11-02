<script lang="ts">
  // The subject page's top (mockup r2-B): crumb + Ask, the big title with the
  // lead conclusion as its lede, the four large stats, and the full-width
  // "Confidence over time" chart, whose bold line is the open conclusion (the
  // lead when the drawer is closed). Pure view over the page's data.
  import type { Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import SubjectChart from "$lib/insights/SubjectChart.svelte";
  import { deriveTrend } from "$lib/insights/subjectsTiers";
  import { relativeTime } from "$lib/insights/conversationStore.svelte";
  import { tip } from "$lib/components/tooltip";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSparkle from "~icons/lucide/sparkle";

  interface Props {
    subject: string;
    onBack: () => void;
    askHref: string;
    askDisabled: boolean;
    /** null while loading / on error: only the crumb shows. */
    conclusions: Conclusion[] | null;
    histories: Map<number, ConfidenceSnapshot[]>;
    /** The open conclusion, if any. */
    openId: number | null;
    /** Points moved over the fortnight; null = formed inside it. */
    delta: (c: Conclusion) => number | null;
  }
  let { subject, onBack, askHref, askDisabled, conclusions, histories, openId, delta }: Props = $props();

  const pct = (v: number) => Math.round(v * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const signed = (d: number) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d)}`;

  const shown = $derived(conclusions ?? []);
  const lead = $derived(shown[0] ?? null);
  const leadDelta = $derived(lead ? delta(lead) : null);
  const sinceMs = $derived(Math.min(...shown.map((c) => c.formedAtMs)));
  const trendWord = $derived.by(() => {
    if (shown.every((c) => c.status === "faded")) return "fading";
    const points = new Map([...histories].map(([id, h]) => [id, h.map((s) => s.confidence)]));
    return { up: "warming", steady: "steady", down: "cooling" }[deriveTrend(shown, points)];
  });
  const aboveFloor = $derived(shown.filter((c) => c.status !== "faded").length);
  const evidenceCount = $derived(new Set(shown.flatMap((c) => c.evidence.map((e) => e.activityId))).size);
  const lastSupportMs = $derived(Math.max(...shown.map((c) => c.lastSupportedAtMs)));
  const lastSupport = $derived.by(() => {
    const r = relativeTime(lastSupportMs);
    const m = /^(\d+)(\D+)$/.exec(r);
    return m ? { n: m[1], unit: `${m[2]} ago` } : { n: r, unit: "" };
  });
  const lastSupportDay = $derived(
    new Date(lastSupportMs).toDateString() === new Date().toDateString() ? "today" : day(lastSupportMs),
  );
  const chartId = $derived(openId ?? lead?.id ?? null);
</script>

<div class="in-crumb mx-reveal">
  <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onBack}
    ><IconBack width="13" height="13" aria-hidden="true" />Subjects</button
  >
  <span>/</span>
  <b>{subject}</b>
  <span class="mx-spacer"></span>
  <a
    class="mx-btn mx-btn--ghost mx-btn--sm"
    href={askDisabled ? undefined : askHref}
    aria-disabled={askDisabled || undefined}
    use:tip={askDisabled ? "Ask needs AI — turn it on in Settings" : "Opens Chat with this question"}
    ><IconSparkle class="mx-btn__accent" width="13" height="13" aria-hidden="true" /><span class="in-crumb__ask">Ask about {subject}</span><kbd>⌘J</kbd></a
  >
</div>

{#if lead}
  <div class="in-read">
    <div class="mx-reveal" style="--i:1">
      <span class="mx-kicker">Subject · since {day(sinceMs)} · {trendWord}</span>
      <h1 class="mx-display">{subject}</h1>
      <p class="mx-prose lede">
        {lead.statement} <span>— its strongest conclusion, at {pct(lead.confidence)}%.</span>
      </p>
    </div>
    <div class="in-lede-stats mx-reveal" style="--i:2">
      <div class="mx-stat">
        <span class="mx-stat__label">Lead confidence</span>
        <span class="mx-stat__value">{pct(lead.confidence)}<small>%</small></span>
        <span class="mx-stat__delta" class:mx-stat__delta--up={(leadDelta ?? 0) > 0}
          >{leadDelta === null ? "formed this fortnight" : `${signed(leadDelta)} pts in 14d`}</span
        >
      </div>
      <div class="mx-stat">
        <span class="mx-stat__label">Conclusions</span>
        <span class="mx-stat__value">{aboveFloor}</span>
        <span class="mx-stat__delta">{shown.length - aboveFloor} faded below the floor</span>
      </div>
      <div class="mx-stat">
        <span class="mx-stat__label">Evidence</span>
        <span class="mx-stat__value">{evidenceCount}</span>
        <span class="mx-stat__delta">{evidenceCount === 1 ? "activity" : "activities"} cited</span>
      </div>
      <div class="mx-stat">
        <span class="mx-stat__label">Last support</span>
        <span class="mx-stat__value">{lastSupport.n}{#if lastSupport.unit}<small>{lastSupport.unit}</small>{/if}</span>
        <span class="mx-stat__delta">{lastSupportDay}</span>
      </div>
    </div>
  </div>

  <div class="mx-panel in-week mx-reveal" style="--i:3">
    <div class="in-sec">
      <span class="mx-kicker">Confidence over time</span><span class="mx-spacer"></span>
      <span class="mx-label"
        >{openId === null ? "lead" : "open"} conclusion in bold · {shown.length > 11 ? "top 10" : "others"} in grey · ● evidence</span
      >
    </div>
    <SubjectChart conclusions={shown} {histories} selectedId={chartId} />
  </div>
{/if}

<style>
  .in-crumb { display: flex; align-items: center; gap: var(--s-2); margin: 0 0 var(--s-3) -9px; font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle); }
  .in-crumb b { color: var(--app-text-strong); font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .in-crumb > a:last-child { margin-right: -9px; }
  .in-read {
    display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s-7); align-items: end;
    padding: var(--s-4) 0 var(--s-6); margin-bottom: var(--s-6); border-bottom: 1px solid var(--mx-hairline);
  }
  .in-read .mx-display { margin: var(--s-4) 0; max-width: 24ch; }
  .lede { font-size: var(--text-lg); max-width: 60ch; }
  .lede span { color: var(--app-text-subtle); }
  .in-lede-stats { display: grid; grid-template-columns: repeat(2, minmax(132px, auto)); gap: var(--s-5) var(--s-6); padding-bottom: 4px; }
  .in-week { padding: var(--s-5); margin-bottom: var(--s-6); }
  .in-sec { display: flex; align-items: baseline; gap: var(--s-3); margin: 0 0 var(--s-3); }
  .in-sec .mx-label { font-size: var(--text-sm); text-align: right; }

  /* `sd` = the page column, which narrows while the drawer is open. */
  @container sd (max-width: 760px) {
    .in-read { grid-template-columns: minmax(0, 1fr); gap: var(--s-5); }
    .in-lede-stats { grid-template-columns: repeat(4, minmax(0, 1fr)); gap: var(--s-4); }
    .in-lede-stats .mx-stat__value { font-size: 24px; }
  }
  @container sd (max-width: 560px) {
    .in-read .mx-display { font-size: 32px; }
    .in-lede-stats { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .in-week { padding: var(--s-4); }
    .in-sec .mx-label { display: none; }
    .in-crumb__ask { display: none; }
  }
</style>
