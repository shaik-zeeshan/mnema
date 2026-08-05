<script lang="ts">
  // The day's digest, as reading prose.
  //
  // G11: Open Threads v1 is digest prose ONLY — the digest's own
  // "one open thread…" sentence is already inside `narrative`, so rendering the
  // narrative *is* the feature. No entity, no table, no extraction pass.
  //
  // The mockup drew inline frame citations inside this prose. The stored digest
  // carries no citation refs (narrative + headline + range, nothing else), so
  // there is nothing real to cite from and the citations are not drawn (G8).
  import Tile from "./Tile.svelte";
  import { formatClock } from "./format";
  import type { Cell } from "./data";
  import type { UserContextDigest } from "$lib/types/recording";

  interface Props {
    digest: Cell<UserContextDigest | null>;
    loaded: boolean;
    open: () => void;
  }

  let { digest, loaded, open }: Props = $props();

  const updated = $derived(
    digest.data ? `updated ${formatClock(digest.data.generatedAtMs)}` : null,
  );
</script>

<Tile
  id="digest"
  title="Today"
  kbd="⌃D"
  more={updated}
  span={2}
  {open}
  openLabel="Open Insights"
>
  {#if digest.error}
    <p class="tile-empty t-meta">Digest unavailable — {digest.error}</p>
  {:else if loaded && !digest.data}
    <p class="tile-empty t-meta">
      No digest yet. Mnema writes one once it has enough of the day to summarise.
    </p>
  {:else if digest.data}
    <p class="digest">{digest.data.narrative}</p>
  {/if}
</Tile>
