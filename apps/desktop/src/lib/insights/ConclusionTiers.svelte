<script lang="ts" module>
  import type { Conclusion } from "$lib/types/recording";
  import type { Page } from "$lib/insights/subjectsTiers";

  /** One tier section as the page pages it. */
  export interface TierView {
    id: string;
    title: string;
    note: string;
    slice: Page<Conclusion>;
    size: number;
  }
</script>

<script lang="ts">
  // The subject page's conclusions (mockup r2-B): full-width rows grouped by
  // tier, each tier paging on its own with the pager in its kicker heading:
  //   // FORMING · 9  confidence 30–68%  ─────────  6–9 OF 9 ‹ ›
  // The open conclusion's row is shaded with a dark left edge. Pure view: the
  // page owns paging, the open row and the keys.
  import type { ConfidenceSnapshot } from "$lib/types/recording";
  import Pager from "$lib/insights/Pager.svelte";
  import IconChevron from "~icons/lucide/chevron-right";

  interface Props {
    total: number;
    tiers: TierView[];
    openId: number | null;
    histories: Map<number, ConfidenceSnapshot[]>;
    delta: (c: Conclusion) => number | null;
    onOpen: (id: number) => void;
    onPage: (tierId: string, page: number) => void;
    onSize: (tierId: string, size: number) => void;
  }
  let { total, tiers, openId, histories, delta, onOpen, onPage, onSize }: Props = $props();

  const pct = (v: number) => Math.round(v * 100);
  const signed = (d: number) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d)}`;
  // Sparkline points on a 100×24 box; a conclusion without history is flat.
  function spark(c: Conclusion): string {
    const h = histories.get(c.id) ?? [];
    const ys = h.length > 1 ? h.map((s) => s.confidence) : [c.confidence, c.confidence];
    return ys.map((v, i) => `${((i / (ys.length - 1)) * 100).toFixed(1)},${(2 + (1 - v) * 20).toFixed(1)}`).join(" ");
  }
</script>

<div class="in-sec">
  <span class="mx-kicker">Conclusions</span><span class="mx-spacer"></span>
  <span class="mx-label">{total} · by confidence</span>
</div>

{#each tiers as t (t.id)}
  {@const s = t.slice}
  <section class="sd-tier" class:sd-tier--faded={t.id === "fading"} data-sec="t:{t.id}" aria-label={t.title}>
    <div class="sd-h">
      <span class="mx-kicker">{t.title} · {s.total}<small>{t.note}</small></span>
      <i class="sd-h__rule"></i>
      <Pager
        page={s.page}
        pages={s.pages}
        from={s.from}
        to={s.to}
        total={s.total}
        size={t.size}
        onPage={(p) => onPage(t.id, p)}
        onSize={(n) => onSize(t.id, n)}
        label={t.title}
      />
    </div>
    {#key s.page}
      <div class="sd-rows">
        {#each s.visible as c (c.id)}
          {@const d = delta(c)}
          {@const on = c.id === openId}
          <button type="button" class="sd-row" class:is-open={on} data-cid={c.id} aria-expanded={on} onclick={() => onOpen(c.id)}>
            <span class="num">{pct(c.confidence)}%</span>
            <span class="sd-row__t">{c.statement}</span>
            <span class="sd-row__spark" aria-hidden="true">
              <svg viewBox="0 0 100 24" preserveAspectRatio="none"
                ><polyline points={spark(c)} fill="none" stroke="currentColor" stroke-width="1.5" vector-effect="non-scaling-stroke" /></svg
              >
            </span>
            <span class="sd-row__d" class:in-new={d === null} class:in-up={(d ?? 0) > 0} class:in-down={(d ?? 0) < 0}
              >{d === null ? "new" : signed(d)}</span
            >
            <span class="sd-row__n">{c.evidence.length} ev</span>
            <IconChevron class="sd-row__arrow" width="14" height="14" aria-hidden="true" />
          </button>
        {/each}
      </div>
    {/key}
  </section>
{/each}

<style>
  .in-sec { display: flex; align-items: baseline; gap: var(--s-3); margin: 0 0 var(--s-2); }
  .in-sec .mx-label { font-size: var(--text-sm); }
  /* The heading row carries kicker type so the Pager inside inherits it. */
  .sd-h {
    display: flex; align-items: center; gap: var(--s-3); min-height: 24px; margin: var(--s-5) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono); letter-spacing: 0.08em; text-transform: uppercase; color: var(--app-text-subtle);
  }
  .sd-h .mx-kicker { white-space: nowrap; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .sd-h .mx-kicker small { margin-left: 6px; font: 400 var(--text-xs) / 1 var(--font-sans); letter-spacing: 0; text-transform: none; color: var(--app-text-faint); }
  .sd-h__rule { flex: 1; min-width: 12px; height: 1px; background: var(--mx-hairline); }
  .sd-rows { animation: sd-fade var(--t-med) var(--ease-quart); }
  @keyframes sd-fade { from { opacity: 0.3; } }

  .sd-row {
    width: 100%; display: grid; grid-template-columns: 44px minmax(0, 1fr) 132px 40px 52px 14px; gap: var(--s-4); align-items: center;
    padding: 11px var(--s-3); border: 0; border-bottom: 1px solid var(--mx-hairline); background: none; text-align: left; cursor: pointer;
    color: var(--app-text); font: 400 var(--text-base) / 1.45 var(--font-sans);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .sd-row:hover { background: var(--mx-wash); }
  .sd-row:focus-visible { outline: 2px solid var(--app-accent); outline-offset: -2px; }
  .sd-row .num { font-size: var(--text-md); color: var(--app-text-strong); }
  .sd-row__spark { color: var(--chart-grey-4); }
  .sd-row__spark svg { display: block; width: 100%; height: 24px; }
  .sd-row__d, .sd-row__n { text-align: right; font: 500 var(--text-xs) / 1 var(--font-mono); }
  .sd-row__n { color: var(--app-text-subtle); }
  .sd-row :global(.sd-row__arrow) { color: var(--app-text-faint); transition: transform var(--t-med) var(--ease-expo); }
  .sd-row:hover :global(.sd-row__arrow) { transform: translateX(2px); color: var(--app-text-strong); }
  .sd-tier--faded .sd-row:not(.is-open) { opacity: 0.62; }
  .sd-row.is-open { background: var(--mx-selected); color: var(--app-text-strong); box-shadow: inset 2px 0 0 var(--app-text-strong); }
  .sd-row.is-open .sd-row__spark, .sd-row.is-open :global(.sd-row__arrow) { color: var(--app-text-strong); }
  .in-up { color: var(--focus-deep); }
  .in-down { color: var(--focus-distracted); }
  .in-new { color: var(--app-accent); }

  @container sd (max-width: 760px) {
    .sd-row { grid-template-columns: 40px minmax(0, 1fr) 72px 36px 14px; gap: var(--s-3); }
    .sd-row__n { display: none; }
  }
  @container sd (max-width: 560px) {
    .sd-row { grid-template-columns: 38px minmax(0, 1fr) 34px 14px; }
    .sd-row__spark { display: none; }
    .sd-h .mx-kicker small { display: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .sd-rows { animation: none; }
    .sd-row, .sd-row :global(.sd-row__arrow) { transition: none; }
  }
</style>
