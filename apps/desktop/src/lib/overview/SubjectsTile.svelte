<script lang="ts">
  // Subjects (2×1) — the two most recently supported beliefs, one row per
  // Subject, with conviction drawn as filled `.conv` dots.
  //
  // Data: `list_user_context_conclusions`, reduced to one row per Subject here.
  // Rows are static and the HEADER is the door: Insights selects a Subject from
  // component state, not a URL, so a per-row link would land on the index
  // anyway. Better one honest door than four rows that all go to the same place.
  import { goto } from "$app/navigation";
  import type { Conclusion } from "$lib/types/recording";
  import { confidenceDots, subjectRows } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let {
    conclusions,
    subjectCount,
    loaded,
  }: { conclusions: Conclusion[]; subjectCount: number; loaded: boolean } = $props();

  const rows = $derived(subjectRows(conclusions, 2));
  const dots = [0, 1, 2, 3, 4];
</script>

<div class="tile tile--w2">
  <div class="tile__h">
    <span class="t-label">Subjects</span>
    <button type="button" class="tile__more" onclick={() => void goto("/subjects")}>
      {#if subjectCount > 0}<span class="is-num">{subjectCount}</span> active{:else}Subjects{/if}
      <span class="chev"><Glyph name="chevr" /></span>
    </button>
  </div>

  {#if rows.length}
    <div class="pay pay--rows">
      {#each rows as row (row.subject)}
        <div class="row row--static">
          <span class="aicon"><Glyph name="spark-o" /></span>
          <span class="row__txt">
            <span class="row__lbl">{row.subject}</span>
            <span class="row__sub">{row.statement}</span>
          </span>
          <span class="row__val">
            <span class="conv">
              {#each dots as dot (dot)}
                <i class:on={dot < confidenceDots(row.confidence)}></i>
              {/each}
            </span>
          </span>
        </div>
      {/each}
    </div>
  {:else}
    <div class="pay quiet">
      <span class="t-meta subtle">
        {loaded ? "No subjects yet — they form as Mnema sees repeated work" : "Reading…"}
      </span>
    </div>
  {/if}
</div>

<style>
  button.tile__more {
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
  }
  button.tile__more:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  button.tile__more .chev {
    width: 8px;
    height: 12px;
  }
  .aicon {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    padding: 5px;
  }
  .quiet {
    display: flex;
    align-items: center;
  }
  .subtle {
    color: var(--app-text-subtle);
  }
</style>
