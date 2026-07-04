<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // Insights workspace — the second top-level Surface of Main (alongside the
  // Timeline). It hosts four sub-surfaces (Overview / Subjects / Context / Chat)
  // switched via local state (NOT separate routes), plus a Subject drill-in.
  // The surface toggle that brings the user here lives in the shared titlebar
  // (`+layout.svelte`); this page owns only the sub-nav + sub-surface content.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettings } from "$lib/surface-windows";
  import type {
    AiRuntimeStatus,
    UserContextStatus,
    RecordingSettings,
  } from "$lib/types/recording";
  import Overview from "$lib/insights/Overview.svelte";
  import DayTimeline from "$lib/insights/DayTimeline.svelte";
  import Subjects from "$lib/insights/Subjects.svelte";
  import SubjectDetail from "$lib/insights/SubjectDetail.svelte";
  import Context from "$lib/insights/Context.svelte";
  import Chat from "$lib/insights/Chat.svelte";
  import InsightsRail from "$lib/insights/InsightsRail.svelte";
  import RailResizer from "$lib/insights/RailResizer.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";

  type InsightsTab = "overview" | "journal" | "subjects" | "context" | "chat";

  // Active sub-surface. Default is Overview. Subject-detail is a drill-in over
  // the Subjects tab held in `selectedSubject` (null = the index).
  let view = $state<InsightsTab>("overview");
  let selectedSubject = $state<string | null>(null);

  // Quick Recall → Chat handoff (issue #111, ADR 0031). When a Quick Recall
  // thread is promoted into Chat, the main window is shown/navigated here and a
  // conversation id is delivered (a live `insights_open_conversation` event for
  // a warm window, or the cold-window drain on mount). The handoff now routes
  // through the shared store's selection BUS (`requestOpen`), which Chat watches;
  // the effect below switches this shell to the Chat sub-surface when the bus
  // fires (so a request that arrives while on another tab still lands on Chat).
  function handoffConversation(conversationId: string): void {
    conversationStore.requestOpen(conversationId);
    view = "chat";
    selectedSubject = null;
  }

  // A bus request (from the handoff above, or — in a later slice — the rail
  // clicking a row from another surface) switches the shell to the Chat
  // sub-surface. Track the nonce; skip 0 (nothing requested yet on mount).
  let lastHandoffNonce = 0;
  $effect(() => {
    const pending = conversationStore.pendingOpen;
    untrack(() => {
      if (pending.nonce === 0 || pending.nonce === lastHandoffNonce) return;
      lastHandoffNonce = pending.nonce;
      view = "chat";
      selectedSubject = null;
    });
  });

  function openTab(tab: InsightsTab): void {
    view = tab;
    if (tab !== "subjects") selectedSubject = null;
    // Leaving Chat unmounts it, so its mirror effect can no longer clear the
    // store's open-thread id — clear it here so the rail stops highlighting the
    // previously-open row while a non-Chat sub-surface is showing. (Re-entering
    // Chat remounts it; the bus/handoff sets the active id back as needed.)
    if (tab !== "chat") conversationStore.activeConversationId = null;
  }

  function openSubject(subject: string): void {
    view = "subjects";
    selectedSubject = subject;
  }

  function backToSubjects(): void {
    selectedSubject = null;
  }

  // ── Engine status ────────────────────────────────────────────────────
  // The status state stays in this shell; it is passed down to the rail's
  // footer (<RailFooter> via <InsightsRail>), which renders "engine · <model>"
  // when the Reasoning Engine is on/available, or "engine off · Enable"
  // otherwise. The Enable link opens the Reasoning Engine settings (Access tab).
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let modelLabel = $state<string>("");
  // Distinguishes "still loading the status calls" from "loaded → engine off".
  // Without this the pill flashes "Engine off · Enable" before the status calls
  // resolve, so we show a small skeleton placeholder until the first load lands.
  let statusLoaded = $state(false);

  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) ||
      Boolean(ctxStatus?.engineAvailable),
  );

  function shortModel(model: string): string {
    const trimmed = model.trim();
    if (!trimmed) return "engine";
    // Drop a leading "provider:" prefix and any path, keep the model id tail.
    const afterProvider = trimmed.includes(":") ? trimmed.split(":").pop()! : trimmed;
    const tail = afterProvider.split("/").pop() ?? afterProvider;
    return tail.length > 28 ? `${tail.slice(0, 27)}…` : tail;
  }

  async function loadEngineStatus(): Promise<void> {
    try {
      const [ai, ctx, settings] = await Promise.all([
        invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
        invoke<UserContextStatus>("get_user_context_status").catch(() => null),
        invoke<RecordingSettings>("get_recording_settings").catch(() => null),
      ]);
      aiStatus = ai;
      ctxStatus = ctx;
      if (settings?.aiRuntime) {
        modelLabel = shortModel(settings.aiRuntime.defaultModel?.model ?? "");
      }
    } catch {
      // Best-effort: leave the pill in its "engine off" default on error.
    } finally {
      statusLoaded = true;
    }
  }

  function enableEngine(): void {
    void openSettings("intelligence");
  }

  // ── Rail collapse / expand (Slice 6) ─────────────────────────────────────
  // The rail can be collapsed to give the active sub-surface full width. Two
  // independent inputs decide the EFFECTIVE collapsed state:
  //   • userCollapsed — the user's EXPLICIT preference, persisted to
  //     localStorage. Only the toggle button writes this.
  //   • windowNarrow  — a TRANSIENT, automatic collapse on narrow windows
  //     (< NARROW_PX). Never persisted; recomputed from a resize listener.
  // Effective = userCollapsed || windowNarrow. Keeping them separate means an
  // auto-collapse on a narrow window does NOT clobber the user's saved choice:
  // widen the window again and the rail returns to whatever the user last set.
  //
  // Semantics of the toggle (intuitive, documented per the plan):
  //   • Collapse  → userCollapsed = true (persisted). Rail hides immediately.
  //   • Expand    → userCollapsed = false (persisted). If the window is wide the
  //     rail returns at once. If the window is currently narrow, the rail still
  //     appears (the user explicitly asked) but may auto-collapse again on the
  //     next narrow resize — acceptable, and the natural reading of "show it now".
  const RAIL_COLLAPSED_KEY = "mnema.insights.rail-collapsed";
  const NARROW_PX = 760;

  function readPersistedCollapsed(): boolean {
    try {
      return localStorage.getItem(RAIL_COLLAPSED_KEY) === "1";
    } catch {
      // SSR / disabled storage — default to expanded.
      return false;
    }
  }

  let userCollapsed = $state(readPersistedCollapsed());
  let windowNarrow = $state(false);
  const railCollapsed = $derived(userCollapsed || windowNarrow);

  function toggleRailCollapsed(): void {
    // Expanding while narrow re-shows the rail by clearing the explicit
    // preference; collapsing sets it. Either way persist the explicit choice.
    userCollapsed = !railCollapsed;
    try {
      localStorage.setItem(RAIL_COLLAPSED_KEY, userCollapsed ? "1" : "0");
    } catch {
      // Best-effort persistence — a disabled store just won't survive reload.
    }
  }

  // ── Rail width (drag-resize, Slice 7) ───────────────────────────────────
  // Independent of collapse: the user can drag the rail/main boundary to any
  // width in [RAIL_MIN_WIDTH, RAIL_MAX_WIDTH], persisted to localStorage and
  // restored on mount. <RailResizer/> reports a desired px width; the shell is
  // the single owner that clamps + persists (so storage never holds an out-of-
  // range value). Only matters while expanded — when collapsed the rail (and the
  // resizer) aren't rendered, but the saved width is what returns on expand.
  const RAIL_WIDTH_KEY = "mnema.insights.rail-width";
  const RAIL_MIN_WIDTH = 180;
  const RAIL_MAX_WIDTH = 400;
  // First-run width sits in the conventional 240-280px expanded-sidebar band so
  // long conversation titles + the engine/model footer get room (still drag-
  // resizable within [min,max] and persisted).
  const RAIL_DEFAULT_WIDTH = 240;

  function clampRailWidth(px: number): number {
    return Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, Math.round(px)));
  }

  function readPersistedWidth(): number {
    try {
      const raw = localStorage.getItem(RAIL_WIDTH_KEY);
      if (raw === null) return RAIL_DEFAULT_WIDTH;
      const parsed = Number.parseInt(raw, 10);
      return Number.isNaN(parsed) ? RAIL_DEFAULT_WIDTH : clampRailWidth(parsed);
    } catch {
      // SSR / disabled storage — fall back to the default width.
      return RAIL_DEFAULT_WIDTH;
    }
  }

  let railWidth = $state(readPersistedWidth());

  function setRailWidth(px: number): void {
    railWidth = clampRailWidth(px);
    try {
      localStorage.setItem(RAIL_WIDTH_KEY, String(railWidth));
    } catch {
      // Best-effort persistence — a disabled store just won't survive reload.
    }
  }

  function resetRailWidth(): void {
    setRailWidth(RAIL_DEFAULT_WIDTH);
  }

  // Track the narrow-window condition with a matchMedia listener (cheaper than a
  // raw resize handler and fires only on the threshold crossing). Set up in an
  // effect so the listener is cleaned up on unmount.
  $effect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mql = window.matchMedia(`(max-width: ${NARROW_PX - 1}px)`);
    const apply = () => {
      windowNarrow = mql.matches;
    };
    apply();
    mql.addEventListener("change", apply);
    return () => mql.removeEventListener("change", apply);
  });

  // Drain any pending Quick Recall → Chat handoff queued before this surface
  // mounted (cold main window): the event may have fired while the window was
  // opening, so the latest queued conversation id lands the Chat tab on the
  // handed-off thread. Best-effort; a transport failure just leaves the default
  // Overview tab. The newest queued entry wins (handoffConversation is called in
  // order, so the last call sets the active id).
  async function drainPendingHandoff(): Promise<void> {
    try {
      const pending = await invoke<{ conversationId: string }[]>(
        "drain_pending_insights_open_conversations",
      );
      for (const entry of pending) {
        handoffConversation(entry.conversationId);
      }
    } catch {
      // Best-effort: no pending handoff, or the command is unavailable.
    }
  }

  $effect(() => {
    void untrack(() => loadEngineStatus());
    void untrack(() => drainPendingHandoff());
    // Kick the shared store's first history fetch so the rail populates even
    // when Chat isn't mounted (idempotent — Chat also calls it on its mount).
    void conversationStore.ensureStarted();

    let unlisten: UnlistenFn | undefined;
    let unlistenSettings: UnlistenFn | undefined;
    let unlistenHandoff: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // Settings saves (default model / engine on-off) emit this, not
    // `user_context_changed`; refresh the engine pill so it doesn't stay stale.
    void listen("recording_settings_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlistenSettings = fn;
    });

    // Warm-window handoff: a live event switches to Chat + selects the thread.
    void listen<{ conversationId: string }>(
      "insights_open_conversation",
      (event) => {
        handoffConversation(event.payload.conversationId);
      },
    ).then((fn) => {
      if (disposed) fn();
      else unlistenHandoff = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
      unlistenSettings?.();
      unlistenHandoff?.();
    };
  });
