<script lang="ts">
  // The subject page's right pane (mockup 1-A `.n-detail`): the chosen
  // conclusion, Pin / Dismiss, a one-line summary, and its evidence timeline
  // (newest first), paged EVIDENCE_PAGE at a time with the Pager pinned under
  // the pane's own scroll area. Pure view: the page owns state and actions.
  import type { Activity, ActivityEvidenceRef, Conclusion, ConfidenceSnapshot } from "$lib/types/recording";
  import type { TimelineEvent } from "$lib/insights/subjectTimeline";
  import Pager from "$lib/insights/Pager.svelte";
  import { EVIDENCE_PAGE, pageSlice } from "$lib/insights/subjectsTiers";
  import { fmtDuration } from "$lib/insights/lede-stats";
  import { formatClock } from "$lib/time-format";
  import IconPin from "~icons/lucide/pin";
  import IconArrow from "~icons/lucide/arrow-right";

  interface Props {
    conclusion: Conclusion;
    index: number;
    total: number;
    events: TimelineEvent[];
    history: ConfidenceSnapshot[];
    activities: Map<number, Activity>;
    newestRef: ActivityEvidenceRef | null;
    page: number;
    active: boolean;
    pinBusy: boolean;
    onPage: (page: number) => void;
    onPin: () => void;
    onDismiss: () => void;
    onOpenEvidence: (activityId: number) => void;
    onOpenRef: (ref: ActivityEvidenceRef) => void;
    onFocus: () => void;
  }
  let {
    conclusion: c,
    index,
    total,
    events,
    history,
    activities,
    newestRef,
    page,
    active,
    pinBusy,
    onPage,
    onPin,
    onDismiss,
    onOpenEvidence,
    onOpenRef,
    onFocus,
  }: Props = $props();

  let scrollEl = $state<HTMLElement>();
  const pct = (v: number) => Math.round(v * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  const slice = $derived(pageSlice(events, page, EVIDENCE_PAGE));
  const peak = $derived(Math.max(c.confidence, ...history.map((s) => s.confidence)));
  const support = $derived(c.evidence.filter((e) => e.stance === "support").length);
  const contra = $derived(c.evidence.length - support);

  // A new conclusion or evidence page starts at the pane's top.
  const cid = $derived(c.id);
  const shownPage = $derived(slice.page);
  $effect(() => {
    cid;
    shownPage;
    if (scrollEl) scrollEl.scrollTop = 0;
  });

  function evidenceNote(a: Activity): string {
    const frames = a.evidence.filter((r) => r.subjectType === "frame").length;
    // B6: the activity's clock time, not a transcript offset Mnema can't pin.
    const parts = [formatClock(a.startedAtMs), fmtDuration(a.endedAtMs - a.startedAtMs)];
    if (frames) parts.push(`cited ${frames} ${frames === 1 ? "frame" : "frames"}`);
    return parts.join(" · ");
  }
</script>

<section class="sd-pane sd-detail" aria-label="Why Mnema believes this" onpointerdown={onFocus}>
  <div class="sd-scroll" bind:this={scrollEl}>
    <span class="mx-kicker">Why Mnema believes this · {index + 1} of {total}</span>
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
    <ol class="in-tl">
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
              {ev.title}<small>{ev.kind === "contradict" ? "contradicts" : evidenceNote(act)}</small>
            </button>
          {:else}
            <div>{ev.title}{#if ev.kind === "contradict"}<small>contradicts</small>{/if}</div>
          {/if}
        </li>
      {/each}
    </ol>
  </div>
  {#if slice.pages > 1}
    <div class="sd-foot">
      <Pager page={slice.page} pages={slice.pages} from={slice.from} to={slice.to} total={slice.total} {onPage} {active} label="Evidence">
        {#snippet extra()}<span class="sd-newest">newest first</span>{/snippet}
      </Pager>
    </div>
  {/if}
</section>

<style>
  .sd-pane { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .sd-scroll { flex: 1 1 auto; min-height: 0; overflow: auto; overscroll-behavior: contain; padding: 16px 28px 24px; }
  .sd-foot { flex: none; padding: 6px 22px 7px; border-top: 1px solid var(--mx-hairline); }
  .sd-newest { font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-subtle); }

  .in-cd__statement {
    font: 600 22px / 1.25 var(--font-sans); letter-spacing: -0.02em; color: var(--app-text-strong);
    margin: 8px 0 12px; max-width: 46ch; text-wrap: balance;
  }
  .in-cd__acts { display: flex; gap: 6px; }
  .sd-spec {
    display: flex; flex-wrap: wrap; align-items: center; gap: 4px 14px; margin: 12px 0 0; padding: 8px 0;
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

  .in-tl { list-style: none; margin: 10px 0 0; padding: 0; position: relative; }
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

  @container sd (max-width: 920px) {
    .sd-scroll { padding: 14px 20px 22px; }
    .sd-foot { padding: 6px 16px 7px; }
  }
  @container sd (max-width: 620px) {
    .sd-scroll { padding: 14px 18px 20px; }
    .sd-foot { padding: 6px 14px 7px; }
    .in-cd__statement { font-size: 18px; }
  }
  @media (prefers-reduced-motion: reduce) {
    .in-tl li { animation: none; }
  }
</style>
