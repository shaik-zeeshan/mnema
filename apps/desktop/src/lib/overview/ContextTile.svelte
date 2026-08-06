<script lang="ts">
  // Context (1×1) — what Mnema has learned about you, and one door into it.
  //
  // Data: `get_user_context_status` (counts) + `list_user_context_conclusions`
  // (the newest belief's own words). The mockup's "3 pending" meta has no field
  // behind it — `UserContextStatus` carries no pending-review count — so the
  // meta shows the Subject count, which it does carry.
  //
  // The tile navigates, its rows do not: Insights has no per-belief deep link,
  // so a row chevron would promise a landing the app cannot make.
  import { goto } from "$app/navigation";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";
  import { newestStatement } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let {
    status,
    conclusions,
    loaded,
  }: {
    status: UserContextStatus | null;
    conclusions: Conclusion[];
    loaded: boolean;
  } = $props();

  const facts = $derived(status?.conclusionCount ?? 0);
  const subjects = $derived(status?.subjectCount ?? 0);
  const newest = $derived(newestStatement(conclusions));
</script>

<div class="tile tile--static">
  <div class="tile__h">
    <span class="t-label">Context</span>
    {#if subjects > 0}
      <span class="tile__more is-num">{subjects} {subjects === 1 ? "subject" : "subjects"}</span>
    {/if}
  </div>

  {#if facts > 0}
    <div class="trow">
      <span class="t-ui strong is-num">
        {facts} {facts === 1 ? "fact" : "facts"} about you
      </span>
    </div>
    {#if newest}
      <div class="trow newest"><span class="t-meta">Newest: “{newest}”</span></div>
    {/if}
    <button type="button" class="trow more" onclick={() => void goto("/context")}>
      <span class="t-meta subtle">Review all</span>
      <span class="chev"><Glyph name="chevr" /></span>
    </button>
  {:else}
    <div class="pay quiet">
      <span class="t-meta subtle">
        {#if !loaded}
          Reading…
        {:else if status && !status.engineAvailable}
          No engine configured — set one up in Intelligence
        {:else}
          Nothing learned yet
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
  button.more {
    width: 100%;
    margin-top: auto;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  button.more .chev {
    margin-left: auto;
    width: 8px;
    height: 12px;
  }
  button.more:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .quiet {
    display: flex;
    align-items: center;
  }
</style>
