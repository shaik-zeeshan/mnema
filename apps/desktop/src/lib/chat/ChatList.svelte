<script lang="ts">
  // The Chat surface's list column (chat.html › Chat list): New chat, search,
  // date-grouped history, inline rename and delete. All list state lives in the
  // shared `conversationStore` (search_conversations / rename / delete); a row
  // click goes through its selection bus, which Chat watches.
  import { tip } from "$lib/components/tooltip";
  import { openSettings } from "$lib/surface-windows";
  import { conversationStore as store } from "$lib/insights/conversationStore.svelte";
  import type { ConversationSummary } from "$lib/insights/conversation";
  import { chatWhen } from "./chat-format";
  import IconPlus from "~icons/lucide/plus";
  import IconSearch from "~icons/lucide/search";
  import IconEdit from "~icons/lucide/pencil";
  import IconTrash from "~icons/lucide/trash-2";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconChat from "~icons/lucide/message-circle";
  import IconArrow from "~icons/lucide/arrow-right";

  interface Props {
    /** What "Default" resolves to (footer), or null before settings load. */
    defaultModel: string | null;
    /** A chat was deleted (the page shows the toast). */
    ondeleted: () => void;
  }
  let { defaultModel, ondeleted }: Props = $props();

  const query = $derived(store.searchQuery.trim());
  let searchEl = $state<HTMLInputElement | null>(null);

  function focusSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  function onRenameKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    event.stopPropagation();
    if (event.key === "Enter") {
      event.preventDefault();
      void store.commitRename();
    } else if (event.key === "Escape") {
      store.cancelRename();
    }
  }

  function onRowKeydown(event: KeyboardEvent, c: ConversationSummary): void {
    if (event.key === "Enter" && event.target === event.currentTarget) store.requestOpen(c.conversationId);
  }

  async function remove(c: ConversationSummary): Promise<void> {
    if (await store.deleteConversation(c)) ondeleted();
  }

  function clearSearch(): void {
    store.searchQuery = "";
    void store.refreshHistory();
    searchEl?.focus();
  }
</script>

