<script lang="ts">
  // The day's conversations — a read-time join of Activities against speaker
  // turns (`get_conversations`), so a row is only ever a window that actually
  // held speech.
  //
  // Three rows and nothing else: the row's record (span, speakers, spoken time)
  // goes to the inspector. That is the trade the direction pays 256px for.
  import type { ConversationCluster } from "$lib/highlights";
  import type { LoadState, Selection } from "./overview-data.svelte";
  import { formatClock, formatSpan } from "./overview-format";
  import TileShell from "./TileShell.svelte";

  interface Props {
    conversations: LoadState<ConversationCluster[]>;
    selectedKey: string | null;
    /** The 800px floor: two single-line rows, which is what actually fits. */
    compact?: boolean;
    onselect: (selection: Selection) => void;
  }

  let { conversations, selectedKey, compact = false, onselect }: Props = $props();

  const all = $derived(conversations.status === "ok" ? conversations.value : []);
  const rows = $derived(all.slice(0, compact ? 2 : 3));

  const quiet = $derived(
    conversations.status === "failed"
      ? "Couldn't read this day's conversations."
      : conversations.status === "loading"
        ? null
        : all.length === 0
          ? "No recorded speech on this day."
          : null,
  );

  const more = $derived(all.length > 0 ? `${all.length} today` : undefined);

  function keyOf(c: ConversationCluster): string {
    return `conversation:${c.activityId}`;
  }

  function toSelection(c: ConversationCluster): Selection {
    const durationMs = c.endedAtMs - c.startedAtMs;
    return {
      key: keyOf(c),
      source: "Conversations",
      title: c.title,
      sections: [
        {
          label: "Conversation",
          rows: [
            // Both ends or neither — half a range reads as a bug, not a fact.
            {
              k: "When",
              v:
                formatClock(c.startedAtMs) === null || formatClock(c.endedAtMs) === null
                  ? ""
                  : `${formatClock(c.startedAtMs)} – ${formatClock(c.endedAtMs)}`,
              mono: true,
            },
            { k: "Duration", v: formatSpan(durationMs), mono: true },
            { k: "Speakers", v: `${c.speakerCount} ${c.speakerCount === 1 ? "cluster" : "clusters"}`, mono: true },
            { k: "Spoken", v: formatSpan(c.spokenMs), mono: true },
          ],
        },
      ],
    };
  }

  function subtitle(c: ConversationCluster): string {
    const speakers = `${c.speakerCount} ${c.speakerCount === 1 ? "speaker" : "speakers"}`;
    return `${formatSpan(c.spokenMs)} spoken · ${speakers}`;
  }
</script>

<TileShell
  label="Conversations"
  {more}
  span="ss-tile--2 ov-half"
  {quiet}
  selected={rows.some((c) => keyOf(c) === selectedKey)}
>
  {#each rows as c (c.activityId)}
    <button
      type="button"
      class="ss-row row"
      class:ss-row--sel={selectedKey === keyOf(c)}
      onclick={() => onselect(toSelection(c))}
    >
      <span class="ss-row__txt">
        <span class="ss-row__lbl">{c.title}</span>
        {#if !compact}<span class="ss-row__sub">{subtitle(c)}</span>{/if}
      </span>
      <span class="ss-row__val"><span class="t-meta is-mono is-num">{formatClock(c.startedAtMs)}</span></span>
    </button>
  {/each}
</TileShell>

<style>
  /* `.ss-row` is a div rule in the kit; as a <button> it needs the element
     defaults cleared so the hairline + full-row accent land unchanged. */
  .row {
    width: 100%;
    border: 0;
    border-top: 0;
    background: transparent;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: default;
    min-height: 0;
    padding: 0 var(--s-10);
  }
</style>
