<script lang="ts">
  // What Mnema believes it knows about you: the live fact count and the newest
  // one. "3 pending" from the mockup is dropped — nothing in
  // `get_user_context_status` counts pending facts (G8).
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import type { Cell } from "./data";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";

  interface Props {
    context: Cell<UserContextStatus | null>;
    conclusions: Cell<Conclusion[]>;
    loaded: boolean;
    open: () => void;
  }

  let { context, conclusions, loaded, open }: Props = $props();

  const count = $derived(context.data?.conclusionCount ?? null);
  const newest = $derived.by(() => {
    const list = conclusions.data ?? [];
    if (list.length === 0) return null;
    return list.reduce((a, b) => (b.formedAtMs > a.formedAtMs ? b : a)).statement;
  });
</script>

<Tile id="context" title="Context" kbd="⌃K" {open} openLabel="Open Insights">
  {#if context.error}
    <p class="tile-empty t-meta">Context unavailable — {context.error}</p>
  {:else if loaded && (count === null || count === 0)}
    <p class="tile-empty t-meta">Nothing learned about you yet.</p>
  {:else}
    <div class="tile-row">
      <span class="t-ui strong">{count ?? 0} facts about you</span>
    </div>
    {#if newest}
      <div class="tile-row ctx__newest">
        <span class="t-meta">Newest: “{newest}”</span>
      </div>
    {/if}
    <div class="tile-row ctx__foot">
      <span class="t-meta ctx__review">Review all</span>
      <Chev />
    </div>
  {/if}
</Tile>
