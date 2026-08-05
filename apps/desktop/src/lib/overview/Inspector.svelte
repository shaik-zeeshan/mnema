<script lang="ts">
  // The 256px right inspector — the direction's load-bearing piece.
  //
  // It is why the tiles can stay headlines: no tile ever has to grow a second
  // column, because the selected row's record lives here. The panel is dumb on
  // purpose — each tile hands it a `Selection` it already assembled — so a new
  // tile arrives without touching this file.
  import IconPanel from "~icons/lucide/panel-right";
  import type { Selection } from "./overview-data.svelte";

  interface Props {
    selection: Selection | null;
  }

  let { selection }: Props = $props();
</script>

<aside class="ss-insp" aria-label="Inspector">
  <div class="ss-insp__h">
    <span class="ic" aria-hidden="true"><IconPanel /></span>
    <span>Inspector</span>
  </div>
  <div class="ss-insp__b">
    {#if selection}
      <div class="ss-insp__sec">
        <span>Selection</span>
        <span class="src">{selection.source}</span>
      </div>
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Title</span>
        <span class="ss-kv__v title">{selection.title}</span>
      </div>
      {#if selection.lede}
        <div class="ss-kv ss-kv--stack">
          <span class="ss-kv__k">Statement</span>
          <span class="ss-kv__v">{selection.lede}</span>
        </div>
      {/if}

      {#each selection.sections as section (section.label)}
        <div class="ss-insp__sec"><span>{section.label}</span></div>
        {#each section.rows as row (row.k)}
          <div class="ss-kv">
            <span class="ss-kv__k">{row.k}</span>
            <span class="ss-kv__v" class:is-mono={row.mono}>{row.v}</span>
          </div>
        {/each}
      {/each}
    {:else}
      <p class="ss-insp__empty">Select a row in any tile to see its record here.</p>
    {/if}
  </div>
</aside>

<style>
  .ic {
    display: flex;
    font-size: 11px;
  }

  .src {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    text-transform: none;
    letter-spacing: 0;
  }

  .title {
    font: var(--w-semi) 15px / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-title);
  }
</style>
