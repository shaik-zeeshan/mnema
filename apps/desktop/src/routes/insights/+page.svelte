<script lang="ts">
  // Insights workspace — the story-first surfaces of the Main window (Warm
  // Paper redesign, Slice 2). It renders inside the shared <InsightsShell>
  // (rail + main column) and hosts Today / Meetings / Subjects / Chat switched
  // via local state (NOT separate routes), plus a Subject drill-in. Triggers is
  // the rail's fourth nav item but keeps its own route (`/triggers`), which
  // renders the same shell around the triggers surface.
  //
  // Today is the front page (Overview retired; the real Today page lands in
  // Slice 3 — for now it shows the existing journal river). Context retired as
  // a tab: its authored-context surface folds into Subjects below the inferred
  // dossier.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import DayTimeline from "$lib/insights/DayTimeline.svelte";
  import Subjects from "$lib/insights/Subjects.svelte";
  import SubjectDetail from "$lib/insights/SubjectDetail.svelte";
  import Context from "$lib/insights/Context.svelte";
  import Chat from "$lib/insights/Chat.svelte";
  import MeetingsSurface from "$lib/meetings/MeetingsSurface.svelte";
  import InsightsShell from "$lib/insights/InsightsShell.svelte";
  import { type RailTab } from "$lib/insights/InsightsRail.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";

  // Active surface. `?tab=` seeds the initial view (the /triggers route
  // navigates here with an explicit destination); default is Today.
  // Subject-detail is a drill-in over the Subjects tab held in
  // `selectedSubject` (null = the index).
  function initialView(): Exclude<RailTab, "triggers"> {
    const tab = $page.url.searchParams.get("tab");
    return tab === "meetings" || tab === "subjects" || tab === "chat"
      ? tab
      : "today";
  }
  let view = $state<Exclude<RailTab, "triggers">>(initialView());
  let selectedSubject = $state<string | null>(null);

  // Quick Recall → Chat handoff (issue #111, ADR 0031). When a Quick Recall
  // thread is promoted into Chat, the main window is shown/navigated here and a
  // conversation id is delivered (a live `insights_open_conversation` event for
  // a warm window, or the cold-window drain on mount). The handoff routes
  // through the shared store's selection BUS (`requestOpen`), which Chat
  // watches; the effect below switches this shell to the Chat surface when the
  // bus fires (so a request that arrives while on another tab still lands on
  // Chat).
  function handoffConversation(conversationId: string): void {
    conversationStore.requestOpen(conversationId);
    view = "chat";
    selectedSubject = null;
  }

  // A bus request switches the shell to the Chat surface. The nonce is
  // snapshotted at mount so a STALE pre-mount request (e.g. a chat opened
  // earlier this session, then the user navigated Triggers → Today) does not
  // yank the view to Chat — cross-route chat handoffs arrive as `?tab=chat`
  // instead, and Chat's own bus watcher picks up the pending request.
  let lastHandoffNonce = conversationStore.pendingOpen.nonce;
  $effect(() => {
    const pending = conversationStore.pendingOpen;
    untrack(() => {
      if (pending.nonce === lastHandoffNonce) return;
      lastHandoffNonce = pending.nonce;
      view = "chat";
      selectedSubject = null;
    });
  });

  function openTab(tab: RailTab): void {
    if (tab === "triggers") {
      void goto("/triggers");
      return;
    }
    view = tab;
    if (tab !== "subjects") selectedSubject = null;
    // Leaving Chat unmounts it, so its mirror effect can no longer clear the
    // store's open-thread id — clear it here so the rail stops highlighting the
    // previously-open row while a non-Chat surface is showing.
    if (tab !== "chat") conversationStore.activeConversationId = null;
  }

  function openSubject(subject: string): void {
    view = "subjects";
    selectedSubject = subject;
  }

  function backToSubjects(): void {
    selectedSubject = null;
  }

  // Drain any pending Quick Recall → Chat handoff queued before this surface
  // mounted (cold main window). Best-effort; a transport failure just leaves
  // the default Today tab. The newest queued entry wins.
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
    void untrack(() => drainPendingHandoff());

    let unlistenHandoff: UnlistenFn | undefined;
    let disposed = false;
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
      unlistenHandoff?.();
    };
  });
</script>

<InsightsShell {view} onOpenTab={openTab} gate bare={view === "chat"}>
  {#if view === "today"}
    <!-- Today front page (Slice 3): greeting → digest → composer + chips →
         ledger-prose river. Composer submits ride the conversation bus, which
         this page's bus effect already routes to the Chat surface. -->
    <DayTimeline />
  {:else if view === "meetings"}
    <!-- Recap summaries live in trigger-run conversations; opening one routes
         through the same chat handoff Quick Recall uses. -->
    <MeetingsSurface onOpenConversation={handoffConversation} />
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
      <!-- Context folds into Subjects (both are "what Mnema believes"): the
           user-authored context surface renders below the inferred dossier. -->
      <div class="authored-fold">
        <Context />
      </div>
    {/if}
  {:else}
    <Chat />
  {/if}
</InsightsShell>

<style>
  /* Authored context folded under the Subjects dossier. */
  .authored-fold {
    margin-top: 28px;
    padding-top: 22px;
    border-top: 1px solid var(--app-border);
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
