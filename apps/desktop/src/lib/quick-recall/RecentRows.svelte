<!-- ↓ on the empty Quick Recall bar (recall.html "recent"): the last few
     searches and asks (`search.states.recent`, kept in the encrypted DB), then
     Try examples. Recognised phrases are tinted as they will become pills.
     Keys while open: ↑/↓ pick · ↵ use · ⌫ forgets a Recent row · Esc closes ·
     typing closes and types on. -->
<script lang="ts">
  import IconHistory from "~icons/lucide/history";
  import IconSearch from "~icons/lucide/search";
  import IconSparkle from "~icons/lucide/sparkle";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { parseNatural } from "./natural-tokens";
  import { agoLabel } from "./recall-states";

  const states = search.states;
  const now = new Date();
  let el = $state<HTMLElement | null>(null);

  // ponytail: three fixed examples that teach the phrases (one names an app
  // you actually use); no suggestion engine.
  const tries = $derived([
    { text: `notes in ${search.appNames[0] ?? "Arc"} yesterday afternoon`, ask: false },
    { text: "what did I work on this morning?", ask: true },
    { text: "standup on a call this week", ask: false },
  ]);
  const count = $derived(states.recent.length + tries.length);
  const textAt = (i: number) => (states.recent[i] ?? tries[i - states.recent.length])?.text;

  // The text split into plain runs and the phrases that will become pills.
  function segments(text: string): { text: string; live: boolean }[] {
    const out: { text: string; live: boolean }[] = [];
    let at = 0;
    for (const span of parseNatural(text, now, search.appNames).spans) {
      if (span.start > at) out.push({ text: text.slice(at, span.start), live: false });
      out.push({ text: text.slice(span.start, span.end), live: true });
      at = span.end;
    }
    if (at < text.length) out.push({ text: text.slice(at), live: false });
    return out;
  }

  function use(i: number): void {
    const text = textAt(i);
    if (text) states.useRecent(text);
  }

  function onKey(event: KeyboardEvent): void {
    if (event.isComposing || event.metaKey || event.ctrlKey || event.altKey) return;
    const i = states.recentIndex;
    switch (event.key) {
      case "ArrowDown":
        states.recentIndex = Math.min(count - 1, i + 1);
        break;
      case "ArrowUp":
        states.recentIndex = Math.max(0, i - 1);
        break;
      case "Enter":
        use(i);
        break;
      case "Backspace":
      case "Delete":
        if (i >= states.recent.length) return;
        void states.forgetRecent(i);
        states.recentIndex = Math.min(i, count - 2);
        break;
      case "Escape":
        states.recentOpen = false;
        break;
      default:
        // Typing leaves Recent and edits the query.
        if (event.key.length === 1) states.recentOpen = false;
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  $effect(() => {
    el?.querySelectorAll(".rc-row")[states.recentIndex]?.scrollIntoView({ block: "nearest" });
  });
</script>

<svelte:window onkeydowncapture={onKey} />

{#snippet row(i: number, text: string, kind: "hist" | "ask" | "try", time: string)}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="rc-row mx-reveal"
    role="option"
    tabindex="-1"
    aria-selected={states.recentIndex === i}
    style:--i={i}
    onmousemove={() => (states.recentIndex = i)}
    onclick={() => use(i)}
  >
    {#if kind === "ask"}<IconSparkle width="14" height="14" aria-hidden="true" />
    {:else if kind === "hist"}<IconHistory width="14" height="14" aria-hidden="true" />
    {:else}<IconSearch width="14" height="14" aria-hidden="true" />{/if}
    <span class="rc-row__t"
      >{#each segments(text) as seg, k (k)}{#if seg.live}<em>{seg.text}</em>{:else}{seg.text}{/if}{/each}</span
    >
    <time>{time}</time>
  </div>
{/snippet}

<div bind:this={el} role="listbox" aria-label="Recent and suggested searches">
  {#if states.recent.length > 0}
    <div class="rc-sec">Recent</div>
    {#each states.recent as q, i (q.text)}
      {@render row(i, q.text, q.ask ? "ask" : "hist", agoLabel(q.at, now))}
    {/each}
  {/if}
  <div class="rc-sec">Try</div>
  {#each tries as t, k (t.text)}
    {@render row(states.recent.length + k, t.text, t.ask ? "ask" : "try", t.ask ? "ask" : "")}
  {/each}
</div>

<style>
  .rc-sec {
    margin: 10px 10px 4px;
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .rc-row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r-md);
    cursor: default;
    color: var(--app-text-subtle);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .rc-row:hover {
    background: var(--mx-wash);
  }
  .rc-row[aria-selected="true"] {
    background: var(--mx-selected);
  }
  .rc-row__t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 400 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text);
  }
  .rc-row[aria-selected="true"] .rc-row__t {
    color: var(--app-text-strong);
  }
  .rc-row__t em {
    font-style: normal;
    border-radius: 3px;
    background: var(--mx-wash-strong);
    box-shadow: 0 0 0 2px var(--mx-wash-strong);
    color: var(--app-text-strong);
  }
  time {
    min-width: 64px;
    text-align: right;
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums;
  }
  .rc-row[aria-selected="true"] time {
    color: var(--app-text);
  }
</style>
