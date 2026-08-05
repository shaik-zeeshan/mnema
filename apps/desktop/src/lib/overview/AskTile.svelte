<script lang="ts">
  // Ask (2×1) — a LAUNCHER, never an answer surface. The field opens Quick
  // Access (the same command the titlebar's Search door and the global shortcut
  // take); a history row hands its conversation to Chat through
  // `open_conversation_in_chat`, which the shell layout turns into a route.
  //
  // Ask history is IN per round-4 decision G11, and it is a plain read of the
  // persistent conversation store (`list_conversations`) — no new storage.
  import { invoke } from "@tauri-apps/api/core";
  import type { ConversationSummary } from "$lib/insights/conversation";
  import { getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { formatShortcut } from "$lib/keyboard";
  import { historyStamp } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let { asks, loaded }: { asks: ConversationSummary[]; loaded: boolean } = $props();

  const rows = $derived(asks.slice(0, 2));
  const now = new Date();

  const shortcut = (() => {
    const binding = getEffectiveGlobalShortcut("toggleQuickRecall").bindings[0];
    return binding ? formatShortcut(binding, "macos").join("") : null;
  })();

  async function ask(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // The global shortcut stays the canonical summon path.
    }
  }

  async function open(row: ConversationSummary): Promise<void> {
    try {
      await invoke("open_conversation_in_chat", { conversationId: row.conversationId });
    } catch {
      // Best-effort hand-off.
    }
  }
</script>

<div class="tile tile--w2">
  <div class="tile__h">
    <span class="t-label">Ask</span>
    {#if rows.length}<span class="tile__more">recent</span>{/if}
  </div>

  <div class="pay pay--rows">
    {#each rows as row (row.conversationId)}
      <button type="button" class="row hist" onclick={() => void open(row)}>
        <span class="row__txt">
          <span class="row__lbl">{row.title || row.preview}</span>
          <span class="row__sub">
            {row.turnCount}
            {row.turnCount === 1 ? "turn" : "turns"}
          </span>
        </span>
        <span class="row__val">
          <span class="t-meta is-mono is-num">{historyStamp(row.updatedAtMs, now)}</span>
          <span class="chev"><Glyph name="chevr" /></span>
        </span>
      </button>
    {/each}

    {#if !rows.length && loaded}
      <div class="row row--static none">
        <span class="t-meta subtle">Nothing asked yet</span>
      </div>
    {/if}

    <div class="row row--static field">
      <button type="button" class="input launcher" onclick={() => void ask()}>
        <span class="launcher__g"><Glyph name="spark-o" /></span>
        <span class="ph">Ask about your day…</span>
      </button>
      {#if shortcut}<span class="kbd kbd--mod">{shortcut}</span>{/if}
    </div>
  </div>
</div>

<style>
  button.hist {
    width: 100%;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  button.hist:focus-visible,
  .launcher:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }
  .none {
    padding-bottom: 0;
    min-height: 28px;
  }
  .field {
    padding-top: var(--s-4);
  }
  .launcher {
    flex: 1;
    display: inline-flex;
    align-items: center;
    gap: var(--gap-inline);
    cursor: pointer;
    text-align: left;
  }
  .launcher__g {
    flex: 0 0 auto;
    width: 13px;
    height: 13px;
    color: var(--app-accent);
  }
  .ph {
    color: var(--app-text-subtle);
  }
  .subtle {
    color: var(--app-text-subtle);
  }
</style>
