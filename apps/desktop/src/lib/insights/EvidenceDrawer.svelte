<script lang="ts">
  // The subject page's evidence drawer (mockup r2-B): pinned to the window
  // under the Insights bar, beside the page (which narrows to make room) or, in
  // a narrow window, over it as a full sheet with a back arrow. It holds the
  // open conclusion: Pin / Dismiss, a one-line spec, and its evidence timeline
  // (newest first), as many rows as fit the drawer's height (at least
  // EVIDENCE_MIN_FIT) with the pager in its heading. It
  // scrolls on its own and starts at its top for each conclusion. Pure view:
  // the page owns state, geometry and actions.
  import type { Activity, ActivityEvidenceRef, Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import type { TimelineEvent } from "$lib/insights/subjectTimeline";
  import { tick } from "svelte";
  import Pager from "$lib/insights/Pager.svelte";
  import { EVIDENCE_MIN_FIT, pageSlice } from "$lib/insights/subjectsTiers";
  import { fmtDuration } from "$lib/insights/lede-stats";
  import { formatClock } from "$lib/time-format";
  import IconPin from "~icons/lucide/pin";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconPrev from "~icons/lucide/chevron-left";
  import IconNext from "~icons/lucide/chevron-right";
  import IconClose from "~icons/lucide/x";
  import IconBack from "~icons/lucide/arrow-left";

  interface Props {
    conclusion: Conclusion;
    index: number;
    total: number;
    tierTitle: string;
    subject: string;
    events: TimelineEvent[];
    history: ConfidenceSnapshot[];
    activities: Map<number, Activity>;
    newestRef: ActivityEvidenceRef | null;
    page: number;
    /** Evidence rows per page; the drawer reports what fits via `onFit`. */
    size: number;
    onFit: (size: number) => void;
    pinBusy: boolean;
    /** Window-pinned box (px) and whether it covers the page as a sheet. */
    box: { top: number; right: number; bottom: number; width: number };
    sheet: boolean;
    onPage: (page: number) => void;
    onStep: (d: 1 | -1) => void;
    onClose: () => void;
    onPin: () => void;
    onDismiss: () => void;
    onOpenEvidence: (activityId: number) => void;
    onOpenRef: (ref: ActivityEvidenceRef) => void;
  }
  let {
    conclusion: c,
    index,
    total,
    tierTitle,
    subject,
    events,
    history,
    activities,
    newestRef,
    page,
    size,
    onFit,
    pinBusy,
    box,
    sheet,
    onPage,
    onStep,
    onClose,
    onPin,
    onDismiss,
    onOpenEvidence,
    onOpenRef,
  }: Props = $props();

  let scrollEl = $state<HTMLElement>();
  let listEl = $state<HTMLOListElement>();
  const pct = (v: number) => Math.round(v * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  const slice = $derived(pageSlice(events, page, size));
  const peak = $derived(Math.max(c.confidence, ...history.map((s) => s.confidence)));
  const support = $derived(c.evidence.filter((e) => e.stance === "support").length);
  const contra = $derived(c.evidence.length - support);

  // Each conclusion starts at the drawer's top.
  const cid = $derived(c.id);
  $effect(() => {
    cid;
    if (scrollEl) scrollEl.scrollTop = 0;
  });

  // Height-responsive page: fill the drawer from the list's top to its foot,
  // using the rendered rows' average height. Re-measured when the drawer
  // resizes or the conclusion changes (its statement height moves the list),
  // never on a page turn, so pages don't reshuffle under you.
  function fit(): void {
    if (!scrollEl || !listEl || listEl.children.length === 0) return;
    const rowH = listEl.offsetHeight / listEl.children.length;
    const top = listEl.getBoundingClientRect().top - scrollEl.getBoundingClientRect().top + scrollEl.scrollTop;
    const foot = parseFloat(getComputedStyle(scrollEl).paddingBottom) || 0;
    const n = Math.max(EVIDENCE_MIN_FIT, Math.floor((scrollEl.clientHeight - top - foot) / rowH));
    if (n !== size) onFit(n);
  }
  $effect(() => {
    cid;
    const el = scrollEl;
    if (!el) return;
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);
    void tick().then(fit);
    return () => ro.disconnect();
  });

  function evidenceNote(a: Activity): string {
    const frames = a.evidence.filter((r) => r.subjectType === "frame").length;
    // B6: the activity's clock time, not a transcript offset Mnema can't pin.
    const parts = [formatClock(a.startedAtMs), fmtDuration(a.endedAtMs - a.startedAtMs)];
    if (frames) parts.push(`cited ${frames} ${frames === 1 ? "frame" : "frames"}`);
    return parts.join(" · ");
  }
</script>

<aside
  class="sd-drawer"
  class:sd-drawer--sheet={sheet}
  bind:this={scrollEl}
  style:top="{box.top}px"
  style:right="{box.right}px"
  style:bottom="{box.bottom}px"
  style:width="{box.width}px"
  aria-label="Evidence for the open conclusion"
>
  <div class="sd-dr__bar">
    {#if sheet}
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm sd-dr__icon" aria-label="Back to {subject}" onclick={onClose}>
        <IconBack width="15" height="15" aria-hidden="true" />
      </button>
    {/if}
    <span class="mx-kicker">Conclusion · {index + 1} of {total} · {tierTitle}</span>
    <span class="mx-spacer"></span>
    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm sd-dr__icon" aria-label="Previous conclusion" disabled={index === 0} onclick={() => onStep(-1)}>
      <IconPrev width="14" height="14" aria-hidden="true" />
    </button>
    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm sd-dr__icon" aria-label="Next conclusion" disabled={index >= total - 1} onclick={() => onStep(1)}>
      <IconNext width="14" height="14" aria-hidden="true" />
    </button>
    {#if !sheet}
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm sd-dr__icon" aria-label="Close" onclick={onClose}>
        <IconClose width="14" height="14" aria-hidden="true" />
      </button>
    {/if}
  </div>

  <h2 class="in-cd__statement">{c.statement}</h2>
  <div class="in-cd__acts">
    <button type="button" class="mx-btn mx-btn--sm" aria-pressed={c.pinned} aria-busy={pinBusy || undefined} onclick={onPin}
      ><IconPin width="13" height="13" aria-hidden="true" />{c.pinned ? "Pinned" : "Pin"}</button
    >
    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onDismiss}>Dismiss</button>
  </div>
  <p class="sd-spec">
    <span>now <b>{pct(c.confidence)}%</b></span>
    <span>peak <b>{pct(peak)}%</b></span>
    <span><b>{support}</b> support · <b class:in-down={contra > 0}>{contra}</b> contradict</span>
    <span>formed <b>{day(c.formedAtMs)}</b></span>
    <span>last support <b>{day(c.lastSupportedAtMs)}</b></span>
    <span>decay <b>{c.pinned ? "pinned, won't fade" : "halves in 30d without support"}</b></span>
    {#if newestRef}
      {@const ref = newestRef}
      <button type="button" class="sd-open" onclick={() => onOpenRef(ref)}
        >Open newest in Timeline<IconArrow width="12" height="12" aria-hidden="true" /></button
      >
    {/if}
  </p>

  <section data-sec="e" aria-label="Evidence">
    <div class="sd-h">
      <span class="mx-kicker">Evidence · {slice.total}<small>newest first</small></span>
      <i class="sd-h__rule"></i>
      <Pager page={slice.page} pages={slice.pages} from={slice.from} to={slice.to} total={slice.total} {size} sizes={null} {onPage} label="Evidence" />
    </div>
    {#key `${c.id}:${slice.page}`}
      <ol class="in-tl" bind:this={listEl}>
        {#each slice.visible as ev, i (slice.from + i)}
          {@const act = ev.kind === "evidence" || ev.kind === "contradict" ? activities.get(ev.activityId) : undefined}
          <li class:origin={ev.kind === "formed"} class:conf={ev.kind === "marker"} class:contra={ev.kind === "contradict"} style:--i={i}>
            <time class="num">{ev.atMs == null ? "—" : day(ev.atMs)}</time>
            {#if ev.kind === "formed"}
              {@const n = c.formedEvidenceCount}
              <!-- Beliefs formed before the count was recorded keep the old line. -->
              <div>
                {n == null
                  ? `Formed at ${pct(ev.confidence)}%`
                  : `Formed from ${n} supporting ${n === 1 ? "activity" : "activities"} · ${pct(ev.confidence)}%`}
              </div>
            {:else if ev.kind === "marker"}
              <div>Confidence {pct(ev.from)}% → {pct(ev.to)}%</div>
            {:else if ev.kind === "replaced"}
              <div>Replaced an earlier take<small>{ev.statement}</small></div>
            {:else if act}
              <button type="button" class="in-tl__ev" onclick={() => onOpenEvidence(ev.activityId)}>
                {ev.title}<small>{ev.kind === "contradict" ? `contradicts this · ${evidenceNote(act)}` : evidenceNote(act)}</small>
              </button>
            {:else}
              <div>{ev.title}{#if ev.kind === "contradict"}<small>contradicts this</small>{/if}</div>
            {/if}
          </li>
        {/each}
      </ol>
    {/key}
  </section>
</aside>

<style>
  .sd-drawer {
    position: fixed; z-index: 15; min-width: 0; overflow: auto; overscroll-behavior: contain; padding: 0 24px 28px;
    border-left: 1px solid var(--app-border-strong); background: var(--app-surface-subtle);
    animation: sd-in var(--t-slow) var(--ease-expo);
  }
  .sd-drawer--sheet { border-left: 0; padding: 0 16px 24px; background: var(--app-bg); animation-name: sd-up; }
  @keyframes sd-in { from { opacity: 0; transform: translateX(28px); } }
  @keyframes sd-up { from { opacity: 0; transform: translateY(18px); } }

  .sd-dr__bar {
    position: sticky; top: 0; z-index: 2; display: flex; align-items: center; gap: var(--s-2); height: 46px;
    margin: 0 -24px; padding: 0 12px 0 24px; background: var(--app-surface-subtle); border-bottom: 1px solid var(--mx-hairline);
  }
  .sd-drawer--sheet .sd-dr__bar { margin: 0 -16px; padding: 0 12px 0 8px; background: var(--app-bg); }
  .sd-dr__bar .mx-kicker { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; }
  .sd-dr__bar .mx-kicker::before { display: inline-block; margin-right: var(--s-2); }
  .sd-dr__icon { width: 26px; padding: 0; justify-content: center; }

  .in-cd__statement {
    font: 600 22px / 1.25 var(--font-sans); letter-spacing: -0.02em; color: var(--app-text-strong);
    margin: var(--s-4) 0 var(--s-4); text-wrap: balance;
  }
  .in-cd__acts { display: flex; gap: 6px; }
  .sd-spec {
    display: flex; flex-wrap: wrap; align-items: center; gap: 4px 14px; margin: 14px 0 0; padding: 8px 0;
    border-top: 1px solid var(--mx-hairline); border-bottom: 1px solid var(--mx-hairline);
    font: 500 var(--text-xs) / 1.5 var(--font-mono); color: var(--app-text-subtle);
  }
  .sd-spec b { font-weight: 600; color: var(--app-text-strong); }
  .sd-spec b.in-down { color: var(--focus-distracted); }
  .sd-open {
    display: inline-flex; align-items: center; gap: 4px; padding: 0; border: 0; background: none; cursor: pointer;
    font: inherit; color: var(--app-text-muted); border-radius: var(--r-xs);
  }
  .sd-open:hover { color: var(--app-text-strong); }
  .sd-open:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }

  /* Same heading as a tier's, so the Pager inherits kicker type. */
  .sd-h {
    display: flex; align-items: center; gap: var(--s-3); min-height: 24px; margin: var(--s-5) 0 var(--s-1);
    font: 500 var(--text-xs) / 1 var(--font-mono); letter-spacing: 0.08em; text-transform: uppercase; color: var(--app-text-subtle);
  }
  .sd-h .mx-kicker { white-space: nowrap; }
  .sd-h .mx-kicker small { margin-left: 6px; font: 400 var(--text-xs) / 1 var(--font-sans); letter-spacing: 0; text-transform: none; color: var(--app-text-faint); }
  .sd-h__rule { flex: 1; min-width: 12px; height: 1px; background: var(--mx-hairline); }

  .in-tl { list-style: none; margin: var(--s-2) 0 0; padding: 0; position: relative; }
  .in-tl::before { content: ""; position: absolute; left: 5px; top: 6px; bottom: 6px; width: 1px; background: var(--app-border); }
  .in-tl li {
    position: relative; display: grid; grid-template-columns: 22px 64px minmax(0, 1fr); padding: 9px 0;
    animation: mx-rise var(--t-slow) var(--ease-expo) both; animation-delay: calc(var(--i) * 30ms);
  }
  .in-tl li::before { content: ""; width: 11px; height: 11px; border-radius: 50%; margin-top: 3px; background: var(--app-bg); border: 1.5px solid var(--app-border-strong); }
  .in-tl li.conf::before { border-color: var(--app-text-strong); background: var(--app-text-strong); width: 7px; height: 7px; margin: 5px 0 0 2px; }
  .in-tl li.origin::before { border-color: var(--app-accent); background: var(--app-accent); box-shadow: 0 0 0 3px var(--app-accent-glow); }
  .in-tl li.contra::before { border-color: var(--focus-distracted); }
  .in-tl time { font-size: var(--text-xs); color: var(--app-text-subtle); line-height: 1.9; }
  .in-tl li > div, .in-tl__ev { color: var(--app-text); font-size: var(--text-base); min-width: 0; }
  .in-tl small { display: block; margin-top: 3px; font: 400 var(--text-sm) / 1.4 var(--font-sans); color: var(--app-text-subtle); }
  .in-tl li.contra small { color: var(--focus-distracted); }
  .in-tl li.conf > div { font: 500 var(--text-sm) / 1.6 var(--font-mono); font-variant-numeric: tabular-nums; color: var(--app-text-strong); }
  .in-tl__ev { padding: 0; border: 0; background: none; text-align: left; font-family: inherit; line-height: inherit; cursor: pointer; border-radius: var(--r-xs); }
  .in-tl__ev:hover { color: var(--app-text-strong); }
  .in-tl__ev:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }

  @media (prefers-reduced-motion: reduce) {
    .sd-drawer, .in-tl li { animation: none; }
  }
</style>
