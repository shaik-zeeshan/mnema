<script lang="ts">
  // The receipt's frame tile (mockup 08) — a 2×2 whose media bleeds to all four
  // edges with the app / window / clock / OCR chips floating on it. Same three
  // honest states as the shipping viewer (ADR 0029 / ADR 0049), re-dressed as
  // tiles: the frame, the "footage expired" panel (retention removes pixels,
  // never cards), and the bounded audio-only player for an activity grounded in
  // sound with no frames at all — which drops the filmstrip rather than faking
  // one. Purely presentational; the parent owns every bit of playback state.
  import IconClock from "~icons/lucide/clock";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import { clock, clockSec } from "$lib/insights/receipt-clock";
  import { audioFooterLeft, type ReceiptViewState, type TurnView } from "$lib/insights/receipt-audio";
  import type { FramePreviewDto } from "$lib/types/app-infra";

  let {
    loading,
    turnsPending,
    viewState,
    isPlaying,
    selectedTurn,
    currentUrl,
    metaApp,
    metaTitle,
    currentMs,
    hasOcr,
    currentPreview,
    frameEvidenceCount,
    onTogglePlay,
  }: {
    loading: boolean;
    turnsPending: boolean;
    viewState: ReceiptViewState;
    isPlaying: boolean;
    selectedTurn: TurnView | null;
    currentUrl: string | null;
    metaApp: string | null;
    metaTitle: string | null;
    currentMs: number | null;
    hasOcr: boolean;
    currentPreview: FramePreviewDto | null;
    frameEvidenceCount: number;
    onTogglePlay: () => void;
  } = $props();
</script>

{#if !loading && viewState === "expired"}
  <div class="tile tile--w3 tile--static">
    <div class="tile__h">
      <span class="t-label">Receipt</span><span class="tile__more">footage expired</span>
    </div>
    <div class="pay panel">
      <span class="glyph"><IconClock /></span>
      <span class="t-ui strong">Footage expired</span>
      <span class="t-meta narrow">
        The raw frames behind this card were removed by Retention Cleanup. The card, its
        summary, and its evidence list are kept — only the pixels age out.
      </span>
    </div>
  </div>
{:else if !loading && viewState === "audio-only"}
  <div class="tile tile--w2 tile--h2 tile--static">
    <div class="tile__h">
      <span class="t-label">Receipt</span><span class="tile__more">audio only</span>
    </div>
    <div class="pay panel panel--audio">
      <div class="arow">
        <button
          type="button"
          class="btn btn--icon play"
          aria-label={isPlaying ? "Pause spoken evidence" : "Play spoken evidence"}
          disabled={selectedTurn == null}
          onclick={onTogglePlay}
        >{#if isPlaying}<IconPause />{:else}<IconPlay />{/if}</button>
        {#if selectedTurn}
          <span>
            <span class="t-ui strong" style="color:var({selectedTurn.colorVar})">
              {selectedTurn.speaker}
            </span>
            {#if selectedTurn.sourceMeta}
              <span class="t-label via">via {selectedTurn.sourceMeta}</span>
            {/if}
          </span>
        {/if}
      </div>
      {#if selectedTurn}
        <span class="t-meta is-num">
          spoken segment · {clock(selectedTurn.startMs)}–{clock(selectedTurn.endMs)} · captured as
          audio
        </span>
      {:else if turnsPending}
        <span class="t-meta">Loading spoken evidence…</span>
      {:else}
        <span class="t-meta">No readable speech in the cited audio</span>
      {/if}
      <span class="t-label faint">{audioFooterLeft(frameEvidenceCount)}</span>
    </div>
  </div>
{:else}
  <div class="tile tile--w2 tile--h2 tile--static frame">
    <div class="vp">
      {#if currentUrl}
        <img src={currentUrl} alt={metaTitle ?? "Captured frame"} />
      {/if}
      {#if currentPreview?.hasSecretRedactions}
        <span class="vpred">
          <span class="chip chip--verdict chip--warn">
            {currentPreview.secretRedactionCount}
            {currentPreview.secretRedactionCount === 1 ? "redaction" : "redactions"}
          </span>
        </span>
      {/if}
      <span class="vpchips">
        {#if metaApp}<span class="vpchip">{metaApp}</span>{/if}
        {#if metaTitle}<span class="vpchip vpchip--title">{metaTitle}</span>{/if}
        {#if currentMs != null}<span class="vpchip">{clockSec(currentMs)}</span>{/if}
        {#if hasOcr}<span class="vpchip">OCR</span>{/if}
      </span>
    </div>
  </div>
{/if}

<style>
  /* The frame tile carries no header row: the media IS the payload and it bleeds
     to all four edges, clipped by the tile radius. */
  .frame {
    padding: 0;
  }
  .vp {
    position: absolute;
    inset: 0;
    background: var(--media-void);
  }
  .vp img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .vpchips {
    position: absolute;
    left: var(--s-8);
    bottom: var(--s-8);
    z-index: 3;
    display: flex;
    gap: var(--s-4);
    max-width: calc(100% - 16px);
  }
  .vpchip {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    background: var(--mat-hud);
    backdrop-filter: blur(12px);
    box-shadow: 0 0 0 var(--hairline) var(--menu-edge);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
    white-space: nowrap;
  }
  .vpchip--title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .vpred {
    position: absolute;
    right: var(--s-8);
    top: var(--s-8);
    z-index: 3;
  }

  .panel {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-6);
    text-align: center;
  }
  .panel--audio {
    align-items: flex-start;
    justify-content: flex-start;
    gap: var(--s-8);
    text-align: left;
  }
  .glyph {
    width: 20px;
    height: 20px;
    color: var(--app-text-faint);
  }
  .glyph :global(svg) {
    width: 100%;
    height: 100%;
  }
  .narrow {
    max-width: 44ch;
  }
  .strong {
    color: var(--app-text-strong);
  }
  .faint {
    color: var(--app-text-faint);
  }
  .arow {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .play {
    cursor: pointer;
    color: var(--cat-communication);
  }
  .play:disabled {
    opacity: var(--opacity-disabled);
    cursor: default;
  }
  .play :global(svg) {
    width: 12px;
    height: 12px;
  }
  .via {
    margin-left: var(--s-6);
    color: var(--app-text-subtle);
  }
</style>
