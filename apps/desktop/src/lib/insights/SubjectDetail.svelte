<script lang="ts">
  // SubjectDetail — the drill-in detail surface for a single Subject (#106).
  //
  // Mirrors `docs/user-context/mockups/subject.html`: a Subject shows its
  // INDIVIDUAL Conclusions, each with its OWN confidence-over-time trajectory —
  // NOT a single rolled-up sentiment score. Layout:
  //   1. Subject hero (title + meta pills).
  //   2. Overlay TrajectoryChart — one line per Conclusion (its Confidence
  //      History), each coloured by cycling the category palette; faded
  //      conclusions render dimmed, floor 0.15. A 2-col legend follows.
  //   3. Master-detail grid: left = conclusions list (statement, ConfidenceBar
  //      with trend, Pin/Dismiss, "view evidence"); right = sticky Evidence
  //      Inspector for the selected conclusion (evidence rows + Confidence
  //      History list). Selecting a conclusion highlights its line + drives the
  //      inspector. Faded conclusions stay listed with their historical arc.
  //
  // The breadcrumb back affordance is rendered by the Insights workspace shell
  // (insights/+page.svelte), so we do NOT duplicate it here. `onBack` is exposed
  // but the shell already provides the primary back control.
  //
  // Props:
  //   subject: string     — the Subject name being inspected.
  //   onBack: () => void  — return to the Subjects index.

  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import type {
    Conclusion,
    SubjectView,
    SubjectTrajectory,
    ConclusionEvidenceRef,
    Activity,
  } from "$lib/types/recording";
  import TrajectoryChart from "$lib/insights/charts/TrajectoryChart.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  interface Props {
    subject: string;
    onBack: () => void;
  }

  let { subject, onBack }: Props = $props();
  // `onBack` is accepted for parity with the stub interface; the workspace shell
  // already renders the breadcrumb back control, so we keep it referenced inside
  // a closure (avoids the `state_referenced_locally` warning).
  const _interface = () => onBack;
  void _interface;

  const CAT_PALETTE = [
    "--cat-coding",
    "--cat-research",
    "--cat-communication",
    "--cat-design",
    "--cat-testing",
    "--cat-personal",
    "--cat-distractions",
  ] as const;

  const FLOOR = 0.15;

  type Trend = "up" | "steady" | "down" | "faded";

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  let actionId = $state<number | null>(null);

  // Activities resolved lazily for richer evidence rows + Timeline handoff. Maps
  // activityId → Activity (title/time/category + raw evidence refs).
  let activities = $state<Map<number, Activity>>(new Map());

  function colorVarFor(index: number): string {
    return CAT_PALETTE[index % CAT_PALETTE.length];
  }

  // Stable display order: conclusions sorted by confidence desc. Index in this
  // ordering drives the colour assignment (so the chart line, legend swatch,
  // and inspector dot all agree).
  const orderedConclusions = $derived.by<Conclusion[]>(() => {
    if (!view) return [];
    return [...view.conclusions].sort((a, b) => b.confidence - a.confidence);
  });

  const colorById = $derived.by<Map<number, string>>(() => {
    const m = new Map<number, string>();
    orderedConclusions.forEach((c, i) => m.set(c.id, colorVarFor(i)));
    return m;
  });

  const trajectoryById = $derived.by<Map<number, SubjectTrajectory>>(() => {
    const m = new Map<number, SubjectTrajectory>();
    if (view) for (const t of view.trajectories) m.set(t.conclusionId, t);
    return m;
  });

  // Trend for a conclusion: faded conclusions are always 'faded'; otherwise
  // derive from its real trajectory (first vs last), else steady.
  function trendFor(c: Conclusion): Trend {
    if (c.status === "faded") return "faded";
    const t = trajectoryById.get(c.id);
    if (t && t.history.length >= 2) {
      const delta =
        t.history[t.history.length - 1].confidence - t.history[0].confidence;
      if (delta > 0.04) return "up";
      if (delta < -0.04) return "down";
      return "steady";
    }
    return "steady";
  }

  function trendLabel(t: Trend): string {
    return t === "up"
      ? "▲ rising"
      : t === "down"
        ? "▼ cooling"
        : t === "faded"
          ? "⊘ faded"
          : "– steady";
  }

  function confidenceBarTrend(c: Conclusion): "up" | "steady" | "down" | "faded" {
    const t = trendFor(c);
    return t;
  }

  // Feed the overlay chart: one series per trajectory, mapped from Confidence
  // History (oldest-first), coloured by the conclusion's stable colour, faded if
  // the conclusion is below the display floor. The selected line is emphasised by
  // colour parity (the chart itself dims faded series).
  const chartSeries = $derived.by(() => {
    if (!view) return [];
    return view.trajectories
      .map((t) => {
        const c = view!.conclusions.find((x) => x.id === t.conclusionId);
        return {
          id: t.conclusionId,
          label: t.statement,
          colorVar: colorById.get(t.conclusionId) ?? CAT_PALETTE[0],
          faded: c?.status === "faded",
          points: t.history.map((h) => ({
            atMs: h.snapshotAtMs,
            confidence: h.confidence,
          })),
        };
      })
      .filter((s) => s.points.length > 0);
  });

  const selectedConclusion = $derived.by<Conclusion | null>(() => {
    if (!view || selectedId === null) return null;
    return view.conclusions.find((c) => c.id === selectedId) ?? null;
  });

  const selectedTrajectory = $derived.by<SubjectTrajectory | null>(() => {
    if (selectedId === null) return null;
    return trajectoryById.get(selectedId) ?? null;
  });

  // ---- Hero meta ----
  const conclusionCount = $derived(view?.conclusions.length ?? 0);
  const fadedCount = $derived(
    view?.conclusions.filter((c) => c.status === "faded").length ?? 0,
  );
  const firstSeenMs = $derived(
    view && view.conclusions.length
      ? Math.min(...view.conclusions.map((c) => c.formedAtMs))
      : 0,
  );
  const lastEvidenceMs = $derived(
    view && view.conclusions.length
      ? Math.max(...view.conclusions.map((c) => c.lastSupportedAtMs))
      : 0,
  );
  // Linked-activity count = distinct activityIds across all conclusions' evidence.
  const linkedActivityCount = $derived.by<number>(() => {
    if (!view) return 0;
    const ids = new Set<number>();
    for (const c of view.conclusions)
      for (const e of c.evidence) ids.add(e.activityId);
    return ids.size;
  });

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

  function fmtMonth(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleDateString(undefined, { month: "short" });
  }

  function pct(confidence: number): number {
    return Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  }

  // Evidence rows for the selected conclusion. Cross-reference each ref's
  // activityId against the resolved Activity for richer title/time/category +
  // a source-type hint from the Activity's first raw evidence ref.
  interface EvidenceRow {
    activityId: number;
    stance: "support" | "contradict";
    title: string;
    atMs: number | null;
    category: string | null;
    sourceType: "screen" | "audio" | null;
  }

  const evidenceRows = $derived.by<EvidenceRow[]>(() => {
    const c = selectedConclusion;
    if (!c) return [];
    return c.evidence.map((e: ConclusionEvidenceRef) => {
      const activity = activities.get(e.activityId);
      const firstRef = activity?.evidence?.[0];
      const sourceType: "screen" | "audio" | null = firstRef
        ? firstRef.subjectType === "audio_segment"
          ? "audio"
          : "screen"
        : null;
      return {
        activityId: e.activityId,
        stance: e.stance,
        title: activity?.title ?? e.activityTitle ?? `Activity #${e.activityId}`,
        atMs: activity?.startedAtMs ?? e.activityStartedAtMs ?? null,
        category: activity?.category ?? null,
        sourceType,
      };
    });
  });

  async function loadSubject(): Promise<void> {
    loading = true;
    try {
      const next = await invoke<SubjectView>("get_user_context_subject", {
        subject,
      });
      view = next;
      loadError = null;
      // Preserve the selection if it still exists; else select the top one.
      const stillExists =
        selectedId !== null &&
        next.conclusions.some((c) => c.id === selectedId);
      if (!stillExists) {
        const top = [...next.conclusions].sort(
          (a, b) => b.confidence - a.confidence,
        )[0];
        selectedId = top?.id ?? null;
      }
      void loadActivities(next);
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      loading = false;
    }
  }

  // Resolve the Activities this Subject's conclusions cite so evidence rows show
  // real titles/times/source type and so "view in Timeline" can hand off to a
  // raw frame/audio segment. Best-effort: paged scan of recent Activities; rows
  // without a resolved Activity fall back to the conclusion's stored ref data.
  async function loadActivities(v: SubjectView): Promise<void> {
    const wanted = new Set<number>();
    for (const c of v.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
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
    activities = resolved;
  }

  function selectConclusion(id: number): void {
    selectedId = id;
  }

  async function togglePinned(c: Conclusion): Promise<void> {
    if (actionId !== null) return;
    actionId = c.id;
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      await loadSubject();
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      actionId = null;
    }
  }

  async function dismiss(c: Conclusion): Promise<void> {
    if (actionId !== null) return;
    actionId = c.id;
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
      await loadSubject();
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      actionId = null;
    }
  }

  // "view in Timeline" — best-effort Activity-span handoff to the raw Timeline.
  // We resolve the Activity's first raw evidence ref (frame/audio segment) and
  // ask the main window to land there. If no raw ref is resolvable, fall back to
  // navigating to the Timeline surface so the action never dead-ends.
  async function viewInTimeline(row: EvidenceRow): Promise<void> {
    const activity = activities.get(row.activityId);
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
    // Graceful fallback: open the Timeline surface.
    void goto("/");
  }

  $effect(() => {
    // Re-run whenever the Subject changes. Reading `subject` here is the only
    // tracked dependency; the loader itself is awaited inside.
    subject;
    void loadSubject();

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadSubject();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<section class="subject-detail" aria-label={`Subject — ${subject}`}>
  {#if loadError}
    <div class="state state--error">
      <p class="state-title">Couldn't load this subject.</p>
      <p class="state-detail">{loadError}</p>
    </div>
  {:else if loading && !view}
    <!-- Loading skeleton — hero + overlay chart + master/detail, matching the
         real layout so the swap to loaded content causes no layout shift. The
         "nothing concluded" empty state only renders once loaded. -->
    <div aria-label={`Loading ${subject}`} aria-busy="true">
      <div class="subj-hero">
        <div class="subj-hero-main">
          <div class="sk-hero-title">
            <Skeleton variant="text" width="240px" height="25px" />
          </div>
          <div class="subj-meta">
            <Skeleton variant="text" width="92px" height="18px" radius="999px" />
            <Skeleton variant="text" width="118px" height="18px" radius="999px" />
            <Skeleton variant="text" width="132px" height="18px" radius="999px" />
          </div>
        </div>
      </div>

      <div class="card traj-card">
        <div class="traj-head">
          <Skeleton variant="text" width="180px" height="13px" />
          <Skeleton variant="text" width="110px" height="11px" />
        </div>
        <div class="sk-traj-sub">
          <Skeleton variant="text" width="70%" height="11px" />
        </div>
        <div class="traj-chartwrap">
          <Skeleton height="160px" radius="8px" />
        </div>
        <div class="traj-legend">
          {#each Array.from({ length: 4 }) as _, i (i)}
            <div class="sk-legend-item">
              <Skeleton width="18px" height="8px" radius="3px" />
              <Skeleton variant="text" width="70%" height="11px" />
            </div>
          {/each}
        </div>
      </div>

      <div class="md-grid">
        <div class="md-left">
          <div class="md-head">
            <Skeleton variant="text" width="150px" height="13px" />
          </div>
          <div class="concl-list">
            {#each Array.from({ length: 3 }) as _, i (i)}
              <div class="card concl-card concl-card--skeleton">
                <Skeleton variant="text" width="92%" height="13px" />
                <Skeleton variant="text" width="60%" height="13px" />
                <div class="sk-concl-conf">
                  <Skeleton height="8px" radius="999px" />
                </div>
                <div class="sk-concl-foot">
                  <Skeleton variant="text" width="56px" height="22px" radius="6px" />
                  <Skeleton variant="text" width="64px" height="22px" radius="6px" />
                </div>
              </div>
            {/each}
          </div>
        </div>

        <aside class="card inspector inspector--skeleton">
          <div class="insp-head">
            <Skeleton variant="text" width="70px" height="11px" />
            <Skeleton variant="text" width="96px" height="11px" />
          </div>
          <div class="sk-insp-body">
            {#each Array.from({ length: 3 }) as _, i (i)}
              <div class="sk-ev-item">
                <Skeleton width="44px" height="32px" radius="4px" />
                <div class="sk-ev-body">
                  <Skeleton variant="text" width="88%" height="11px" />
                  <Skeleton variant="text" width="54%" height="10px" />
                </div>
              </div>
            {/each}
          </div>
        </aside>
      </div>
    </div>
  {:else if view && conclusionCount === 0}
    <div class="state">
      <p class="state-title">Nothing concluded about {subject} yet.</p>
      <p class="state-detail">
        Conclusions form as evidence accumulates. This subject has no active or
        faded conclusions to chart.
      </p>
    </div>
  {:else if view}
    <!-- Hero -->
    <div class="subj-hero">
      <div class="subj-hero-main">
        <h1 class="subj-title">{subject}</h1>
        <div class="subj-meta">
          <span class="pill">
            {conclusionCount}
            {conclusionCount === 1 ? "conclusion" : "conclusions"}
          </span>
          <span class="pill">first seen {relativeTime(firstSeenMs)}</span>
          <span class="pill">last evidence {relativeTime(lastEvidenceMs)}</span>
          {#if linkedActivityCount > 0}
            <span class="pill">{linkedActivityCount} linked activities</span>
          {/if}
        </div>
      </div>
    </div>

    <!-- Overlay trajectory chart -->
    <div class="card traj-card">
      <div class="traj-head">
        <div class="section-title">Conclusion trajectories</div>
        <span class="traj-head-note">confidence over time</span>
      </div>
      <p class="traj-sub">
        Each line is one conclusion — they warm and cool independently. Not a
        single sentiment score.
      </p>

      {#if chartSeries.length > 0}
        <div class="traj-chartwrap">
          <TrajectoryChart series={chartSeries} floor={FLOOR} />
        </div>

        <!-- Legend: each colored line → its conclusion -->
        <div class="traj-legend">
          {#each orderedConclusions as c (c.id)}
            {@const t = trendFor(c)}
            <button
              type="button"
              class="legend-item"
              class:legend-item--faded={c.status === "faded"}
              class:is-selected={selectedId === c.id}
              onclick={() => selectConclusion(c.id)}
            >
              <span
                class="legend-swatch"
                class:legend-swatch--faded={c.status === "faded"}
                style="border-top-color: var({colorById.get(c.id)});"
              ></span>
              <span class="legend-label">{c.statement}</span>
              <span class="legend-trend">{pct(c.confidence)}% {trendLabel(t).split(" ")[0]}</span>
            </button>
          {/each}
        </div>
      {:else}
        <p class="traj-empty">No confidence history recorded yet.</p>
      {/if}
    </div>

    <!-- Master-detail -->
    <div class="md-grid">
      <!-- LEFT: conclusions list -->
      <div class="md-left">
        <div class="md-head">
          <div class="section-title">Conclusions ({conclusionCount})</div>
          <span class="md-head-note">
            click a row to inspect its evidence
            {#if fadedCount > 0}· {fadedCount} below floor{/if}
          </span>
        </div>

        <div class="concl-list">
          {#each orderedConclusions as c (c.id)}
            {@const t = trendFor(c)}
            <div
              class="card concl-card"
              class:is-selected={selectedId === c.id}
              class:concl-card--faded={c.status === "faded"}
              role="button"
              tabindex="0"
              onclick={() => selectConclusion(c.id)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  selectConclusion(c.id);
                }
              }}
            >
              <div class="concl-main">
                <p class="concl-statement">{c.statement}</p>
                <div class="concl-conf">
                  <ConfidenceBar confidence={c.confidence} trend={confidenceBarTrend(c)} />
                  <span class="trend trend--{t}">{trendLabel(t)}</span>
                </div>
                {#if c.status === "faded"}
                  <div class="floor-note">
                    <span class="glyph" aria-hidden="true">⊘</span>
                    Below display floor — kept for history.
                  </div>
                {/if}
                <div class="concl-footrow">
                  <button
                    type="button"
                    class="btn"
                    class:btn--accent={c.pinned}
                    disabled={actionId !== null}
                    onclick={(e) => {
                      e.stopPropagation();
                      void togglePinned(c);
                    }}
                  >
                    {c.pinned ? "★ Pinned" : "Pin"}
                  </button>
                  <button
                    type="button"
                    class="btn"
                    disabled={actionId !== null}
                    onclick={(e) => {
                      e.stopPropagation();
                      void dismiss(c);
                    }}
                  >
                    Dismiss
                  </button>
                  <span class="spacer"></span>
                  <span class="ev-affordance">
                    view evidence · {c.evidence.length}
                  </span>
                </div>
              </div>
            </div>
          {/each}
        </div>
      </div>

      <!-- RIGHT: evidence inspector (sticky) -->
      <aside class="card inspector">
        {#if selectedConclusion}
          <div class="insp-head">
            <span class="ih-title">Evidence</span>
            <span class="ih-count">
              {selectedConclusion.evidence.length}
              {selectedConclusion.evidence.length === 1
                ? "linked activity"
                : "linked activities"}
            </span>
          </div>
          <div class="insp-conclusion">
            <span
              class="ic-dot"
              style="background: var({colorById.get(selectedConclusion.id)});"
            ></span>
            {selectedConclusion.statement}
            <span class="insp-conf">
              · {pct(selectedConclusion.confidence)}%
              {trendLabel(trendFor(selectedConclusion)).split(" ")[0]}
            </span>
          </div>

          <div class="ev-list">
            {#if evidenceRows.length === 0}
              <p class="ev-empty">No grounding evidence linked.</p>
            {:else}
              {#each evidenceRows as ev (ev.activityId)}
                <div class="ev-item">
                  <div
                    class="ev-thumb"
                    class:is-screen={ev.sourceType !== "audio"}
                    class:is-audio={ev.sourceType === "audio"}
                  >
                    <span
                      class="ev-src-tag"
                      class:is-screen={ev.sourceType !== "audio"}
                      class:is-audio={ev.sourceType === "audio"}
                    >
                      {ev.sourceType === "audio" ? "mic" : "scr"}
                    </span>
                  </div>
                  <div class="ev-body">
                    <div class="ev-title">{ev.title}</div>
                    <div class="ev-meta">
                      {#if ev.category}
                        <span class="ev-app">{ev.category}</span>
                        <span>·</span>
                      {/if}
                      {#if ev.stance === "contradict"}
                        <span class="ev-stance">contradicts</span>
                        <span>·</span>
                      {/if}
                      <span>{ev.atMs ? relativeTime(ev.atMs) : "—"}</span>
                    </div>
                    <button
                      type="button"
                      class="ev-link"
                      onclick={() => viewInTimeline(ev)}
                    >
                      view in Timeline →
                    </button>
                  </div>
                </div>
              {/each}
            {/if}
          </div>

          <div class="insp-subhead">Confidence history</div>
          <div class="conf-hist">
            {#if selectedTrajectory && selectedTrajectory.history.length > 0}
              {#each selectedTrajectory.history as h, i (i)}
                <div class="ch-row">
                  <span class="ch-date">{fmtMonth(h.snapshotAtMs)}</span>
                  <div class="ch-track">
                    <div class="ch-fill" style="width:{pct(h.confidence)}%;"></div>
                  </div>
                  <span class="ch-val">{pct(h.confidence)}</span>
                </div>
              {/each}
            {:else}
              <p class="ev-empty">No history snapshots yet.</p>
            {/if}
          </div>
        {:else}
          <div class="insp-head">
            <span class="ih-title">Evidence</span>
          </div>
          <p class="ev-empty" style="padding: 14px 13px;">
            Select a conclusion to inspect its evidence.
          </p>
        {/if}
      </aside>
    </div>
  {/if}
</section>

<style>
  .subject-detail {
    display: flex;
    flex-direction: column;
  }

  .card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 14px;
  }

  /* ---- Hero ---- */
  .subj-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 18px;
  }
  .subj-hero-main {
    min-width: 0;
  }
  .subj-title {
    font-size: 25px;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    font-weight: 600;
    margin: 0 0 10px;
  }
  .subj-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pill {
    font-size: 10.5px;
    padding: 2px 9px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    white-space: nowrap;
  }

  /* ---- Trajectory chart ---- */
  .traj-card {
    margin-bottom: 18px;
  }
  .traj-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 4px;
  }
  .section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }
  .traj-head-note {
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .traj-sub {
    font-size: 11px;
    color: var(--app-text-muted);
    margin: 2px 0 12px;
    line-height: 1.5;
  }
  .traj-chartwrap {
    width: 100%;
  }
  .traj-empty {
    font-size: 11.5px;
    color: var(--app-text-muted);
    margin: 8px 0 0;
  }

  /* ---- Legend ---- */
  .traj-legend {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 7px 22px;
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
  }
  .legend-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font: inherit;
    font-size: 11.5px;
    color: var(--app-text);
    min-width: 0;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 3px 5px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease;
  }
  .legend-item:hover {
    background: var(--app-surface-hover);
  }
  .legend-item.is-selected {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .legend-item--faded {
    opacity: 0.6;
  }
  .legend-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .legend-swatch {
    width: 18px;
    height: 0;
    border-top: 2px solid var(--app-text-faint);
    flex: 0 0 auto;
  }
  .legend-swatch--faded {
    border-top-style: dashed;
    opacity: 0.5;
  }
  .legend-trend {
    color: var(--app-text-muted);
    font-size: 10.5px;
    margin-left: auto;
    flex: 0 0 auto;
    font-variant-numeric: tabular-nums;
  }

  /* ---- Master-detail ---- */
  .md-grid {
    display: grid;
    grid-template-columns: 1.6fr 1fr;
    gap: 16px;
    align-items: start;
  }
  .md-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
  }
  .md-head-note {
    font-size: 11px;
    color: var(--app-text-muted);
  }

  /* LEFT: conclusions list */
  .concl-list {
    display: flex;
    flex-direction: column;
    gap: 11px;
  }
  .concl-card {
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .concl-card:hover {
    border-color: var(--app-border-hover);
  }
  .concl-card.is-selected {
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .concl-card--faded {
    opacity: 0.6;
  }
  .concl-main {
    min-width: 0;
  }
  .concl-statement {
    font-size: 13.5px;
    line-height: 1.45;
    margin: 0 0 9px;
    color: var(--app-text-strong);
  }
  .concl-conf {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 10px;
    flex-wrap: wrap;
  }
  .trend {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
  }
  .trend--up {
    color: var(--app-accent-strong);
  }
  .trend--down {
    color: var(--app-danger);
  }
  .trend--steady {
    color: var(--app-text-muted);
  }
  .trend--faded {
    color: var(--app-text-faint);
  }

  .floor-note {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 0 0 10px;
    padding: 6px 9px;
    border: 1px dashed var(--app-border-strong);
    border-radius: 6px;
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .floor-note .glyph {
    color: var(--app-text-faint);
  }

  .concl-footrow {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .btn {
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
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .btn--accent {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .spacer {
    flex: 1 1 auto;
  }
  .ev-affordance {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--app-accent-strong);
    border-bottom: 1px dotted var(--app-accent-border);
  }

  /* RIGHT: evidence inspector (sticky) */
  .inspector {
    position: sticky;
    top: 0;
    padding: 0;
    overflow: hidden;
  }
  .insp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 11px 13px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }
  .ih-title {
    font-size: 10px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .ih-count {
    font-size: 10px;
    color: var(--app-text-subtle);
  }
  .insp-conclusion {
    padding: 10px 13px;
    border-bottom: 1px solid var(--app-border);
    font-size: 12px;
    line-height: 1.45;
    color: var(--app-text);
  }
  .ic-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    margin-right: 6px;
    vertical-align: middle;
  }
  .insp-conf {
    color: var(--app-text-muted);
    font-size: 10.5px;
  }

  .ev-list {
    padding: 6px 9px 10px;
  }
  .ev-empty {
    font-size: 11px;
    color: var(--app-text-muted);
    margin: 6px 4px;
  }
  .ev-item {
    display: grid;
    grid-template-columns: 44px 1fr;
    gap: 9px;
    padding: 9px 8px;
    border: 1px solid transparent;
    border-radius: 7px;
    transition:
      background 0.12s ease,
      border-color 0.12s ease;
  }
  .ev-item:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border);
  }
  .ev-thumb {
    width: 44px;
    height: 32px;
    border-radius: 4px;
    border: 1px solid var(--app-border);
    overflow: hidden;
    position: relative;
    flex: 0 0 auto;
  }
  .ev-thumb.is-screen {
    background: var(--app-source-screen-bg);
  }
  .ev-thumb.is-audio {
    background: var(--app-source-mic-bg);
  }
  .ev-src-tag {
    position: absolute;
    left: 2px;
    bottom: 2px;
    font-size: 7px;
    letter-spacing: 0.04em;
    padding: 0 3px;
    border-radius: 3px;
    text-transform: uppercase;
  }
  .ev-src-tag.is-screen {
    color: var(--app-source-screen);
    background: var(--app-bg);
  }
  .ev-src-tag.is-audio {
    color: var(--app-source-mic);
    background: var(--app-bg);
  }
  .ev-body {
    min-width: 0;
  }
  .ev-title {
    font-size: 11.5px;
    line-height: 1.4;
    color: var(--app-text);
    margin-bottom: 3px;
  }
  .ev-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 10px;
    color: var(--app-text-subtle);
    margin-bottom: 4px;
    flex-wrap: wrap;
  }
  .ev-app {
    color: var(--app-text-muted);
    text-transform: capitalize;
  }
  .ev-stance {
    color: var(--app-danger);
  }
  .ev-link {
    font: inherit;
    font-size: 10.5px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--app-accent-strong);
    border-bottom: 1px dotted var(--app-accent-border);
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .ev-link:hover {
    color: var(--app-accent);
  }

  .insp-subhead {
    padding: 9px 13px 5px;
    font-size: 9.5px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    border-top: 1px solid var(--app-border);
  }
  .conf-hist {
    padding: 2px 13px 14px;
  }
  .ch-row {
    display: grid;
    grid-template-columns: 30px 1fr 30px;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
    font-size: 10.5px;
    color: var(--app-text-muted);
  }
  .ch-date {
    color: var(--app-text-subtle);
  }
  .ch-track {
    height: 5px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    overflow: hidden;
    border: 1px solid var(--app-border);
  }
  .ch-fill {
    height: 100%;
    background: var(--app-accent);
    border-radius: 999px;
  }
  .ch-val {
    text-align: right;
    color: var(--app-text);
    font-variant-numeric: tabular-nums;
  }

  /* ---- Loading skeleton helpers ---- */
  .sk-hero-title {
    margin: 0 0 10px;
  }
  .sk-traj-sub {
    margin: 6px 0 12px;
  }
  .sk-legend-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 5px;
    min-width: 0;
  }
  .concl-card--skeleton {
    display: flex;
    flex-direction: column;
    gap: 9px;
    cursor: default;
  }
  .sk-concl-conf {
    margin: 2px 0;
  }
  .sk-concl-foot {
    display: flex;
    gap: 6px;
  }
  .inspector--skeleton {
    padding: 0;
  }
  .sk-insp-body {
    display: flex;
    flex-direction: column;
    gap: 11px;
    padding: 11px 13px 14px;
  }
  .sk-ev-item {
    display: grid;
    grid-template-columns: 44px 1fr;
    gap: 9px;
    align-items: center;
  }
  .sk-ev-body {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
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

  @media (max-width: 900px) {
    .md-grid {
      grid-template-columns: 1fr;
    }
    .inspector {
      position: static;
    }
    .traj-legend {
      grid-template-columns: 1fr;
    }
  }
</style>
