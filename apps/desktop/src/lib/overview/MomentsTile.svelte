<script lang="ts">
  // Moments — the 4×1 tile that opens the bento, and the direction's loudest
  // statement: the payload BLEEDS past the 14px inset to the tile edge and is
  // clipped by the tile radius. The strip is genuinely scrollable and its frames
  // are a fixed fraction of the tile width, so the last visible frame is
  // half-cut whenever there are more moments than fit — the half-cut frame is a
  // real overflow signifier, not a drawn decoration.
  //
  // Data: `get_moments` (read-time query over headline Activity evidence).
  // Click hands the frame to the Timeline through the same
  // `open_capture_result_in_main_window` seam Quick Recall and Chat use; the
  // shell layout listens for its event and routes the window to `/`.
  import { invoke } from "@tauri-apps/api/core";
  import type { Moment } from "$lib/highlights";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { clock } from "./overview-format";
  import Glyph from "./Glyph.svelte";

  let { moments, loaded }: { moments: Moment[]; loaded: boolean } = $props();

  async function open(moment: Moment): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId: moment.frameId,
        audioSegmentId: null,
        spanStartMs: null,
        alignedFrameId: null,
      });
    } catch {
      // Best-effort hand-off: a failed open leaves the Overview where it is.
    }
  }
</script>

<div class="tile tile--w4 tile--static">
  <div class="tile__h">
    <span class="t-label">Moments</span>
    <span class="tile__more">
      the day's main things{#if moments.length}
        · <span class="is-num">{moments.length}</span> in all{/if}
    </span>
  </div>

  {#if moments.length}
    <div class="pay pay--bleed">
      <div class="strip scroll">
        {#each moments as moment (moment.frameId)}
          <button
            type="button"
            class="strip__i"
            title={moment.title}
            onclick={() => void open(moment)}
          >
            <img src={framePreviewAssetUrl(moment.filePath)} alt="" loading="lazy" />
            <span class="strip__c">{moment.title}</span>
            <span class="strip__t">{clock(moment.capturedAtMs)}</span>
          </button>
        {/each}
      </div>
    </div>
  {:else}
    <div class="pay empty">
      <span class="empty__g"><Glyph name="scan" /></span>
      <span class="t-meta">{loaded ? "No moments yet today" : "Reading the day…"}</span>
      {#if loaded}
        <span class="t-meta faint">headline frames appear once the day has activity</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .strip {
    overflow-x: auto;
    overflow-y: hidden;
    scroll-snap-type: x proximity;
  }
  /* A fixed fraction of the tile, so 5.5 frames show at the default width and
     the sixth is genuinely cut by the tile's corner radius. */
  .strip__i {
    flex: 0 0 18.2%;
    padding: 0;
    border: 0;
    background: var(--media-void);
    cursor: pointer;
    scroll-snap-align: start;
  }
  .strip__i:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-6);
    text-align: center;
  }
  .empty__g {
    width: 18px;
    height: 18px;
    color: var(--app-text-faint);
  }
  .faint {
    color: var(--app-text-subtle);
  }

  @media (max-width: 900px) {
    .strip__i {
      flex: 0 0 27%;
    }
  }
</style>
