<script lang="ts">
  // Subject detail (insights.html `openSubject`): crumb with a ghost Ask, the
  // subject lede + four stats, "Confidence over time" beside the conclusions
  // list, then "Why Mnema believes this" beside the "This conclusion" panel.
  // Owns the fetch, the realtime refresh, selection, Pin / delayed-commit
  // Dismiss, and the Timeline hand-off. Each conclusion keeps its OWN
  // confidence — the subject is never rolled up into one score.
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import type {
    Conclusion,
    ConfidenceSnapshot,
    SubjectView,
    Activity,
    ActivityEvidenceRef,
  } from "$lib/types/recording";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import SubjectChart from "$lib/insights/SubjectChart.svelte";
  import { buildTimeline } from "$lib/insights/subjectTimeline";
  import { debounce, deriveTrend, fortnightAgo } from "$lib/insights/subjectsTiers";
  import { DelayedDismiss, DISMISS_UNDO_MS } from "$lib/insights/dismissUndo.svelte";
  import { relativeTime } from "$lib/insights/conversationStore.svelte";
  import { fmtDuration } from "$lib/insights/lede-stats";
  import { formatClock } from "$lib/time-format";
  import { tip } from "$lib/components/tooltip";
  import { humanizeError } from "$lib/format-error";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSparkle from "~icons/lucide/sparkle";
  import IconPin from "~icons/lucide/pin";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconInbox from "~icons/lucide/inbox";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconOk from "~icons/lucide/check";
  import IconFail from "~icons/lucide/circle-alert";

  interface Props {
    subject: string;
    onBack: () => void;
    // The shell's engine is not set up / off: Ask is shown but disabled.
    askDisabled?: boolean;
  }
  let { subject, onBack, askDisabled = false }: Props = $props();

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  let pinBusy = $state(false);
  let destroyed = false;

  // Activities the conclusions cite, resolved for row detail + the hand-off.
  let activities = $state<Map<number, Activity>>(new Map());

  // ── Toasts: the result of the user's own action (kit `.mx-toast`) ──
  type Toast = { tone: "ok" | "danger"; text: string; undo?: Conclusion };
  let toast = $state<Toast | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  function showToast(t: Toast): void {
    clearTimeout(toastTimer);
    toast = t;
    toastTimer = setTimeout(() => (toast = null), DISMISS_UNDO_MS);
  }

  // Dismiss hides the row at once and commits after 5 s unless undone.
  const dismissal = new DelayedDismiss<Conclusion>((c) => {
    invoke("user_context_dismiss_conclusion", { id: c.id })
      .then(() => {
        if (!destroyed) void loadSubject();
      })
      .catch((error) => {
        if (!destroyed) showToast({ tone: "danger", text: `Couldn't dismiss — ${humanizeError(error)}` });
      });
  });
  $effect(() => () => {
    destroyed = true;
    clearTimeout(toastTimer);
    dismissal.flush();
  });

  const pct = (v: number) => Math.round(v * 100);
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const signed = (d: number) => `${d > 0 ? "+" : d < 0 ? "−" : "±"}${Math.abs(d)}`;

  const histories = $derived(
    new Map<number, ConfidenceSnapshot[]>((view?.trajectories ?? []).map((t) => [t.conclusionId, t.history])),
  );
  // Shown = by confidence, minus anything inside its Dismiss undo window.
  const shown = $derived(
    (view?.conclusions ?? []).filter((c) => !dismissal.has(c.id)).sort((a, b) => b.confidence - a.confidence),
  );
  const selected = $derived(shown.find((c) => c.id === selectedId) ?? shown[0] ?? null);
  const selIndex = $derived(selected ? shown.indexOf(selected) : -1);
  const lead = $derived(shown[0] ?? null);

  // ── Lede ──
  const now = Date.now();
  const sinceMs = $derived(Math.min(...shown.map((c) => c.formedAtMs)));
  const trendWord = $derived.by(() => {
    if (shown.every((c) => c.status === "faded")) return "fading";
    const points = new Map([...histories].map(([id, h]) => [id, h.map((s) => s.confidence)]));
    return { up: "warming", steady: "steady", down: "cooling" }[deriveTrend(shown, points)];
  });
  // Confidence change over the fortnight; null = formed inside it ("new").
  const fortnightDelta = (c: Conclusion): number | null => {
    const then = fortnightAgo(c, histories.get(c.id) ?? [], now);
    return then === null ? null : pct(c.confidence) - pct(then);
  };
  const leadDelta = $derived(lead ? fortnightDelta(lead) : null);
  const aboveFloor = $derived(shown.filter((c) => c.status !== "faded").length);
  const evidenceCount = $derived(new Set(shown.flatMap((c) => c.evidence.map((e) => e.activityId))).size);
  const lastSupportMs = $derived(Math.max(...shown.map((c) => c.lastSupportedAtMs)));
  const lastSupport = $derived.by(() => {
    const r = relativeTime(lastSupportMs);
    const m = /^(\d+)(\D+)$/.exec(r);
    return m ? { n: m[1], unit: `${m[2]} ago` } : { n: r, unit: "" };
  });
  const lastSupportDay = $derived(
    new Date(lastSupportMs).toDateString() === new Date(now).toDateString() ? "today" : day(lastSupportMs),
  );

  // ── Selected conclusion ──
  const events = $derived(selected ? buildTimeline(selected, view?.trajectories.find((t) => t.conclusionId === selected.id), activities) : []);
  const selHistory = $derived(selected ? (histories.get(selected.id) ?? []) : []);
  const peak = $derived(selected ? Math.max(selected.confidence, ...selHistory.map((s) => s.confidence)) : 0);
  const selActivities = $derived(selected ? new Set(selected.evidence.map((e) => e.activityId)).size : 0);
  // Newest supporting Activity that resolved to a raw capture ref.
  const newestRef = $derived.by((): ActivityEvidenceRef | null => {
    let best: Activity | null = null;
    for (const e of selected?.evidence ?? []) {
      const a = activities.get(e.activityId);
      if (e.stance === "support" && a?.evidence[0] && (!best || a.startedAtMs > best.startedAtMs)) best = a;
    }
    return best?.evidence[0] ?? null;
  });
  function evidenceNote(a: Activity): string {
    const frames = a.evidence.filter((r) => r.subjectType === "frame").length;
    // B6: the activity's clock time, not a transcript offset Mnema can't pin.
    const parts = [formatClock(a.startedAtMs), fmtDuration(a.endedAtMs - a.startedAtMs)];
    if (frames) parts.push(`cited ${frames} ${frames === 1 ? "frame" : "frames"}`);
    return parts.join(" · ");
  }

  // ── Ask ──
  const askHref = $derived(`/chat?q=${encodeURIComponent(`What do you know about ${subject}?`)}`);
  // ⌘J asks about this subject (capture phase: ahead of the shell's ⌘J).
  function onKeydown(event: KeyboardEvent): void {
    if (askDisabled || frameModalOpen) return;
    if (event.key.toLowerCase() !== "j" || !(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
    event.preventDefault();
    void goto(askHref);
  }

  async function loadSubject(): Promise<void> {
    loading = true;
    try {
      const next = await invoke<SubjectView>("get_user_context_subject", { subject });
      view = next;
      loadError = null;
      void loadActivities(next);
    } catch (error) {
      // A failed background refresh keeps what's on screen (SD-02).
      if (view) showToast({ tone: "danger", text: `Couldn't refresh ${subject} — ${humanizeError(error)}` });
      else loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  // Best-effort paged scan of recent Activities; rows whose Activity doesn't
  // resolve stay plain text (SD-04).
  async function loadActivities(v: SubjectView): Promise<void> {
    const wanted = new Set(v.conclusions.flatMap((c) => c.evidence.map((e) => e.activityId)));
    if (wanted.size === 0) return;
    const resolved = new Map<number, Activity>();
    const PAGE = 200;
    for (let page = 0; page < 6; page++) {
      const batch = await invoke<Activity[]>("list_user_context_activities", { limit: PAGE, offset: page * PAGE }).catch(
        () => [] as Activity[],
      );
      for (const a of batch) if (wanted.has(a.id)) resolved.set(a.id, a);
      if (batch.length < PAGE || resolved.size >= wanted.size) break;
    }
    activities = resolved;
  }

  async function togglePin(c: Conclusion): Promise<void> {
    if (pinBusy) return;
    pinBusy = true;
    const pinned = !c.pinned;
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned });
      if (pinned) showToast({ tone: "ok", text: "Pinned — it won't fade" });
      await loadSubject();
    } catch (error) {
      showToast({ tone: "danger", text: `Couldn't ${pinned ? "pin" : "unpin"} — ${humanizeError(error)}` });
    } finally {
      pinBusy = false;
    }
  }

  function dismiss(c: Conclusion): void {
    dismissal.start(c);
    showToast({ tone: "ok", text: "Dismissed — it won't be re-inferred", undo: c });
  }

  function undoDismiss(c: Conclusion): void {
    dismissal.undo(c);
    selectedId = c.id;
    clearTimeout(toastTimer);
    toast = null;
  }

  // In-place frame peek; audio refs hand off to the Timeline.
  let frameModalOpen = $state(false);
  let frameModalId = $state<number | null>(null);
  let frameModalOpenInTimeline = $state<(() => void) | null>(null);

  function onViewInTimeline(activityId: number): void {
    const ref = activities.get(activityId)?.evidence[0];
    if (ref?.subjectType === "frame") {
      frameModalId = ref.subjectId;
      frameModalOpenInTimeline = () => void openRefInTimeline(ref);
      frameModalOpen = true;
      return;
    }
    void openRefInTimeline(ref);
  }

  async function openRefInTimeline(ref: ActivityEvidenceRef | undefined): Promise<void> {
    try {
      if (ref?.subjectType === "audio_segment") {
        // spanStartMs is an offset WITHIN the segment; null = its start.
        await invoke("open_capture_result_in_main_window", {
          kind: "audio",
          frameId: null,
          audioSegmentId: ref.subjectId,
          spanStartMs: null,
          alignedFrameId: null,
        });
        return;
      }
      if (ref?.subjectType === "frame") {
        await invoke("open_capture_result_in_main_window", { kind: "frame", frameId: ref.subjectId, audioSegmentId: null });
        return;
      }
    } catch {
      // fall through
    }
    showToast({ tone: "danger", text: "Couldn't open that moment in the Timeline" });
  }

  $effect(() => {
    subject;
    void loadSubject();
    // `user_context_changed` fires per derivation pass; coalesce bursts.
    const debouncedReload = debounce(() => void loadSubject(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => debouncedReload()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      debouncedReload.cancel();
    };
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<section aria-label={`Subject — ${subject}`}>
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
      ><IconSparkle class="mx-btn__accent" width="13" height="13" aria-hidden="true" />Ask about {subject}<kbd>⌘J</kbd></a
    >
  </div>

  {#if loadError && !view}
    <div class="mx-empty mx-empty--center" data-tone="danger" role="alert">
      <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">Couldn't load this subject.</h4>
      <p class="mx-empty__text">Reading its conclusions and history failed. {loadError}</p>
      <div class="mx-empty__acts">
        <button type="button" class="mx-btn mx-btn--sm" disabled={loading} onclick={() => void loadSubject()}>
          <IconRetry width="13" height="13" aria-hidden="true" />Try again
        </button>
      </div>
    </div>
  {:else if !view}
    <div aria-busy="true">
      <span class="mx-sr">Loading {subject}</span>
      <div class="in-read">
        <div class="in-skel-read">
          <div class="mx-skel" style="width:140px;height:9px"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
        </div>
        <div class="in-skel-stats">
          {#each Array.from({ length: 4 }) as _, i (i)}<div><div class="mx-skel"></div><div class="mx-skel"></div></div>{/each}
        </div>
      </div>
      <div class="in-ex">
        <div class="mx-panel in-week"><div class="mx-skel" style="height:230px"></div></div>
        <div>
          {#each Array.from({ length: 4 }) as _, i (i)}
            <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
          {/each}
        </div>
      </div>
    </div>
  {:else if !selected || !lead}
    <div class="mx-empty mx-empty--center">
      <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
      <h4 class="mx-empty__title">Nothing concluded about {subject} yet.</h4>
      <p class="mx-empty__text">
        Its conclusions were dismissed or haven't formed. Views appear here once two activities support them. Anything
        you dismissed is in Context › Dismissed.
      </p>
    </div>
  {:else}
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
          <span class="mx-stat__value">{shown.length}</span>
          <span class="mx-stat__delta">{aboveFloor} above the display floor</span>
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

    <div class="in-ex">
      <div class="mx-panel in-week mx-reveal" style="--i:3">
        <div class="in-sec">
          <span class="mx-kicker">Confidence over time</span><span class="mx-spacer"></span>
          <span class="mx-label">selected conclusion · others in grey · ● evidence</span>
        </div>
        <SubjectChart conclusions={shown} {histories} selectedId={selected.id} />
      </div>
      <div class="mx-reveal" style="--i:4">
        <div class="in-sec">
          <span class="mx-kicker">Conclusions</span><span class="mx-spacer"></span>
          <span class="mx-label">{shown.length} · by confidence</span>
        </div>
        <div class="in-cls">
          {#each shown as c (c.id)}
            {@const d = fortnightDelta(c)}
            <button type="button" class="in-cl" aria-pressed={c.id === selected.id} onclick={() => (selectedId = c.id)}>
              <span class="num">{pct(c.confidence)}%</span>
              <span>{c.statement}</span>
              <small class:in-up={(d ?? 0) > 0} class:in-down={(d ?? 0) < 0}>{d === null ? "new" : signed(d)}</small>
            </button>
          {/each}
        </div>
      </div>
    </div>

    <div class="in-feed mx-reveal" style="--i:5">
      <div>
        <span class="mx-kicker">Why Mnema believes this · {selIndex + 1} of {shown.length}</span>
        <h2 class="in-cd__statement">{selected.statement}</h2>
        <div class="in-cd__acts">
          <button
            type="button"
            class="mx-btn mx-btn--sm"
            aria-pressed={selected.pinned}
            aria-busy={pinBusy || undefined}
            onclick={() => void togglePin(selected)}
            ><IconPin width="13" height="13" aria-hidden="true" />{selected.pinned ? "Pinned" : "Pin"}</button
          >
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => dismiss(selected)}>Dismiss</button>
        </div>
        <ol class="in-tl">
          {#each events as ev, i (i)}
            {@const act = ev.kind === "evidence" || ev.kind === "contradict" ? activities.get(ev.activityId) : undefined}
            <li class:origin={ev.kind === "formed"} class:conf={ev.kind === "marker"} style:--i={i}>
              <time class="num">{ev.atMs == null ? "—" : day(ev.atMs)}</time>
              {#if ev.kind === "formed"}
                {@const n = selected.formedEvidenceCount}
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
                <button type="button" class="in-tl__ev" onclick={() => onViewInTimeline(ev.activityId)}>
                  {ev.title}<small>{ev.kind === "contradict" ? "contradicts" : evidenceNote(act)}</small>
                </button>
              {:else}
                <div>{ev.title}{#if ev.kind === "contradict"}<small>contradicts</small>{/if}</div>
              {/if}
            </li>
          {/each}
        </ol>
      </div>
      <div class="mx-panel mx-panel--inset in-guard">
        <div class="in-sec"><span class="mx-kicker">This conclusion</span></div>
        <dl class="mx-spec">
          <div><dt>confidence</dt><dd class="num">{pct(selected.confidence)}%</dd></div>
          <div><dt>peak</dt><dd class="num">{pct(peak)}%</dd></div>
          <div><dt>evidence</dt><dd>{selActivities} {selActivities === 1 ? "activity" : "activities"}</dd></div>
          <div><dt>decay</dt><dd>halves every 30 days without support</dd></div>
          <div><dt>pinned</dt><dd>{selected.pinned ? "yes — it won't fade" : "no — pin it to stop it fading"}</dd></div>
        </dl>
        {#if newestRef}
          {@const ref = newestRef}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm open-tl" onclick={() => void openRefInTimeline(ref)}
            >Open evidence in Timeline<IconArrow width="13" height="13" aria-hidden="true" /></button
          >
        {/if}
      </div>
    </div>
  {/if}
</section>

{#if toast}
  {@const t = toast}
  <div class="mx-toasts">
    <div class="mx-toast" data-tone={t.tone} role={t.tone === "danger" ? "alert" : "status"}>
      {#if t.tone === "ok"}<IconOk width="15" height="15" aria-hidden="true" />{:else}<IconFail width="15" height="15" aria-hidden="true" />{/if}
      <span>{t.text}</span>
      {#if t.undo}
        {@const c = t.undo}
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => undoDismiss(c)}>Undo</button>
        <i class="mx-toast__timer" style="--ttl:{DISMISS_UNDO_MS}ms"></i>
      {/if}
    </div>
  </div>
{/if}

<FrameDetailModal
  open={frameModalOpen}
  frameId={frameModalId}
  onClose={() => (frameModalOpen = false)}
  onOpenInTimeline={frameModalOpenInTimeline ?? undefined}
/>

<style>
  /* insights.html Subject detail + the shared lede/exhibit/feed layout. */
  .in-crumb { display: flex; align-items: center; gap: var(--s-2); margin: 0 0 var(--s-3) -9px; font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle); }
  .in-crumb b { color: var(--app-text-strong); font-weight: 500; }
  .in-crumb > a:last-child { margin-right: -9px; }
  .in-read { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s-7); align-items: end; padding: var(--s-4) 0 var(--s-6); margin-bottom: var(--s-6); border-bottom: 1px solid var(--mx-hairline); }
  .in-read .mx-display { margin: var(--s-4) 0; max-width: 24ch; }
  .lede { font-size: var(--text-lg); }
  .lede span { color: var(--app-text-subtle); }
  .in-lede-stats { display: grid; grid-template-columns: repeat(2, minmax(132px, auto)); gap: var(--s-5) var(--s-6); padding-bottom: 4px; }
  .in-ex { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr); gap: var(--s-6); margin-bottom: var(--s-7); }
  .in-feed { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr); gap: var(--s-6); align-items: start; }
  .in-sec { display: flex; align-items: baseline; gap: var(--s-3); margin: 0 0 var(--s-3); }
  .in-week { padding: var(--s-5); }
  .in-guard { padding: var(--s-4); font-size: var(--text-sm); color: var(--app-text-muted); line-height: 1.55; }
  .in-guard .mx-spec { grid-template-columns: 96px 1fr; }
  .open-tl { margin: var(--s-3) 0 0 -9px; }
  .in-up { color: var(--focus-deep); }
  .in-down { color: var(--focus-distracted); }

  .in-cls { display: grid; gap: 2px; }
  .in-cl {
    display: grid; grid-template-columns: 44px minmax(0, 1fr) 44px; gap: var(--s-3); align-items: start; padding: 11px var(--s-3);
    border: 0; border-radius: var(--r-sm); background: none; cursor: pointer; text-align: left; color: var(--app-text-muted);
    font: 400 var(--text-base) / 1.45 var(--font-sans);
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .in-cl:hover { background: var(--mx-wash); color: var(--app-text); }
  .in-cl:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
  .in-cl[aria-pressed="true"] { background: var(--mx-selected); color: var(--app-text-strong); }
  .in-cl .num { font-size: var(--text-md); color: var(--app-text-strong); line-height: 1.3; }
  .in-cl small { text-align: right; font: 500 var(--text-xs) / 1.9 var(--font-mono); }

  .in-cd__statement { font: 600 24px / 1.25 var(--font-sans); letter-spacing: -0.02em; color: var(--app-text-strong); margin: var(--s-3) 0 var(--s-4); max-width: 34ch; text-wrap: balance; }
  .in-cd__acts { display: flex; gap: 6px; }
  .in-tl { list-style: none; margin: var(--s-5) 0 0; padding: 0; position: relative; }
  .in-tl::before { content: ""; position: absolute; left: 5px; top: 6px; bottom: 6px; width: 1px; background: var(--app-border); }
  .in-tl li { position: relative; display: grid; grid-template-columns: 22px 72px 1fr; padding: 9px 0; animation: mx-rise var(--t-slow) var(--ease-expo) both; animation-delay: calc(var(--i) * 40ms); }
  .in-tl li::before { content: ""; width: 11px; height: 11px; border-radius: 50%; margin-top: 3px; background: var(--app-bg); border: 1.5px solid var(--app-border-strong); }
  .in-tl li.conf::before { border-color: var(--app-text-strong); background: var(--app-text-strong); width: 7px; height: 7px; margin: 5px 0 0 2px; }
  .in-tl li.origin::before { border-color: var(--app-accent); background: var(--app-accent); box-shadow: 0 0 0 3px var(--app-accent-glow); }
  .in-tl time { font-size: var(--text-xs); color: var(--app-text-subtle); line-height: 1.9; }
  .in-tl li > div, .in-tl__ev { color: var(--app-text); font-size: var(--text-base); }
  .in-tl small { display: block; margin-top: 3px; font: 400 var(--text-sm) / 1.4 var(--font-sans); color: var(--app-text-subtle); }
  .in-tl li.conf > div { font: 500 var(--text-sm) / 1.6 var(--font-mono); font-variant-numeric: tabular-nums; color: var(--app-text-strong); }
  .in-tl__ev { padding: 0; border: 0; background: none; text-align: left; font-family: inherit; line-height: inherit; cursor: pointer; border-radius: var(--r-xs); }
  .in-tl__ev:hover { color: var(--app-text-strong); }
  .in-tl__ev:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }

  .in-skel-read { display: grid; gap: 12px; padding-top: var(--s-4); }
  .in-skel-read .mx-skel--title { height: 34px; width: 78%; }
  .in-skel-read .mx-skel--title + .mx-skel--title { width: 52%; }
  .in-skel-stats { display: grid; grid-template-columns: repeat(2, 132px); gap: var(--s-5) var(--s-6); }
  .in-skel-stats div { display: grid; gap: 8px; }
  .in-skel-stats .mx-skel:first-child { height: 8px; width: 60%; }
  .in-skel-stats .mx-skel:last-child { height: 24px; width: 90%; }

  /* `.in-main` scrolls, so the toast stack pins to the window. */
  .mx-toasts { position: fixed; }

  @container mx-window (max-width: 1180px) {
    .in-ex, .in-feed { grid-template-columns: 1fr; }
  }
  @media (prefers-reduced-motion: reduce) {
    .in-tl li { animation: none; }
  }
</style>
