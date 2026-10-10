<script lang="ts">
  // `/chat` — Chat as its own surface in the Main window (chat.html): the chat
  // list beside the conversation. `?c=<id>` opens a chat (Quick Recall's "Open
  // in Chat" lands here via the layout), `?q=<text>` starts one and sends it
  // once (the titlebar's ask field). Params are consumed, then stripped, so a
  // reload or Back never re-sends.
  import { onMount, untrack } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import Chat from "$lib/chat/Chat.svelte";
  import ChatList from "$lib/chat/ChatList.svelte";
  import IconOk from "~icons/lucide/check";

  let defaultModel = $state<string | null>(null);

  $effect(() => {
    const params = $page.url.searchParams;
    const c = params.get("c")?.trim();
    const q = params.get("q")?.trim();
    if (!c && !q) return;
    untrack(() => {
      if (c) conversationStore.requestOpen(c);
      else if (q) conversationStore.requestNewChat(q, true);
      void goto("/chat", { replaceState: true, keepFocus: true, noScroll: true });
    });
  });

  onMount(() => {
    void conversationStore.ensureStarted();
    // A cold Main window: an "Open in Chat" queued before this page mounted.
    void invoke<{ conversationId: string }[]>("drain_pending_insights_open_conversations")
      .then((pending) => {
        const last = pending.at(-1);
        if (last) conversationStore.requestOpen(last.conversationId);
      })
      .catch(() => {});
  });

  // ⌘J here = a new chat in place (elsewhere the shell's ⌘J focuses Ask).
  function onKeydown(event: KeyboardEvent): void {
    if (event.key.toLowerCase() !== "j" || !(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
    event.preventDefault();
    event.stopPropagation();
    conversationStore.requestNewChat();
  }

  let toast = $state(false);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  function showDeleted(): void {
    toast = true;
    if (toastTimer !== null) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = false), 4000);
  }
  $effect(() => () => {
    if (toastTimer !== null) clearTimeout(toastTimer);
  });
</script>

<svelte:window onkeydowncapture={onKeydown} />

<main class="ch-body">
  <ChatList {defaultModel} ondeleted={showDeleted} />
  <Chat bind:defaultModel ondeleted={showDeleted} />
  {#if toast}
    <div class="mx-toasts">
      <div class="mx-toast" data-tone="ok" role="status"><IconOk width="15" height="15" aria-hidden="true" /><span>Chat deleted</span></div>
    </div>
  {/if}
</main>

<style>
  .ch-body {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: 276px minmax(0, 1fr);
    overflow: hidden;
    background: var(--app-bg);
  }
  .ch-body .mx-toasts { bottom: var(--s-6); }
  @media (max-width: 1180px) {
    .ch-body { grid-template-columns: 240px minmax(0, 1fr); }
  }
</style>
