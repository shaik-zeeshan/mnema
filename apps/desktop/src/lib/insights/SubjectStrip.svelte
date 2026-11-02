<script lang="ts" module>
  export interface StripStats {
    leadPct: number;
    leadDelta: number | null; // null = formed this fortnight
    aboveFloor: number;
    faded: number;
    evidence: number;
    lastSupport: string; // "today" | "3d ago"
  }
</script>

<script lang="ts">
  // The subject page's slim header (mockup 1-A `.n-strip`): crumb + name, the
  // four numbers inline, a small confidence chart, and Ask. With no `stats`
  // (loading, error, empty) it is just the crumb and Ask.
  import type { Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import SubjectChart from "$lib/insights/SubjectChart.svelte";
  import { tip } from "$lib/components/tooltip";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSparkle from "~icons/lucide/sparkle";

  interface Props {
    subject: string;
    onBack: () => void;
    askHref: string;
    askDisabled: boolean;
    stats?: StripStats | null;
    conclusions?: Conclusion[];
    histories?: Map<number, ConfidenceSnapshot[]>;
    selectedId?: number | null;
  }
  let { subject, onBack, askHref, askDisabled, stats = null, conclusions = [], histories, selectedId = null }: Props =
    $props();

  const signed = (d: number) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d)}`;
</script>

<div class="sd-strip">
  <div class="in-crumb sd-crumb">
    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onBack}
      ><IconBack width="13" height="13" aria-hidden="true" />Subjects</button
    >
    <span>/</span>
    <b>{subject}</b>
  </div>
  {#if stats}
    <div class="sd-stats">
      <span
        ><b>{stats.leadPct}%</b> lead
        {#if stats.leadDelta === null}<small>new</small>{:else}<small
            class:in-up={stats.leadDelta > 0}
            class:in-down={stats.leadDelta < 0}
            use:tip={"Lead confidence change over 14 days"}>{signed(stats.leadDelta)}</small
          >{/if}</span
      >
      <span use:tip={`${stats.faded} faded below the display floor`}><b>{stats.aboveFloor}</b> conclusions</span>
      <span><b>{stats.evidence}</b> evidence</span>
      <span>last support <b>{stats.lastSupport}</b></span>
    </div>
    {#if histories}
      <div class="sd-chart" use:tip={"Confidence over time: the chosen conclusion in bold, the others in grey"}>
        <SubjectChart {conclusions} {histories} {selectedId} compact />
      </div>
    {/if}
  {/if}
  <a
    class="mx-btn mx-btn--ghost mx-btn--sm sd-ask"
    href={askDisabled ? undefined : askHref}
    aria-disabled={askDisabled || undefined}
    aria-label="Ask about {subject}"
    use:tip={askDisabled ? "Ask needs AI — turn it on in Settings" : "Opens Chat with this question"}
    ><IconSparkle class="mx-btn__accent" width="13" height="13" aria-hidden="true" /><span class="sd-ask__l"
      >Ask about {subject}</span
    ><kbd>⌘J</kbd></a
  >
</div>

<style>
  .sd-strip {
    flex: none; display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto;
    grid-template-areas: "crumb stats chart ask"; align-items: center; gap: 6px var(--s-5); min-height: 44px; padding-bottom: 12px;
  }
  .in-crumb { display: flex; align-items: center; gap: var(--s-2); font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle); }
  .sd-crumb { grid-area: crumb; margin: 0 0 0 -9px; min-width: 0; }
  .sd-crumb b {
    font: 600 18px / 1.2 var(--font-sans); letter-spacing: -0.01em; color: var(--app-text-strong);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 220px;
  }
  .sd-stats { grid-area: stats; display: flex; flex-wrap: wrap; gap: 4px 16px; min-width: 0; font-size: var(--text-sm); color: var(--app-text-subtle); }
  .sd-stats span { white-space: nowrap; }
  .sd-stats b { font: 600 var(--text-md) / 1.2 var(--font-mono); font-variant-numeric: tabular-nums; color: var(--app-text-strong); }
  .sd-stats small { font: 500 var(--text-xs) / 1 var(--font-mono); }
  .in-up { color: var(--focus-deep); }
  .in-down { color: var(--focus-distracted); }
  .sd-chart { grid-area: chart; width: 168px; }
  .sd-ask { grid-area: ask; justify-self: end; margin-right: -9px; }
  .sd-ask kbd { font: 500 10px / 1 var(--font-mono); color: var(--app-text-faint); }

  @container sd (max-width: 920px) {
    .sd-chart { width: 130px; }
    .sd-ask__l { display: none; }
  }
  @container sd (max-width: 620px) {
    .sd-strip { grid-template-columns: minmax(0, 1fr) auto auto; grid-template-areas: "crumb chart ask" "stats stats stats"; }
    .sd-chart { width: 110px; }
  }
  @container sd (max-width: 380px) {
    .sd-chart { width: 60px; }
    .sd-ask kbd { display: none; }
  }
</style>
