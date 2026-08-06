<script lang="ts">
  // The receipt's viewport — the one elastic region of the transport. Renders
  // exactly one of: loading, the honest "footage expired" panel (retention took
  // the pixels, the card and its evidence list survive — ADR 0029), a bounded
  // audio-only player (cited speech with no surviving frames), or the frame with
  // its meta chips.
  //
  // The meta chips are the frame's real metadata: app, window title, wall clock,
  // and an `OCR` PRESENCE flag — recognised text is never rendered here. The
  // redaction badge is drawn only when the preview really carries a count.
  import { clockSec } from "$lib/insights/receipt-clock";
  import type { ReceiptViewState, TurnView } from "$lib/insights/receipt-audio";
  import type { FramePreviewDto } from "$lib/types/app-infra";

  interface Props {
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
    onTogglePlay: () => void;
  }
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
    onTogglePlay,
  }: Props = $props();
</script>

<div class="rcpt__view">
  {#if loading}
    <div class="sk" aria-hidden="true"></div>
  {:else if viewState === "expired"}
    <div class="exp">
      <span class="gl" aria-hidden="true">◇</span>
      <span class="t-ui strong">Footage expired</span>
      <span class="t-meta">
        The raw frames behind this card were removed by Retention Cleanup. The card, its summary,
        and its evidence list are kept — only the pixels age out.
      </span>
      <span class="t-meta is-mono is-num exp__n">0 frames still on disk · summary retained</span>
    </div>
  {:else if viewState === "audio-only"}
    <div class="aud">
      <button
        type="button"
        class="btn btn--icon aud__play"
        aria-label={isPlaying ? "Pause spoken evidence" : "Play spoken evidence"}
        disabled={selectedTurn == null}
        onclick={onTogglePlay}
      >
        {#if isPlaying}
          <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><rect x="4.4" y="3.4" width="2.6" height="9.2" rx=".8" /><rect x="9" y="3.4" width="2.6" height="9.2" rx=".8" /></svg>
        {:else}
          <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M4.6 3.2 12.4 8l-7.8 4.8z" /></svg>
        {/if}
      </button>
      {#if selectedTurn}
        <span class="t-title aud__who" style="--spk: var({selectedTurn.colorVar});">
          <i></i>{selectedTurn.speaker}
        </span>
        <span class="t-meta is-mono aud__when">
          spoken segment · {clockSec(selectedTurn.startMs)}
          {#if selectedTurn.sourceMeta}· via {selectedTurn.sourceMeta}{/if}
        </span>
      {:else if turnsPending}
        <span class="t-meta aud__when">Loading spoken evidence…</span>
      {:else}
        <span class="t-meta aud__when">No readable speech in the cited audio</span>
      {/if}
    </div>
  {:else}
    {#if currentUrl}
      <img class="shot" src={currentUrl} alt={metaTitle ?? "Captured frame"} />
    {:else}
      <div class="sk" aria-hidden="true"></div>
    {/if}
    {#if currentPreview?.hasSecretRedactions}
      <span class="vchip vchip--redact">
        {currentPreview.secretRedactionCount}
        {currentPreview.secretRedactionCount === 1 ? "redaction" : "redactions"}
      </span>
    {/if}
    <span class="vchips">
      {#if metaApp}<span class="vchip">{metaApp}</span>{/if}
      {#if metaTitle}<span class="vchip vchip--ttl">{metaTitle}</span>{/if}
      {#if currentMs != null}<span class="vchip">{clockSec(currentMs)}</span>{/if}
      {#if hasOcr}<span class="vchip">OCR</span>{/if}
    </span>
  {/if}
</div>

<style>
  .rcpt__view {
    flex: 1 1 auto;
    min-height: 0;
    position: relative;
    margin: 0 var(--s-12);
    border-radius: var(--r-lg);
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 0 0 var(--hairline) var(--app-border);
  }
  .shot {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .sk {
    position: absolute;
    inset: var(--s-16);
    border-radius: var(--r-md);
    background: var(--app-surface-raised);
    animation: rcpt-pulse 1.4s ease-in-out infinite;
  }
  @keyframes rcpt-pulse {
    50% {
      opacity: 0.55;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sk {
      animation: none;
    }
  }

  .vchips {
    position: absolute;
    left: var(--s-8);
    bottom: var(--s-8);
    right: var(--s-8);
    display: flex;
    gap: var(--s-4);
    flex-wrap: wrap;
    z-index: 3;
  }
  .vchip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    max-width: 100%;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: rgba(10, 12, 16, 0.62);
    backdrop-filter: blur(8px);
    color: rgba(255, 255, 255, 0.92);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .vchip--ttl {
    overflow: hidden;
    text-overflow: ellipsis;
    display: inline-block;
    line-height: 20px;
  }
  .vchip--redact {
    position: absolute;
    right: var(--s-8);
    top: var(--s-8);
    z-index: 3;
    background: rgba(196, 58, 72, 0.78);
  }

  .exp,
  .aud {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-6);
    padding: var(--s-20);
    text-align: center;
  }
  .exp .t-meta,
  .aud .t-meta {
    max-width: 44ch;
  }
  .exp .gl {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-faint);
  }
  .exp__n {
    color: var(--app-text-subtle);
  }

  .aud__play {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  .aud__play svg {
    width: 16px;
    height: 16px;
  }
  .aud__who {
    display: inline-flex;
    align-items: center;
    gap: var(--s-6);
  }
  .aud__who i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--spk);
  }
  .aud__when {
    color: var(--app-text-subtle);
  }
</style>
