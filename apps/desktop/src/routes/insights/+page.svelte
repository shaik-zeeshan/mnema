<script lang="ts">
  // Chat — a destination, not a surface (direction 01). The old Insights
  // workspace (rail + Overview/Journal/Subjects/Context/Chat sub-surfaces) is
  // gone: Overview is its own route (`/overview`, the bento), and Journal /
  // Subjects / Context are destinations opened from Overview tiles. What this
  // route still owns is Chat — the landing for the Quick Recall → Chat handoff
  // (issue #111, ADR 0031) and Overview's Ask-history rows. The titlebar's
  // `‹ Overview` chevron (owned by +layout.svelte) is the way back; the
  // Insights rail is never drawn.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettings } from "$lib/surface-windows";
  import type { AiRuntimeStatus } from "$lib/types/recording";
  import Chat from "$lib/insights/Chat.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";

  // Quick Recall → Chat handoff. When a Quick Recall thread is promoted into
  // Chat, the main window is shown/navigated here and a conversation id is
  // delivered (a live `insights_open_conversation` event for a warm window, or
  // the cold-window drain on mount). The handoff routes through the shared
  // store's selection bus (`requestOpen`), which Chat watches.
  function handoffConversation(conversationId: string): void {
    conversationStore.requestOpen(conversationId);
  }

  // ── Engine status ────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  // Distinguishes "still loading the status calls" from "loaded → engine off"
  // so the gate never flashes while the status call is in flight.
  let statusLoaded = $state(false);

  // Chat is built entirely from Reasoning Engine output, so with the engine
  // never SET UP (enabled && configured) the surface is uniformly empty — show
  // a pitch instead. A configured engine that is momentarily unreachable keeps
  // the surface and its per-turn error states — transient liveness must not
  // lock the user out of existing conversations.
  const engineGated = $derived(
    statusLoaded && !(aiStatus?.enabled && aiStatus?.configured),
  );

  async function loadEngineStatus(): Promise<void> {
    try {
      aiStatus = await invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null);
    } finally {
      statusLoaded = true;
    }
  }

  function enableEngine(): void {
    void openSettings("intelligence");
  }

  // Drain any pending Quick Recall → Chat handoff queued before this surface
  // mounted (cold main window). Best-effort; the newest queued entry wins.
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
    // Kick the shared store's first history fetch (idempotent — Chat also
    // calls it on its mount).
    void conversationStore.ensureStarted();

    let unlistenSettings: UnlistenFn | undefined;
    let unlistenHandoff: UnlistenFn | undefined;
    let disposed = false;

    // Settings saves (default model / engine on-off) emit this; refresh the
    // gate so finishing setup in Settings unlocks Chat live, no reload needed.
    void listen("recording_settings_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlistenSettings = fn;
    });

    // Warm-window handoff: a live event selects the thread.
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
      unlistenSettings?.();
      unlistenHandoff?.();
    };
  });
</script>

{#if engineGated}
  <!-- Engine never set up — Chat is engine-derived, so pitch it. -->
  <div class="gate">
    <div class="gate-panel">
      <p class="gate-eyebrow">
        <span class="diamond" aria-hidden="true">◆</span>
        Chat
      </p>
      <h1 class="gate-title">Turn on the Reasoning Engine to chat over your history.</h1>
      <p class="gate-detail">
        Chat answers questions over your own captured history — everything here
        is derived from the engine.
      </p>
      <button type="button" class="gate-cta" onclick={enableEngine}>
        Open engine settings
      </button>
      <p class="gate-note">
        Runs on your own provider — local (Ollama, Llamafile) or your cloud API key.
      </p>
    </div>
  </div>
{:else}
  <main class="chat-dest">
    <Chat />
  </main>
{/if}

<style>
  /* ── Engine gate — full-surface pitch shown until the engine is set up ── */
  .gate {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    overflow-y: auto;
    padding: 28px 20px;
  }
  .gate-panel {
    /* Auto margins center when there's room but keep the top reachable when the
       panel is taller than the viewport (flex centering would clip it). */
    margin: auto;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 26px 28px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 11px;
  }
  .gate-eyebrow {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--t-label);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .gate-eyebrow .diamond {
    color: var(--app-accent);
    letter-spacing: 0;
  }
  .gate-title {
    margin: 0;
    font-size: var(--t-title);
    line-height: 1.35;
    color: var(--app-text-strong);
  }
  .gate-detail {
    margin: 0;
    font-size: var(--t-ui);
    line-height: 1.6;
    color: var(--app-text-muted);
  }
  .gate-cta {
    align-self: flex-start;
    margin-top: 8px;
    font: inherit;
    font-size: var(--t-ui);
    padding: 7px 15px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .gate-cta:hover {
    border-color: var(--app-accent);
  }
  .gate-cta:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .gate-cta:active {
    transform: translateY(1px);
  }
  .gate-note {
    margin: 0;
    font-size: var(--t-meta);
    color: var(--app-text-faint);
  }

  /* Chat owns its own full-height, edge-to-edge layout and internal scrolling.
     Grow it as a flex column: WKWebView does not reliably resolve a child's
     `height: 100%` against a flex-stretched parent. */
  .chat-dest {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    padding: 0;
  }
</style>
