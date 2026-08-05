<script lang="ts">
  // The most recent question, and the field that starts the next one.
  //
  // From `list_conversations` (G11: Ask history is a conversation-store read).
  // The mockup's "cites 3 moments" is dropped — a ConversationSummary carries
  // no citation count, and cold-loading every turn to count them would be a
  // model-free but pointless read (G8).
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import { formatClock } from "./format";
  import type { Cell } from "./data";
  import type { ConversationSummary } from "$lib/insights/conversation";

  interface Props {
    asks: Cell<ConversationSummary[]>;
    loaded: boolean;
    /** Opens Quick Access (⌘⏎) — the field and the keycap both fire this. */
    open: () => void;
  }

  let { asks, loaded, open }: Props = $props();

  const latest = $derived((asks.data ?? [])[0] ?? null);
  const more = $derived(
    asks.data && asks.data.length > 0
      ? `last ${asks.data.length} ${asks.data.length === 1 ? "question" : "questions"}`
      : null,
  );

  function turns(n: number): string {
    return n === 1 ? "1 turn" : `${n} turns`;
  }
</script>

<Tile id="ask" title="Ask" kbd="⌘⏎" {more} span={2} {open} openLabel="Open Quick Access">
  {#if asks.error}
    <p class="tile-empty t-meta">Ask history unavailable — {asks.error}</p>
  {:else if latest}
    <button class="grow ask__row" type="button" onclick={open}>
      <span class="grow__txt">
        <span class="grow__lbl">{latest.title || latest.preview}</span>
        <span class="grow__sub">{turns(latest.turnCount)}</span>
      </span>
      <span class="grow__val">
        <span class="t-meta is-mono is-num">{formatClock(latest.updatedAtMs)}</span>
        <Chev />
      </span>
    </button>
  {:else if loaded}
    <p class="tile-empty t-meta">You haven't asked Mnema anything yet.</p>
  {/if}

  <button class="asklaunch" type="button" onclick={open}>
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
      <circle cx="7.2" cy="7.2" r="4.6" /><path d="m10.6 10.6 3 3" />
    </svg>
    <span class="t-ui asklaunch__ph">Search or ask about your day…</span>
    <span class="kbd asklaunch__k">⌘⏎</span>
  </button>
</Tile>
