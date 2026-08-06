<script lang="ts">
  // ReceiptViewer — the Activity Receipt's viewport (the ONE flex-elastic region
  // of the modal). Renders exactly one of: the loading skeleton, the honest
  // "footage expired" panel (ADR 0029), the bounded audio-only player, or the
  // frame image + meta chips. Purely presentational — all playback state and
  // logic live in ActivityReceipt.svelte; this child only reflects derived props
  // and raises `onTogglePlay`. Split out to keep the parent under the 800-line
  // file ceiling (repo rule); its styles moved here with its markup so scoping
  // stays intact. In every branch the root is a single `.viewer*` element, so it
  // remains the `flex: 1 1 auto` child of `.modal-card`.

  import IconExpired from "~icons/lucide/history";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import { clock, clockSec } from "$lib/insights/receipt-clock";
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

{#if loading}
  <div class="viewer"><div class="skeleton" aria-hidden="true"></div></div>
{:else if viewState === "expired"}
  <!-- Retention removes frames while the card is kept (ADR 0029) AND nothing
       spoken was cited, so this expired state is honest, not an edge case. -->
  <div class="viewer viewer--expired">
    <div class="exp">
      <div class="exp__glyph" aria-hidden="true"><IconExpired /></div>
      <h4>Footage expired</h4>
      <p>
        The raw frames behind this card were removed by Retention Cleanup. The
        card, its summary, and its evidence list are kept — only the pixels age
        out.
      </p>
    </div>
  </div>
{:else if viewState === "audio-only"}
  <div class="viewer viewer--audio">
    <button
      type="button"
      class="big-play"
      aria-label={isPlaying ? "Pause spoken evidence" : "Play spoken evidence"}
      disabled={selectedTurn == null}
      onclick={onTogglePlay}
    >{#if isPlaying}<IconPause />{:else}<IconPlay />{/if}</button>
    {#if selectedTurn}
      <div class="a-spk" style="--_c: var({selectedTurn.colorVar});">
        <span class="a-spk__dot"></span>
        <b class="a-spk__name" class:is-fallback={selectedTurn.isFallback}>{selectedTurn.speaker}</b>
        {#if selectedTurn.sourceMeta}<span class="a-spk__meta">via {selectedTurn.sourceMeta}</span>{/if}
      </div>
      <div class="a-when">spoken segment · {clock(selectedTurn.startMs)}–{clock(selectedTurn.endMs)} · captured as audio</div>
    {:else if turnsPending}
      <div class="a-when">Loading spoken evidence…</div>
    {:else}
      <!-- Hydration finished with nothing readable (silent segments, or every
           fallback failed) — say so; a fake eternal "Loading…" reads as a hang. -->
      <div class="a-when">No readable speech in the cited audio</div>
    {/if}
  </div>
{:else}
  <div class="viewer">
    {#if currentUrl}
      <img class="viewer__img" src={currentUrl} alt={metaTitle ?? "Captured frame"} />
    {:else}
      <div class="skeleton" aria-hidden="true"></div>
    {/if}
    {#if currentPreview?.hasSecretRedactions}
      <span class="viewer__redactions">
        {currentPreview.secretRedactionCount}
        {currentPreview.secretRedactionCount === 1 ? "redaction" : "redactions"}
      </span>
    {/if}
    <div class="frame-meta">
      {#if metaApp}<span class="frame-meta__chip">{metaApp}</span>{/if}
      {#if metaTitle}<span class="frame-meta__chip">{metaTitle}</span>{/if}
      {#if currentMs != null}<span class="frame-meta__chip">{clockSec(currentMs)}</span>{/if}
      {#if hasOcr}<span class="frame-meta__chip frame-meta__chip--ocr">OCR</span>{/if}
    </div>
  </div>
{/if}

<style>
  /* The stage is an opaque PLATE — the sheet around it may be glass, pixels and
     prose never are. No transition on the img: instant frame swaps are the
     video feel. */
  .viewer { position: relative; flex: 1 1 auto; min-height: 0; overflow: hidden; border-radius: var(--r-lg); background: var(--app-surface); box-shadow: var(--sh-tile); }
  .viewer__img { display: block; width: 100%; height: 100%; object-fit: contain; }
  .skeleton { position: absolute; inset: 18px 22px; border-radius: var(--r-md); background: var(--app-surface-hover); animation: pulse 1.6s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.5; } }

  /* Frame chips ride ON the picture, so they are the one place a dark scrim is
     the honest background — the plate is behind them, not under the text. */
  .viewer__redactions { position: absolute; top: 8px; right: 8px; z-index: 2; display: inline-flex; align-items: center; height: 20px; padding: 0 8px; border-radius: var(--r-pill); background: rgba(12, 12, 16, 0.62); -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); color: #fff; font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans); }
  .frame-meta { position: absolute; left: 8px; bottom: 8px; z-index: 2; display: flex; gap: 6px; max-width: calc(100% - 16px); overflow: hidden; }
  .frame-meta__chip { display: inline-flex; align-items: center; height: 20px; padding: 0 8px; border-radius: var(--r-pill); background: rgba(12, 12, 16, 0.62); -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); color: #fff; font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  /* The OCR chip says only THAT text was recognized — the receipt never shows
     the recognized text itself (that would be a new surface). */
  .frame-meta__chip--ocr { font-family: var(--app-font-mono); letter-spacing: 0.06em; }

  /* The three viewports that are NOT a picture. All land on the same plate. */
  .viewer--expired, .viewer--audio { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 7px; text-align: center; padding: 0 22px; }
  .exp { max-width: 44ch; display: flex; flex-direction: column; align-items: center; gap: 7px; }
  .exp__glyph { display: flex; color: var(--app-text-faint); }
  .exp__glyph :global(svg) { width: 20px; height: 20px; }
  .exp h4 { margin: 0; font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans); color: var(--app-text-strong); }
  .exp p { margin: 0; font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans); color: var(--app-text-muted); }

  /* Audio-only — a bounded audio player, never a false "footage expired".
     Leads with WHO spoke; the channel is quiet secondary meta. */
  .big-play { width: 44px; height: 44px; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; border: 0; border-radius: 50%; color: var(--app-accent-contrast); background: var(--app-accent); }
  .big-play:disabled { opacity: var(--opacity-disabled); cursor: default; }
  .big-play :global(svg) { width: 16px; height: 16px; }
  .a-spk { display: inline-flex; align-items: center; gap: 8px; }
  .a-spk__dot { flex: none; width: 8px; height: 8px; border-radius: 50%; background: var(--_c); }
  .a-spk__name { font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans); color: var(--app-text-strong); }
  .a-spk__name.is-fallback { color: var(--_c); }
  .a-spk__meta { font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans); color: var(--app-text-muted); }
  .a-when { font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans); color: var(--app-text-muted); font-variant-numeric: tabular-nums; }

  @media (prefers-reduced-motion: reduce) { .skeleton { animation: none; } }
</style>
