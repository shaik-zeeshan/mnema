<script lang="ts">
  // SubjectDetail — the drill-in detail surface for a single Subject (#106).
  //
  // A Subject shows its INDIVIDUAL Conclusions — each scored on its OWN
  // confidence, NOT a single rolled-up sentiment score. Layout:
  //   1. Subject hero (title + meta pills).
  //   2. Ranked confidence bars — one compact row per Conclusion (statement +
  //      a bar filled to its confidence %, bold percentage, trend glyph),
  //      coloured by cycling the category palette and sorted by confidence;
  //      faded conclusions (below the display floor) render dimmed. Clicking a
  //      row selects that Conclusion and drives the inspector.
  //   3. Master-detail grid: left = conclusions list (statement, ConfidenceBar
  //      with trend, Pin/Dismiss, "view evidence"); right = sticky Evidence
  //      Inspector for the selected conclusion (evidence rows + Confidence
  //      History list, which carries the over-time detail). Faded conclusions
  //      stay listed with their historical arc.
  //
  // The breadcrumb back affordance is rendered by the Insights workspace shell
  // (insights/+page.svelte), so we do NOT duplicate it here. `onBack` is exposed
  // but the shell already provides the primary back control.
  //
  // Props:
  //   subject: string     — the Subject name being inspected.
  //   onBack: () => void  — return to the Subjects index.

  import { untrack } from "svelte";
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
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
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

  type Trend = "up" | "steady" | "down" | "faded";

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  // In-flight Pin/Dismiss guard. `actionKind` records WHICH action is running so
  // only that button shows its busy affordance (the sibling stays disabled).
  let actionId = $state<number | null>(null);
  let actionKind = $state<"pin" | "dismiss" | null>(null);

  // Activities resolved lazily for richer evidence rows + Timeline handoff. Maps
  // activityId → Activity (title/time/category + raw evidence refs).
  let activities = $state<Map<number, Activity>>(new Map());

  // Frame previews for screen-sourced evidence rows. Maps frameId → asset URL.
  // Best-effort; rows without a resolved preview keep the colored placeholder.
  let thumbnailCache = $state<Map<number, string>>(new Map());

  // Single-hue magnitude ramp (mirrors MiniBars): confidence is encoded by
  // INTENSITY of one accent hue, not by a category colour. A high-confidence
  // conclusion reads as pure accent; lower confidence blends further toward the
  // track surface. Using the --cat-* palette here previously implied a category
  // encoding the data doesn't carry — this ramp removes that false signal.
  function magnitudeFill(confidence: number): string {
    const v = Math.max(0, Math.min(1, confidence));
    // Blend up to 62% into the track surface at zero confidence; pure accent at 1.
    const fade = Math.round((1 - v) * 62);
    return `color-mix(in oklab, var(--app-accent) ${100 - fade}%, var(--app-surface-hover))`;
  }

  // Stable display order: conclusions sorted by confidence desc.
  const orderedConclusions = $derived.by<Conclusion[]>(() => {
    if (!view) return [];
    return [...view.conclusions].sort((a, b) => b.confidence - a.confidence);
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
    // Raw frame id backing a screen-sourced row, used to load a thumbnail. Null
    // for audio rows or rows whose Activity hasn't resolved yet.
    frameId: number | null;
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
      const frameId =
        firstRef && firstRef.subjectType === "frame" ? firstRef.subjectId : null;
      return {
        activityId: e.activityId,
        stance: e.stance,
        title: activity?.title ?? e.activityTitle ?? `Activity #${e.activityId}`,
        atMs: activity?.startedAtMs ?? e.activityStartedAtMs ?? null,
        category: activity?.category ?? null,
        sourceType,
        frameId,
      };
    });
  });

  // Load frame previews for the visible screen evidence rows. Best-effort and
  // batched; mirrors Chat.svelte's source-thumbnail loader. Skips ids already
  // cached so re-selecting a conclusion is free.
  async function loadEvidenceThumbnails(rows: EvidenceRow[]): Promise<void> {
    // Read the cache untracked: this loader runs synchronously inside a $effect
    // keyed on evidenceRows, so a tracked thumbnailCache.has() read before the
    // first await would make the cache a dependency — and the `thumbnailCache =
    // next` write below would then re-run the effect for one wasted pass.
    const cache = untrack(() => thumbnailCache);
    const wanted = rows
      .map((r) => r.frameId)
      .filter((id): id is number => id != null && !cache.has(id));
    const uniqueIds = Array.from(new Set(wanted));
    if (uniqueIds.length === 0) return;
    try {
      const response = await invoke<FrameScrubPreviewsDto>(
        "get_frame_scrub_previews",
        { request: { frameIds: uniqueIds } },
      );
      const next = new Map(thumbnailCache);
      for (const entry of response.previews) {
        if (entry.preview) {
          next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
        }
      }
      thumbnailCache = next;
    } catch {
      // Thumbnails are best-effort; rows fall back to the colored placeholder.
    }
  }

  $effect(() => {
    // Re-run whenever the selected conclusion's evidence rows change.
    void loadEvidenceThumbnails(evidenceRows);
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
    actionKind = "pin";
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      await loadSubject();
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
      await loadSubject();
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
    } finally {
      actionId = null;
      actionKind = null;
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

      <div class="md-grid">
        <div class="card rank-card">
          <div class="rank-head">
            <Skeleton variant="text" width="170px" height="13px" />
            <Skeleton variant="text" width="120px" height="11px" />
          </div>
          <div class="sk-rank-list">
            {#each Array.from({ length: 5 }) as _, i (i)}
              <div class="sk-rank-row">
                <Skeleton variant="text" width="62%" height="13px" />
                <Skeleton width="150px" height="7px" radius="999px" />
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

    <!-- Master-detail: ranked conclusions (master) + evidence inspector -->
    <div class="md-grid">
      <!-- LEFT: ranked conclusions, sorted by confidence. Selecting a row
           drives the inspector + its actions on the right. -->
      <div class="card rank-card">
        <div class="rank-head">
          <div class="section-title">Conclusion confidence</div>
          <span class="rank-head-note">
            {conclusionCount} ranked{#if fadedCount > 0} · {fadedCount} below floor{/if}
          </span>
        </div>

        {#if orderedConclusions.length > 0}
          <ul class="rank-list">
            {#each orderedConclusions as c (c.id)}
              {@const t = trendFor(c)}
              <li>
                <button
                  type="button"
                  class="rank-row"
                  class:is-selected={selectedId === c.id}
                  class:rank-row--faded={c.status === "faded"}
                  title={c.statement}
                  onclick={() => selectConclusion(c.id)}
                >
                  <span class="rank-statement">
                    {#if c.pinned}<span class="rank-pin" aria-hidden="true">★</span
                      >{/if}{c.statement}
                  </span>
                  <span class="rank-meter">
                    <span class="rank-track">
                      <span
                        class="rank-fill"
                        class:rank-fill--faded={c.status === "faded"}
                        style="width:{pct(c.confidence)}%; background:{magnitudeFill(c.confidence)};"
                      ></span>
                    </span>
                    <span class="rank-pct">{pct(c.confidence)}%</span>
                    <span class="rank-trend rank-trend--{t}" aria-hidden="true">
                      {trendLabel(t).split(" ")[0]}
                    </span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="rank-empty">No conclusions recorded yet.</p>
        {/if}
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
              style="background: {magnitudeFill(selectedConclusion.confidence)};"
            ></span>
            {selectedConclusion.statement}
            <span class="insp-conf">
              · {pct(selectedConclusion.confidence)}%
              {trendLabel(trendFor(selectedConclusion)).split(" ")[0]}
            </span>
          </div>

          <div class="insp-actions">
            {#if selectedConclusion.status === "faded"}
              <div class="floor-note">
                <span class="glyph" aria-hidden="true">⊘</span>
                Below display floor — kept for history.
              </div>
            {/if}
            <div class="insp-action-row">
              <button
                type="button"
                class="btn"
                class:btn--accent={selectedConclusion.pinned}
                class:btn--busy={actionId === selectedConclusion.id &&
                  actionKind === "pin"}
                disabled={actionId !== null}
                onclick={() => void togglePinned(selectedConclusion)}
              >
                {#if actionId === selectedConclusion.id && actionKind === "pin"}
                  <span class="btn-spinner" aria-hidden="true"></span>
                  {selectedConclusion.pinned ? "Unpinning…" : "Pinning…"}
                {:else}
                  {selectedConclusion.pinned ? "★ Pinned" : "Pin"}
                {/if}
              </button>
              <button
                type="button"
                class="btn"
                class:btn--busy={actionId === selectedConclusion.id &&
                  actionKind === "dismiss"}
                disabled={actionId !== null}
                onclick={() => void dismiss(selectedConclusion)}
              >
                {#if actionId === selectedConclusion.id && actionKind === "dismiss"}
                  <span class="btn-spinner" aria-hidden="true"></span>
                  Dismissing…
                {:else}
                  Dismiss
                {/if}
              </button>
            </div>
          </div>

          <div class="ev-list">
            {#if evidenceRows.length === 0}
              <p class="ev-empty">No grounding evidence linked.</p>
            {:else}
              {#each evidenceRows as ev (ev.activityId)}
                {@const thumbUrl =
                  ev.frameId != null
                    ? (thumbnailCache.get(ev.frameId) ?? null)
                    : null}
                <div class="ev-item">
                  <div
                    class="ev-thumb"
                    class:is-screen={ev.sourceType !== "audio"}
                    class:is-audio={ev.sourceType === "audio"}
                  >
                    {#if thumbUrl}
                      <img class="ev-thumb-img" src={thumbUrl} alt="" />
                    {/if}
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

  .section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }

  /* ---- Ranked confidence bars (master column) ---- */
  .rank-card {
    min-width: 0; /* grid cell — allow rank rows to truncate */
  }
  .rank-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }
  .rank-head-note {
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .rank-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rank-row {
    width: 100%;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 16px;
    font: inherit;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    padding: 7px 9px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease;
  }
  .rank-row:hover {
    background: var(--app-surface-hover);
  }
  .rank-row:not(:disabled):active {
    transform: translateY(1px);
  }
  .rank-row:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .rank-row.is-selected {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .rank-row--faded {
    opacity: 0.55;
  }
  .rank-statement {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    color: var(--app-text);
  }
  .rank-pin {
    color: var(--app-accent-strong);
    margin-right: 5px;
    font-size: 11px;
  }
  .rank-meter {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    flex: 0 0 auto;
  }
  .rank-track {
    position: relative;
    width: 150px;
    height: 7px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
  }
  .rank-fill {
    position: absolute;
    inset: 0 auto 0 0;
    height: 100%;
    min-width: 3px;
    border-radius: 999px;
  }
  .rank-fill--faded {
    opacity: 0.5;
  }
  .rank-pct {
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
    min-width: 34px;
    text-align: right;
  }
  .rank-trend {
    font-size: 9px;
    width: 11px;
    text-align: center;
    line-height: 1;
  }
  .rank-trend--up {
    color: var(--app-accent);
  }
  .rank-trend--down {
    color: var(--app-danger);
  }
  .rank-trend--steady {
    color: var(--app-text-subtle);
  }
  .rank-trend--faded {
    color: var(--app-text-faint);
  }
  .rank-empty {
    font-size: 11.5px;
    color: var(--app-text-muted);
    margin: 8px 0 0;
  }

  /* ---- Master-detail ---- */
  .md-grid {
    display: grid;
    grid-template-columns: 1.6fr 1fr;
    gap: 16px;
    align-items: start;
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
  .insp-actions {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 11px 13px;
    border-bottom: 1px solid var(--app-border);
  }
  .insp-actions .floor-note {
    margin: 0;
  }
  .insp-action-row {
    display: flex;
    gap: 6px;
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
  .ev-thumb-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
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
  .ev-link:not(:disabled):active {
    transform: translateY(1px);
  }
  .ev-link:focus-visible {
    outline: none;
    color: var(--app-accent);
    box-shadow: var(--app-ring);
    border-radius: 3px;
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
  .sk-rank-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 4px;
  }
  .sk-rank-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 150px;
    align-items: center;
    gap: 16px;
    padding: 4px 9px;
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

  @media (prefers-reduced-motion: reduce) {
    .rank-row,
    .btn,
    .ev-item,
    .ev-link {
      transition: none;
    }
    .btn:not(:disabled):active,
    .rank-row:not(:disabled):active,
    .ev-link:not(:disabled):active {
      transform: none;
    }
    .btn-spinner {
      animation: none;
    }
  }

  @media (max-width: 900px) {
    .md-grid {
      grid-template-columns: 1fr;
    }
    .inspector {
      position: static;
    }
  }

  @media (max-width: 560px) {
    .rank-track {
      width: 96px;
    }
    .sk-rank-row {
      grid-template-columns: minmax(0, 1fr) 96px;
    }
  }
</style>
