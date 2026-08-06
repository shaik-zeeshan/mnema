<script lang="ts">
  // THE READ — the day's digest as the bento's opening 4×1 tile (mockup 08).
  // Headline + narrative verbatim, the day and how long ago it was written in
  // the header meta, and the re-read control that is the ONE model call this
  // surface can make (`regenerate_user_context_digest`), shown only when an
  // engine is actually available.
  //
  // Like the Overview's digest tile, the prose renders UNCITED: the digest wire
  // type carries `headline` + `narrative` and no frame refs, so an inline
  // citation here would be invented (G8 applies to a citation as much as to a
  // number).
  import IconRefresh from "~icons/lucide/rotate-cw";
  import type { UserContextDigest } from "$lib/types/recording";

  let {
    digest,
    dayLabel,
    engineOn,
    statusLoaded,
    loading,
    regenerating,
    error,
    onReread,
  }: {
    digest: UserContextDigest | null;
    dayLabel: string;
    engineOn: boolean;
    statusLoaded: boolean;
    loading: boolean;
    regenerating: boolean;
    error: string | null;
    onReread: () => void;
  } = $props();

  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    const diff = Date.now() - ms;
    if (diff < 60_000) return "just now";
    const min = Math.floor(diff / 60_000);
    if (min < 60) return `${min} min ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    return `${Math.floor(hr / 24)}d ago`;
  }

  const when = $derived(digest ? relativeTime(digest.generatedAtMs) : "");
</script>

<div class="tile tile--w4 tile--static">
  <div class="tile__h">
    <span class="t-label">The read</span>
    <span class="tile__more">
      <span>{dayLabel}</span>
      {#if when}<span>·</span><span class="is-num">{when}</span>{/if}
    </span>
    {#if engineOn}
      <button
        type="button"
        class="btn btn--ghost btn--sm reread"
        class:is-busy={regenerating}
        disabled={regenerating || (!digest && loading)}
        onclick={onReread}
      >
        <IconRefresh />{regenerating ? "reading…" : "re-read"}
      </button>
    {/if}
  </div>

  {#if digest}
    {#if digest.headline}<p class="read__h">{digest.headline}</p>{/if}
    <p class="read__n">{digest.narrative}</p>
  {:else if loading || regenerating}
    <div class="sk" style="width:64%; height:16px"></div>
    <div class="sk" style="width:94%"></div>
    <div class="sk" style="width:78%"></div>
  {:else}
    <div class="sk" style="width:64%; height:16px"></div>
    <div class="sk" style="width:88%"></div>
    <p class="t-meta why">
      {#if error}
        {error}
      {:else if !statusLoaded}
        Reading the day…
      {:else if !engineOn}
        No engine configured — set one up in Intelligence
      {:else}
        No read yet — Mnema writes one once the day holds enough activity
      {/if}
    </p>
  {/if}
</div>

<style>
  .read__h {
    margin: 0;
    font: var(--w-semi) 20px / 1.25 var(--app-font-sans);
    letter-spacing: -0.018em;
    color: var(--app-text-strong);
  }
  .read__n {
    max-width: 88ch;
    margin: var(--s-6) 0 0;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    color: var(--app-text);
  }
  .reread {
    margin-left: var(--s-8);
    cursor: pointer;
  }
  .reread:disabled {
    opacity: var(--opacity-disabled);
  }
  .reread :global(svg) {
    width: 12px;
    height: 12px;
  }
  .reread.is-busy :global(svg) {
    animation: reread-spin 0.9s linear infinite;
  }
  @keyframes reread-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .reread.is-busy :global(svg) {
      animation: none;
    }
  }
  .sk {
    height: 11px;
    margin-bottom: var(--s-8);
    border-radius: 3px;
    background: var(--app-surface-hover);
  }
  .why {
    margin: var(--s-4) 0 0;
    color: var(--app-text-subtle);
  }
</style>
