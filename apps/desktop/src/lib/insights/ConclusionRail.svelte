<script lang="ts">
  // The subject page's left pane (mockup 1-A `.n-rail`): conclusions by
  // confidence, one page at a time, with the shared Pager pinned under its own
  // scroll area. In a narrow window it is hidden and re-shown as the picker's
  // popover (`open`). Pure view: the page owns selection, page and size.
  import type { Conclusion } from "$lib/types/recording";
  import Pager from "$lib/insights/Pager.svelte";
  import { CONCLUSION_PAGE_SIZES, type Page } from "$lib/insights/subjectsTiers";

  interface Props {
    total: number;
    slice: Page<Conclusion>;
    size: number;
    selectedId: number;
    /** 0-based page holding the selected conclusion. */
    selectedPage: number;
    delta: (c: Conclusion) => number | null;
    active: boolean;
    open: boolean;
    onSelect: (id: number) => void;
    onPage: (page: number) => void;
    onSize: (size: number) => void;
    onFocus: () => void;
  }
  let { total, slice, size, selectedId, selectedPage, delta, active, open, onSelect, onPage, onSize, onFocus }: Props =
    $props();

  let scrollEl = $state<HTMLElement>();
  const pct = (v: number) => Math.round(v * 100);
  const signed = (d: number) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d)}`;

  // A new page starts at its top; a new pick (↑/↓) is kept in view. Scrolls
  // only this pane — never scrollIntoView, which would move the page too.
  const shownPage = $derived(slice.page);
  $effect(() => {
    shownPage;
    if (scrollEl) scrollEl.scrollTop = 0;
  });
  function keepSelected(): void {
    const box = scrollEl;
    const row = box?.querySelector<HTMLElement>('[aria-pressed="true"]');
    if (!box || !row) return;
    const r = row.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    if (r.top < b.top) box.scrollTop += r.top - b.top - 8;
    else if (r.bottom > b.bottom) box.scrollTop += r.bottom - b.bottom + 8;
  }
  $effect(() => {
    selectedId;
    open;
    shownPage;
    keepSelected();
  });
  // …and after the pane changes size (a shorter window, or back from narrow).
  $effect(() => {
    if (!scrollEl) return;
    const ro = new ResizeObserver(keepSelected);
    ro.observe(scrollEl);
    return () => ro.disconnect();
  });
</script>

<section class="sd-pane sd-rail" class:sd-rail--open={open} aria-label="Conclusions" onpointerdown={onFocus}>
  <div class="sd-pane__h">
    <span class="mx-kicker">Conclusions</span><span class="mx-spacer"></span>
    <span class="mx-label">{total} · by confidence</span>
  </div>
  <div class="sd-scroll" bind:this={scrollEl}>
    <div class="in-cls">
      {#each slice.visible as c (c.id)}
        {@const d = delta(c)}
        <button type="button" class="in-cl" aria-pressed={c.id === selectedId} onclick={() => onSelect(c.id)}>
          <span class="num">{pct(c.confidence)}%</span>
          <span class="in-cl__t">{c.statement}</span>
          <small class:in-up={(d ?? 0) > 0} class:in-down={(d ?? 0) < 0}>{d === null ? "new" : signed(d)}</small>
        </button>
      {/each}
    </div>
  </div>
  {#if total > CONCLUSION_PAGE_SIZES[0]}
    <div class="sd-foot">
      <Pager
        page={slice.page}
        pages={slice.pages}
        from={slice.from}
        to={slice.to}
        {total}
        {onPage}
        sizes={CONCLUSION_PAGE_SIZES}
        {size}
        {onSize}
        {active}
        label="Conclusions"
      >
        {#snippet extra()}
          {#if selectedPage !== slice.page}
            <button type="button" class="sd-selp" onclick={() => onPage(selectedPage)}>selected: page {selectedPage + 1}</button>
          {/if}
        {/snippet}
      </Pager>
    </div>
  {/if}
</section>

<style>
  .sd-pane { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .sd-rail { border-right: 1px solid var(--mx-hairline); }
  .sd-pane__h { flex: none; display: flex; align-items: baseline; gap: var(--s-3); padding: 12px 18px 6px 22px; }
  .sd-pane__h .mx-label { font-size: var(--text-sm); }
  .sd-scroll { flex: 1 1 auto; min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 0 8px 10px 12px; }
  .sd-foot { flex: none; padding: 6px 12px 7px; border-top: 1px solid var(--mx-hairline); }

  .in-cls { display: grid; gap: 2px; }
  .in-cl {
    width: 100%; display: grid; grid-template-columns: 36px minmax(0, 1fr) 32px; gap: 10px; align-items: start; padding: 8px 10px;
    border: 0; border-radius: var(--r-sm); background: none; cursor: pointer; text-align: left; color: var(--app-text-muted);
    font: 400 var(--text-sm) / 1.45 var(--font-sans);
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-cl:hover { background: var(--mx-wash); color: var(--app-text); }
  .in-cl:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  .in-cl[aria-pressed="true"] { background: var(--mx-selected); color: var(--app-text-strong); box-shadow: inset 2px 0 0 var(--app-accent); }
  .in-cl .num { font-size: var(--text-md); color: var(--app-text-strong); line-height: 1.3; }
  /* Long statements clip to two lines; the full text is in the right pane. */
  .in-cl__t { display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
  .in-cl small { text-align: right; font: 500 var(--text-xs) / 1.9 var(--font-mono); }
  .in-up { color: var(--focus-deep); }
  .in-down { color: var(--focus-distracted); }
  .sd-selp {
    height: 26px; padding: 0 4px; border: 0; background: none; cursor: pointer; white-space: nowrap; color: var(--app-accent);
    font: 500 var(--text-xs) / 1 var(--font-mono); text-decoration: underline; text-underline-offset: 3px;
  }
  .sd-selp:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 1px; border-radius: var(--r-xs); }

  /* Narrow: folded into the picker; shown as its popover when open. */
  @container sd (max-width: 620px) {
    .sd-rail {
      display: none; position: absolute; z-index: 10; top: 52px; left: 14px; right: 14px; max-height: calc(100% - 64px);
      border: 1px solid var(--app-border-strong); border-radius: var(--r-md);
      background: var(--app-surface-raised); box-shadow: 0 18px 40px -16px rgba(0, 0, 0, 0.5);
    }
    .sd-rail--open { display: flex; animation: sd-pop var(--t-med) var(--ease-expo); }
  }
  @keyframes sd-pop { from { opacity: 0; transform: translateY(-6px); } }
  @media (prefers-reduced-motion: reduce) {
    .in-cl { transition: none; }
    .sd-rail--open { animation: none; }
  }
</style>
