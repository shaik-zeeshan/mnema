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
  import { relativeAgo } from "$lib/journal/band-stats";
  import type { Cell } from "./data";
  import type { UserContextDigest } from "$lib/types/recording";

  interface Props {
    digest: Cell<UserContextDigest | null>;
    loaded: boolean;
    open: () => void;
  }

  let { digest, loaded, open }: Props = $props();

  // "updated 12 min ago" — the age of the read, which is what "is this current?"
  // actually asks. Page 08's Today tile; same relative formatter the journal's
  // lede uses, so the tile and the destination never disagree.
  const updated = $derived(
    digest.data ? `updated ${relativeAgo(digest.data.generatedAtMs)}` : null,
  );
</script>

<Tile
  id="digest"
  title="Today"
  kbd="⌃D"
  more={updated}
  span={2}
  {open}
  openLabel="Open the journal"
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
  <!-- The door to the journal. The ⏎ cap is real: the tile is focusable (⌃D)
       and Enter opens it — Tile.svelte owns that half. -->
  <div class="tile-row digest__open">
    <span class="t-meta digest__open-label">Open the journal</span>
    <span class="kbd digest__open-k">⏎</span>
  </div>
</Tile>

<style>
  .digest__open {
    margin-top: auto;
  }
  .digest__open-label {
    color: var(--app-accent);
  }
  .digest__open-k {
    margin-left: auto;
  }
</style>
