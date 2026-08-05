<script lang="ts">
  // Today — the digest tile (2×1). Round-4 decision G11: **Open Threads v1 is
  // digest PROSE ONLY**. The digest already writes the "one open thread…"
  // sentence, so this tile surfaces the narrative it wrote and nothing more —
  // no entity, no table, no extraction pass, no "mark resolved".
  //
  // Data: `get_latest_user_context_digest`, the read-only door. It never starts
  // an engine call just because a tile mounted, which is exactly why it exists.
  //
  // DEVIATION from the mockup, forced by the wire type: page 01 draws inline
  // `.cite` frame chips inside this prose. `UserContextDigest` carries
  // `narrative` + `headline` and no citation refs at all — the digest preamble
  // asks for plain sentences — so citing a frame here would mean inventing the
  // link. G8's rule ("only where the value is real") applies to a citation as
  // much as to a number: the prose renders uncited.
  import type { UserContextDigest, UserContextStatus } from "$lib/types/recording";
  import { clock } from "./overview-format";

  let {
    digest,
    status,
    loaded,
  }: {
    digest: UserContextDigest | null;
    status: UserContextStatus | null;
    loaded: boolean;
  } = $props();

  // Three states, in precedence order: no engine (nothing will ever arrive
  // until one is configured) → no digest yet → the digest.
  const engineOff = $derived(loaded && status !== null && !status.engineAvailable);
  const stamp = $derived(digest ? clock(digest.generatedAtMs) : "");
  const meta = $derived(stamp ? `updated ${stamp}` : engineOff ? "unavailable" : "");
</script>

<div class="tile tile--w2 tile--static">
  <div class="tile__h">
    <span class="t-label">Today</span>
    {#if meta}<span class="tile__more is-num">{meta}</span>{/if}
  </div>

  {#if digest}
    <div class="pay scroll body">
      {#if digest.headline}
        <p class="t-ui head">{digest.headline}</p>
      {/if}
      <p class="t-read prose">{digest.narrative}</p>
    </div>
  {:else}
    <!-- The designed unavailable state from page 07: three skeleton lines and
         one sentence naming the reason. Never a fabricated summary. -->
    <div class="pay">
      <div class="sk" style="width:94%"></div>
      <div class="sk" style="width:82%"></div>
      <div class="sk" style="width:60%"></div>
      <p class="t-meta why">
        {#if !loaded}
          Reading the day…
        {:else if engineOff}
          No engine configured — set one up in Intelligence
        {:else}
          No digest yet — Mnema writes one once the day holds enough activity
        {/if}
      </p>
    </div>
  {/if}
</div>

<style>
  .body {
    overflow-y: auto;
  }
  .head {
    margin: 0 0 var(--s-4);
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .prose {
    margin: 0;
  }
  .sk {
    height: 11px;
    margin-bottom: var(--s-8);
    border-radius: 3px;
    background: var(--app-surface-hover);
  }
  .why {
    margin: var(--s-8) 0 0;
    color: var(--app-text-subtle);
  }
</style>
