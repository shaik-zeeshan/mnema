<script lang="ts">
  // One subject (page 09, viewport B). A drill-in over local state on the same
  // route — the app is a static-adapter SPA, so there is no dynamic segment.
  //
  // Three registers, top to bottom: hero counts (all real joins over the
  // conclusions this subject holds), the conclusion strip ranked by the chosen
  // key, then the spine — the story over time, evidence events interleaved with
  // the confidence markers between them, ending at "formed".
  //
  // Every event on the spine comes from `buildTimeline` (subjectTimeline.ts);
  // nothing here invents a causal link the engine did not record.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { toast } from "$lib/toast.svelte";
  import { humanizeError } from "$lib/format-error";
  import { resetDeck, setDeck } from "$lib/deck.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import Spine from "./Spine.svelte";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import { DISPLAY_FLOOR, debounce } from "$lib/insights/subjectsTiers";
  import {
    buildTimeline,
    sortConclusions,
    type ConclusionSort,
  } from "$lib/insights/subjectTimeline";
  import type {
    Activity,
    Conclusion,
    ConfidenceSnapshot,
    SubjectTrajectory,
    SubjectView,
  } from "$lib/types/recording";
  import {
    clockLabel,
    dismissConclusion,
    fetchSubject,
    loadFramePreviews,
    openRefInTimeline,
    pct,
    relativeTime,
    resolveActivities,
    setPinned,
    spanLabel,
  } from "./data";

  interface Props {
    subject: string;
    onBack: () => void;
  }

  let { subject, onBack }: Props = $props();

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  let sort = $state<ConclusionSort>("confidence");
  let acting = $state<number | null>(null);
  let activities = $state<Map<number, Activity>>(new Map());
  let previews = $state<Map<number, string>>(new Map());

  let frameOpen = $state(false);
  let frameId = $state<number | null>(null);
  let frameFallback = $state<(() => void) | null>(null);

  const trajectoryById = $derived.by(() => {
    const m = new Map<number, SubjectTrajectory>();
    for (const t of view?.trajectories ?? []) m.set(t.conclusionId, t);
    return m;
  });
  const cards = $derived(
    sortConclusions(view?.conclusions ?? [], trajectoryById, sort),
  );
  const selected = $derived(cards.find((c) => c.id === selectedId) ?? cards[0] ?? null);
  const events = $derived(
    selected ? buildTimeline(selected, trajectoryById.get(selected.id), activities) : [],
  );

  // ---- hero counts: real joins, nothing invented -----------------------------
  const conclusionCount = $derived(view?.conclusions.length ?? 0);
  const belowFloor = $derived(
    view?.conclusions.filter((c) => c.confidence < DISPLAY_FLOOR).length ?? 0,
  );
  const firstSeenMs = $derived(
    view?.conclusions.length ? Math.min(...view.conclusions.map((c) => c.formedAtMs)) : 0,
  );
  const lastEvidenceMs = $derived(
    view?.conclusions.length
      ? Math.max(...view.conclusions.map((c) => c.lastSupportedAtMs))
      : 0,
  );
  const linkedActivities = $derived.by(() => {
    const ids = new Set<number>();
    for (const c of view?.conclusions ?? []) for (const e of c.evidence) ids.add(e.activityId);
    return ids.size;
  });

  // ---- per-card trajectory readout ------------------------------------------
  interface CardRead {
    snapshots: number;
    word: string;
    tone: "up" | "down" | "flat" | "floor";
    range: string;
  }

  function readCard(c: Conclusion): CardRead {
    const history: ConfidenceSnapshot[] = trajectoryById.get(c.id)?.history ?? [];
    const snapshots = history.length;
    const below = c.confidence < DISPLAY_FLOOR;
    if (history.length < 2) {
      return {
        snapshots,
        word: below ? "below floor" : "– steady",
        tone: below ? "floor" : "flat",
        range: below
          ? `${pct(c.confidence)}% · below floor`
          : `no movement recorded yet · ${relativeTime(c.lastSupportedAtMs)}`,
      };
    }
    const first = history[0];
    const last = history[history.length - 1];
    const delta = last.confidence - first.confidence;
    if (below) {
      return {
        snapshots,
        word: "below floor",
        tone: "floor",
        range: `${pct(first.confidence)} → ${pct(last.confidence)} · below floor`,
      };
    }
    if (delta > 0.04) {
      return {
        snapshots,
        word: "↑ rising",
        tone: "up",
        range: `rose ${pct(first.confidence)} → ${pct(last.confidence)} · over ${spanLabel(first.snapshotAtMs, last.snapshotAtMs)}`,
      };
    }
    if (delta < -0.04) {
      return {
        snapshots,
        word: "↓ cooling",
        tone: "down",
        range: `${pct(first.confidence)} → ${pct(last.confidence)} · ${relativeTime(last.snapshotAtMs)}`,
      };
    }
    return {
      snapshots,
      word: "– steady",
      tone: "flat",
      range: `steady near ${pct(last.confidence)} · ${relativeTime(last.snapshotAtMs)}`,
    };
  }

  // ---- loading ---------------------------------------------------------------

  async function load(): Promise<void> {
    try {
      const next = await fetchSubject(subject);
      view = next;
      loadError = null;
      if (!next.conclusions.some((c) => c.id === selectedId)) {
        selectedId =
          [...next.conclusions].sort((a, b) => b.confidence - a.confidence)[0]?.id ?? null;
      }
      const wanted = new Set<number>();
      for (const c of next.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
      activities = await resolveActivities(wanted);
    } catch (error) {
      loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    subject;
    untrack(() => void load());
    const reload = debounce(() => void load(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => reload()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      reload.cancel();
    };
  });

  // Spine thumbnails, best-effort — an event without one keeps its empty box.
  $effect(() => {
    const wanted = events
      .map((e) => {
        if (e.kind === "evidence") return e.frameId;
        if (e.kind !== "contradict") return null;
        const raw = activities.get(e.activityId)?.evidence?.[0];
        return raw?.subjectType === "frame" ? raw.subjectId : null;
      })
      .filter((id): id is number => id !== null && !untrack(() => previews).has(id));
    if (wanted.length === 0) return;
    void loadFramePreviews([...new Set(wanted)]).then((loaded) => {
      if (loaded.size > 0) previews = new Map([...untrack(() => previews), ...loaded]);
    });
  });

  $effect(() => {
    setDeck({
      context: `${subject} · ${conclusionCount} ${conclusionCount === 1 ? "conclusion" : "conclusions"} · ${linkedActivities} linked ${linkedActivities === 1 ? "activity" : "activities"}`,
      hints: [
        { keys: "↑↓", label: "Move" },
        { keys: "⌘P", label: "Pin" },
        { keys: "⌘⌫", label: "Dismiss" },
        { keys: "esc", label: "Back to Subjects", separator: true },
      ],
    });
    return resetDeck;
  });

  // ---- actions ---------------------------------------------------------------

  async function pin(c: Conclusion): Promise<void> {
    if (acting !== null) return;
    acting = c.id;
    try {
      await setPinned(c.id, !c.pinned);
      await load();
    } catch (error) {
      toast({
        tone: "error",
        title: c.pinned ? "Couldn't unpin conclusion" : "Couldn't pin conclusion",
        message: humanizeError(error),
      });
    } finally {
      acting = null;
    }
  }

  async function dismiss(c: Conclusion): Promise<void> {
    if (acting !== null) return;
    acting = c.id;
    try {
      await dismissConclusion(c.id);
      await load();
    } catch (error) {
      toast({
        tone: "error",
        title: "Couldn't dismiss conclusion",
        message: humanizeError(error),
      });
    } finally {
      acting = null;
    }
  }

  function askInQuickAccess(): void {
    void invoke("summon_quick_recall_window_command").catch(() => {});
  }

  function viewActivity(activityId: number): void {
    const ref = activities.get(activityId)?.evidence?.[0];
    if (ref?.subjectType === "frame") {
      frameId = ref.subjectId;
      frameFallback = () => void openRefInTimeline(ref);
      frameOpen = true;
      return;
    }
    void openRefInTimeline(ref);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      onBack();
      return;
    }
    if (event.metaKey && event.key === "Enter") {
      event.preventDefault();
      askInQuickAccess();
      return;
    }
    if (event.metaKey && event.key.toLowerCase() === "p") {
      event.preventDefault();
      if (selected) void pin(selected);
      return;
    }
    if (event.metaKey && (event.key === "Backspace" || event.key === "Delete")) {
      event.preventDefault();
      if (selected) void dismiss(selected);
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    if (cards.length === 0) return;
    event.preventDefault();
    const at = cards.findIndex((c) => c.id === selected?.id);
    const next = event.key === "ArrowDown" ? at + 1 : at - 1;
    selectedId = cards[Math.max(0, Math.min(next, cards.length - 1))].id;
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="det">
  {#if loadError && !view}
    <div class="det__state">
      <p class="t-ui">Couldn't load this subject.</p>
      <p class="t-meta">{loadError}</p>
      <button type="button" class="btn btn--sm" onclick={() => void load()}>Try again</button>
    </div>
  {:else if loading && !view}
    <p class="det__state t-meta">Loading {subject}…</p>
  {:else}
    <div class="det__hero">
      <div class="det__id">
        <p class="t-title det__ttl">{subject}</p>
        <div class="pills">
          <span class="pillx"><b>{conclusionCount}</b>{conclusionCount === 1 ? "conclusion" : "conclusions"}</span>
          {#if belowFloor > 0}
            <span class="pillx"><b>{belowFloor}</b>below floor</span>
          {/if}
          <span class="pillx">first seen <b class="pillx__mid">{relativeTime(firstSeenMs)}</b></span>
          <span class="pillx">last evidence <b class="pillx__mid">{relativeTime(lastEvidenceMs)}</b></span>
          <span class="pillx"><b>{linkedActivities}</b>linked {linkedActivities === 1 ? "activity" : "activities"}</span>
        </div>
      </div>
      <button type="button" class="btn btn--sm" onclick={askInQuickAccess}>
        <svg class="det__spark" viewBox="0 0 16 16" fill="none" stroke="currentColor"
          stroke-width="1.4" stroke-linejoin="round" aria-hidden="true">
          <path d="M8 1.8 9.5 6 13.8 7.5 9.5 9 8 13.2 6.5 9 2.2 7.5 6.5 6z" />
        </svg>
        Ask about this in Quick Access <span class="kbd">⌘⏎</span>
      </button>
      <Segmented
        options={[
          { value: "confidence", label: "Confidence" },
          { value: "recent", label: "Recent" },
          { value: "warming", label: "Warming" },
        ]}
        value={sort}
        onValueChange={(v) => (sort = v as ConclusionSort)}
        ariaLabel="Order conclusions by"
      />
    </div>

    <div class="cstrip">
      {#each cards as c (c.id)}
        {@const read = readCard(c)}
        {@const faded = c.status === "faded"}
        <button
          type="button"
          class="ccard"
          class:is-on={c.id === selected?.id}
          onclick={() => (selectedId = c.id)}
        >
          <span class="ccard__hd">
            {#if c.pinned}<span class="pinstar" aria-hidden="true">★</span>{/if}
            <span class="t-label" class:is-faded={faded}>
              {faded ? "faded · " : ""}{read.snapshots} snap
            </span>
            {#if c.id === selected?.id}<span class="kbd ccard__k">⌘P</span>{/if}
          </span>
          <span class="ccard__x" class:is-faded={faded}>{c.statement}</span>
          <span class="cbar">
            <i style="width:{pct(c.confidence)}%" class:is-faded={faded}></i>
            <u style="left:{pct(DISPLAY_FLOOR)}%"></u>
          </span>
          <span class="ccard__n">
            <b class:is-faded={faded}>{pct(c.confidence)}%</b>
            <span class="t-meta ccard__w ccard__w--{read.tone}">{read.word}</span>
          </span>
          <span class="t-meta is-mono is-num ccard__r">{read.range}</span>
        </button>
      {/each}
    </div>

    <Spine {events} {activities} {previews} onView={viewActivity} />
  {/if}
</div>

<FrameDetailModal
  open={frameOpen}
  {frameId}
  onClose={() => (frameOpen = false)}
  onOpenInTimeline={frameFallback ?? undefined}
/>

<style>
  .det {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .det__state {
    padding: var(--s-16);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-6);
  }
  .det__hero {
    flex: 0 0 auto;
    display: flex;
    align-items: flex-start;
    gap: var(--s-12);
    padding: var(--s-12) var(--s-16) var(--s-8);
  }
  .det__id {
    flex: 1 1 auto;
    min-width: 0;
  }
  .det__ttl {
    margin: 0;
  }
  .det__spark {
    width: 12px;
    height: 12px;
  }

  .pills {
    display: flex;
    gap: var(--s-6);
    flex-wrap: wrap;
    margin-top: var(--s-4);
  }
  .pillx {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 7px;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
  }
  .pillx b {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
    margin-right: 4px;
    font-family: var(--app-font-mono);
    font-variant-numeric: tabular-nums;
  }
  .pillx__mid {
    margin: 0 0 0 4px;
  }

  /* ── conclusion strip ─────────────────────────────────────────────────── */
  .cstrip {
    flex: 0 0 auto;
    display: flex;
    gap: var(--s-8);
    padding: 0 var(--s-16) var(--s-8);
    overflow-x: auto;
  }
  .ccard {
    flex: 1 1 0;
    min-width: 180px;
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    padding: var(--s-8) var(--s-12) var(--s-12);
    border: 0;
    border-radius: var(--r-lg);
    background: var(--app-surface);
    text-align: left;
    cursor: pointer;
  }
  .ccard:hover {
    background: var(--app-surface-raised);
  }
  .ccard.is-on {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--app-accent-border),
      inset 3px 0 0 var(--app-accent);
  }
  .ccard:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .ccard__hd {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
  }
  .ccard__k {
    margin-left: auto;
  }
  .ccard__x {
    min-height: 42px;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text);
  }
  .ccard__n {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
  }
  .ccard__n b {
    font: var(--w-semi) var(--t-title) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  .ccard__w--up {
    color: var(--app-accent);
  }
  .ccard__w--down {
    color: var(--app-info);
  }
  .ccard__w--flat,
  .ccard__w--floor {
    color: var(--app-text-muted);
  }
  .ccard__r {
    color: var(--app-text-subtle);
  }
  .is-faded {
    color: var(--app-text-muted);
  }

  .cbar {
    position: relative;
    height: 5px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    overflow: hidden;
  }
  .cbar i {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: var(--app-accent);
  }
  .cbar i.is-faded {
    opacity: 0.5;
  }
  .cbar u {
    position: absolute;
    top: 0;
    bottom: 0;
    width: var(--hairline);
    background: var(--app-text-faint);
  }
  .pinstar {
    color: var(--app-warn);
    font-size: 11px;
    line-height: 1;
  }

</style>
