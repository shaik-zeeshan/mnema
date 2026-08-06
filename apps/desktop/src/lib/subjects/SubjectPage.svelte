<script lang="ts">
  // One subject, opened — mockup 09's second frame, in direction 01. Same grid,
  // four tiles: the SUBJECT header, the horizontally scrolling CONCLUSIONS
  // strip, the selected CONCLUSION with the two controls that change it, and
  // THE STORY OVER TIME.
  //
  // This shell owns the reads and the writes; the three payload tiles only
  // render + call back. Every read is a command the shipping Subject detail
  // already uses — `get_user_context_subject`, `list_user_context_activities`,
  // `get_frame_scrub_previews` — and Pin/Dismiss are the real
  // `user_context_set_pinned` / `user_context_dismiss_conclusion` writes.
  // Neither is a delete: Pin exempts a conclusion from decay, Dismiss resets it
  // behind a 2× resurface bar.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { toast } from "$lib/toast.svelte";
  import type {
    Activity,
    ActivityEvidenceRef,
    Conclusion,
    SubjectTrajectory,
    SubjectView,
  } from "$lib/types/recording";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { humanizeError } from "$lib/format-error";
  import { buildTimeline } from "$lib/insights/subjectTimeline";
  import { debounce } from "$lib/insights/subjectsTiers";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import Glyph from "$lib/overview/Glyph.svelte";
  import ConclusionsStrip from "./ConclusionsStrip.svelte";
  import ConclusionTile from "./ConclusionTile.svelte";
  import StoryTile from "./StoryTile.svelte";
  import { ago } from "./subjects-format";

  let { subject }: { subject: string } = $props();

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let selectedId = $state<number | null>(null);
  let actionId = $state<number | null>(null);
  let actionKind = $state<"pin" | "dismiss" | null>(null);
  let activities = $state<Map<number, Activity>>(new Map());
  let thumbnails = $state<Map<number, string>>(new Map());

  const trajectoryById = $derived.by<Map<number, SubjectTrajectory>>(() => {
    const m = new Map<number, SubjectTrajectory>();
    if (view) for (const t of view.trajectories) m.set(t.conclusionId, t);
    return m;
  });
  const selected = $derived.by<Conclusion | null>(() => {
    if (!view || selectedId === null) return null;
    return view.conclusions.find((c) => c.id === selectedId) ?? null;
  });
  const selectedTrajectory = $derived(
    selectedId === null ? undefined : trajectoryById.get(selectedId),
  );
  const events = $derived(
    selected ? buildTimeline(selected, selectedTrajectory, activities) : [],
  );

  // ── header chips — all five are counted, never estimated (G8) ────────────
  const fadedCount = $derived(
    view?.conclusions.filter((c) => c.status === "faded").length ?? 0,
  );
  const firstSeenMs = $derived(
    view?.conclusions.length
      ? Math.min(...view.conclusions.map((c) => c.formedAtMs))
      : 0,
  );
  const lastEvidenceMs = $derived(
    view?.conclusions.length
      ? Math.max(...view.conclusions.map((c) => c.lastSupportedAtMs))
      : 0,
  );
  // Distinct activity ids across every conclusion's evidence.
  const linkedActivities = $derived.by<number>(() => {
    const ids = new Set<number>();
    for (const c of view?.conclusions ?? [])
      for (const e of c.evidence) ids.add(e.activityId);
    return ids.size;
  });

  async function loadSubject(): Promise<void> {
    loading = true;
    try {
      const next = await invoke<SubjectView>("get_user_context_subject", {
        subject,
      });
      view = next;
      loadError = null;
      const stillExists =
        selectedId !== null && next.conclusions.some((c) => c.id === selectedId);
      if (!stillExists) {
        selectedId =
          [...next.conclusions].sort((a, b) => b.confidence - a.confidence)[0]
            ?.id ?? null;
      }
      void loadActivities(next);
    } catch (error) {
      loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  // Bounded paged scan resolving the Activities this subject's conclusions cite
  // (port of the shipping detail's loader) so the story rows carry real titles,
  // times and categories rather than an id.
  async function loadActivities(v: SubjectView): Promise<void> {
    const wanted = new Set<number>();
    for (const c of v.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
    if (wanted.size === 0) return;
    const resolved = new Map<number, Activity>();
    const PAGE = 200;
    for (let page = 0; page < 6; page++) {
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

  // Frame previews for the story's screen events. Best-effort: an unresolved id
  // keeps the empty media well rather than an invented picture.
  async function loadThumbnails(): Promise<void> {
    const cache = untrack(() => thumbnails);
    const wanted = events
      .map((ev) => (ev.kind === "evidence" && ev.sourceType === "screen" ? ev.frameId : null))
      .filter((id): id is number => id != null && !cache.has(id));
    const ids = Array.from(new Set(wanted));
    if (ids.length === 0) return;
    try {
      const response = await invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
        request: { frameIds: ids },
      });
      const next = new Map(cache);
      for (const entry of response.previews) {
        if (entry.preview) next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
      }
      thumbnails = next;
    } catch {
      // Thumbnails are best-effort.
    }
  }
  $effect(() => {
    void loadThumbnails();
  });

  // ── writes ───────────────────────────────────────────────────────────────
  async function togglePin(id: number, pinned: boolean): Promise<void> {
    if (actionId !== null) return;
    actionId = id;
    actionKind = "pin";
    try {
      await invoke("user_context_set_pinned", { id, pinned });
      await loadSubject();
    } catch (error) {
      // A failed write must not blank the surface — the loaded subject stays.
      toast({
        tone: "error",
        title: pinned ? "Couldn't pin conclusion" : "Couldn't unpin conclusion",
        message: humanizeError(error),
      });
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  async function dismiss(id: number): Promise<void> {
    if (actionId !== null) return;
    actionId = id;
    actionKind = "dismiss";
    try {
      await invoke("user_context_dismiss_conclusion", { id });
      await loadSubject();
    } catch (error) {
      toast({
        tone: "error",
        title: "Couldn't dismiss conclusion",
        message: humanizeError(error),
      });
    } finally {
      actionId = null;
      actionKind = null;
    }
  }

  // Subject → Chat hand-off. The store's selection bus is a singleton that
  // outlives the navigation, so seeding it and then routing to the Chat
  // destination lands a prefilled, un-sent composer.
  function askAboutSubject(): void {
    conversationStore.requestNewChat(
      `Tell me what you know about ${subject} and what I've been doing related to it.`,
    );
    void goto("/insights");
  }

  // ── the Timeline hand-off ────────────────────────────────────────────────
  let frameModalOpen = $state(false);
  let frameModalId = $state<number | null>(null);
  let frameModalOpenInTimeline = $state<(() => void) | null>(null);

  function onOpenEvidence(activityId: number, frameId: number | null): void {
    if (frameId != null) {
      frameModalId = frameId;
      frameModalOpenInTimeline = () =>
        void openRef({ subjectType: "frame", subjectId: frameId, isHeadline: false });
      frameModalOpen = true;
      return;
    }
    void openRef(activities.get(activityId)?.evidence?.[0]);
  }

  async function openRef(ref: ActivityEvidenceRef | undefined): Promise<void> {
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
      // fall through to a plain Timeline navigation
    }
    toast({ title: "Opening Timeline", message: "Couldn't pinpoint the exact moment." });
    void goto("/");
  }

  $effect(() => {
    subject; // reload when the drill-in target changes
    void loadSubject();
    const reload = debounce(() => void loadSubject(), 500);
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
</script>

<div class="sb scroll">
  {#if loadError && !view}
    <div class="bento">
      <div class="tile tile--w4 tile--static state">
        <div class="tile__h"><span class="t-label">Subject</span></div>
        <p class="t-ui strong">Couldn't load this subject.</p>
        <p class="t-meta">{loadError}</p>
        <button type="button" class="btn btn--sm" onclick={() => void loadSubject()}>
          Try again
        </button>
      </div>
    </div>
  {:else}
    <div class="bento subj__grid">
      <!-- SUBJECT — the header tile. Five chips, five counted values. -->
      <div class="tile tile--w4 tile--static subj__head">
        <div class="tile__h">
          <span class="t-label">Subject</span>
          <button type="button" class="btn btn--sm ask" onclick={askAboutSubject}>
            <span class="ask__g"><Glyph name="spark-o" /></span>
            Ask AI about {subject}
          </button>
        </div>
        <div>
          <p class="t-display">{subject}</p>
          {#if view}
            <div class="pillrow">
              <span class="mpill"><b>{view.conclusions.length}</b>conclusions</span>
              {#if fadedCount}<span class="mpill"><b>{fadedCount}</b>below floor</span>{/if}
              <span class="mpill mpill--trail">first seen <b>{ago(firstSeenMs)}</b></span>
              <span class="mpill mpill--trail">last evidence <b>{ago(lastEvidenceMs)}</b></span>
              <span class="mpill"><b>{linkedActivities}</b>linked activities</span>
            </div>
          {:else}
            <div class="pillrow"><span class="mpill">{loading ? "Reading…" : "—"}</span></div>
          {/if}
        </div>
      </div>

      <ConclusionsStrip
        conclusions={view?.conclusions ?? []}
        trajectories={trajectoryById}
        {selectedId}
        onSelect={(id) => (selectedId = id)}
      />

      <ConclusionTile
        conclusion={selected}
        trajectory={selectedTrajectory}
        {actionId}
        {actionKind}
        onTogglePin={togglePin}
        onDismiss={dismiss}
      />

      <StoryTile {events} {thumbnails} onOpenEvidence={onOpenEvidence} />
    </div>
  {/if}
</div>

<FrameDetailModal
  open={frameModalOpen}
  frameId={frameModalId}
  onClose={() => (frameModalOpen = false)}
  onOpenInTimeline={frameModalOpenInTimeline ?? undefined}
/>

<style>
  .sb {
    flex: 1 1 auto; /* height:100% collapses under WKWebView — always flex */
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: var(--s-16) var(--pad-window) var(--s-16);
  }
  /* Two fixed rows and one that takes the rest: the header and the strip are
     the same height on every subject, so the belief and its story get whatever
     the window has left and scroll inside themselves. */
  .subj__grid {
    flex: 1 1 auto;
    min-height: 0;
    grid-template-rows: 108px 176px minmax(0, 1fr);
  }
  .subj__head > div {
    min-width: 0;
  }
  .subj__head .t-display {
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ask {
    margin-left: auto;
  }
  .ask__g {
    flex: 0 0 auto;
    width: 12px;
    height: 12px;
    color: var(--app-accent);
  }

  .pillrow {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-6);
    margin-top: var(--s-6);
  }
  .mpill {
    display: inline-flex;
    align-items: center;
    height: var(--o-badge);
    padding: 0 var(--s-8);
    border-radius: var(--r-pill);
    background: var(--app-surface-hover);
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .mpill b {
    margin-right: 4px;
    font-family: var(--app-font-mono);
    font-weight: var(--w-medium);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .mpill--trail b {
    margin: 0 0 0 4px;
  }

  .state {
    gap: var(--s-6);
  }
  .state p {
    margin: 0;
    max-width: 70ch;
  }
  .state .btn {
    align-self: flex-start;
    margin-top: var(--s-4);
  }
  .strong {
    color: var(--app-text-strong);
  }

  /* 800×600: the header tightens but the strip keeps its height — a narrower
     card needs the SAME vertical room, because the statement wraps to more
     lines, not fewer. The story tile absorbs the difference by scrolling. */
  @media (max-width: 900px) {
    .subj__grid {
      grid-template-rows: 104px 178px minmax(0, 1fr);
    }
  }
</style>
