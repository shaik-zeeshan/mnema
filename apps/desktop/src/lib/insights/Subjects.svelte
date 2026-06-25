<script lang="ts">
  // Subjects — the browsable Subjects index sub-surface, "Conviction view"
  // redesign (Subjects-tab Conviction redesign, Slice 2).
  //
  // A Subject is a browsable entity that holds MULTIPLE individual Conclusions,
  // each with its OWN confidence-over-time trajectory — NEVER a single rolled-up
  // sentiment score. Instead of a flat card grid, the view groups Subjects into
  // ordered TIERS along a grouping axis (conviction: how firmly held / movement:
  // which way it's heading). Each Subject renders as a row whose HERO is a
  // multi-line micro-sparkline (one faint line per Conclusion). Tiers read
  // top→bottom like a sentence; faded subjects sink to a "kept for history" tier.
  //
  // Tiering thresholds live ENTIRELY in `subjectsTiers.ts` (Slice 1) — this
  // component never re-derives them. Below SPARSE_LIMIT subjects we skip tiers
  // and render one flat list so early users don't see mostly-empty headers.
  //
  // Subjects are derived CLIENT-SIDE from `list_user_context_conclusions`,
  // grouped by `subject`. To draw HONEST per-conclusion trajectories (not flat
  // baselines), we lazily fetch `get_user_context_subject` per subject and use
  // each trajectory's real Confidence History; if that fetch fails or has no
  // history we fall back to a flat single-point baseline from current confidence.
  //
  // Props:
  //   onOpenSubject: (subject: string) => void

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import type {
    Conclusion,
    SubjectView,
    ConclusionEvidenceRef,
    Activity,
  } from "$lib/types/recording";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import {
    type Axis,
    type Trend,
    type TierSubject,
    buildTiers,
    deriveTrend,
    isSparse,
    summaryCounts,
    subjectsDiff,
    decideRefresh,
    debounce,
  } from "$lib/insights/subjectsTiers";
  import { rankSubjects } from "$lib/insights/subjectSearch";

  // Number of placeholder rows shown while the conclusions load.
  const SKELETON_COUNT = 6;

  interface Props {
    onOpenSubject: (subject: string) => void;
  }

  let { onOpenSubject }: Props = $props();

  // Category palette cycled to colour each conclusion's sparkline line.
  const CAT_PALETTE = [
    "--cat-creating",
    "--cat-communication",
    "--cat-meetings",
    "--cat-research",
    "--cat-learning",
    "--cat-organizing",
    "--cat-personal",
    "--cat-entertainment",
  ] as const;

  const FLOOR = 0.15;

  interface SubjectSpark {
    colorVar: string;
    faded: boolean;
    points: number[];
  }

  // SubjectRow satisfies TierSubject (topConfidence/faded/trend/lastMovedAtMs)
  // so the tiering helpers can group it directly without a separate projection.
  interface SubjectRow extends TierSubject {
    subject: string;
    conclusions: Conclusion[];
    conclusionCount: number;
    pinned: boolean;
    faded: boolean; // all conclusions faded
    headline: string; // top (highest-confidence) conclusion statement
    lastMovedAtMs: number; // most recent updated/last-supported across conclusions
    trend: Trend;
    spark: SubjectSpark[];
    topConfidence: number; // highest-confidence conclusion's confidence
    catColorVar: string; // top conclusion's palette colour (dot + strongest line)
  }

  let conclusions = $state<Conclusion[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);

  // Grouping axis for the tier layout. "conviction" = how firmly held (default);
  // "movement" = which way it's heading. Drives `buildTiers`.
  let axis = $state<Axis>("conviction");

  // Search box. `searchQuery` is the raw input; `appliedQuery` is the debounced
  // value the ranking actually runs on (filtering is in-memory and cheap, so the
  // debounce only spares us re-ranking on every keystroke). A non-empty applied
  // query swaps the tiered/sparse layout for one flat relevance-ranked list.
  let searchQuery = $state("");
  let appliedQuery = $state("");
  const applySearch = debounce((q: string) => {
    appliedQuery = q;
  }, 200);
  function onSearchInput(): void {
    applySearch(searchQuery);
  }

  // Which row's detail is expanded (by subject name), or null for none. The
  // expand container is an interaction skeleton — Slice 3 fills its content.
  let expandedSubject = $state<string | null>(null);

  // Per-tier paging: each tier shows at most TIER_PAGE rows until the reader
  // opts into "Show more", which adds the tier's id to `expandedTiers`. Keeps a
  // crowded tier (e.g. a big "Forming" pile) from dominating the surface while
  // still being one click from the full list. Reassign the Set on change so the
  // $state reacts (plain Sets aren't deep-proxied in Svelte 5).
  const TIER_PAGE = 10;
  let expandedTiers = $state<Set<string>>(new Set());

  function toggleTier(id: string): void {
    const next = new Set(expandedTiers);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expandedTiers = next;
  }

  // Per-subject real trajectory history, fetched lazily. Maps subject → (map of
  // conclusionId → oldest-first confidence points). Used to draw honest spark
  // lines + derive warming/steady/cooling from the start-vs-end of the arc.
  let trajectories = $state<Map<string, Map<number, number[]>>>(new Map());
  // Monotonic generation token for `loadTrajectories`. Each call bumps it; only
  // the call whose token still matches at completion may write `trajectories`,
  // so a slow earlier load can't clobber a newer one's results (stale clobber).
  let trajectoriesGen = 0;

  // Guards a single in-flight Pin/Dismiss action by conclusion id, so the
  // expanded detail's per-conclusion buttons disable while one is running.
  // `actionKind` records WHICH action is running so only that button shows its
  // busy affordance (the sibling stays disabled but unlabelled).
  let actionId = $state<number | null>(null);
  let actionKind = $state<"pin" | "dismiss" | null>(null);

  // ---- Slice 4: realtime staging buffer + refresh pill --------------------
  // Engine `user_context_changed` events never reflow the page while the user
  // reads. A debounced reload lands in `stagedConclusions`; the pill surfaces
  // "{pendingCount} views updated · refresh" and the swap happens only on the
  // pill click (or silently when no row is open and the list is at the top).
  let stagedConclusions = $state<Conclusion[] | null>(null);
  let pendingCount = $state(0);
  // Whether the scroll surface is at the top (best-effort) — gates silent apply.
  let atTop = $state(true);
  // The section root, used to resolve the nearest scroll container on mount.
  let rootEl = $state<HTMLElement | null>(null);

  // Resolved evidence Activities, keyed by activityId, populated lazily when a
  // subject's row is expanded. Maps activityId → Activity (title/time/category +
  // raw evidence refs for the Timeline handoff). A bounded paged scan fills it.
  let activitiesById = $state<Map<number, Activity>>(new Map());
  // Subjects whose evidence resolution has already been kicked off, so the lazy
  // loader never double-fetches the same subject across re-expands.
  let resolvedSubjects = $state<Set<string>>(new Set());

  function groupSubjects(list: Conclusion[]): Map<string, Conclusion[]> {
    const groups = new Map<string, Conclusion[]>();
    for (const c of list) {
      const key = c.subject;
      const bucket = groups.get(key);
      if (bucket) bucket.push(c);
      else groups.set(key, [c]);
    }
    return groups;
  }

  function buildSpark(
    cs: Conclusion[],
    history: Map<number, number[]> | undefined,
  ): SubjectSpark[] {
    // One line per Conclusion, coloured by cycling the category palette. Prefer
    // real history points; fall back to a flat baseline from current confidence.
    return cs.map((c, i) => {
      const pts = history?.get(c.id);
      // A polyline needs >= 2 points to draw a visible segment. A single
      // snapshot (one history point, or none) would render an invisible line,
      // so flatten it into a 2-point baseline at that confidence.
      const points =
        pts && pts.length >= 2
          ? pts
          : pts && pts.length === 1
            ? [pts[0], pts[0]]
            : [c.confidence, c.confidence]; // flat baseline (2 pts so a line draws)
      return {
        colorVar: CAT_PALETTE[i % CAT_PALETTE.length],
        faded: c.status === "faded",
        points,
      };
    });
  }

  const rows = $derived.by<SubjectRow[]>(() => {
    if (!conclusions) return [];
    const groups = groupSubjects(conclusions);
    const out: SubjectRow[] = [];
    for (const [subject, cs] of groups) {
      const history = trajectories.get(subject);
      const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
      const top = sorted[0];
      const lastMovedAtMs = cs.reduce(
        (acc, c) => Math.max(acc, c.updatedAtMs, c.lastSupportedAtMs),
        0,
      );
      const spark = buildSpark(sorted, history);
      out.push({
        subject,
        conclusions: sorted,
        conclusionCount: cs.length,
        pinned: cs.some((c) => c.pinned),
        faded: cs.every((c) => c.status === "faded"),
        headline: top?.statement ?? subject,
        lastMovedAtMs,
        trend: deriveTrend(cs, history),
        spark,
        topConfidence: top?.confidence ?? 0,
        catColorVar: spark[0]?.colorVar ?? CAT_PALETTE[0],
      });
    }
    return out;
  });

  // Flat sorted list: non-faded by topConfidence desc, faded sunk to the bottom.
  // This single ordering feeds both the sparse flat list and `buildTiers`.
  const displayRows = $derived.by<SubjectRow[]>(() => {
    const list = [...rows];
    list.sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.topConfidence - a.topConfidence ||
        a.subject.localeCompare(b.subject),
    );
    return list;
  });

  // Ordered, non-empty tiers for the current axis (faded tier last).
  const tiers = $derived.by(() =>
    buildTiers(displayRows, axis).filter((t) => t.items.length > 0),
  );

  // Sparse mode: too few subjects to justify tier headers — render one flat list.
  const sparse = $derived(isSparse(displayRows.length));

  // Honest header counts (no rolled-up score) from the real displayed rows:
  // "{active} active views · {fading} fading — {warming} ▲ · {steady} · {cooling} ▼".
  const summary = $derived(summaryCounts(displayRows));

  // True when a search is active. Drives the layout swap (flat ranked list) and
  // the header line (match count instead of the conviction/movement tallies).
  const searching = $derived(appliedQuery.trim().length > 0);

  // Relevance-ranked matches for the active query. Ranks over `displayRows` so
  // ties fall back to the same confidence-desc, faded-last order the tiers use.
  // Matches name + conclusion statements across ALL loaded rows (fading too).
  const searchResults = $derived.by<SubjectRow[]>(() =>
    searching ? rankSubjects(displayRows, appliedQuery) : [],
  );

  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    if (day < 7) return `${day}d ago`;
    const wk = Math.floor(day / 7);
    if (wk < 5) return `${wk}w ago`;
    const mo = Math.floor(day / 30);
    if (mo < 12) return `${mo}mo ago`;
    const yr = Math.floor(day / 365);
    return `${yr}y ago`;
  }

  function trendPillClass(t: Trend): string {
    return t === "up"
      ? "trend-up"
      : t === "down"
        ? "trend-down"
        : "trend-steady";
  }
  function trendLabel(t: Trend): string {
    return t === "up" ? "▲ warming" : t === "down" ? "▼ cooling" : "– steady";
  }

  function openSubject(row: SubjectRow): void {
    onOpenSubject(row.subject);
  }
  function onRowKey(e: KeyboardEvent, row: SubjectRow): void {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openSubject(row);
    }
  }
  function toggleExpand(subject: string): void {
    expandedSubject = expandedSubject === subject ? null : subject;
    if (expandedSubject) void resolveActivitiesFor(expandedSubject);
  }

  function pct(confidence: number): number {
    return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  }

  // ---- Per-conclusion actions (mirrors SubjectDetail's signatures exactly) ----
  // After a successful pin/dismiss we reload the whole conclusion set so the
  // confidence arc + tier grouping reflect the change. User-initiated actions
  // are IMMEDIATE (never gated behind the refresh pill) — applyConclusions swaps
  // the data straight in; the open row stays open (`expandedSubject` preserved).
  async function togglePinned(c: Conclusion): Promise<void> {
    if (actionId !== null) return;
    actionId = c.id;
    actionKind = "pin";
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      applyConclusions(await fetchConclusions());
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  async function dismiss(c: Conclusion): Promise<void> {
    if (actionId !== null) return;
    actionId = c.id;
    actionKind = "dismiss";
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
      applyConclusions(await fetchConclusions());
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  // ---- Lazy, bounded evidence resolution (quick look, not the deep dive) ----
  // One evidence chip per resolved Activity cited by the subject's conclusions.
  interface EvidenceChip {
    activityId: number;
    sourceType: "screen" | "audio";
    atMs: number | null;
  }

  // Aggregate the distinct evidence Activities cited across a subject's
  // conclusions and project the resolved ones into source-typed chips. Capped to
  // a handful so the expanded row stays a glance, not the full inspector.
  function evidenceChipsFor(r: SubjectRow): EvidenceChip[] {
    const seen = new Set<number>();
    const order: number[] = [];
    for (const c of r.conclusions) {
      for (const e of c.evidence as ConclusionEvidenceRef[]) {
        if (!seen.has(e.activityId)) {
          seen.add(e.activityId);
          order.push(e.activityId);
        }
      }
    }
    const chips: EvidenceChip[] = [];
    for (const id of order) {
      const activity = activitiesById.get(id);
      if (!activity) continue;
      const firstRef = activity.evidence?.[0];
      const sourceType: "screen" | "audio" =
        firstRef?.subjectType === "audio_segment" ? "audio" : "screen";
      chips.push({ activityId: id, sourceType, atMs: activity.startedAtMs ?? null });
      if (chips.length >= 5) break;
    }
    return chips;
  }

  // True while the expanded subject cites evidence but none has resolved yet.
  function hasEvidenceRefs(r: SubjectRow): boolean {
    return r.conclusions.some((c) => c.evidence.length > 0);
  }

  // Resolve the Activities a subject's conclusions cite via a bounded paged scan
  // (port of SubjectDetail.loadActivities). Runs at most once per subject; merges
  // resolved Activities into the shared `activitiesById` cache so re-expanding is
  // instant. Best-effort — unresolved refs simply yield no chip.
  async function resolveActivitiesFor(subject: string): Promise<void> {
    if (resolvedSubjects.has(subject)) return;
    const nextResolved = new Set(resolvedSubjects);
    nextResolved.add(subject);
    resolvedSubjects = nextResolved;

    const row = rows.find((r) => r.subject === subject);
    if (!row) return;
    const wanted = new Set<number>();
    for (const c of row.conclusions)
      for (const e of c.evidence) wanted.add(e.activityId);
    if (wanted.size === 0) return;

    const resolved = new Map<number, Activity>();
    const PAGE = 200;
    const MAX_PAGES = 6; // bounded scan; evidence is recent for live subjects
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
      for (const a of batch) {
        if (wanted.has(a.id)) resolved.set(a.id, a);
      }
      if (resolved.size >= wanted.size) break;
      if (batch.length < PAGE) break;
    }
    if (resolved.size > 0) {
      const merged = new Map(activitiesById);
      for (const [id, a] of resolved) merged.set(id, a);
      activitiesById = merged;
    }
  }

  // "View in Timeline" — best-effort Activity-span handoff to the raw Timeline
  // (port of SubjectDetail.viewInTimeline). Resolves the first cited Activity's
  // first raw evidence ref (frame/audio) and asks the main window to land there;
  // if nothing resolves, fall back to navigating to the Timeline surface.
  async function viewInTimeline(r: SubjectRow): Promise<void> {
    const chips = evidenceChipsFor(r);
    const first = chips[0];
    const activity = first ? activitiesById.get(first.activityId) : undefined;
    const ref = activity?.evidence?.[0];
    try {
      if (ref && ref.subjectType === "audio_segment") {
        await invoke("open_capture_result_in_main_window", {
          kind: "audio",
          frameId: null,
          audioSegmentId: ref.subjectId,
          spanStartMs: ref.capturedAtMs ?? null,
          alignedFrameId: null,
        });
        return;
      }
      if (ref && ref.subjectType === "frame") {
        await invoke("open_capture_result_in_main_window", {
          kind: "frame",
          frameId: ref.subjectId,
          audioSegmentId: null,
        });
        return;
      }
    } catch {
      // fall through to a plain Timeline navigation
    }
    void goto("/");
  }

  // Compute the subject display order for an arbitrary conclusions list — the
  // SAME grouping + sort `displayRows` uses (non-faded by topConfidence desc,
  // faded sunk to the bottom, ties broken by subject name). Used to diff the
  // displayed list against a staged reload in the SAME order the user sees.
  function displayedSubjectOrder(list: Conclusion[]): string[] {
    const groups = groupSubjects(list);
    const summaries: { subject: string; faded: boolean; topConfidence: number }[] =
      [];
    for (const [subject, cs] of groups) {
      const top = [...cs].sort((a, b) => b.confidence - a.confidence)[0];
      summaries.push({
        subject,
        faded: cs.every((c) => c.status === "faded"),
        topConfidence: top?.confidence ?? 0,
      });
    }
    summaries.sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.topConfidence - a.topConfidence ||
        a.subject.localeCompare(b.subject),
    );
    return summaries.map((s) => s.subject);
  }

  // The single network read. Returns the fresh list; sets loadError on failure
  // (and returns the current list so callers don't blow away what's displayed).
  async function fetchConclusions(): Promise<Conclusion[]> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      loadError = null;
      return list;
    } catch (error) {
      // Only surface the full error screen when there's nothing to preserve
      // (initial load — `conclusions` still null). A background realtime refetch
      // failure keeps the intact rendered rows instead of flashing the error
      // state over good content; we still return the current list below.
      if (!conclusions?.length) {
        loadError = error instanceof Error ? error.message : String(error);
      }
      return conclusions ?? [];
    }
  }

  // Swap a list into the DISPLAYED dataset and refresh its trajectories. This is
  // the immediate path: first paint, user pin/dismiss, and the pill click all
  // call it directly. `expandedSubject` is untouched, so an open row stays open.
  function applyConclusions(list: Conclusion[]): void {
    conclusions = list;
    void loadTrajectories(list);
  }

  // Initial mount load (immediate — first paint is never gated behind a pill).
  async function loadConclusions(): Promise<void> {
    loading = true;
    try {
      applyConclusions(await fetchConclusions());
    } finally {
      loading = false;
    }
  }

  // Apply the staged reload now (the refresh-pill click, or auto-apply on idle).
  function applyStaged(): void {
    if (!stagedConclusions) return;
    applyConclusions(stagedConclusions);
    stagedConclusions = null;
    pendingCount = 0;
  }

  // Lazily fetch real per-subject Confidence History so the sparklines + trend
  // glyphs reflect actual movement rather than flat baselines. Best-effort: a
  // failed fetch just leaves that subject on its baseline. Bounded concurrency
  // keeps a large dossier responsive.
  async function loadTrajectories(list: Conclusion[]): Promise<void> {
    const gen = ++trajectoriesGen;
    const subjects = [...new Set(list.map((c) => c.subject))];
    const next = new Map<string, Map<number, number[]>>();
    const CONCURRENCY = 4;
    let cursor = 0;
    async function worker(): Promise<void> {
      while (cursor < subjects.length) {
        const subject = subjects[cursor++];
        try {
          const view = await invoke<SubjectView>("get_user_context_subject", {
            subject,
          });
          const byId = new Map<number, number[]>();
          for (const t of view.trajectories) {
            byId.set(
              t.conclusionId,
              t.history.map((h) => h.confidence),
            );
          }
          next.set(subject, byId);
        } catch {
          // Best-effort; subject keeps its flat baseline.
        }
      }
    }
    await Promise.all(
      Array.from({ length: Math.min(CONCURRENCY, subjects.length) }, worker),
    );
    // Drop the result if a newer load started while this one was in flight.
    if (gen !== trajectoriesGen) return;
    trajectories = next;
  }

  // The debounced engine-change handler: fetch fresh data, diff it against what
  // is displayed (in the SAME display order), then decide. "apply" swaps it in
  // silently; "stage" holds it behind the pill; "ignore" discards it. This is
  // the ONLY staged path — user actions and first paint stay immediate.
  async function onContextChanged(): Promise<void> {
    const next = await fetchConclusions();
    const displayedOrder = displayedSubjectOrder(
      untrack(() => conclusions) ?? [],
    );
    const stagedOrder = displayedSubjectOrder(next);
    const diff = subjectsDiff(displayedOrder, stagedOrder);
    const action = decideRefresh({
      changed: diff.changed,
      expanded: untrack(() => expandedSubject) !== null,
      atTop: untrack(() => atTop),
    });
    if (action === "ignore") {
      // Set unchanged — drop the staged copy. Trajectories stay frozen until an
      // apply (see live-arc note); a no-op here avoids reflow.
      return;
    }
    if (action === "apply") {
      applyConclusions(next);
      stagedConclusions = null;
      pendingCount = 0;
      return;
    }
    // "stage": hold behind the pill; do NOT touch `conclusions`.
    stagedConclusions = next;
    pendingCount = diff.count;
  }

  // Auto-apply on idle: once no row is open and the surface is back at the top,
  // a staged reload swaps in without needing the pill click. Best-effort — the
  // pill remains the primary path. Never fires while a row is expanded.
  $effect(() => {
    if (expandedSubject === null && atTop && stagedConclusions !== null) {
      applyStaged();
    }
  });

  $effect(() => {
    void untrack(() => loadConclusions());

    // Debounce the engine-change reload (store the wrapped fn so cleanup can
    // cancel a pending trailing call on unmount).
    const debounced = debounce(() => void onContextChanged(), 500);

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      debounced();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // atTop detection: find the nearest scroll container (the surface scrolls
    // inside `.insights-main`) and track its scrollTop. Best-effort — fall back
    // to window scroll if no container resolves. Listener removed on unmount.
    let scrollEl: HTMLElement | null = null;
    let usingWindow = false;
    const updateAtTop = (): void => {
      const top = scrollEl ? scrollEl.scrollTop : window.scrollY;
      atTop = top <= 8;
    };
    if (rootEl) {
      let el: HTMLElement | null = rootEl.closest(".insights-main");
      if (!el) {
        // Walk parents for the first scrollable ancestor.
        let p = rootEl.parentElement;
        while (p) {
          const oy = getComputedStyle(p).overflowY;
          if (oy === "auto" || oy === "scroll") {
            el = p;
            break;
          }
          p = p.parentElement;
        }
      }
      scrollEl = el;
    }
    if (scrollEl) {
      scrollEl.addEventListener("scroll", updateAtTop, { passive: true });
    } else {
      usingWindow = true;
      window.addEventListener("scroll", updateAtTop, { passive: true });
    }
    updateAtTop();

    return () => {
      disposed = true;
      unlisten?.();
      debounced.cancel();
      applySearch.cancel();
      if (scrollEl) scrollEl.removeEventListener("scroll", updateAtTop);
      if (usingWindow) window.removeEventListener("scroll", updateAtTop);
    };
  });
