<script lang="ts">
  // Subject page (mockup 1-A "Numbered pages, pinned two-pane"): a slim strip
  // (crumb, four numbers, a small chart, Ask) over two panes that fill the
  // window — conclusions on the left, the chosen one's evidence on the right —
  // each scrolling on its own with its pager pinned at its foot, so the row you
  // picked and its evidence are always on screen together. The page never
  // scrolls. Owns the fetch, the realtime refresh, selection, paging, keys,
  // Pin / delayed-commit Dismiss, and the Timeline hand-off. Each conclusion
  // keeps its OWN confidence — the subject is never rolled up into one score.
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
  import SubjectStrip, { type StripStats } from "$lib/insights/SubjectStrip.svelte";
  import ConclusionRail from "$lib/insights/ConclusionRail.svelte";
  import ConclusionPicker from "$lib/insights/ConclusionPicker.svelte";
  import EvidencePane from "$lib/insights/EvidencePane.svelte";
  import { buildTimeline } from "$lib/insights/subjectTimeline";
  import {
    CONCLUSION_PAGE_SIZES,
    EVIDENCE_PAGE,
    debounce,
    fortnightAgo,
    pageSlice,
  } from "$lib/insights/subjectsTiers";
  import { DelayedDismiss, DISMISS_UNDO_MS } from "$lib/insights/dismissUndo.svelte";
  import { relativeTime } from "$lib/insights/conversationStore.svelte";
  import { humanizeError } from "$lib/format-error";
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

  // Paging + which pane ←/→ turns (the one last clicked, or E / C).
  let clPage = $state(0);
  let clSize = $state<number>(CONCLUSION_PAGE_SIZES[0]);
  let evPage = $state(0);
  let focus = $state<"c" | "e">("c");
  // Narrow window: the rail shows as the picker's popover.
  let pickOpen = $state(false);

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
  const clSlice = $derived(pageSlice(shown, clPage, clSize));
  const selPage = $derived(Math.max(0, Math.floor(selIndex / clSize)));

  // Confidence change over the fortnight; null = formed inside it ("new").
  const now = Date.now();
  const fortnightDelta = (c: Conclusion): number | null => {
    const then = fortnightAgo(c, histories.get(c.id) ?? [], now);
    return then === null ? null : pct(c.confidence) - pct(then);
  };

  // ── The strip's four numbers ──
  const stats = $derived.by((): StripStats | null => {
    if (!lead) return null;
    const aboveFloor = shown.filter((c) => c.status !== "faded").length;
    const lastMs = Math.max(...shown.map((c) => c.lastSupportedAtMs));
    const r = relativeTime(lastMs);
    return {
      leadPct: pct(lead.confidence),
      leadDelta: fortnightDelta(lead),
      aboveFloor,
      faded: shown.length - aboveFloor,
      evidence: new Set(shown.flatMap((c) => c.evidence.map((e) => e.activityId))).size,
      lastSupport: new Date(lastMs).toDateString() === new Date().toDateString() ? "today" : /^\d/.test(r) ? `${r} ago` : r,
    };
  });

  // ── Selected conclusion ──
  const events = $derived(
    selected ? buildTimeline(selected, view?.trajectories.find((t) => t.conclusionId === selected.id), activities) : [],
  );
  // Newest supporting Activity that resolved to a raw capture ref.
  const newestRef = $derived.by((): ActivityEvidenceRef | null => {
    let best: Activity | null = null;
    for (const e of selected?.evidence ?? []) {
      const a = activities.get(e.activityId);
      if (e.stance === "support" && a?.evidence[0] && (!best || a.startedAtMs > best.startedAtMs)) best = a;
    }
    return best?.evidence[0] ?? null;
  });
  // A new pick starts its evidence at page 1 (keyed on the id, so a live
  // refresh of the same conclusion keeps the page).
  const selId = $derived(selected?.id ?? null);
  $effect(() => {
    selId;
    evPage = 0;
  });

  function select(id: number): void {
    selectedId = id;
    pickOpen = false;
  }
  // Turn the list to the page holding the selection.
  function reveal(): void {
    if (selIndex >= 0) clPage = Math.floor(selIndex / clSize);
  }
  function step(d: 1 | -1): void {
    const n = Math.min(shown.length - 1, Math.max(0, selIndex + d));
    if (n === selIndex || !shown[n]) return;
    selectedId = shown[n].id;
    reveal();
  }
  function flip(d: 1 | -1): void {
    if (focus === "e") {
      const pages = Math.max(1, Math.ceil(events.length / EVIDENCE_PAGE));
      evPage = Math.min(pages - 1, Math.max(0, Math.min(evPage, pages - 1) + d));
    } else {
      clPage = Math.min(clSlice.pages - 1, Math.max(0, clSlice.page + d));
    }
  }
  // A new size keeps the selected row on screen (or the first row you saw).
  function resize(size: number): void {
    const anchor = selPage === clSlice.page ? selIndex : clSlice.from;
    clSize = size;
    clPage = Math.floor(anchor / size);
  }

  // ── Ask ──
  const askHref = $derived(`/chat?q=${encodeURIComponent(`What do you know about ${subject}?`)}`);
  // ⌘J asks about this subject (capture phase: ahead of the shell's ⌘J).
  function onAskKey(event: KeyboardEvent): void {
    if (askDisabled || frameModalOpen) return;
    if (event.key.toLowerCase() !== "j" || !(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
    event.preventDefault();
    void goto(askHref);
  }

  // ↑/↓ move the pick, ←/→ turn the focused pane's page, E / C focus a pane,
  // Esc closes the picker, else goes back. Never while typing or in a modal.
  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
    if (frameModalOpen || document.querySelector("[aria-modal='true'], dialog[open]")) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.("input, textarea, select, [contenteditable], [role=textbox], [role=combobox]")) return;
    const k = e.key;
    if (k === "Escape") {
      if (pickOpen) pickOpen = false;
      else onBack();
    } else if (!selected) {
      return;
    } else if (k === "ArrowDown" || k === "ArrowUp") {
      step(k === "ArrowDown" ? 1 : -1);
    } else if (k === "ArrowRight" || k === "ArrowLeft" || k === "PageDown" || k === "PageUp") {
      flip(k === "ArrowRight" || k === "PageDown" ? 1 : -1);
    } else if (k.toLowerCase() === "e" || k.toLowerCase() === "c") {
      focus = k.toLowerCase() as "e" | "c";
    } else {
      return;
    }
    e.preventDefault();
  }

  // A press outside the open picker closes it.
  function onPointerDown(e: PointerEvent): void {
    if (!pickOpen) return;
    const t = e.target as HTMLElement | null;
    if (!t?.closest?.(".sd-rail, [data-pick-toggle]")) pickOpen = false;
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

  // Dismissing the pick moves it to its neighbour, on whatever page that is.
  function dismiss(c: Conclusion): void {
    const i = shown.indexOf(c);
    const next = c.id === selected?.id ? (shown[i + 1] ?? shown[i - 1]) : undefined;
    dismissal.start(c);
    if (next) {
      selectedId = next.id;
      reveal();
    }
    showToast({ tone: "ok", text: "Dismissed — it won't be re-inferred", undo: c });
  }

  function undoDismiss(c: Conclusion): void {
    dismissal.undo(c);
    selectedId = c.id;
    reveal();
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
    // A new subject starts fresh: first conclusion, first pages, list focused.
    selectedId = null;
    clPage = 0;
    evPage = 0;
    focus = "c";
    pickOpen = false;
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

<svelte:window onkeydowncapture={onAskKey} onkeydown={onKey} onpointerdown={onPointerDown} />

<section class="sd" aria-label={`Subject — ${subject}`}>
  <SubjectStrip
    {subject}
    {onBack}
    {askHref}
    {askDisabled}
    stats={view ? stats : null}
    conclusions={shown}
    {histories}
    selectedId={selected?.id ?? null}
  />

  {#if loadError && !view}
    <div class="sd-state">
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
    </div>
  {:else if !view}
    <div class="sd-panes" aria-busy="true">
      <span class="mx-sr">Loading {subject}</span>
      <div class="sd-skel-rail">
        {#each Array.from({ length: 6 }) as _, i (i)}
          <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
        {/each}
      </div>
      <div class="sd-skel-detail">
        <div class="mx-skel" style="width:140px;height:9px"></div>
        <div class="mx-skel mx-skel--title"></div>
        <div class="mx-skel mx-skel--title"></div>
        <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
      </div>
    </div>
  {:else if !selected}
    <div class="sd-state">
      <div class="mx-empty mx-empty--center">
        <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">Nothing concluded about {subject} yet.</h4>
        <p class="mx-empty__text">
          Its conclusions were dismissed or haven't formed. Views appear here once two activities support them. Anything
          you dismissed is in Context › Dismissed.
        </p>
      </div>
    </div>
  {:else}
    {@const sel = selected}
    <div class="sd-panes">
      <ConclusionPicker
        conclusion={sel}
        index={selIndex}
        total={shown.length}
        open={pickOpen}
        onToggle={() => {
          pickOpen = !pickOpen;
          focus = "c";
        }}
        onStep={step}
      />
      <ConclusionRail
        total={shown.length}
        slice={clSlice}
        size={clSize}
        selectedId={sel.id}
        selectedPage={selPage}
        delta={fortnightDelta}
        active={focus === "c"}
        open={pickOpen}
        onSelect={select}
        onPage={(p) => (clPage = p)}
        onSize={resize}
        onFocus={() => (focus = "c")}
      />
      <EvidencePane
        conclusion={sel}
        index={selIndex}
        total={shown.length}
        {events}
        history={histories.get(sel.id) ?? []}
        {activities}
        {newestRef}
        page={evPage}
        active={focus === "e"}
        {pinBusy}
        onPage={(p) => (evPage = p)}
        onPin={() => void togglePin(sel)}
        onDismiss={() => dismiss(sel)}
        onOpenEvidence={onViewInTimeline}
        onOpenRef={(ref) => void openRefInTimeline(ref)}
        onFocus={() => (focus = "e")}
      />
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
  /* The host (insights/+page.svelte) gives this the height left under the bar;
     the panes take what the strip leaves and each scrolls on its own. */
  .sd { container: sd / inline-size; flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; margin-top: -14px; }
  .sd-panes {
    position: relative; flex: 1 1 auto; min-height: 0; display: grid;
    grid-template-columns: 380px minmax(0, 1fr); grid-template-rows: minmax(0, 1fr);
    margin: 0 calc(-1 * var(--s-7)); border-top: 1px solid var(--mx-hairline);
  }
  .sd-state { flex: 1 1 auto; min-height: 0; overflow: auto; display: grid; padding: var(--s-6) 0; border-top: 1px solid var(--mx-hairline); }

  .sd-skel-rail { display: grid; align-content: start; gap: 4px; padding: 40px 18px 0 22px; border-right: 1px solid var(--mx-hairline); }
  .sd-skel-detail { display: grid; align-content: start; gap: 12px; padding: 16px 28px; }
  .sd-skel-detail .mx-skel--title { height: 26px; width: 78%; }
  .sd-skel-detail .mx-skel--title + .mx-skel--title { width: 52%; }

  /* `.in-main` doesn't scroll here either; the toast stack pins to the window. */
  .mx-toasts { position: fixed; }

  @container sd (max-width: 920px) {
    .sd-panes { grid-template-columns: 290px minmax(0, 1fr); }
  }
  @container sd (max-width: 620px) {
    .sd-panes { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto minmax(0, 1fr); }
    .sd-skel-rail { display: none; }
  }
</style>
