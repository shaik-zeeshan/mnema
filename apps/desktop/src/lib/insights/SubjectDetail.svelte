<script lang="ts">
  // Subject page (mockup r2-B "Evidence drawer"): the whole window is the
  // subject — crumb + Ask, big title, four stats, the full-width chart, then
  // every conclusion grouped by tier, each tier paging on its own. Clicking a
  // conclusion opens its evidence in a drawer pinned to the window's right
  // under the Insights bar; the page PUSHES (narrows) instead of hiding under
  // it, so the clicked row stays on screen, shaded, beside its proof. Owns the
  // fetch, the realtime refresh, the open conclusion, paging, keys, the drawer's
  // geometry, Pin / delayed-commit Dismiss, and the Timeline hand-off. Each
  // conclusion keeps its OWN confidence — the subject is never rolled up.
  import { tick } from "svelte";
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
  import SubjectHeader from "$lib/insights/SubjectHeader.svelte";
  import ConclusionTiers, { type TierView } from "$lib/insights/ConclusionTiers.svelte";
  import EvidenceDrawer from "$lib/insights/EvidenceDrawer.svelte";
  import { buildTimeline } from "$lib/insights/subjectTimeline";
  import {
    DISPLAY_FLOOR,
    EVIDENCE_PAGE,
    INITIAL_BASE,
    STRONGLY_HELD,
    TIER_PAGE_SIZES,
    convictionTierId,
    debounce,
    fortnightAgo,
    pageSlice,
  } from "$lib/insights/subjectsTiers";
  import { DelayedDismiss, DISMISS_UNDO_MS } from "$lib/insights/dismissUndo.svelte";
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

  const pct = (v: number) => Math.round(v * 100);
  // Top→bottom; membership is the Subjects list's own tier test, per conclusion.
  const TIERS = [
    { id: "strong", title: "Strongly held", note: `confidence ${pct(STRONGLY_HELD)}% and up` },
    { id: "forming", title: "Forming", note: `confidence ${pct(INITIAL_BASE)}–${pct(STRONGLY_HELD)}%` },
    { id: "shaping", title: "Just taking shape", note: `confidence ${pct(DISPLAY_FLOOR)}–${pct(INITIAL_BASE)}%` },
    { id: "fading", title: "Fading", note: `confidence under ${pct(DISPLAY_FLOOR)}% · kept for history` },
  ];
  const tierOf = (c: Conclusion): string =>
    convictionTierId({ topConfidence: c.confidence, faded: c.status === "faded", trend: "steady", lastMovedAtMs: 0 });
  // The drawer takes this share of the Insights pane; below SHEET_BELOW px it
  // covers the pane as a sheet instead of pushing the page.
  const DRAWER_SHARE = 0.44;
  const SHEET_BELOW = 700;
  const PUSH_GAP = 24;

  let view = $state<SubjectView | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let openId = $state<number | null>(null);
  let pinBusy = $state(false);
  let destroyed = false;
  let sectionEl = $state<HTMLElement>();

  // Per-tier page + rows per page, the evidence page, and the section under
  // the pointer (what ←/→ turn).
  let tierPg = $state<Record<string, { page: number; size: number }>>({});
  let evPage = $state(0);
  let evSize = $state(EVIDENCE_PAGE); // replaced by what fits the drawer's height
  let hovered: string | null = null;

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

  const histories = $derived(
    new Map<number, ConfidenceSnapshot[]>((view?.trajectories ?? []).map((t) => [t.conclusionId, t.history])),
  );
  // Shown = by confidence, minus anything inside its Dismiss undo window.
  const shown = $derived(
    (view?.conclusions ?? []).filter((c) => !dismissal.has(c.id)).sort((a, b) => b.confidence - a.confidence),
  );
  const tiers = $derived(
    TIERS.map((t) => ({ ...t, items: shown.filter((c) => tierOf(c) === t.id) })).filter((t) => t.items.length > 0),
  );
  // Reading order for ↑/↓: tier by tier.
  const flat = $derived(tiers.flatMap((t) => t.items));
  const tierViews = $derived(
    tiers.map((t): TierView => {
      const st = tierPg[t.id] ?? { page: 0, size: TIER_PAGE_SIZES[0] };
      return { id: t.id, title: t.title, note: t.note, size: st.size, slice: pageSlice(t.items, st.page, st.size) };
    }),
  );
  const open = $derived(flat.find((c) => c.id === openId) ?? null);
  const openIndex = $derived(open ? flat.indexOf(open) : -1);

  // Confidence change over the fortnight; null = formed inside it ("new").
  const now = Date.now();
  const fortnightDelta = (c: Conclusion): number | null => {
    const then = fortnightAgo(c, histories.get(c.id) ?? [], now);
    return then === null ? null : pct(c.confidence) - pct(then);
  };

  // ── The open conclusion ──
  const events = $derived(
    open ? buildTimeline(open, view?.trajectories.find((t) => t.conclusionId === open.id), activities) : [],
  );
  // Newest supporting Activity that resolved to a raw capture ref.
  const newestRef = $derived.by((): ActivityEvidenceRef | null => {
    let best: Activity | null = null;
    for (const e of open?.evidence ?? []) {
      const a = activities.get(e.activityId);
      if (e.stance === "support" && a?.evidence[0] && (!best || a.startedAtMs > best.startedAtMs)) best = a;
    }
    return best?.evidence[0] ?? null;
  });

  // ── Drawer geometry: pinned to the Insights pane under its bar ──
  let geo = $state({ top: 0, right: 0, bottom: 0, width: 0, push: 0, sheet: false });
  const mainEl = (): HTMLElement | null => sectionEl?.closest<HTMLElement>(".in-main") ?? null;
  function measure(): void {
    const main = mainEl();
    if (!main || !sectionEl) return;
    const r = main.getBoundingClientRect();
    const bar = (main.firstElementChild as HTMLElement | null)?.offsetHeight ?? 0;
    const sheet = r.width < SHEET_BELOW;
    const width = sheet ? r.width : Math.round(r.width * DRAWER_SHARE);
    // The section's own padding (the push) never moves its border box.
    const s = sectionEl.getBoundingClientRect();
    const push = sheet ? 0 : Math.max(0, Math.round(s.right - (r.right - width) + PUSH_GAP));
    const next = { top: r.top + bar, right: innerWidth - r.right, bottom: innerHeight - r.bottom, width, push, sheet };
    if (JSON.stringify(next) !== JSON.stringify(geo)) geo = next;
  }
  $effect(() => {
    const el = sectionEl;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    const main = mainEl();
    if (main) ro.observe(main);
    addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      removeEventListener("resize", measure);
    };
  });

  // ── Scrolling helpers ──
  const rowEl = (id: number) => sectionEl?.querySelector<HTMLElement>(`.sd-row[data-cid="${id}"]`) ?? null;
  const drawerEl = () => document.querySelector<HTMLElement>(".sd-drawer");
  // Bring `el` fully into `box`, below the box's own sticky bar (its first child).
  function keepInView(el: Element | null, box: HTMLElement | null): void {
    if (!el || !box) return;
    const r = el.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    const top = b.top + ((box.firstElementChild as HTMLElement | null)?.offsetHeight ?? 0);
    if (r.top < top) box.scrollTop -= top - r.top + 8;
    else if (r.bottom > b.bottom) box.scrollTop += r.bottom - b.bottom + 8;
  }
  // Run `change`, then scroll the page so row `id` stays where it was on screen
  // (the push reflows every row) and fully in view.
  async function holdRow(id: number, change: () => void): Promise<void> {
    const before = rowEl(id)?.getBoundingClientRect().top;
    change();
    await tick();
    const main = mainEl();
    const row = rowEl(id);
    if (main && row && before !== undefined) main.scrollTop += row.getBoundingClientRect().top - before;
    keepInView(row, main);
  }

  // ── Opening, stepping, paging ──
  function setTier(id: string, page: number, size = tierPg[id]?.size ?? TIER_PAGE_SIZES[0]): void {
    tierPg = { ...tierPg, [id]: { page, size } };
  }
  // Turn the tier holding conclusion `id` to the page that shows it.
  function reveal(id: number): void {
    for (const t of tiers) {
      const i = t.items.findIndex((c) => c.id === id);
      if (i >= 0) return setTier(t.id, Math.floor(i / (tierPg[t.id]?.size ?? TIER_PAGE_SIZES[0])));
    }
  }
  function openConclusion(id: number): void {
    void holdRow(id, () => {
      if (id !== openId) evPage = 0;
      openId = id;
      reveal(id);
    });
  }
  function close(): void {
    if (openId === null) return;
    const id = openId;
    void holdRow(id, () => (openId = null));
  }
  function toggle(id: number): void {
    if (id === openId) close();
    else openConclusion(id);
  }
  function step(d: 1 | -1): void {
    const n = openIndex < 0 ? (d > 0 ? 0 : flat.length - 1) : openIndex + d;
    if (flat[n]) openConclusion(flat[n].id);
  }
  // Turn a section's page; its heading holds still (only rows below it change)
  // and is brought back into view if it had scrolled away.
  async function turnTo(key: string, page: number): Promise<void> {
    if (key === "e") {
      evPage = Math.min(Math.max(0, page), Math.max(0, Math.ceil(events.length / evSize) - 1));
    } else {
      const v = tierViews.find((t) => `t:${t.id}` === key);
      if (!v) return;
      setTier(v.id, Math.min(Math.max(0, page), v.slice.pages - 1));
    }
    await tick();
    const head = document.querySelector(`[data-sec="${key}"] > .sd-h`);
    keepInView(head, key === "e" ? drawerEl() : mainEl());
  }
  function turn(d: 1 | -1): void {
    const firstPaged = tierViews.find((t) => t.slice.pages > 1);
    const key = hovered ?? (open ? "e" : firstPaged ? `t:${firstPaged.id}` : null);
    if (key === "e") void turnTo("e", evPage + d);
    else if (key) {
      const v = tierViews.find((t) => `t:${t.id}` === key);
      if (v) void turnTo(key, v.slice.page + d);
    }
  }
  // A new size keeps the open row (or the first row you saw) on screen.
  function resize(tierId: string, size: number): void {
    const t = tiers.find((x) => x.id === tierId);
    const v = tierViews.find((x) => x.id === tierId);
    if (!t || !v) return;
    const openAt = t.items.findIndex((c) => c.id === openId);
    const anchor = openAt >= v.slice.from && openAt < v.slice.to ? openAt : v.slice.from;
    setTier(tierId, Math.floor(anchor / size), size); // SHOW_ALL → page 0
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

  // ↑/↓ walk the conclusions (across tiers and tier pages) and the drawer
  // follows; ←/→ turn the section under the pointer (else the evidence, else
  // the first paged tier); Enter opens the first; Esc closes the drawer, else
  // goes back. Never while typing or in a modal.
  function onKey(e: KeyboardEvent): void {
    if (e.defaultPrevented || e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
    if (frameModalOpen || document.querySelector("[aria-modal='true'], dialog[open]")) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest?.("input, textarea, select, [contenteditable], [role=textbox], [role=combobox]")) return;
    const k = e.key;
    if (k === "Escape") {
      if (openId !== null) close();
      else onBack();
    } else if (!flat.length) {
      return;
    } else if (k === "ArrowDown" || k === "ArrowUp") {
      step(k === "ArrowDown" ? 1 : -1);
    } else if (k === "ArrowRight" || k === "ArrowLeft") {
      turn(k === "ArrowRight" ? 1 : -1);
    } else if (k === "Enter" && openId === null && !t?.closest?.("button, a")) {
      step(1);
    } else {
      return;
    }
    e.preventDefault();
  }
  function onPointerOver(e: PointerEvent): void {
    hovered = (e.target as Element | null)?.closest?.("[data-sec]")?.getAttribute("data-sec") ?? null;
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

  // Dismissing the open conclusion moves the drawer to its neighbour.
  function dismiss(c: Conclusion): void {
    const i = flat.indexOf(c);
    const next = c.id === openId ? (flat[i + 1] ?? flat[i - 1]) : undefined;
    dismissal.start(c);
    if (next) openConclusion(next.id);
    showToast({ tone: "ok", text: "Dismissed — it won't be re-inferred", undo: c });
  }

  function undoDismiss(c: Conclusion): void {
    dismissal.undo(c);
    clearTimeout(toastTimer);
    toast = null;
    void tick().then(() => openConclusion(c.id));
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
    // A new subject starts fresh: drawer closed, every section on page 1.
    openId = null;
    tierPg = {};
    evPage = 0;
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

<svelte:window onkeydowncapture={onAskKey} onkeydown={onKey} onpointerover={onPointerOver} />

<section
  class="sd"
  bind:this={sectionEl}
  style:padding-right={open ? `${geo.push}px` : null}
  aria-label={`Subject — ${subject}`}
>
  <div class="sd-page">
    <SubjectHeader
      {subject}
      {onBack}
      {askHref}
      {askDisabled}
      conclusions={view && !loadError ? shown : null}
      {histories}
      openId={open?.id ?? null}
      delta={fortnightDelta}
    />

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
        <div class="sd-skel-read">
          <div class="sd-skel-title">
            <div class="mx-skel" style="width:140px;height:9px"></div>
            <div class="mx-skel mx-skel--title"></div>
            <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
          </div>
          <div class="sd-skel-stats">
            {#each Array.from({ length: 4 }) as _, i (i)}<div><div class="mx-skel"></div><div class="mx-skel"></div></div>{/each}
          </div>
        </div>
        <div class="mx-panel sd-skel-chart"><div class="mx-skel" style="height:230px"></div></div>
        {#each Array.from({ length: 5 }) as _, i (i)}
          <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
        {/each}
      </div>
    {:else if !shown.length}
      <div class="mx-empty mx-empty--center">
        <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
        <h4 class="mx-empty__title">Nothing concluded about {subject} yet.</h4>
        <p class="mx-empty__text">
          Its conclusions were dismissed or haven't formed. Views appear here once two activities support them. Anything
          you dismissed is in Context › Dismissed.
        </p>
      </div>
    {:else}
      <div class="mx-reveal" style="--i:4">
        <ConclusionTiers
          total={shown.length}
          tiers={tierViews}
          openId={open?.id ?? null}
          {histories}
          delta={fortnightDelta}
          onOpen={toggle}
          onPage={(id, p) => void turnTo(`t:${id}`, p)}
          onSize={resize}
        />
      </div>
    {/if}
  </div>
</section>

{#if open}
  {@const c = open}
  <EvidenceDrawer
    conclusion={c}
    index={openIndex}
    total={flat.length}
    tierTitle={TIERS.find((t) => t.id === tierOf(c))?.title ?? ""}
    {subject}
    {events}
    history={histories.get(c.id) ?? []}
    {activities}
    {newestRef}
    page={evPage}
    size={evSize}
    onFit={(n) => {
      evPage = Math.floor((evPage * evSize) / n); // keep the first row you were reading
      evSize = n;
    }}
    {pinBusy}
    box={geo}
    sheet={geo.sheet}
    onPage={(p) => void turnTo("e", p)}
    onStep={step}
    onClose={close}
    onPin={() => void togglePin(c)}
    onDismiss={() => dismiss(c)}
    onOpenEvidence={onViewInTimeline}
    onOpenRef={(ref) => void openRefInTimeline(ref)}
  />
{/if}

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
  /* `.sd-page` is the container the page's breakpoints query: the column left
     beside the drawer (the push is the section's padding). */
  .sd-page { container: sd / inline-size; }

  .sd-skel-read {
    display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s-7); align-items: end;
    padding: var(--s-4) 0 var(--s-6); margin-bottom: var(--s-6); border-bottom: 1px solid var(--mx-hairline);
  }
  .sd-skel-title { display: grid; gap: 12px; }
  .sd-skel-title .mx-skel--title { height: 34px; width: 50%; }
  .sd-skel-stats { display: grid; grid-template-columns: repeat(2, 132px); gap: var(--s-5) var(--s-6); }
  .sd-skel-stats div { display: grid; gap: 8px; }
  .sd-skel-stats .mx-skel:first-child { height: 8px; width: 60%; }
  .sd-skel-stats .mx-skel:last-child { height: 24px; width: 90%; }
  .sd-skel-chart { padding: var(--s-5); margin-bottom: var(--s-6); }

  /* `.in-main` scrolls, so the toast stack pins to the window. */
  .mx-toasts { position: fixed; }

  @container sd (max-width: 760px) {
    .sd-skel-read { grid-template-columns: minmax(0, 1fr); }
  }
</style>
