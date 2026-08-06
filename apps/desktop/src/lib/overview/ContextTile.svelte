<script lang="ts">
  // Context (1×1) — what you told Mnema, and what it concluded. One door into
  // the Context destination (mockup 10).
  //
  // It counts TWO separate real numbers: standing statements (authored rows,
  // `list_user_context_authored`) and inferred conclusions
  // (`UserContextStatus.conclusionCount`). Page 01's "142 facts about you · 3
  // pending" is the older, looser wording — the backend has no pending-review
  // state at all (a conclusion is visible, faded, dismissed or superseded; an
  // authored statement is live the moment you add it), so the tile must not
  // claim one.
  //
  // The authored list is the one read the Overview's shared burst does not
  // carry, so this tile fetches it itself and re-reads on `user_context_changed`
  // — which is also what keeps the count honest right after you add a statement
  // on the destination and come back.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { AuthoredContext, Conclusion, UserContextStatus } from "$lib/types/recording";
  import Glyph from "./Glyph.svelte";

  let {
    status,
    loaded,
  }: {
    status: UserContextStatus | null;
    /** Kept for the Overview's one-burst contract; the tile reads the authored
     *  list itself and does not need the conclusion bodies. */
    conclusions: Conclusion[];
    loaded: boolean;
  } = $props();

  let statements = $state<AuthoredContext[]>([]);

  onMount(() => {
    const read = () => {
      void invoke<AuthoredContext[]>("list_user_context_authored")
        .then((list) => (statements = list ?? []))
        .catch(() => {});
    };
    read();

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", read).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  const standing = $derived(statements.length);
  const inferred = $derived(status?.conclusionCount ?? 0);
  // Authored statements come back newest-first.
  const newest = $derived(statements[0]?.text ?? null);
</script>

<div class="tile tile--static">
  <div class="tile__h">
    <span class="t-label">Context</span>
    <button type="button" class="tile__more door" onclick={() => void goto("/context")}>
      review <span class="chev"><Glyph name="chevr" /></span>
    </button>
  </div>

  {#if standing > 0 || inferred > 0}
    <div class="trow">
      <span class="t-ui strong is-num">
        {standing} standing {standing === 1 ? "statement" : "statements"}
      </span>
    </div>
    {#if newest}
      <div class="trow newest"><span class="t-meta">Newest: “{newest}”</span></div>
    {/if}
    <div class="trow bottom">
      <span class="t-meta subtle is-num">
        {inferred} inferred {inferred === 1 ? "conclusion" : "conclusions"}
      </span>
    </div>
  {:else}
    <div class="pay quiet">
      <span class="t-meta subtle">
        {#if !loaded}
          Reading…
        {:else if status && !status.engineAvailable}
          No engine configured — set one up in Intelligence
        {:else}
          Nothing yet — tell Mnema about you
        {/if}
      </span>
    </div>
  {/if}
</div>

<style>
  .strong {
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .subtle {
    color: var(--app-text-subtle);
  }
  .newest {
    align-items: flex-start;
    line-height: 1.35;
  }
  .newest .t-meta {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }
  .bottom {
    margin-top: auto;
  }
  /* The header row IS the door — the rows below are a read-out, so none of them
     wears a chevron it cannot honour. */
  .door {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--app-accent);
    cursor: pointer;
  }
  .door .chev {
    width: 8px;
    height: 12px;
    color: currentColor;
  }
  .door:hover {
    filter: brightness(1.1);
  }
  .door:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--app-accent-glow);
    border-radius: var(--r-sm);
  }
  .quiet {
    display: flex;
    align-items: center;
  }
</style>
