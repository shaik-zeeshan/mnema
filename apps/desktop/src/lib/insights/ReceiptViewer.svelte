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
      {#if metaApp}<span class="frame-meta__chip frame-meta__chip--app">{metaApp}</span>{/if}
      {#if metaTitle}<span class="frame-meta__chip">{metaTitle}</span>{/if}
      {#if currentMs != null}<span class="frame-meta__chip">{clockSec(currentMs)}</span>{/if}
      {#if hasOcr}<span class="frame-meta__chip">OCR</span>{/if}
    </div>
  </div>
{/if}

<style>
  /* Viewer — the ONE elastic region; no transition on the img: instant frame
     swaps are the video feel. */
  .viewer { position: relative; flex: 1 1 auto; min-height: 0; overflow: hidden; background: var(--app-bg); border-bottom: 1px solid var(--app-border); }
  .viewer__img { display: block; width: 100%; height: 100%; object-fit: contain; }
  .skeleton { position: absolute; inset: 18px 22px; background: linear-gradient(160deg, var(--app-surface-raised), var(--app-bg) 70%); border: 1px solid var(--app-border); border-radius: 8px; animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 0%, 100% { opacity: 0.55; } 50% { opacity: 0.85; } }
  .viewer__redactions { position: absolute; top: 8px; right: 8px; padding: 3px 7px; font-size: 10px; color: var(--app-text-muted); background: var(--app-overlay-bg); border: 1px solid var(--app-border-strong); border-radius: 5px; backdrop-filter: blur(4px); }
  .frame-meta { position: absolute; left: 16px; bottom: 12px; display: flex; gap: 8px; max-width: calc(100% - 32px); overflow: hidden; }
  .frame-meta__chip { padding: 2px 8px; font-size: 10px; color: var(--app-text-muted); background: var(--app-overlay-bg); border: 1px solid var(--app-border-strong); border-radius: 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; backdrop-filter: blur(4px); }
  .frame-meta__chip--app { flex: 0 0 auto; color: var(--app-text); }

  .viewer--expired { aspect-ratio: 16 / 6; display: flex; align-items: center; justify-content: center; }
  .exp { max-width: 440px; padding: 24px; text-align: center; }
  .exp__glyph { display: flex; justify-content: center; margin-bottom: 10px; color: var(--app-text-faint); }
  .exp__glyph :global(svg) { width: 30px; height: 30px; }
  .exp h4 { margin: 0 0 6px; font-size: 13px; font-weight: 600; color: var(--app-text-strong); }
  .exp p { margin: 0; font-size: 11.5px; line-height: 1.7; color: var(--app-text-muted); }

  /* Audio-only viewer — a bounded audio player, never a false "footage expired".
     Leads with WHO spoke; the channel is quiet secondary meta. */
  .viewer--audio { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; text-align: center; }
  .big-play { width: 48px; height: 48px; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; border-radius: 50%; color: var(--cat-communication); background: var(--app-accent-bg); border: 1px solid var(--cat-communication); }
  .big-play:disabled { opacity: 0.5; cursor: default; }
  .big-play :global(svg) { width: 17px; height: 17px; }
  .a-spk { display: inline-flex; align-items: center; gap: 8px; }
  .a-spk__dot { flex: none; width: 9px; height: 9px; border-radius: 50%; background: var(--_c); box-shadow: 0 0 7px var(--_c); }
  .a-spk__name { font-size: 15px; font-weight: 600; color: var(--app-text-strong); }
  .a-spk__name.is-fallback { color: var(--_c); }
  .a-spk__meta { font-size: 10px; letter-spacing: 0.06em; text-transform: uppercase; color: var(--app-text-subtle); }
  .a-when { font-size: 10.5px; color: var(--app-text-subtle); font-variant-numeric: tabular-nums; }

  @media (prefers-reduced-motion: reduce) { .skeleton { animation: none; } }
</style>