</script>

<div class="insights" class:insights--collapsed={railCollapsed}>
  <InsightsRail
    {view}
    onOpenTab={openTab}
    {engineOn}
    {modelLabel}
    {statusLoaded}
    onEnable={enableEngine}
    collapsed={railCollapsed}
    onToggleCollapse={toggleRailCollapsed}
    width={railWidth}
  />

  <!-- Drag handle between the rail and the active sub-surface. Only present when
       the rail is (so there is a boundary to drag). -->
  {#if !railCollapsed}
    <RailResizer
      width={railWidth}
      min={RAIL_MIN_WIDTH}
      max={RAIL_MAX_WIDTH}
      onWidth={setRailWidth}
      onReset={resetRailWidth}
    />
  {/if}

  <main class="insights-main" class:insights-main--chat={view === "chat"}>
    <!-- When the rail is collapsed, a quiet floating button (top-left, with a
         subtle backdrop so it reads above sub-surface content) brings it back. -->
    {#if railCollapsed}
      <button
        type="button"
        class="rail-expand-float"
        aria-label="Expand sidebar"
        aria-expanded="false"
        use:tip={"Expand sidebar"}
        onclick={toggleRailCollapsed}
      >
        <span aria-hidden="true">»</span>
      </button>
    {/if}
    {#if view === "overview"}
      <Overview onOpenSubject={openSubject} onOpenTab={openTab} />
    {:else if view === "journal"}
      <DayTimeline />
    {:else if view === "subjects"}
      {#if selectedSubject}
        <div class="breadcrumb">
          <button type="button" class="breadcrumb-back" onclick={backToSubjects}>‹ back</button>
          <button type="button" class="breadcrumb-link" onclick={backToSubjects}>Subjects</button>
          <span class="sep">/</span>
          <span class="current">{selectedSubject}</span>
        </div>
        <SubjectDetail subject={selectedSubject} onBack={backToSubjects} />
      {:else}
        <Subjects onOpenSubject={openSubject} />
      {/if}
    {:else if view === "context"}
      <Context />
    {:else}
      <Chat />
    {/if}
  </main>
</div>

<style>
  /* Insights workspace shell — mirrors `.insights` from the mockup (app.css),
     token-driven. A persistent left rail (<InsightsRail>) sits beside the
     `.insights-main` scroll column; the rail carries the sub-surface nav,
     new-chat, chat search/history, and the engine-status footer. */
  .insights {
    display: flex;
    flex-direction: row;
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
  }

  .insights-main {
    flex: 1 1 auto;
    min-width: 0;
    /* Position context for the floating expand button (collapsed state). */
    position: relative;
    overflow-y: auto;
    padding: 18px 20px 28px;
  }
  /* When the rail is collapsed, the padded sub-surfaces (overview / subjects /
     context) reserve a little extra top-left room so the floating expand button
     never sits on top of their content. Chat floats above its own header, so it
     keeps its edge-to-edge `--chat` padding (the button's backdrop separates
     it). */
  .insights--collapsed .insights-main:not(.insights-main--chat) {
    padding-top: 46px;
  }

  /* Floating expand affordance — only rendered when the rail is collapsed.
     Anchored top-left of the content area with a small inset + a subtle backdrop
     so it reads cleanly above whatever sub-surface is showing. Quiet by default,
     accent-on-hover, keyboard focusable with a visible focus ring. */
  .rail-expand-float {
    position: absolute;
    top: 12px;
    left: 12px;
    z-index: 5;
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .rail-expand-float:hover {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-surface-hover);
  }
  .rail-expand-float:focus-visible {
    outline: none;
    color: var(--app-accent);
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }
  /* Chat owns its own full-height, edge-to-edge layout and internal scrolling,
     so the shell main drops its padding and outer scroll (mirrors the mockup's
     `.insights-main` override). The other tabs keep the padded scroll above. */
  .insights-main--chat {
    padding: 0;
    overflow: hidden;
    /* Become a flex column so the chat surface fills via flex-grow rather than
       a percentage height. WKWebView (Tauri) does not reliably resolve a child's
       `height: 100%` against a flex-stretched parent, so `.chat` collapses to its
       content height; growing it as a flex item instead fills the surface. */
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* Drill-in breadcrumb (Subjects / <name>). Mirrors app.css `.breadcrumb`. */
  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--app-text-muted);
    margin-bottom: 14px;
  }
  .breadcrumb-link {
    font: inherit;
    font-size: 11.5px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .breadcrumb-link:hover {
    color: var(--app-text-strong);
  }
  .breadcrumb-link:focus-visible {
    outline: none;
    color: var(--app-text-strong);
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }
  .breadcrumb .sep {
    color: var(--app-text-faint);
  }
  .breadcrumb .current {
    color: var(--app-text-strong);
  }
  .breadcrumb-back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-right: 4px;
    padding: 2px 7px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .breadcrumb-back:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .breadcrumb-back:focus-visible {
    outline: none;
    color: var(--app-text-strong);
    box-shadow: var(--app-ring);
  }
</style>
