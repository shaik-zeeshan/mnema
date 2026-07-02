<script lang="ts">
  // SubjectDetail — the drill-in detail surface for a single Subject (#106).
  //
  // A Subject shows its INDIVIDUAL Conclusions — each scored on its OWN
  // confidence, NOT a single rolled-up sentiment score. Composition:
  //   1. Subject hero (title + meta pills + "Ask AI" hand-off).
  //   2. ConclusionStrip — horizontally-scrollable, self-sorting strip of
  //      conclusion cards; selecting one drives the timeline below.
  //   3. ConclusionTimeline — the selected conclusion's header + unified
  //      vertical timeline of evidence, confidence markers, and its origin.
  //
  // This shell owns state/fetch, the realtime refresh, selection preservation,
  // pin/dismiss commands, thumbnail loading, and the Timeline hand-off; the two
  // child components only render + call back.
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
  import { message } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import type {
    Conclusion,
    SubjectView,
    SubjectTrajectory,
    Activity,
  } from "$lib/types/recording";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import ConclusionStrip from "$lib/insights/ConclusionStrip.svelte";
  import ConclusionTimeline from "$lib/insights/ConclusionTimeline.svelte";
  import { buildTimeline } from "$lib/insights/subjectTimeline";
  import { humanizeError } from "$lib/format-error";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";

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

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  // In-flight Pin/Dismiss guard. `actionKind` records WHICH action is running so
  // only that button shows its busy affordance (the sibling stays disabled).
  let actionId = $state<number | null>(null);
  let actionKind = $state<"pin" | "dismiss" | null>(null);

  // Activities resolved lazily for richer timeline events + Timeline handoff.
  // Maps activityId → Activity (title/time/category + raw evidence refs).
  let activities = $state<Map<number, Activity>>(new Map());

  // Frame previews for screen-sourced timeline events. Maps frameId → asset URL.
  // Best-effort; events without a resolved preview keep the colored placeholder.
  let thumbnailCache = $state<Map<number, string>>(new Map());

  const trajectoryById = $derived.by<Map<number, SubjectTrajectory>>(() => {
    const m = new Map<number, SubjectTrajectory>();
    if (view) for (const t of view.trajectories) m.set(t.conclusionId, t);
    return m;
  });

  const selectedConclusion = $derived.by<Conclusion | null>(() => {
    if (!view || selectedId === null) return null;
    return view.conclusions.find((c) => c.id === selectedId) ?? null;
  });

  const selectedTrajectory = $derived(
    selectedId === null ? undefined : trajectoryById.get(selectedId),
  );

  // The selected conclusion's merged, newest-first event stream. Empty when
  // nothing is selected. buildTimeline is pure + tested (subjectTimeline.ts).
  const timelineEvents = $derived(
    selectedConclusion
      ? buildTimeline(selectedConclusion, selectedTrajectory, activities)
      : [],
  );

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

  // Load frame previews for the visible screen timeline events. Best-effort and
  // batched; mirrors Chat.svelte's source-thumbnail loader. Skips ids already
  // cached so re-selecting a conclusion is free.
  async function loadTimelineThumbnails(): Promise<void> {
    // Read the cache untracked: this loader runs synchronously inside a $effect
    // keyed on timelineEvents, so a tracked thumbnailCache.has() read before the
    // first await would make the cache a dependency — and the `thumbnailCache =
    // next` write below would then re-run the effect for one wasted pass.
    const cache = untrack(() => thumbnailCache);
    const wanted = timelineEvents
      .map((ev) =>
        ev.kind === "evidence" && ev.sourceType === "screen"
          ? ev.frameId
          : null,
      )
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
      // Thumbnails are best-effort; events fall back to the colored placeholder.
    }
  }

  $effect(() => {
    // Re-run whenever the selected conclusion's timeline events change.
    void loadTimelineThumbnails();
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
      loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  // Resolve the Activities this Subject's conclusions cite so timeline events
  // show real titles/times/source type and so "view in Timeline" can hand off to
  // a raw frame/audio segment. Best-effort: paged scan of recent Activities;
  // events without a resolved Activity fall back to the conclusion's stored ref.
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

  // Subject → Chat hand-off (#106 / fix #5). Open a fresh Chat thread with the
  // composer prefilled to ask about THIS subject. The hand-off routes through
  // the shared store's selection bus (the Insights shell watches the same bus
  // and switches to the Chat sub-surface); the prompt is seeded, not auto-sent,
  // so the user can review/edit before pressing Enter. The engine recalls what
  // it knows about the subject through its brokered tools when answering.
  function askAboutSubject(): void {
    conversationStore.requestNewChat(
      `Tell me what you know about ${subject} and what I've been doing related to it.`,
    );
  }

  async function togglePin(id: number, pinned: boolean): Promise<void> {
    if (actionId !== null) return;
    actionId = id;
    actionKind = "pin";
    try {
      await invoke("user_context_set_pinned", { id, pinned });
      await loadSubject();
    } catch (error) {
      // A write failure must NOT blank the surface — surface it in a dialog and
      // leave the loaded subject intact. `pinned` is the DESIRED new state.
      const detail = humanizeError(error);
      await message(detail, {
        title: pinned ? "Couldn't pin conclusion" : "Couldn't unpin conclusion",
        kind: "error",
      });
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  async function dismissConclusion(id: number): Promise<void> {
    if (actionId !== null) return;
    actionId = id;
    actionKind = "dismiss";
    try {
      await invoke("user_context_dismiss_conclusion", { id });
      await loadSubject();
    } catch (error) {
      // A write failure must NOT blank the surface — surface it in a dialog and
      // leave the loaded subject intact.
      const detail = humanizeError(error);
      await message(detail, {
        title: "Couldn't dismiss conclusion",
        kind: "error",
      });
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  // "view in Timeline" — best-effort Activity-span handoff to the raw Timeline.
  // We resolve the Activity's first raw evidence ref (frame/audio segment) and
  // ask the main window to land there. If no raw ref is resolvable, fall back to
  // navigating to the Timeline surface so the action never dead-ends.
  async function onViewInTimeline(activityId: number): Promise<void> {
    const activity = activities.get(activityId);
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
    // No precise frame/audio span was resolvable (or the handoff threw) — tell
    // the user before the graceful fallback so jumping to the Timeline top isn't
    // a silent surprise that looks like the wrong moment opened.
    await message("Couldn't pinpoint the exact moment — opening the Timeline.", {
      title: "Opening Timeline",
      kind: "info",
    });
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
  {#if loadError && !view}
    <div class="state state--error">
      <p class="state-title">Couldn't load this subject.</p>
      <p class="state-detail">{loadError}</p>
      <button
        type="button"
        class="state-retry"
        onclick={() => void loadSubject()}
        disabled={loading}
      >
        <span class="state-retry-ico" aria-hidden="true">↻</span>
        Try again
      </button>
    </div>
  {:else if loading && !view}
    <!-- Loading skeleton — hero + conclusion strip, matching the real layout so
         the swap to loaded content causes no layout shift. The "nothing
         concluded" empty state only renders once loaded. -->
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

      <div class="sk-strip">
        {#each Array.from({ length: 3 }) as _, i (i)}
          <div class="sk-card">
            <Skeleton variant="text" width="42px" height="11px" />
            <Skeleton variant="text" width="90%" height="13px" />
            <Skeleton variant="text" width="70%" height="13px" />
            <Skeleton width="100%" height="6px" radius="999px" />
          </div>
        {/each}
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
          {#if fadedCount > 0}
            <span class="pill">{fadedCount} below floor</span>
          {/if}
          <span class="pill">first seen {relativeTime(firstSeenMs)}</span>
          <span class="pill">last evidence {relativeTime(lastEvidenceMs)}</span>
          {#if linkedActivityCount > 0}
            <span class="pill">{linkedActivityCount} linked activities</span>
          {/if}
        </div>
      </div>
      <!-- Subject → Chat hand-off: open a fresh chat prefilled to ask the engine
           about this subject (the prompt is seeded for review/edit, not sent). -->
      <button type="button" class="ask-subject" onclick={askAboutSubject}>
        <span class="ask-subject-glyph" aria-hidden="true">✦</span>
        Ask AI about {subject}
      </button>
    </div>

    <!-- Conclusion strip (master). Passes ALL conclusions incl. faded — the strip
         de-emphasizes faded cards but keeps them selectable. -->
    <ConclusionStrip
      conclusions={view.conclusions}
      trajectories={trajectoryById}
      {selectedId}
      onSelect={(id) => (selectedId = id)}
      onTogglePin={togglePin}
      {actionId}
    />

    <!-- Selected conclusion's timeline (detail). -->
    {#if selectedConclusion}
      <ConclusionTimeline
        events={timelineEvents}
        conclusion={selectedConclusion}
        trajectory={selectedTrajectory}
        thumbnails={thumbnailCache}
        {actionId}
        {actionKind}
        onTogglePin={togglePin}
        onDismiss={dismissConclusion}
        {onViewInTimeline}
      />
    {/if}
  {/if}
</section>

<style>
  .subject-detail {
    display: flex;
    flex-direction: column;
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
    font-size: 24px;
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

  /* Subject → Chat hand-off button (top-right of the hero). The primary outbound
     action from a subject, so it carries the accent treatment. */
  .ask-subject {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 11.5px;
    padding: 6px 12px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    white-space: nowrap;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .ask-subject:hover {
    border-color: var(--app-accent);
  }
  .ask-subject:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .ask-subject:not(:disabled):active {
    transform: translateY(1px);
  }
  .ask-subject-glyph {
    font-size: 11px;
    line-height: 1;
  }

  /* ---- Loading skeleton helpers ---- */
  .sk-hero-title {
    margin: 0 0 10px;
  }
  .sk-strip {
    display: flex;
    gap: 10px;
  }
  .sk-card {
    flex: 0 0 auto;
    width: 240px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface);
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
  /* Retry affordance — mirrors the Overview lede's "↻ re-read" pill. */
  .state-retry {
    align-self: flex-start;
    margin-top: 4px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 7px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: transparent;
    color: var(--app-text-subtle);
    font: inherit;
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .state-retry:hover:not(:disabled) {
    color: var(--app-accent);
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .state-retry:not(:disabled):active {
    transform: translateY(1px);
  }
  .state-retry:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .state-retry-ico {
    font-size: 12px;
    line-height: 1;
    letter-spacing: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .ask-subject,
    .state-retry {
      transition: none;
    }
    .ask-subject:not(:disabled):active,
    .state-retry:not(:disabled):active {
      transform: none;
    }
  }
</style>
