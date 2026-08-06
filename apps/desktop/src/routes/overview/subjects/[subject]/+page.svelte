<script lang="ts">
  // ══ ONE SUBJECT, OPEN ══════════════════════════════════════════════════════
  //
  // The drill-in under Subjects (`/overview/subjects/<encodeURIComponent(name)>`).
  // The subject name is the path segment, so the destination is addressable and
  // the titlebar's back trail needs no special case — `+layout.svelte` owns it.
  //
  // Same instrument as the tier list, at hero size: the selected conclusion's
  // own confidence history, FILLED (at this size the area carries the shape
  // better than a hairline), with the engine's 0.15 floor still printed and the
  // x-axis labelled in real time. Under it the subject's other conclusions as a
  // sortable strip, and under that the story that produced the number.
  //
  // TWO corrections exist, both per-conclusion: Pin (exempt from decay) and
  // Dismiss. There is deliberately NO edit — the engine supersedes a belief and
  // keeps the retired wording on the timeline, so an edit box would promise
  // something the store cannot keep — and no per-subject forget, because
  // dismissal is per-conclusion and the subject would re-form on fresh evidence
  // anyway.
  //
  // G8: every chip, percent and caption below is a real read. A conclusion with
  // fewer than two snapshots draws no trace and no "rose X → Y" caption.
  import { untrack } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { toast } from "$lib/toast.svelte";
  import type {
    Activity,
    ActivityEvidenceRef,
    Conclusion,
    SubjectTrajectory,
    SubjectView,
  } from "$lib/types/recording";
  import { DISPLAY_FLOOR, debounce } from "$lib/insights/subjectsTiers";
  import {
    buildTimeline,
    sortConclusions,
    type ConclusionSort,
  } from "$lib/insights/subjectTimeline";
  import { humanizeError } from "$lib/format-error";
  import ConfidenceTrace from "$lib/overview/ConfidenceTrace.svelte";
  import ConclusionCard from "$lib/overview/subjects/ConclusionCard.svelte";
  import StoryTimeline from "$lib/overview/subjects/StoryTimeline.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { agoLabel, pct, pctLabel, spanLabel } from "$lib/overview/subjects/format";

  const subject = $derived(decodeURIComponent($page.params.subject ?? ""));

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let selectedId = $state<number | null>(null);
  let sort = $state<ConclusionSort>("confidence");
  let activities = $state(new Map<number, Activity>());
  // One in-flight correction at a time, so the pair of buttons can't race.
  let busy = $state<"pin" | "dismiss" | null>(null);

  const SORT_OPTIONS = [
    { value: "confidence", label: "Confidence" },
    { value: "recent", label: "Recent" },
    { value: "warming", label: "Warming" },
  ];

  const trajectoryById = $derived.by(() => {
    const m = new Map<number, SubjectTrajectory>();
    if (view) for (const t of view.trajectories) m.set(t.conclusionId, t);
    return m;
  });
  const ordered = $derived(
    view ? sortConclusions(view.conclusions, trajectoryById, sort) : [],
  );
  const selected = $derived<Conclusion | null>(
    view && selectedId !== null
      ? (view.conclusions.find((c) => c.id === selectedId) ?? null)
      : null,
  );
  const history = $derived(
    selectedId === null ? [] : (trajectoryById.get(selectedId)?.history ?? []),
  );
  const events = $derived(
    selected ? buildTimeline(selected, trajectoryById.get(selected.id), activities) : [],
  );

  // ── the hero's facts ─────────────────────────────────────────────────────
  const conclusionCount = $derived(view?.conclusions.length ?? 0);
  const belowFloor = $derived(
    view?.conclusions.filter((c) => c.status === "faded").length ?? 0,
  );
  const firstSeen = $derived(
    view?.conclusions.length
      ? agoLabel(Math.min(...view.conclusions.map((c) => c.formedAtMs)))
      : null,
  );
  const lastEvidence = $derived(
    view?.conclusions.length
      ? agoLabel(Math.max(...view.conclusions.map((c) => c.lastSupportedAtMs)))
      : null,
  );
  const linkedActivities = $derived.by(() => {
    if (!view) return 0;
    const ids = new Set<number>();
    for (const c of view.conclusions) for (const e of c.evidence) ids.add(e.activityId);
    return ids.size;
  });
  const pinned = $derived(view?.conclusions.some((c) => c.pinned) ?? false);

  // The x-axis, written out: "6 WEEKS AGO … FLOOR 15% … NOW · 86%". It uses the
  // same span vocabulary as the caption below it, so the two can't disagree
  // about the same distance ("1mo ago" over "6 weeks").
  const axisStart = $derived.by(() => {
    if (history.length < 2) return null;
    const span = spanLabel(history[0].snapshotAtMs, Date.now());
    return span ? `${span} ago` : null;
  });
  // "31 snapshots · rose 54% → 86% over 6 weeks" — the verb comes from the real
  // endpoints, and the span from real timestamps.
  const traceCaption = $derived.by(() => {
    if (history.length < 2) return null;
    const first = history[0];
    const last = history[history.length - 1];
    const from = pct(first.confidence);
    const to = pct(last.confidence);
    const span = spanLabel(first.snapshotAtMs, last.snapshotAtMs);
    const snaps = `${history.length} snapshot${history.length === 1 ? "" : "s"}`;
    const verb = from === to ? "held near" : to > from ? "rose" : "fell";
    const move = from === to ? `held near ${to}%` : `${verb} ${from}% → ${to}%`;
    return span ? `${snaps} · ${move} over ${span}` : `${snaps} · ${move}`;
  });

  // ── reads ────────────────────────────────────────────────────────────────
  async function loadSubject(): Promise<void> {
    const name = untrack(() => subject);
    if (!name) return;
    try {
      const next = await invoke<SubjectView>("get_user_context_subject", { subject: name });
      view = next;
      loadError = null;
      const held = untrack(() => selectedId);
      if (held === null || !next.conclusions.some((c) => c.id === held)) {
        selectedId =
          [...next.conclusions].sort((a, b) => b.confidence - a.confidence)[0]?.id ?? null;
      }
      void loadActivities(next);
    } catch (error) {
      // A failed refresh keeps what is on screen; only a bare surface errors.
      if (!untrack(() => view)) loadError = humanizeError(error);
    }
  }

  // Resolve the Activities this subject's conclusions cite, so the story shows
  // real titles/times/source types and can hand off to the raw Timeline.
  // Best-effort bounded scan: an unresolved ref simply keeps its stored title.
  async function loadActivities(v: SubjectView): Promise<void> {
    const wanted = new Set<number>();
    for (const c of v.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
    if (wanted.size === 0) return;
    const resolved = new Map<number, Activity>();
    const PAGE = 200;
    const MAX_PAGES = 6;
    for (let page = 0; page < MAX_PAGES; page++) {
      let batch: Activity[];
      try {
        batch = await invoke<Activity[]>("list_user_context_activities", {
          limit: PAGE,
          offset: page * PAGE,
        });
      } catch {
        break;
      }
      if (batch.length === 0) break;
      for (const a of batch) if (wanted.has(a.id)) resolved.set(a.id, a);
      if (resolved.size >= wanted.size || batch.length < PAGE) break;
    }
    activities = resolved;
  }

  // ── the two corrections ──────────────────────────────────────────────────
  async function togglePin(): Promise<void> {
    const c = selected;
    if (!c || busy) return;
    busy = "pin";
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      await loadSubject();
    } catch (error) {
      // A write failure must never blank the surface.
      toast({
        tone: "error",
        title: c.pinned ? "Couldn't unpin conclusion" : "Couldn't pin conclusion",
        message: humanizeError(error),
      });
    } finally {
      busy = null;
    }
  }

  async function dismiss(): Promise<void> {
    const c = selected;
    if (!c || busy) return;
    const ok = await confirm(
      `Dismiss “${c.statement}”? Mnema stops using this conclusion; the subject can still re-form on fresh evidence.`,
      { title: "Dismiss conclusion", kind: "warning" },
    );
    if (!ok) return;
    busy = "dismiss";
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
      selectedId = null;
      await loadSubject();
    } catch (error) {
      toast({
        tone: "error",
        title: "Couldn't dismiss conclusion",
        message: humanizeError(error),
      });
    } finally {
      busy = null;
    }
  }

  // Ask AI about this subject. The Quick Access summon takes no seed today, so
  // this opens the door rather than pretending to prefill the question.
  async function askAi(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // Best-effort: the global shortcut stays the summon path.
    }
  }

  // Hand the moment off to the raw Timeline — a frame opens as a frame, an
  // audio segment as audio, and anything unresolvable falls back to Timeline.
  async function openActivity(activityId: number): Promise<void> {
    const ref: ActivityEvidenceRef | undefined = activities.get(activityId)?.evidence?.[0];
    try {
      if (ref?.subjectType === "audio_segment") {
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
        await invoke("open_capture_result_in_main_window", {
          kind: "frame",
          frameId: ref.subjectId,
          audioSegmentId: null,
        });
        return;
      }
    } catch {
      // fall through
    }
    toast({ title: "Opening Timeline", message: "Couldn't pinpoint the exact moment." });
    void goto("/");
  }

  $effect(() => {
    subject;
    void loadSubject();
    // The engine writes per derivation pass; coalesce the burst into one reload.
    const debounced = debounce(() => void loadSubject(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => debounced()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      debounced.cancel();
    };
  });
</script>

<div class="dest">
  <header class="dest__bar">
    <span class="t-title">{subject}</span>
    {#if pinned}<span class="ti-chip ti-chip--acc">★ pinned</span>{/if}
  </header>

  <div class="dest__pane">
    <div class="scol">
      {#if loadError && !view}
        <div class="estate">
          <span class="t-ui estate__t">Couldn't load this subject.</span>
          <span class="t-meta">{loadError}</span>
          <button type="button" class="btn btn--sm estate__go" onclick={() => void loadSubject()}>
            Try again
          </button>
        </div>
      {:else if view && conclusionCount === 0}
        <div class="estate">
          <span class="t-ui estate__t">Nothing held about {subject}.</span>
          <span class="t-meta">
            Every conclusion on this subject has been dismissed. It can re-form as
            fresh evidence arrives.
          </span>
        </div>
      {:else if view && selected}
        <div class="hero">
          <div class="hero__txt">
            <p class="t-read hero__stmt">{selected.statement}</p>
            <div class="pills">
              <span class="ti-chip is-num">
                {conclusionCount} conclusion{conclusionCount === 1 ? "" : "s"}
              </span>
              {#if belowFloor > 0}
                <span class="ti-chip is-num">{belowFloor} below floor</span>
              {/if}
              {#if firstSeen}<span class="ti-chip">first seen {firstSeen}</span>{/if}
              {#if lastEvidence}<span class="ti-chip">last evidence {lastEvidence}</span>{/if}
              {#if linkedActivities > 0}
                <span class="ti-chip is-num">{linkedActivities} linked activities</span>
              {/if}
            </div>
            <div class="hero__actions">
              <button type="button" class="btn btn--sm btn--primary" onclick={() => void askAi()}>
                Ask AI about {subject}
              </button>
              <button
                type="button"
                class="btn btn--sm"
                class:btn--accent={selected.pinned}
                disabled={busy !== null}
                onclick={() => void togglePin()}
              >
                {selected.pinned ? "★ Pinned — protected from decay" : "☆ Pin — protect from decay"}
              </button>
              <button
                type="button"
                class="btn btn--sm btn--ghost"
                disabled={busy !== null}
                onclick={() => void dismiss()}
              >
                Dismiss
              </button>
            </div>
          </div>
          <div class="hero__trace">
            {#if history.length >= 2}
              <ConfidenceTrace
                {history}
                size="hero"
                label="Confidence over time for {selected.statement}"
              />
              <div class="ti-gauge__scale hero__scale">
                <span>{axisStart ?? ""}</span>
                <span>FLOOR {Math.round(DISPLAY_FLOOR * 100)}%</span>
                <span>NOW · {pctLabel(selected.confidence)}</span>
              </div>
              {#if traceCaption}<p class="t-meta hero__cap is-num">{traceCaption}</p>{/if}
            {:else}
              <p class="t-meta hero__cap">
                One snapshot so far — the trace draws once this belief has moved at
                least twice.
              </p>
            {/if}
          </div>
        </div>

        <div class="sec">
          <span class="t-label">Conclusions</span>
          <span class="t-meta is-num">{conclusionCount}</span>
          <span class="sec__sp"></span>
          <Segmented
            options={SORT_OPTIONS}
            value={sort}
            onValueChange={(v) => (sort = v as ConclusionSort)}
            ariaLabel="Sort conclusions"
            compact
          />
        </div>
        <div class="strip2">
          {#each ordered as c (c.id)}
            <ConclusionCard
              conclusion={c}
              history={trajectoryById.get(c.id)?.history ?? []}
              selected={c.id === selectedId}
              onSelect={() => (selectedId = c.id)}
            />
          {/each}
        </div>

        <div class="sec sec--tl">
          <span class="t-label">The story over time</span>
          <span class="t-meta">newest first</span>
        </div>
        <StoryTimeline {events} {activities} onOpen={(id) => void openActivity(id)} />
      {/if}
    </div>
  </div>
</div>

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .dest__bar {
    flex: 0 0 auto;
    height: 40px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--s-16);
    box-shadow: inset 0 -1px 0 var(--app-border);
  }
  .dest__pane {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--s-20) var(--s-24);
  }
  .scol {
    max-width: 820px;
    margin: 0 auto;
  }

  .hero {
    display: flex;
    gap: var(--s-20);
    align-items: flex-start;
    padding: 0 var(--s-2) var(--s-12);
  }
  .hero__txt {
    flex: 1 1 auto;
    min-width: 0;
  }
  .hero__stmt {
    margin: 0;
    max-width: 58ch;
  }
  .hero__trace {
    flex: 0 0 300px;
  }
  .hero__scale {
    margin-top: 5px;
    gap: var(--s-8);
  }
  .hero__cap {
    margin: var(--s-6) 0 0;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-6);
    margin-top: var(--s-8);
  }
  .hero__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-8);
    margin-top: var(--s-12);
  }
  /* The size modifier carries height only, so the three actions need the meta
     type to sit on one line inside the reading column. */
  .hero__actions :global(.btn--sm) {
    font-size: var(--t-meta);
  }
  .hero__scale {
    text-transform: uppercase;
  }

  .sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    padding: var(--s-8) var(--s-2) var(--s-6);
  }
  .sec--tl {
    padding-top: var(--s-16);
  }
  .sec__sp {
    flex: 1 1 auto;
  }
  /* The strip scrolls sideways: a subject can hold more conclusions than fit,
     and wrapping them into a grid would bury the leading one. */
  .strip2 {
    display: flex;
    gap: var(--s-8);
    overflow-x: auto;
    padding-bottom: var(--s-4);
  }

  .estate {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    align-items: flex-start;
    max-width: 52ch;
    margin-top: var(--s-16);
    padding: var(--s-16);
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
  }
  .estate__t {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }
  .estate__go {
    margin-top: var(--s-4);
  }
</style>
