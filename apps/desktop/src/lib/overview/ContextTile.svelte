<script lang="ts">
  // What YOU told Mnema — the door to the Context destination (page 10).
  //
  // It counts AUTHORED lines only. Inferred beliefs are counted on the Subjects
  // tile, because they are a different thing: one you asserted and it never
  // fades, the other Mnema concluded and its confidence rises and fades. Mixing
  // them into one number would blur the distinction the destination exists for.
  //
  // The mockup's "3 pending" header note is absent: there is no pending-review
  // count in the backend, and G8 says a number that isn't real does not ship.
  import type { AuthoredContext } from "$lib/types/recording";
  import type { LoadState } from "./overview-data.svelte";
  import TileShell from "./TileShell.svelte";

  interface Props {
    authored: LoadState<AuthoredContext[]>;
    /** The tile is the Context destination's door (page 10). */
    onopen?: () => void;
  }

  let { authored, onopen }: Props = $props();

  const count = $derived(authored.status === "ok" ? authored.value.length : null);

  const newest = $derived.by<AuthoredContext | null>(() => {
    if (authored.status !== "ok") return null;
    let best: AuthoredContext | null = null;
    for (const a of authored.value) if (!best || a.createdAtMs > best.createdAtMs) best = a;
    return best;
  });

  const quiet = $derived(
    authored.status === "failed"
      ? "Couldn't read your context."
      : count === 0
        ? "You haven't told Mnema anything about yourself yet."
        : null,
  );
</script>

<!-- The mockup's "7 standing" header note and the "Review all ›" opener want the
     same seat in `TileShell`'s header; the opener wins, because the tile is the
     destination's door and the count is already this tile's headline row. -->
<TileShell label="Context" more={onopen ? "Review all" : undefined} {onopen} {quiet}>
  {#if count !== null && count > 0}
    <div class="ss-trow">
      <span class="t-ui strong is-num">{count.toLocaleString()}</span>
      <span class="t-ui">{count === 1 ? "thing you told Mnema" : "things you told Mnema"}</span>
    </div>
  {/if}
  {#if newest}
    <div class="ss-trow newest">
      <span class="t-meta">Newest: “{newest.text}”</span>
    </div>
  {/if}
</TileShell>

<style>
  .strong {
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }

  .newest {
    align-items: flex-start;
    overflow: hidden;
  }

  .newest .t-meta {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
