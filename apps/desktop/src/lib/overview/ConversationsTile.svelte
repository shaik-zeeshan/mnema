<script lang="ts">
  // Conversations, not minutes — three of the day's spoken stretches.
  //
  // From `get_conversations` (read-time join of Activities against speaker
  // turns). The mockup's pull-quote ("the webhook is still 400-ing") is not
  // drawn: the cluster carries no transcript text, and inventing one would be
  // fabrication. The waveform thumb is a source glyph, not a rendered envelope
  // — the cluster carries no audio-segment id to read peaks from.
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import { formatClock, formatMinutes } from "./format";
  import type { Cell } from "./data";
  import type { ConversationCluster } from "$lib/highlights";

  interface Props {
    conversations: Cell<ConversationCluster[]>;
    loaded: boolean;
    open: () => void;
  }

  let { conversations, loaded, open }: Props = $props();

  const rows = $derived((conversations.data ?? []).slice(0, 3));
  const count = $derived(
    conversations.data ? `${conversations.data.length} today` : null,
  );

  function speakers(n: number): string {
    return n === 1 ? "1 speaker" : `${n} speakers`;
  }
</script>

<Tile
  id="conversations"
  title="Conversations"
  kbd="⌃C"
  more={count}
  span={2}
  {open}
  openLabel="Open the timeline"
>
  {#if conversations.error}
    <p class="tile-empty t-meta">Conversations unavailable — {conversations.error}</p>
  {:else if loaded && rows.length === 0}
    <p class="tile-empty t-meta">No conversations heard today.</p>
  {:else}
    {#each rows as row (row.activityId)}
      <button class="grow" type="button" onclick={open}>
        <span class="wavethumb" aria-hidden="true">
          <svg viewBox="0 0 44 26" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M5 13v0M11 8v10M17 4.5v17M23 9.5v7M29 6v14M35 10.5v5M39 12v2" />
          </svg>
        </span>
        <span class="grow__txt">
          <span class="grow__lbl">{row.title}</span>
          <span class="grow__sub">
            {formatMinutes(row.spokenMs)} · {speakers(row.speakerCount)}
          </span>
        </span>
        <span class="grow__val">
          <span class="t-meta is-mono is-num">{formatClock(row.startedAtMs)}</span>
          <Chev />
        </span>
      </button>
    {/each}
  {/if}
</Tile>
