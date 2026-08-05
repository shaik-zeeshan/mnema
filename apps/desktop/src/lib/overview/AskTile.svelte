<script lang="ts">
  // Ask history (round-4 decision **G11**) plus the door into Quick Access.
  //
  // The rows are a read of the shared conversation store (`list_conversations`)
  // — the same store both Ask doors persist to. Selecting a row fills the
  // inspector; the field at the bottom is not a second composer, it summons
  // Quick Access, which is where asking actually happens.
  import IconSpark from "~icons/lucide/sparkles";
  import { invoke } from "@tauri-apps/api/core";
  import type { ConversationSummary } from "$lib/insights/conversation";
  import type { LoadState, Selection } from "./overview-data.svelte";
  import { formatClock } from "./overview-format";
  import TileShell from "./TileShell.svelte";

  interface Props {
    asks: LoadState<ConversationSummary[]>;
    selectedKey: string | null;
    /** The 800px floor: the composer only — the row it would show is one line
     *  taller than the 40px the mockup gives this tile there. */
    compact?: boolean;
    onselect: (selection: Selection) => void;
  }

  let { asks, selectedKey, compact = false, onselect }: Props = $props();

  const all = $derived(asks.status === "ok" ? asks.value : []);
  // One row, the way the mockup draws it: the composer below it is the point of
  // the tile, and two rows plus a 28px field is taller than the bento row.
  const rows = $derived(compact ? [] : all.slice(0, 1));

  const quiet = $derived(
    asks.status === "failed"
      ? "Couldn't read your ask history."
      : asks.status === "loading"
        ? null
        : all.length === 0
          ? null // the composer below is the empty state
          : null,
  );

  function keyOf(c: ConversationSummary): string {
    return `ask:${c.conversationId}`;
  }

  function toSelection(c: ConversationSummary): Selection {
    return {
      key: keyOf(c),
      source: "Ask",
      title: c.title || c.preview,
      lede: c.title ? c.preview : undefined,
      sections: [
        {
          label: "Conversation",
          rows: [
            { k: "Turns", v: String(c.turnCount), mono: true },
            { k: "Asked", v: new Date(c.createdAtMs).toLocaleString(), mono: true },
            { k: "Updated", v: new Date(c.updatedAtMs).toLocaleString(), mono: true },
            { k: "Door", v: c.origin === "quick_recall" ? "Quick Access" : "Chat" },
          ],
        },
      ],
    };
  }

  async function openConversation(c: ConversationSummary): Promise<void> {
    try {
      await invoke("open_conversation_in_chat", { conversationId: c.conversationId });
    } catch {
      // Best-effort: the row stays selected and the inspector still holds it.
    }
  }

  async function summon(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // The global shortcut stays the canonical summon path.
    }
  }
</script>

{#if compact}
  <!-- The 800px floor: the composer alone, no header — the mockup's own frame
       makes this call, and a 46px row cannot hold a header plus a 28px field. -->
  <section class="ss-tile ss-tile--2 bare">
    <button type="button" class="input composer" onclick={() => void summon()}>
      <span class="composer__ic" aria-hidden="true"><IconSpark /></span>
      <span class="ph">Ask about your day…</span>
      <kbd class="kbd">⌥Space</kbd>
    </button>
  </section>
{:else}
<TileShell
  label="Ask"
  more={compact ? undefined : "opens Quick Access"}
  span="ss-tile--2"
  {quiet}
  selected={rows.some((c) => keyOf(c) === selectedKey)}
>
  {#each rows as c (c.conversationId)}
    <button
      type="button"
      class="ss-row row"
      class:ss-row--sel={selectedKey === keyOf(c)}
      ondblclick={() => void openConversation(c)}
      onclick={() => onselect(toSelection(c))}
    >
      <span class="ss-row__txt">
        <span class="ss-row__lbl">{c.title || c.preview}</span>
        <span class="ss-row__sub">
          {c.turnCount}
          {c.turnCount === 1 ? "turn" : "turns"} · {c.origin === "quick_recall" ? "Quick Access" : "Chat"}
        </span>
      </span>
      <span class="ss-row__val"><span class="t-meta is-mono is-num">{formatClock(c.updatedAtMs)}</span></span>
    </button>
  {/each}

  <button type="button" class="input composer" onclick={() => void summon()}>
    <span class="composer__ic" aria-hidden="true"><IconSpark /></span>
    <span class="ph">Ask about your day…</span>
    <kbd class="kbd">⌥Space</kbd>
  </button>
</TileShell>
{/if}

<style>
  .row {
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: default;
    min-height: 0;
    padding: 0 var(--s-10);
  }

  .bare {
    justify-content: center;
  }

  /* The app's `.input` primitive styles a real <input>: it carries the box but
     not a row layout. This is a button standing in for a field, so it lays out
     its own icon / placeholder / key hint. */
  .composer {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    width: 100%;
    margin-top: auto;
    flex: 0 0 auto;
    text-align: left;
    cursor: default;
  }

  .composer__ic {
    display: flex;
    flex: 0 0 auto;
    font-size: 13px;
    color: var(--app-text-subtle);
  }

  .composer .ph {
    color: var(--app-text-subtle);
  }

  .composer .kbd {
    margin-left: auto;
  }
</style>
