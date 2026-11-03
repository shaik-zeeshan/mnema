<script lang="ts" module>
  import type { Conclusion } from "$lib/types/recording";
  import type { TierSubject } from "$lib/insights/subjectsTiers";

  export interface SubjectRow extends TierSubject {
    subject: string;
    conclusions: Conclusion[]; // confidence desc
    headline: string; // the lead conclusion's statement
    sinceMs: number; // earliest formation
    then: number | null; // lead's confidence 14 days ago; null = formed this fortnight
    spark: { colorVar: string; faded: boolean; points: number[] }[];
  }

  /** One section of the list: a tier, the search matches, or (sparse) an
   *  untitled run. `id` keys its page + size, so a tier's id carries the axis
   *  ("conviction:forming") and each axis keeps its own pages. `faded` = the
   *  Fading tier, which starts folded. */
  export interface SubjectGroup {
    id: string;
    title: string;
    note: string;
    faded: boolean;
    items: SubjectRow[];
  }
</script>

<script lang="ts">
  // The Subjects list (mockup r2-A "Every tier, its own pager"): every tier is
  // its own section with its own page and page size, its pager in the heading
  // line ("// FORMING · 69 ── 11–20 OF 69 ‹ ›"). Fading starts folded. All of
  // it survives a trip to a subject's page (this stays mounted under it); on the
  // way back only the opened subject's tier turns to it, and its row is marked.
  import { tick } from "svelte";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import Pager from "$lib/insights/Pager.svelte";
  import { TIER_PAGE_SIZES, pageHolding, pageSlice, pageToShow } from "$lib/insights/subjectsTiers";
  import IconRight from "~icons/lucide/chevron-right";

  interface Props {
    groups: SubjectGroup[];
    /** The subject last opened from this tab — marked, and paged back to. */
    current: string | null;
    onOpen: (subject: string) => void;
  }
  let { groups, current, onOpen }: Props = $props();

  const PER = TIER_PAGE_SIZES[0];
  // Per section id; a missing entry is page 0 / PER rows.
  let pages = $state<Record<string, number>>({});
  let sizes = $state<Record<string, number>>({});
  let showFading = $state(false);
  let hov = $state<string | null>(null); // the section under the pointer
  let rootEl = $state<HTMLElement | null>(null);

  const slices = $derived(new Map(groups.map((g) => [g.id, pageSlice(g.items, pages[g.id] ?? 0, sizes[g.id] ?? PER)])));
  const folded = (g: SubjectGroup) => g.faded && !showFading;

  /** The filter text changed: Matches back to page 1 (the tiers keep theirs). */
  export function reset(): void {
    pages.search = 0;
  }

  /** "Go to the list" — bring the list's top into view. */
  export function show(): void {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    rootEl?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  }

  // The heading sits above the rows, so turning a page never moves it.
  const turn = (id: string, p: number) => (pages[id] = p);
  // A new size keeps the first row you were looking at on the page.
  function resize(id: string, s: number): void {
    pages[id] = pageHolding(slices.get(id)?.from ?? 0, s);
    sizes[id] = s;
  }
  // Fold/unfold, keeping focus on the link that took the toggle's place.
  function fold(id: string, show: boolean): void {
    showFading = show;
    void tick().then(() => rootEl?.querySelector<HTMLElement>(`[data-tier="${id}"] .in-group__link`)?.focus());
  }

  // ←/→ turn the tier under the pointer, else the one holding focus, else the
  // one holding the marked row.
  function target(): string | null {
    const f = document.activeElement?.closest<HTMLElement>("[data-tier]");
    const c = groups.find((g) => g.items.some((r) => r.subject === current));
    return hov ?? (f && rootEl?.contains(f) ? f.dataset.tier! : null) ?? c?.id ?? null;
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (!rootEl?.offsetParent) return; // not showing (another tab, or under a subject's page)
    const t = e.target as HTMLElement | null;
    if (t?.closest?.("input, textarea, select, [contenteditable], [role=radiogroup], [role=tablist], [role=slider]")) return;
    const modals = document.querySelectorAll("[aria-modal='true'], dialog[open]");
    if ([...modals].some((m) => m.getClientRects().length > 0)) return; // a rendered modal owns the keys
    const id = target();
    const g = groups.find((x) => x.id === id);
    const sl = id ? slices.get(id) : undefined;
    if (!g || !sl || folded(g)) return;
    const next = sl.page + (e.key === "ArrowRight" ? 1 : -1);
    if (next < 0 || next >= sl.pages) return;
    e.preventDefault();
    turn(g.id, next);
  }

  function onOver(e: PointerEvent): void {
    hov = (e.target as HTMLElement).closest<HTMLElement>("[data-tier]")?.dataset.tier ?? null;
  }

  // Back from a subject's page (the host hid us with display:none): turn just
  // the opened subject's tier to its row (unfolding Fading if that's where it
  // is) and centre it. The host zeroes the scroll on the way back before we're
  // shown, so this lands after it.
  $effect(() => {
    if (!rootEl) return;
    let hidden = rootEl.offsetParent === null;
    const ro = new ResizeObserver(() => {
      const nowHidden = rootEl?.offsetParent === null;
      if (hidden && !nowHidden && current) {
        const g = groups.find((x) => x.items.some((r) => r.subject === current));
        const sl = g && slices.get(g.id);
        if (g && sl) {
          const i = g.items.findIndex((r) => r.subject === current);
          if (g.faded) showFading = true;
          const p = pageToShow(i, sl, sizes[g.id] ?? PER);
          if (p !== null) pages[g.id] = p;
        }
        void tick().then(() => rootEl?.querySelector(".is-cur")?.scrollIntoView({ block: "center" }));
      }
      hidden = nowHidden;
    });
    ro.observe(rootEl);
    return () => ro.disconnect();
  });

  const pct = (c: number) => Math.round(Math.max(0, Math.min(1, c)) * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
</script>

<svelte:window onkeydown={onKey} />

<div class="in-list" bind:this={rootEl} onpointerover={onOver} onpointerleave={() => (hov = null)} role="presentation">
  {#each groups as g (g.id)}
    {@const sl = slices.get(g.id)!}
    <section class="in-tier" class:in-tier--faded={g.faded} data-tier={g.id} aria-label={g.title || "Subjects"}>
      {#if g.title}
        <div class="in-group">
          <span class="in-group__k"><i>//</i> {g.title} · {sl.total}</span>
          {#if g.note}<span class="in-group__note">{g.note}</span>{/if}
          <span class="in-group__rule"></span>
          {#if folded(g)}
            {#if sl.total}<button type="button" class="in-group__link" onclick={() => fold(g.id, true)}>Show {sl.total} fading</button>{/if}
          {:else}
            <Pager
              page={sl.page}
              pages={sl.pages}
              from={sl.from}
              to={sl.to}
              total={sl.total}
              onPage={(p) => turn(g.id, p)}
              size={sizes[g.id] ?? PER}
              onSize={(s) => resize(g.id, s)}
              label={g.title}
            />
            {#if g.faded}<button type="button" class="in-group__link" onclick={() => fold(g.id, false)}>Hide</button>{/if}
          {/if}
        </div>
      {/if}
      {#if !folded(g)}
        {#each sl.visible as r (r.subject)}
          <button type="button" class="in-subj" class:is-cur={r.subject === current} onclick={() => onOpen(r.subject)}>
            <span class="in-subj__name">{r.subject}{#if r.subject === current}<span class="in-subj__tag">opened</span>{/if}<small>{r.conclusions.length} concl. · since {day(r.sinceMs)}</small></span>
            <span class="in-subj__lead">{r.headline}</span>
            <span class="in-subj__spark">
              <Sparkline series={r.spark} label={`${r.subject} — confidence over time across ${r.conclusions.length} conclusions`} />
            </span>
            <span class="num">{pct(r.topConfidence)}%</span>
            <IconRight class="arrow" width="14" height="14" aria-hidden="true" />
          </button>
        {:else}
          <p class="in-empty">None right now.</p>
        {/each}
      {/if}
    </section>
  {/each}
</div>

<style>
  /* insights.html Subjects tier rows + mockup r2-A's tier headings. */
  .in-list {
    scroll-margin-top: calc(var(--in-bar-h, 56px) + 32px); /* clear the sticky tab bar (taller once it wraps) on "Go to the list" */
  }
  .in-tier {
    margin-bottom: var(--s-5);
  }
  /* The tier heading: a kicker line that also carries the tier's pager. */
  .in-group {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 28px;
    margin: var(--s-4) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .in-group__k {
    white-space: nowrap;
  }
  .in-group__k i {
    font-style: normal;
    color: var(--app-text-faint);
    margin-right: 2px;
  }
  .in-group__note {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: none;
    letter-spacing: 0;
    color: var(--app-text-faint);
  }
  .in-group__rule {
    flex: 1 1 12px;
    height: 1px;
    background: var(--mx-hairline);
  }
  .in-group__link {
    height: 22px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--r-xs);
    background: none;
    cursor: pointer;
    white-space: nowrap;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    color: var(--app-text-subtle);
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-group__link:hover {
    background: var(--mx-wash-strong);
    color: var(--app-text-strong);
  }
  .in-group__link:focus-visible {
    outline: 2px solid var(--app-accent);
    outline-offset: 1px;
  }
  .in-empty {
    margin: var(--s-2) 0 0;
    font-size: var(--text-sm);
    color: var(--app-text-faint);
  }
  .in-subj {
    display: grid;
    grid-template-columns: 170px minmax(0, 1fr) 100px 44px 14px;
    gap: var(--s-4);
    align-items: center;
    width: calc(100% + var(--s-3) * 2);
    padding: 12px var(--s-3);
    margin: 0 calc(var(--s-3) * -1);
    border: 0;
    border-bottom: 1px solid var(--mx-hairline);
    border-radius: var(--r-sm);
    background: none;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .in-subj:last-of-type {
    border-bottom: 0;
  }
  .in-subj:hover {
    background: var(--app-surface-hover);
  }
  .in-subj.is-cur {
    background: var(--mx-wash);
  }
  .in-subj__tag {
    margin-left: 6px;
    font: 500 10px / 1 var(--font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-accent);
    white-space: nowrap;
  }
  .in-subj:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .in-subj__name {
    font: 600 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
  }
  .in-subj__name small {
    display: block;
    margin-top: 5px;
    white-space: nowrap;
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .in-subj__lead {
    color: var(--app-text-muted);
    font-size: var(--text-base);
    line-height: 1.45;
  }
  .in-subj__spark {
    display: block;
    height: 30px;
  }
  .in-subj__spark :global(svg) {
    width: 100%;
    height: 100%;
  }
  .in-subj .num {
    text-align: right;
    color: var(--app-text-strong);
  }
  .in-subj :global(.arrow) {
    color: var(--app-text-faint);
    transition: transform var(--t-med) var(--ease-expo);
  }
  .in-subj:hover :global(.arrow) {
    transform: translateX(3px);
    color: var(--app-text-strong);
  }
  .in-tier--faded .in-subj {
    opacity: 0.62;
  }
</style>