<aside class="ch-side" aria-label="Chats">
  <div class="ch-side__head">
    <span class="mx-kicker">Chats</span>
    {#if store.historyLoaded && !store.historyError && !query}
      <span class="mx-label num">{store.conversations.length}</span>
    {/if}
  </div>
  <div class="ch-side__tools">
    <button type="button" class="mx-btn" onclick={() => store.requestNewChat()}>
      <IconPlus width="14" height="14" aria-hidden="true" />New chat<kbd>⌘J</kbd>
    </button>
    <label class="mx-search">
      <IconSearch width="13" height="13" aria-hidden="true" />
      <input
        bind:this={searchEl}
        bind:value={store.searchQuery}
        oninput={() => store.onSearchInput()}
        placeholder="Search chats"
        aria-label="Search chats"
        autocomplete="off"
        spellcheck="false"
      />
    </label>
  </div>

  <nav class="ch-list" aria-label="Conversation history" aria-busy={!store.historyLoaded}>
    {#if !store.historyLoaded}
      <span class="mx-sr">Loading chats</span>
      {#each Array(7) as _, i (i)}
        <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
      {/each}
    {:else if store.historyError}
      <div class="mx-empty mx-empty--compact" data-tone="danger" role="alert">
        <span class="mx-empty__glyph"><IconAlert width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">Couldn’t load your chats</b>
        <p class="mx-empty__text">They’re still saved on this Mac.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={() => void store.refreshHistory()}>
            <IconRetry width="13" height="13" aria-hidden="true" />Retry
          </button>
        </div>
      </div>
    {:else if store.conversations.length === 0 && query}
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconSearch width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">Nothing matches “{query}”</b>
        <p class="mx-empty__text">Searched titles, questions and answers.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={clearSearch}>Clear search</button>
        </div>
      </div>
    {:else if store.conversations.length === 0}
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconChat width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">No chats yet</b>
        <p class="mx-empty__text">Questions you ask here or in Quick Recall are kept here.</p>
      </div>
    {:else}
      {#each store.historyGroups as group (group.label)}
        <h6>{group.label}</h6>
        {#each group.items as c (c.conversationId)}
          {@const title = c.title || c.preview || "Untitled chat"}
          <div
            class="ch-item"
            role="link"
            tabindex="0"
            aria-current={c.conversationId === store.activeConversationId}
            onclick={() => store.requestOpen(c.conversationId)}
            onkeydown={(e) => onRowKeydown(e, c)}
          >
            {#if store.renamingId === c.conversationId}
              <input
                aria-label="Rename chat"
                spellcheck="false"
                autocomplete="off"
                bind:value={store.renameDraft}
                use:focusSelect
                onkeydown={onRenameKeydown}
                onblur={() => void store.commitRename()}
                onclick={(e) => e.stopPropagation()}
              />
            {:else}
              <span class="ch-item__title" use:tip={title}>{title}</span>
              <span class="ch-item__acts">
                <button
                  type="button"
                  class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
                  aria-label="Rename"
                  use:tip={"Rename"}
                  onclick={(e) => {
                    e.stopPropagation();
                    store.startRename(c);
                  }}><IconEdit width="14" height="14" aria-hidden="true" /></button
                >
                <button
                  type="button"
                  class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
                  aria-label="Delete"
                  use:tip={"Delete"}
                  onclick={(e) => {
                    e.stopPropagation();
                    void remove(c);
                  }}><IconTrash width="14" height="14" aria-hidden="true" /></button
                >
              </span>
            {/if}
            <span class="ch-item__meta">{chatWhen(c.updatedAtMs)} · {c.turnCount} turn{c.turnCount === 1 ? "" : "s"}</span>
          </div>
        {/each}
      {/each}
    {/if}
  </nav>

  {#if defaultModel}
    <button type="button" class="ch-side__foot" use:tip={"Change the default in Settings → AI"} onclick={() => void openSettings("intelligence")}>
      Default <b>{defaultModel}</b><IconArrow width="12" height="12" aria-hidden="true" />
    </button>
  {/if}
</aside>

<style>
  .ch-side {
    min-height: 0;
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    background: color-mix(in srgb, var(--app-surface) 82%, transparent);
    border-right: 1px solid var(--app-border);
  }
  .ch-side__head { display: flex; align-items: center; gap: var(--s-2); padding: var(--s-4) var(--s-3) var(--s-2) var(--s-4); }
  .ch-side__head .mx-label { margin-left: auto; font-size: var(--text-xs); }
  .ch-side__tools { display: grid; gap: var(--s-2); padding: 0 var(--s-3) var(--s-3); border-bottom: 1px solid var(--mx-hairline); }
  .ch-side__tools .mx-btn { justify-content: flex-start; }
  .ch-side__tools .mx-btn kbd { margin-left: auto; }
  .ch-list { overflow: auto; padding: var(--s-1) var(--s-2) var(--s-4); }
  .ch-list h6 { margin: var(--s-4) 10px 4px; font: 500 var(--text-sm)/1 var(--font-sans); color: var(--app-text-subtle); }
  .ch-list :global(.mx-empty--compact) { padding: var(--s-4) 8px; }
  .ch-list :global(.mx-empty--compact .mx-empty__acts) { grid-column: 2; grid-row: auto; margin-top: 6px; }
  .ch-list .mx-skel-lines { padding: 10px; }
  .ch-list .mx-skel-lines + .mx-skel-lines { border-top: 1px solid var(--mx-hairline); }
  .ch-item {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 2px var(--s-2);
    padding: 7px 8px 7px 10px;
    border-radius: var(--r-md);
    cursor: pointer;
    color: var(--app-text);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .ch-item:hover { background: var(--mx-wash); }
  .ch-item:focus-visible { outline: none; box-shadow: var(--app-ring); }
  .ch-item[aria-current="true"] { background: var(--mx-selected); color: var(--app-text-strong); }
  .ch-item__title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: 500 var(--text-md)/1.35 var(--font-sans); }
  .ch-item__meta { font: 400 var(--text-xs)/1.3 var(--font-mono); font-variant-numeric: tabular-nums; color: var(--app-text-subtle); }
  .ch-item__acts {
    position: absolute;
    right: 4px;
    top: 50%;
    translate: 0 -50%;
    display: flex;
    opacity: 0;
    pointer-events: none;
    padding-left: 18px;
    border-radius: var(--r-sm);
    background: linear-gradient(to right, transparent, color-mix(in srgb, var(--app-fg) 4%, var(--app-surface)) 16px);
    transition: opacity var(--t-fast) var(--ease-quart);
  }
  .ch-item[aria-current="true"] .ch-item__acts {
    background: linear-gradient(to right, transparent, color-mix(in srgb, var(--app-fg) 8%, var(--app-surface)) 16px);
  }
  .ch-item:hover .ch-item__acts,
  .ch-item:focus-within .ch-item__acts { opacity: 1; pointer-events: auto; }
  .ch-item input {
    width: 100%;
    height: 22px;
    padding: 0 6px;
    margin: -2px 0 -1px -6px;
    border: 1px solid var(--app-accent-border);
    border-radius: var(--r-xs);
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    font: 500 var(--text-md)/1 var(--font-sans);
    outline: 0;
    box-shadow: var(--app-ring);
  }
  .ch-side__foot {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-2) var(--s-4);
    border: 0;
    border-top: 1px solid var(--mx-hairline);
    background: none;
    cursor: pointer;
    font: 500 var(--text-sm)/1 var(--font-sans);
    color: var(--app-text-subtle);
    text-align: left;
  }
  .ch-side__foot b { font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch-side__foot :global(svg) { margin-left: auto; flex: none; color: var(--app-text-faint); transition: transform var(--t-med) var(--ease-expo); }
  .ch-side__foot:hover b { color: var(--app-text-strong); }
  .ch-side__foot:hover :global(svg) { transform: translateX(2px); color: var(--app-text-strong); }
</style>
