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

  /** One divider's worth of rows: a tier, the search matches, or (sparse) an
   *  untitled run. `count` is printed after the title ("Forming · 70"). */
  export interface SubjectGroup {
    id: string;
    title: string;
    note: string;
    faded: boolean;
    count?: number;
    items: SubjectRow[];
  }
</script>

<script lang="ts">
  // The Subjects list (mockup 1-A "Numbered pages"): the tiers are dividers
  // inside ONE paged list with ONE pager at its foot. A tier cut by a page
  // boundary repeats its divider as "Forming (cont.)". The page survives a trip
  // to a subject's page (this stays mounted under it) and, on the way back,
  // turns to the subject that was opened and marks it.
  import { tick } from "svelte";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import Pager from "$lib/insights/Pager.svelte";
  import { SUBJECT_PAGE_SIZES, pageSlice } from "$lib/insights/subjectsTiers";
  import IconRight from "~icons/lucide/chevron-right";

  interface Props {
    groups: SubjectGroup[];
    /** The subject last opened from this tab — marked, and paged back to. */
    current: string | null;
    onOpen: (subject: string) => void;
  }
  let { groups, current, onOpen }: Props = $props();

  let page = $state(0);
  let size = $state<number>(SUBJECT_PAGE_SIZES[0]);
  let rootEl = $state<HTMLElement | null>(null);
  let pagerEl = $state<HTMLElement | null>(null);

  const flat = $derived(groups.flatMap((g) => g.items.map((r, i) => ({ r, g, i }))));
  const slice = $derived(pageSlice(flat, page, size));
  // The page's rows, split wherever the tier changes; `first` is the row's
  // index inside its tier, so a section that starts mid-tier is a "(cont.)".
  const sections = $derived.by(() => {
    const out: { g: SubjectGroup; first: number; rows: SubjectRow[] }[] = [];
    for (const e of slice.visible) {
      const last = out.at(-1);
      if (last?.g === e.g) last.rows.push(e.r);
      else out.push({ g: e.g, first: e.i, rows: [e.r] });
    }
    return out;
  });
  // ponytail: hidden only when the list fits the smallest size, so a reader on
  // 50/page can still get back to 20 (as the mockup).
  const showPager = $derived(slice.total > SUBJECT_PAGE_SIZES[0]);

  /** Back to page 1 — the filter text or the axis changed. */
  export function reset(): void {
    page = 0;
  }

  /** "Go to the list" — bring the list's top into view. */
  export function show(): void {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    rootEl?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
  }

  function scroller(): HTMLElement | null {
    return rootEl?.closest<HTMLElement>(".in-main") ?? null;
  }

  // Turn the page without the pager jumping out from under the pointer: hold
  // its top where it was, even when the new page is shorter.
  async function keepPager(change: () => void): Promise<void> {
    const before = pagerEl?.getBoundingClientRect().top;
    change();
    await tick();
    const after = pagerEl?.getBoundingClientRect().top;
    const sc = scroller();
    if (before === undefined || after === undefined || !sc) return;
    sc.scrollTop += after - before;
  }

  const turn = (p: number) => void keepPager(() => (page = p));
  // A new size keeps the first row you were looking at on screen.
  const resize = (s: number) =>
    void keepPager(() => {
      page = Math.floor(slice.from / s);
      size = s;
    });

  function onKey(e: KeyboardEvent): void {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (!showPager || !rootEl?.offsetParent) return; // not showing (another tab, or under a subject's page)
    const t = e.target as HTMLElement | null;
    if (t?.closest?.("input, textarea, select, [contenteditable], [role=radiogroup], [role=tablist], [role=slider]")) return;
    const modals = document.querySelectorAll("[aria-modal='true'], dialog[open]");
    if ([...modals].some((m) => m.getClientRects().length > 0)) return; // a rendered modal owns the keys
    const next = slice.page + (e.key === "ArrowRight" ? 1 : -1);
    if (next < 0 || next >= slice.pages) return;
    e.preventDefault();
    turn(next);
  }

  // Back from a subject's page (the host hid us with display:none): turn to
  // the page holding the subject opened and centre its row. The host zeroes the
  // scroll on the way back before we're shown, so this lands after it.
  $effect(() => {
    if (!rootEl) return;
    let hidden = rootEl.offsetParent === null;
    const ro = new ResizeObserver(() => {
      const nowHidden = rootEl?.offsetParent === null;
      if (hidden && !nowHidden && current) {
        const i = flat.findIndex((e) => e.r.subject === current);
        if (i >= 0) page = Math.floor(i / size);
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

<div class="in-list" bind:this={rootEl}>
  {#each sections as s (`${s.g.id}:${s.first}`)}
    <div class="in-tier" class:in-tier--faded={s.g.faded}>
      {#if s.g.title}
        <p class="in-group">
          {s.g.title}{s.first > 0 ? " (cont.)" : ""}{s.g.count !== undefined ? ` · ${s.g.count}` : ""}
          {#if s.first > 0}<span class="in-group__note">from #{s.first + 1}</span>{:else if s.g.note}<span class="in-group__note">{s.g.note}</span>{/if}
        </p>
      {/if}
      {#each s.rows as r (r.subject)}
        <button type="button" class="in-subj" class:is-cur={r.subject === current} onclick={() => onOpen(r.subject)}>
          <span class="in-subj__name">{r.subject}<small>{r.conclusions.length} concl. · since {day(r.sinceMs)}</small></span>
          <span class="in-subj__lead">{r.headline}</span>
          <span class="in-subj__spark">
            <Sparkline series={r.spark} label={`${r.subject} — confidence over time across ${r.conclusions.length} conclusions`} />
          </span>
          <span class="num">{pct(r.topConfidence)}%</span>
          <IconRight class="arrow" width="14" height="14" aria-hidden="true" />
        </button>
      {/each}
    </div>
  {/each}
  {#if showPager}
    <div class="in-list__pg" bind:this={pagerEl}>
      <Pager
        page={slice.page}
        pages={slice.pages}
        from={slice.from}
        to={slice.to}
        total={slice.total}
        onPage={turn}
        sizes={SUBJECT_PAGE_SIZES}
        {size}
        onSize={resize}
        label="Subjects"
      />
    </div>
  {/if}
</div>

<style>
  /* insights.html Subjects tier rows + the mockup's numbered pager. */
  .in-list {
    scroll-margin-top: 88px; /* clear the sticky tab bar on "Go to the list" */
  }
  .in-list__pg {
    margin-top: var(--s-3);
  }
  .in-tier {
    margin-bottom: var(--s-5);
  }
  .in-group {
    margin: var(--s-4) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .in-group::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--mx-hairline);
  }
  .in-group__note {
    text-transform: none;
    letter-spacing: 0;
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
    box-shadow: inset 2px 0 0 var(--app-accent);
    background: var(--mx-wash);
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