</script>

{#snippet row(r: SubjectRow)}
  {@const open = expandedSubject === r.subject}
  <div
    class="card conv-row"
    class:is-faded={r.faded}
    class:is-open={open}
    role="button"
    tabindex="0"
    onclick={() => openSubject(r)}
    onkeydown={(e) => onRowKey(e, r)}
  >
    <div class="conv-rowmain">
      <!-- LEFT: meta -->
      <div class="conv-meta">
        <div class="conv-metatop">
          <span
            class="conv-catdot"
            style="background:var({r.catColorVar})"
          ></span>
          <span class="conv-name">{r.subject}</span>
          {#if r.pinned}
            <span class="conv-pin" title="Pinned">📌</span>
          {/if}
          <span class="pill {trendPillClass(r.trend)}">{trendLabel(r.trend)}</span>
          <span class="conv-cc">
            · {r.conclusionCount}
            {r.conclusionCount === 1 ? "conclusion" : "conclusions"}
          </span>
        </div>
        <div class="conv-headline">{r.headline}</div>
      </div>

      <!-- RIGHT: hero sparkline + figure + caret -->
      <div class="conv-hero">
        <div class="conv-spark">
          <Sparkline series={r.spark} floor={FLOOR} />
        </div>
        <div class="conv-figure">
          <div class="conv-conf">{r.topConfidence.toFixed(2)}</div>
          <div class="conv-moved">{relativeTime(r.lastMovedAtMs)}</div>
        </div>
        <button
          type="button"
          class="conv-caret"
          class:is-open={open}
          aria-label="Expand"
          aria-expanded={open}
          onclick={(e) => {
            e.stopPropagation();
            toggleExpand(r.subject);
          }}
        >
          ▶
        </button>
      </div>
    </div>

    <!-- Expand container — shown only when this row is open. -->
    {#if open}
      {@const chips = evidenceChipsFor(r)}
      <div class="conv-detail">
        <div class="conv-detail-inner">
          <!-- Block 1: ranked conclusions with per-conclusion Pin/Dismiss. -->
          <div class="conv-detail-block">
            <p class="conv-detail-label">Conclusions · ranked by confidence</p>
            <div class="conv-concl">
              {#each r.conclusions as c (c.id)}
                {@const faded = c.status === "faded"}
                <div class="conv-concl-row" class:is-faded={faded}>
                  <span class="conv-concl-stmt" title={c.statement}>
                    {#if c.pinned}<span class="conv-concl-pin" aria-hidden="true"
                        >★</span
                      >{/if}{c.statement}
                  </span>
                  <span class="confidence-bar">
                    <span
                      class="confidence-bar-fill"
                      class:is-faded={faded}
                      style="width:{pct(c.confidence)}%"
                    ></span>
                  </span>
                  <span class="conv-concl-pct">{pct(c.confidence)}%</span>
                  <span class="chip {faded ? 'status-faded' : 'status-active'}">
                    {faded ? "faded" : "active"}
                  </span>
                  <span class="conv-concl-actions">
                    <button
                      type="button"
                      class="btn"
                      class:btn--accent={c.pinned}
                      class:btn--busy={actionId === c.id && actionKind === "pin"}
                      disabled={actionId !== null}
                      onclick={(e) => {
                        e.stopPropagation();
                        void togglePinned(c);
                      }}
                    >
                      {#if actionId === c.id && actionKind === "pin"}
                        <span class="btn-spinner" aria-hidden="true"></span>
                        {c.pinned ? "Unpinning…" : "Pinning…"}
                      {:else}
                        {c.pinned ? "★ Pinned" : "Pin"}
                      {/if}
                    </button>
                    <button
                      type="button"
                      class="btn"
                      class:btn--busy={actionId === c.id &&
                        actionKind === "dismiss"}
                      disabled={actionId !== null}
                      onclick={(e) => {
                        e.stopPropagation();
                        void dismiss(c);
                      }}
                    >
                      {#if actionId === c.id && actionKind === "dismiss"}
                        <span class="btn-spinner" aria-hidden="true"></span>
                        Dismissing…
                      {:else}
                        Dismiss
                      {/if}
                    </button>
                  </span>
                </div>
              {/each}
            </div>
          </div>

          <!-- Block 2: grounding evidence (lazy, bounded) + Timeline handoff. -->
          <div class="conv-detail-block">
            <p class="conv-detail-label">Grounded in</p>
            {#if chips.length > 0}
              <div class="conv-chiprow">
                {#each chips as chip (chip.activityId)}
                  <span
                    class="chip {chip.sourceType === 'audio'
                      ? 'src-mic'
                      : 'src-screen'}"
                  >
                    {chip.sourceType === "audio" ? "audio" : "screen"}
                    <span class="chip-time">{relativeTime(chip.atMs ?? 0)}</span>
                  </span>
                {/each}
                <button
                  type="button"
                  class="btn btn--ghost conv-timeline-btn"
                  onclick={(e) => {
                    e.stopPropagation();
                    void viewInTimeline(r);
                  }}
                >
                  View in Timeline ›
                </button>
              </div>
            {:else if hasEvidenceRefs(r) && !resolvedSubjects.has(r.subject)}
              <p class="ev-empty">Resolving evidence…</p>
            {:else}
              <p class="ev-empty">No grounding evidence linked.</p>
            {/if}
          </div>
        </div>
      </div>
    {/if}
  </div>
{/snippet}

<section class="subjects" aria-label="Subjects" bind:this={rootEl}>
  <!-- Header -->
  <div class="conv-head">
    <h1>Subjects</h1>
    <p class="conv-sub">
      What Mnema has come to believe about you — and how firmly. Strongest views
      first; fading ones are kept for history.
    </p>
    <!-- Honest counts line (no rolled-up score). Hidden while loading and when
         there are zero subjects — the empty state covers that. The line simply
         isn't rendered until conclusions load, so the header doesn't jank. -->
    {#if conclusions && displayRows.length > 0}
      {#if searching}
        <p class="conv-summary">
          <span class="num">{searchResults.length}</span>
          {searchResults.length === 1 ? "match" : "matches"} for
          <span class="num">“{appliedQuery.trim()}”</span>
        </p>
      {:else}
        <p class="conv-summary">
          <span class="num">{summary.active}</span> active views ·
          <span class="num">{summary.fading}</span> fading<span class="sep"
            >—</span
          ><span class="num">{summary.warming}</span> warming
          <span class="up">▲</span> · <span class="num">{summary.steady}</span>
          steady · <span class="num">{summary.cooling}</span> cooling
          <span class="down">▼</span>
        </p>
      {/if}
    {/if}
  </div>

  <!-- Realtime refresh pill — appears when an engine update is staged behind the
       reader (a row is open, or the page is scrolled). Click swaps it in. The
       pill is the only signal that fresh views are waiting; the page never
       reflows out from under the reader. -->
  {#if pendingCount > 0 || stagedConclusions !== null}
    <div class="conv-refresh-bar">
      <button
        type="button"
        class="conv-refresh-pill"
        onclick={applyStaged}
      >
        ↻ {pendingCount > 0
          ? `${pendingCount} ${pendingCount === 1 ? "view" : "views"} updated`
          : "views updated"} · refresh
      </button>
    </div>
  {/if}

  <!-- Controls: search box + grouping-axis toggle -->
  <div class="conv-controls">
    <div class="search">
      <span class="search-glyph" aria-hidden="true">⌕</span>
      <input
        type="search"
        class="search-input"
        placeholder="Search subjects…"
        aria-label="Search subjects"
        autocomplete="off"
        spellcheck="false"
        bind:value={searchQuery}
        oninput={onSearchInput}
      />
    </div>
    <div class="sort-seg" role="group" aria-label="Organize subjects by">
      <button
        type="button"
        class:active={axis === "conviction"}
        aria-pressed={axis === "conviction"}
        aria-current={axis === "conviction" ? "true" : undefined}
        onclick={() => (axis = "conviction")}>By conviction</button
      >
      <button
        type="button"
        class:active={axis === "movement"}
        aria-pressed={axis === "movement"}
        aria-current={axis === "movement" ? "true" : undefined}
        onclick={() => (axis = "movement")}>By movement</button
      >
    </div>
  </div>

  {#if loadError}
    <div class="state state--error">
      <p class="state-title">Couldn't load Subjects.</p>
      <p class="state-detail">{loadError}</p>
    </div>
  {:else if loading && !conclusions}
    <!-- Loading skeleton — a few rows matching the loaded row shape so the swap
         to loaded content causes no layout shift. Distinct from the empty state
         below, which only renders AFTER loading completes with no subjects. -->
    <div class="conv-rows" aria-label="Loading subjects" aria-busy="true">
      {#each Array.from({ length: SKELETON_COUNT }) as _, i (i)}
        <div class="card conv-row conv-row--skeleton">
          <div class="conv-rowmain">
            <div class="conv-meta">
              <div class="conv-metatop">
                <Skeleton variant="text" width="42%" height="14px" />
                <Skeleton variant="text" width="64px" height="14px" radius="999px" />
              </div>
              <Skeleton variant="text" width="68%" height="12px" />
            </div>
            <div class="conv-hero">
              <Skeleton width="120px" height="32px" radius="6px" />
              <Skeleton variant="text" width="48px" height="22px" />
            </div>
          </div>
        </div>
      {/each}
    </div>
  {:else if displayRows.length === 0}
    <div class="state">
      <p class="state-title">No subjects yet.</p>
      <p class="state-detail">
        As the Reasoning Engine forms views about you, each one appears here with
        its own confidence trajectory. If the engine is off, turn it on in
        Settings → Access to begin.
      </p>
    </div>
  {:else if searching}
    <!-- Search active: one flat list ranked by relevance, no tier headers. -->
    {#if searchResults.length === 0}
      <div class="state">
        <p class="state-title">No subjects match “{appliedQuery.trim()}”.</p>
        <p class="state-detail">
          Search looks at subject names and the wording of each belief. Try a
          shorter or different term, or clear the search to browse all subjects.
        </p>
      </div>
    {:else}
      <div class="conv-rows">
        {#each searchResults as r (r.subject)}
          {@render row(r)}
        {/each}
      </div>
    {/if}
  {:else if sparse}
    <!-- Sparse: one ungrouped list, no tier headers. -->
    <div class="conv-rows">
      {#each displayRows as r (r.subject)}
        {@render row(r)}
      {/each}
    </div>
    <p class="conv-foot">
      Confidence is recency-weighted — views warm with fresh evidence and cool on
      their own. Faded views are kept for history, never deleted.
    </p>
  {:else}
    <!-- Tiered layout — one section per non-empty tier. -->
    {#each tiers as tier (tier.id)}
      {@const tierOpen = expandedTiers.has(tier.id)}
      {@const shown =
        tierOpen ? tier.items : tier.items.slice(0, TIER_PAGE)}
      {@const hidden = tier.items.length - shown.length}
      <section class="conv-tier" class:conv-tier--faded={tier.faded}>
        <div class="conv-tier-head">
          <span class="section-title">{tier.title}</span>
          <span class="conv-tier-note">{tier.note}</span>
        </div>
        <div class="conv-rule"></div>
        <div class="conv-rows">
          {#each shown as r (r.subject)}
            {@render row(r)}
          {/each}
        </div>
        {#if tier.items.length > TIER_PAGE}
          <button
            type="button"
            class="conv-tier-more"
            aria-expanded={tierOpen}
            onclick={() => toggleTier(tier.id)}
          >
            {#if tierOpen}
              Show less
            {:else}
              Show {hidden} more
            {/if}
          </button>
        {/if}
      </section>
    {/each}

    <p class="conv-foot">
      Confidence is recency-weighted — views warm with fresh evidence and cool on
      their own. Faded views are kept for history, never deleted.
    </p>
  {/if}
</section>

<style>
  .subjects {
    display: flex;
    flex-direction: column;
  }

  /* ---- Header ---- */
  .conv-head {
    margin-bottom: 14px;
  }
  .conv-head h1 {
    margin: 0 0 4px;
    font-size: 18px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: 0.01em;
  }
  .conv-head .conv-sub {
    margin: 0;
    font-size: 12px;
    color: var(--app-text-muted);
    max-width: 760px;
  }

  /* Honest counts line — token-driven, tabular figures so it never reflows. */
  .conv-summary {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .conv-summary .num {
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .conv-summary .up {
    color: var(--app-accent-strong);
  }
  .conv-summary .down {
    color: var(--app-danger);
  }
  .conv-summary .sep {
    color: var(--app-text-faint);
    padding: 0 6px;
  }

  /* ---- Controls row ---- */
  .conv-controls {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin: 14px 0 10px;
  }

  /* Search box — same surface as the Chat rail's search so the two read alike. */
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    flex: 1 1 220px;
    min-width: 180px;
    max-width: 320px;
  }
  .search:focus-within {
    border-color: var(--app-border-hover);
  }
  .search-glyph {
    color: var(--app-text-subtle);
    font-size: 12px;
  }
  .search-input {
    flex: 1 1 auto;
    min-width: 0;
    font: inherit;
    font-size: 11.5px;
    border: none;
    background: transparent;
    color: var(--app-text);
    outline: none;
  }
  .search-input::placeholder {
    color: var(--app-text-faint);
  }
  .search-input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  /* Segmented grouping-axis control — the canonical segmented control look. */
  .sort-seg {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .sort-seg button {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    padding: 0 11px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .sort-seg button:hover {
    color: var(--app-text-strong);
  }
  .sort-seg button:not(:disabled):active {
    transform: translateY(1px);
  }
  .sort-seg button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .sort-seg button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  /* ---- Realtime refresh pill ---- */
  /* Sticky banner so the pill stays visible while the reader scrolls or has a
     row open. Accent-styled from tokens only; snappy entrance. */
  .conv-refresh-bar {
    position: sticky;
    top: 6px;
    z-index: 3;
    display: flex;
    justify-content: center;
    pointer-events: none;
    margin: 0 0 8px;
  }
  .conv-refresh-pill {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 11.5px;
    letter-spacing: 0.02em;
    padding: 4px 13px;
    border-radius: 999px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    box-shadow: 0 2px 10px var(--app-accent-glow);
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease,
      transform 0.12s ease;
  }
  .conv-refresh-pill:hover {
    background: var(--app-accent);
    border-color: var(--app-accent);
    color: var(--app-accent-contrast, var(--app-text-strong));
    transform: translateY(-1px);
  }
  .conv-refresh-pill:not(:disabled):active {
    transform: translateY(0);
  }
  .conv-refresh-pill:focus-visible {
    outline: 2px solid var(--app-accent-border);
    outline-offset: 2px;
  }

  /* ---- Tier section ---- */
  .conv-tier {
    margin-top: 16px;
  }
  .conv-tier:first-of-type {
    margin-top: 4px;
  }
  .conv-tier-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 8px;
  }
  .section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }
  .conv-tier-note {
    margin-left: auto;
    font-size: 11px;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
  }
  .conv-rule {
    height: 1px;
    background: var(--app-border);
    margin-bottom: 10px;
  }
  .conv-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  /* Per-tier "Show N more / Show less" toggle — quiet, left-aligned under the
     rows so it reads as a continuation of the list, not a primary action. */
  .conv-tier-more {
    align-self: flex-start;
    margin-top: 8px;
    font: inherit;
    font-size: 11.5px;
    letter-spacing: 0.02em;
    padding: 4px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .conv-tier-more:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .conv-tier-more:not(:disabled):active {
    transform: translateY(1px);
  }
  .conv-tier-more:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .conv-tier--faded .conv-tier-head .section-title {
    color: var(--app-text-subtle);
  }
  .conv-tier--faded .conv-rule {
    background: var(--app-border);
    opacity: 0.6;
  }

  /* ---- Card / Subject row ---- */
  .card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .conv-row {
    padding: 0;
    cursor: pointer;
    overflow: hidden;
    transition:
      background 0.12s ease,
      border-color 0.12s ease;
  }
  .conv-row:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  .conv-row:focus-visible {
    outline: none;
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .conv-row.is-faded {
    opacity: 0.6;
  }
  .conv-row.is-faded:hover {
    opacity: 0.85;
  }
  .conv-row--skeleton {
    cursor: default;
  }

  .conv-rowmain {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 11px 14px;
  }

  /* row LEFT — meta */
  .conv-meta {
    flex: 1 1 auto;
    min-width: 0;
  }
  .conv-metatop {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .conv-catdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .conv-name {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: 0.01em;
    min-width: 0;
  }
  .conv-pin {
    font-size: 11px;
    color: var(--app-accent-strong);
    flex: 0 0 auto;
  }
  .conv-cc {
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .conv-headline {
    margin-top: 3px;
    font-size: 12px;
    color: var(--app-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* trend pill */
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 1px 7px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
    white-space: nowrap;
    flex: 0 0 auto;
  }
  .pill.trend-up {
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .pill.trend-down {
    color: var(--app-danger);
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .pill.trend-steady {
    color: var(--app-text-muted);
  }

  /* row RIGHT — hero sparkline + figure */
  .conv-hero {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
  }
  .conv-spark {
    display: block;
    flex: 0 0 auto;
    width: 120px;
  }
  .conv-figure {
    text-align: right;
    min-width: 56px;
  }
  .conv-conf {
    font-size: 15px;
    font-weight: 600;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
  }
  .conv-moved {
    font-size: 10.5px;
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }
  .conv-caret {
    flex: 0 0 auto;
    color: var(--app-text-subtle);
    font-size: 11px;
    line-height: 1;
    padding: 4px;
    border: none;
    background: transparent;
    cursor: pointer;
    transition:
      transform 0.12s ease,
      color 0.12s ease;
  }
  .conv-row:hover .conv-caret {
    color: var(--app-text-muted);
  }
  .conv-caret:not(:disabled):active {
    transform: translateY(1px);
  }
  .conv-caret.is-open {
    transform: rotate(90deg);
    color: var(--app-accent-strong);
  }
  .conv-caret.is-open:not(:disabled):active {
    transform: rotate(90deg) translateY(1px);
  }
  .conv-caret:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
    border-radius: 4px;
  }

  /* ---- Expanded detail (interaction skeleton; Slice 3 fills content) ---- */
  .conv-detail {
    overflow: hidden;
  }
  .conv-detail-inner {
    border-top: 1px solid var(--app-border);
    padding: 12px 14px 13px;
  }
  .conv-detail-label {
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    margin: 0 0 7px;
  }
  .conv-detail-block + .conv-detail-block {
    margin-top: 13px;
  }
  .ev-empty {
    font-size: 11px;
    color: var(--app-text-muted);
    margin: 0;
  }

  /* Block 1 — ranked conclusions with per-conclusion actions. */
  .conv-concl {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .conv-concl-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 120px 42px 54px auto;
    align-items: center;
    gap: 10px;
  }
  .conv-concl-row.is-faded {
    opacity: 0.55;
  }
  .conv-concl-stmt {
    font-size: 12px;
    color: var(--app-text-strong);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .conv-concl-row.is-faded .conv-concl-stmt {
    color: var(--app-text);
  }
  .conv-concl-pin {
    color: var(--app-accent-strong);
    margin-right: 5px;
    font-size: 11px;
  }
  .conv-concl-pct {
    font-size: 11.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .conv-concl-actions {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    justify-self: end;
  }

  /* confidence-bar — ported scoped from the mockup's global-only styles. */
  .confidence-bar {
    position: relative;
    width: 100%;
    height: 5px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
  }
  .confidence-bar-fill {
    position: absolute;
    inset: 0 auto 0 0;
    height: 100%;
    border-radius: 999px;
    background: var(--app-accent);
    box-shadow: 0 0 8px var(--app-accent-glow);
  }
  .confidence-bar-fill.is-faded {
    background: var(--app-text-subtle);
    box-shadow: none;
  }

  /* chip base + status / source variants (ported scoped, token-driven). */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .chip.status-active {
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .chip.status-faded {
    color: var(--app-text-faint);
  }
  .chip.src-screen {
    color: var(--app-source-screen);
    background: var(--app-source-screen-bg);
    border-color: var(--app-source-screen-border);
  }
  .chip.src-mic {
    color: var(--app-source-mic);
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
  }
  .chip .chip-time {
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  /* Block 2 — grounding evidence chip row + Timeline button. */
  .conv-chiprow {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }

  /* .btn — compact button (ported scoped). Variants: --accent, --ghost. */
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 11.5px;
    padding: 3px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .btn:not(:disabled):active {
    transform: translateY(1px);
  }
  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  /* Busy: the acting button stays legible (no dim) while its sibling dims via
     :disabled, so the spinner + "Pinning…/Dismissing…" label reads clearly. */
  .btn--busy:disabled {
    opacity: 1;
    cursor: progress;
  }
  .btn-spinner {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--app-border-hover);
    border-top-color: var(--app-text-strong);
    animation: btn-spin 0.6s linear infinite;
    flex: 0 0 auto;
  }
  @keyframes btn-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .btn--accent {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .btn--ghost {
    border-color: transparent;
    background: transparent;
    color: var(--app-text-muted);
  }
  .btn--ghost:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: transparent;
    color: var(--app-text-strong);
  }

  /* ---- Footer ---- */
  .conv-foot {
    margin-top: 22px;
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
    font-size: 11px;
    color: var(--app-text-muted);
  }

  /* ---- States ---- */
  .state {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .state--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .state-title {
    margin: 0;
    font-size: 13px;
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
    line-height: 1.6;
  }

  @media (prefers-reduced-motion: reduce) {
    .conv-row,
    .conv-caret,
    .conv-refresh-pill,
    .conv-tier-more,
    .sort-seg button,
    .btn {
      transition: none;
    }
    .conv-refresh-pill:hover {
      transform: none;
    }
    .btn:not(:disabled):active,
    .conv-caret:not(:disabled):active,
    .conv-caret.is-open:not(:disabled):active,
    .sort-seg button:not(:disabled):active,
    .conv-tier-more:not(:disabled):active {
      transform: none;
    }
    .btn-spinner {
      animation: none;
    }
  }
</style>
