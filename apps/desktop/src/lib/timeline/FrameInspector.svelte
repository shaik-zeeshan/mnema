<script lang="ts">
  // ── The Timeline's inspector (direction 02, Studio Shell) ─────────────────
  // The 256px right panel every surface carries. On Timeline its subject is
  // the frame under the playhead: capture facts, the app that owned the
  // window, what OCR found, and the audio segment when one is selected.
  //
  // Rendering only — every value arrives as a prop already resolved by
  // `routes/+page.svelte`. No `invoke`, no data loading, no derived state that
  // the page doesn't already own.
  //
  // G8 (honest numbers): a row renders ONLY when the fact behind it is real on
  // this machine. No zeros standing in for unknowns, no placeholder dashes for
  // things we never measured — the row is simply absent. That is why there is
  // no "Size · 214 KB · H.264" row from the mockup: nothing in the frame DTO
  // carries an encoded byte size.
  import type { FrameDto } from "$lib/types/app-infra";
  import type { OcrStatus } from "$lib/frame-ocr";

  interface Props {
    frame: FrameDto | null;
    /** 1-based position of `frame` in the loaded window. */
    index: number;
    total: number;
    /** More frames exist beyond the loaded window (renders as `4,512+`). */
    hasMore: boolean;
    capturedLabel: string;
    ocrStatus: OcrStatus;
    /** True only when the OCR state above describes THIS frame. */
    ocrIsForFrame: boolean;
    ocrProviderLabel: string | null;
    ocrRegionCount: number;
    ocrCharCount: number | null;
    /** Selected audio segment, already formatted by the page. */
    audio: {
      sourceLabel: string;
      rangeLabel: string;
      durationLabel: string;
      speakerLabel: string | null;
    } | null;
    copyImageDisabled: boolean;
    copyTextAvailable: boolean;
    copyTextDisabled: boolean;
    onCopyImage: () => void;
    onCopyText: () => void;
  }

  let {
    frame,
    index,
    total,
    hasMore,
    capturedLabel,
    ocrStatus,
    ocrIsForFrame,
    ocrProviderLabel,
    ocrRegionCount,
    ocrCharCount,
    audio,
    copyImageDisabled,
    copyTextAvailable,
    copyTextDisabled,
    onCopyImage,
    onCopyText,
  }: Props = $props();

  const num = new Intl.NumberFormat();
  const indexLabel = $derived(
    `${num.format(index)} of ${num.format(total)}${hasMore ? "+" : ""}`,
  );
  const dimsLabel = $derived(
    frame?.width != null && frame?.height != null
      ? `${frame.width} × ${frame.height}`
      : null,
  );
  const ocrStateLabel = $derived.by<string>(() => {
    if (!ocrIsForFrame) return "not run for this frame";
    switch (ocrStatus) {
      case "running":
        return "reading…";
      case "success":
        return "read";
      case "empty":
        return "no text on this frame";
      case "missing":
        return "no stored result";
      case "error":
        return "failed";
      default:
        return "not run for this frame";
    }
  });
</script>

<aside class="ss-insp timeline__insp" aria-label="Frame inspector">
  <div class="ss-insp__h">
    <span>Frame</span>
  </div>
  <div class="ss-insp__b">
    {#if !frame}
      <p class="ss-insp__empty">
        Nothing selected. Scrub the rail to park the playhead on a frame — its
        capture, application and recognised text land here.
      </p>
    {:else}
      <div class="ss-insp__sec"><span>Capture</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">Time</span>
        <span class="ss-kv__v is-mono">{capturedLabel}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Index</span>
        <span class="ss-kv__v is-mono">{indexLabel}</span>
      </div>
      {#if dimsLabel}
        <div class="ss-kv">
          <span class="ss-kv__k">Display</span>
          <span class="ss-kv__v is-mono">{dimsLabel}</span>
        </div>
      {/if}
      <div class="ss-kv">
        <span class="ss-kv__k">Session</span>
        <span class="ss-kv__v is-mono">{frame.sessionId}</span>
      </div>

      <div class="ss-insp__sec"><span>Application</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">App</span>
        <span class="ss-kv__v">{frame.appName ?? "unknown"}</span>
      </div>
      {#if frame.windowTitle}
        <div class="ss-kv ss-kv--stack">
          <span class="ss-kv__k">Window</span>
          <span class="ss-kv__v">{frame.windowTitle}</span>
        </div>
      {/if}
      {#if frame.appBundleId}
        <div class="ss-kv">
          <span class="ss-kv__k">Bundle</span>
          <span class="ss-kv__v is-mono">{frame.appBundleId}</span>
        </div>
      {/if}
      {#if frame.url}
        <div class="ss-kv ss-kv--stack">
          <span class="ss-kv__k">URL</span>
          <span class="ss-kv__v is-mono">{frame.url}</span>
        </div>
      {/if}

      <div class="ss-insp__sec"><span>Recognised text</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">State</span>
        <span class="ss-kv__v">{ocrStateLabel}</span>
      </div>
      {#if ocrIsForFrame && ocrProviderLabel}
        <div class="ss-kv">
          <span class="ss-kv__k">Engine</span>
          <span class="ss-kv__v is-mono">{ocrProviderLabel}</span>
        </div>
      {/if}
      {#if ocrIsForFrame && ocrStatus === "success" && ocrRegionCount > 0}
        <div class="ss-kv">
          <span class="ss-kv__k">Blocks</span>
          <span class="ss-kv__v is-mono"
            >{num.format(ocrRegionCount)}{ocrCharCount != null
              ? ` · ${num.format(ocrCharCount)} chars`
              : ""}</span
          >
        </div>
      {/if}

      {#if audio}
        <div class="ss-insp__sec"><span>Audio segment</span></div>
        <div class="ss-kv">
          <span class="ss-kv__k">Source</span>
          <span class="ss-kv__v">{audio.sourceLabel}</span>
        </div>
        <div class="ss-kv">
          <span class="ss-kv__k">Range</span>
          <span class="ss-kv__v is-mono">{audio.rangeLabel}</span>
        </div>
        <div class="ss-kv">
          <span class="ss-kv__k">Length</span>
          <span class="ss-kv__v is-mono">{audio.durationLabel}</span>
        </div>
        {#if audio.speakerLabel}
          <div class="ss-kv">
            <span class="ss-kv__k">Speakers</span>
            <span class="ss-kv__v is-mono">{audio.speakerLabel}</span>
          </div>
        {/if}
      {/if}

      <div class="timeline__insp-acts">
        <button
          type="button"
          class="btn btn--sm timeline__insp-act"
          onclick={onCopyImage}
          disabled={copyImageDisabled}>Copy image</button
        >
        {#if copyTextAvailable}
          <button
            type="button"
            class="btn btn--sm timeline__insp-act"
            onclick={onCopyText}
            disabled={copyTextDisabled}>Copy recognised text</button
          >
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  /* `.ss-insp*`, `.ss-kv*`, `.btn` are global (studio-shell.css / +layout).
     Only the timeline's own placement rules live here. */
  .timeline__insp {
    /* The kit's panel is `overflow: hidden`; the body scrolls, and long window
       titles / URLs wrap rather than widening the 256px column. */
    min-height: 0;
  }

  .timeline__insp-acts {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 10px var(--s-10) 0;
  }

  .timeline__insp-act {
    width: 100%;
    justify-content: flex-start;
  }
</style>
