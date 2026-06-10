<script lang="ts">
  // Insights workspace — the second top-level Surface of Main (alongside the
  // Timeline). It hosts four sub-surfaces (Overview / Subjects / Context / Chat)
  // switched via local state (NOT separate routes), plus a Subject drill-in.
  // The surface toggle that brings the user here lives in the shared titlebar
  // (`+layout.svelte`); this page owns only the sub-nav + sub-surface content.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettingsWindow } from "$lib/surface-windows";
  import type {
    AiRuntimeStatus,
    UserContextStatus,
    RecordingSettings,
  } from "$lib/types/recording";
  import Overview from "$lib/insights/Overview.svelte";
  import Subjects from "$lib/insights/Subjects.svelte";
  import SubjectDetail from "$lib/insights/SubjectDetail.svelte";
  import Context from "$lib/insights/Context.svelte";
  import Chat from "$lib/insights/Chat.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  type InsightsTab = "overview" | "subjects" | "context" | "chat";

  const TABS: { id: InsightsTab; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "subjects", label: "Subjects" },
    { id: "context", label: "Context" },
    { id: "chat", label: "Chat" },
  ];

  // Active sub-surface. Default is Overview. Subject-detail is a drill-in over
  // the Subjects tab held in `selectedSubject` (null = the index).
  let view = $state<InsightsTab>("overview");
  let selectedSubject = $state<string | null>(null);

  // Quick Recall → Chat handoff (issue #111, ADR 0031). When a Quick Recall
  // thread is promoted into Chat, the main window is shown/navigated here and a
  // conversation id is delivered (a live `insights_open_conversation` event for
  // a warm window, or the cold-window drain on mount). We switch to the Chat tab
  // and pass the id down so Chat selects + loads that persisted thread. A
  // monotonically bumped `nonce` lets Chat re-react when the SAME conversation
  // is handed off twice in a row (the prop value alone wouldn't change).
  let openConversationId = $state<string | null>(null);
  let openConversationNonce = $state(0);

  function handoffConversation(conversationId: string): void {
    const id = conversationId.trim();
    if (id.length === 0) return;
    openConversationId = id;
    openConversationNonce += 1;
    view = "chat";
    selectedSubject = null;
  }

  function openTab(tab: InsightsTab): void {
    view = tab;
    if (tab !== "subjects") selectedSubject = null;
  }

  function openSubject(subject: string): void {
    view = "subjects";
    selectedSubject = subject;
  }

  function backToSubjects(): void {
    selectedSubject = null;
  }

  // ── Engine status pill ───────────────────────────────────────────────
  // Mirrors the mockup's `.engine-status` chip: "Engine: <model>" when the
  // Reasoning Engine is on/available, or "Engine off · Enable" otherwise. The
  // Enable link opens the Reasoning Engine settings (Access tab).
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
        const rt = settings.aiRuntime;
        modelLabel = shortModel(
          rt.engineKind === "local" ? rt.localModel : rt.cloudModel,
        );
      }
    } catch {
      // Best-effort: leave the pill in its "engine off" default on error.
    } finally {
      statusLoaded = true;
    }
  }

  function enableEngine(): void {
    void openSettingsWindow("intelligence");
  }

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

    let unlisten: UnlistenFn | undefined;
    let unlistenHandoff: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
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
      unlistenHandoff?.();
    };
  });
</script>

<div class="insights">
  <nav class="subnav">
    <div class="subnav-tabs" role="tablist" aria-label="Insights sub-surface">
      {#each TABS as tab (tab.id)}
        <button
          type="button"
          role="tab"
          class="subnav-tab"
          class:active={view === tab.id}
          aria-selected={view === tab.id}
          aria-current={view === tab.id ? "page" : undefined}
          onclick={() => openTab(tab.id)}
        >
          {tab.label}
        </button>
      {/each}
    </div>
    <div class="subnav-meta">
      <button
        type="button"
        class="subnav-search"
        title="Jump to a subject or ask"
        onclick={() => openTab("chat")}
      >
        <span class="subnav-search-glyph" aria-hidden="true">⌕</span>
        <span class="subnav-search-text">Jump to a subject or ask…</span>
      </button>
      {#if !statusLoaded}
        <span class="engine-status-skeleton" aria-label="Loading engine status">
          <Skeleton width="116px" height="22px" radius="999px" />
        </span>
      {:else if engineOn}
        <span class="engine-status" title="Reasoning Engine is on">
          <span class="dot" aria-hidden="true"></span>
          Engine: {modelLabel || "on"}
        </span>
      {:else}
        <span class="engine-status engine-status--off" title="Reasoning Engine is off">
          <span class="dot" aria-hidden="true"></span>
          Engine off ·
          <button type="button" class="engine-status-enable" onclick={enableEngine}>
            Enable
          </button>
        </span>
      {/if}
    </div>
  </nav>

  <main class="insights-main" class:insights-main--chat={view === "chat"}>
    {#if view === "overview"}
      <Overview onOpenSubject={openSubject} onOpenTab={openTab} />
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
      <Chat {openConversationId} {openConversationNonce} />
    {/if}
  </main>
</div>

<style>
  /* Insights workspace shell — mirrors `.insights` + `.subnav` from the mockup
     (app.css), token-driven. The sub-nav switcher shares the canonical
     segmented-control look with the titlebar surface toggle. */
  .insights {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
  }

  .subnav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex: 0 0 40px;
    height: 40px;
    width: 100%;
    padding: 0 16px;
    border-bottom: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }

  /* Segmented control — shared contract with `.surface-toggle`. */
  .subnav-tabs {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .subnav-tab {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 13px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .subnav-tab:hover {
    color: var(--app-text-strong);
  }
  .subnav-tab.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  .subnav-meta {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
  }

  .subnav-search {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 250px;
    min-width: 0;
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font: inherit;
    font-size: 11.5px;
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .subnav-search-glyph {
    color: var(--app-text-subtle);
    transition: color 0.12s ease;
  }
  .subnav-search-text {
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.12s ease;
  }
  .subnav-search:hover {
    border-color: var(--app-border-hover);
  }
  .subnav-search:hover .subnav-search-glyph,
  .subnav-search:hover .subnav-search-text {
    color: var(--app-text-strong);
  }

  .engine-status-skeleton {
    display: inline-flex;
    align-items: center;
  }

  .engine-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    padding: 3px 9px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    white-space: nowrap;
  }
  .engine-status .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .engine-status--off {
    border-color: var(--app-border);
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
  }
  .engine-status--off .dot {
    background: var(--app-status-dot);
    box-shadow: none;
  }
  .engine-status-enable {
    font: inherit;
    font-size: 11px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--app-accent-strong);
    cursor: pointer;
    border-bottom: 1px dotted var(--app-accent-border);
  }
  .engine-status-enable:hover {
    color: var(--app-accent);
  }

  .insights-main {
    flex: 1 1 auto;
    min-width: 0;
    overflow-y: auto;
    padding: 18px 20px 28px;
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
    transition: background 0.12s ease, color 0.12s ease;
  }
  .breadcrumb-back:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
</style>
