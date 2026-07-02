<script lang="ts">
  // FrameDetailModal — an in-place peek at a single captured frame: a hero
  // image, a metadata strip, quick actions (copy image, open URL), a redaction
  // badge, and a demoted "open full timeline →" escape hatch. Meant to replace
  // the old "View on timeline" hop with something that stays in context (e.g. an
  // Ask AI source card). A lazy OCR overlay + toggle (slice 3) reuses the shared
  // helpers in `$lib/frame-ocr` and mirrors the Timeline's box/chip visuals;
  // unlike the Timeline (background-image) the hero is a real <img>, so the
  // rendered rect is measured straight off its natural dims + the hero box.
  //
  // Chrome (overlay/panel, backdrop-pointerdown-to-close, Escape, Tab-trap, and
  // the opener focus handoff WebKit doesn't do for us) mirrors AppDetailModal.

  import { tick } from "svelte";
  import { invoke, convertFileSrc } from "@tauri-apps/api/core";
  import { Image } from "@tauri-apps/api/image";
  import { writeImage } from "@tauri-apps/plugin-clipboard-manager";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { trapTabKey } from "$lib/keyboard";
  import FrameOcrOverlay from "$lib/components/FrameOcrOverlay.svelte";
  import { framePreviewAssetUrl, readFramePreviewBytes } from "$lib/frame-preview";
  import { formatTimestampCompact } from "$lib/format-time";
  import { humanizeError } from "$lib/format-error";
  import {
    loadOcrForFrame,
    loadOcrFromJob,
    ocrBoxStyle,
    type OcrLoadResult,
    type OcrStatus,
  } from "$lib/frame-ocr";
  import type {
    FrameDto,
    FramePreviewDto,
    GetFramePreviewRequest,
    GetProcessingJobRequest,
    OcrObservation,
    ProcessingJobDto,
  } from "$lib/types/app-infra";

  const OCR_POLL_INTERVAL_MS = 1000;

  type AppIconResolution = { bundleId: string; iconPath: string | null };

  interface Props {
    open: boolean;
    frameId: number | null;
    // Optional prefetched hints from the caller for instant header paint,
    // superseded by the loaded FrameDto once get_frame resolves.
    appName?: string | null;
    windowTitle?: string | null;
    capturedAt?: string | null;
    onClose: () => void;
    // Escape hatch: when provided, renders the demoted link; clicking it closes
    // the modal then hands off to the caller (which knows how to open the full
    // timeline for this frame).
    onOpenInTimeline?: () => void;
  }

  let {
    open,
    frameId,
    appName = null,
    windowTitle = null,
    capturedAt = null,
    onClose,
    onOpenInTimeline,
  }: Props = $props();

  // ---- Loaded data ------------------------------------------------------
  let frame = $state<FrameDto | null>(null);
  let preview = $state<FramePreviewDto | null>(null);
  let loading = $state(false);
  let loadError = $state(false);

  // Header prefers the loaded frame, falling back to the caller's hints so the
  // chrome paints instantly before get_frame resolves.
  const displayApp = $derived(frame?.appName ?? appName ?? null);
  const displayTitle = $derived(frame?.windowTitle ?? windowTitle ?? null);
  const displayCapturedAt = $derived(frame?.capturedAt ?? capturedAt ?? null);
  const displayTime = $derived(
    displayCapturedAt ? formatTimestampCompact(displayCapturedAt) : null,
  );
  const dims = $derived(
    frame?.width && frame?.height ? `${frame.width}×${frame.height}` : null,
  );
  const frameUrl = $derived(frame?.url ?? null);
  const imgSrc = $derived(
    preview ? framePreviewAssetUrl(preview.filePath) : null,
  );
  // Resolved via `resolve_app_icons` off the loaded frame's bundle id (best
  // effort; a letter avatar covers the null case). Same command the timeline uses.
  let appIconSrc = $state<string | null>(null);
  const appFallback = $derived(
    ((displayApp ?? frame?.appBundleId ?? "").trim() || "?").slice(0, 1).toUpperCase(),
  );

  async function resolveAppIcon(bundleId: string, token: number): Promise<void> {
    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: [bundleId] },
      });
      if (token !== loadToken) return; // superseded
      const iconPath = icons.find((i) => i.bundleId === bundleId)?.iconPath ?? null;
      appIconSrc = iconPath ? convertFileSrc(iconPath) : null;
    } catch {
      // best-effort; the letter avatar stands in.
    }
  }

  // ---- Stale-load guard -------------------------------------------------
  // Bumped on every load start; an in-flight load bails if the token moved on
  // (frameId changed / modal reopened) so a slow response never paints onto a
  // newer frame. Mirrors the timeline's frameId-capture guard.
  let loadToken = 0;

  async function loadFrame(id: number): Promise<void> {
    const token = ++loadToken;
    loading = true;
    loadError = false;
    frame = null;
    preview = null;
    appIconSrc = null;
    try {
      const [frameDto, previewDto] = await Promise.all([
        invoke<FrameDto | null>("get_frame", { request: { frameId: id } }),
        invoke<FramePreviewDto | null>("get_frame_preview", {
          request: { frameId: id } satisfies GetFramePreviewRequest,
        }),
      ]);
      if (token !== loadToken) return; // superseded
      frame = frameDto;
      preview = previewDto;
      if (!frameDto) loadError = true;
      const bundleId = frameDto?.appBundleId?.trim();
      if (bundleId) void resolveAppIcon(bundleId, token);
    } catch {
      if (token !== loadToken) return;
      loadError = true;
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  // Load when the modal opens (or the target frame changes while open). When it
  // closes we invalidate any in-flight load and drop the data so a reopen never
  // flashes the previous frame.
  $effect(() => {
    // Any frameId/open change clears the OCR overlay first (cancels its poll,
    // hides + drops observations) so a reopen or a new frame never shows stale
    // boxes. Reading frameId+open keeps this reactive to both.
    frameId;
    open;
    resetOcr();
    if (open && frameId != null) {
      void loadFrame(frameId);
    } else {
      loadToken++;
      frame = null;
      preview = null;
      appIconSrc = null;
      loading = false;
      loadError = false;
    }
  });

  // ---- Copy image -------------------------------------------------------
  // Same path the timeline uses: decode the preview bytes into an RGBA canvas,
  // hand raw pixels to the Tauri clipboard Image. Transient "copied" feedback.
  let copyState = $state<"idle" | "copying" | "copied" | "failed">("idle");
  let copyRevertTimer: ReturnType<typeof setTimeout> | null = null;

  async function previewFilePathToClipboardImage(filePath: string): Promise<Image> {
    const blob = new Blob([await readFramePreviewBytes(filePath)]);
    const bitmap = await createImageBitmap(blob);
    try {
      const canvas = document.createElement("canvas");
      canvas.width = bitmap.width;
      canvas.height = bitmap.height;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) throw new Error("2d canvas context unavailable");
      ctx.drawImage(bitmap, 0, 0);
      const { data, width, height } = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      return await Image.new(new Uint8Array(data.buffer.slice(0)), width, height);
    } finally {
      bitmap.close();
    }
  }

  async function copyImage(): Promise<void> {
    if (!preview || copyState === "copying") return;
    if (copyRevertTimer) clearTimeout(copyRevertTimer);
    copyState = "copying";
    try {
      const image = await previewFilePathToClipboardImage(preview.filePath);
      try {
        await writeImage(image);
      } finally {
        image.close();
      }
      copyState = "copied";
    } catch {
      copyState = "failed";
    } finally {
      copyRevertTimer = setTimeout(() => (copyState = "idle"), 1600);
    }
  }

  // ---- OCR overlay (lazy) ----------------------------------------------
  // Nothing is fetched on open; the first toggle-on kicks a load. Subsequent
  // toggles just show/hide the already-loaded overlay. A running job is polled
  // until terminal. Every load/poll is guarded by `ocrGeneration` (bumped on
  // frame change / close / new load) so a slow response never paints onto a
  // newer frame, mirroring the frame-load `loadToken` guard.
  let ocrStatus = $state<OcrStatus>("idle");
  let ocrError = $state<string | null>(null);
  let ocrObservations = $state<OcrObservation[]>([]);
  let ocrProviderLabel = $state<string | null>(null);
  let ocrVisible = $state(false);
  let ocrGeneration = 0;
  let ocrPollTimer: ReturnType<typeof setTimeout> | null = null;
  let ocrPollJobId: number | null = null;

  function clearOcrPoll(): void {
    if (ocrPollTimer) {
      clearTimeout(ocrPollTimer);
      ocrPollTimer = null;
    }
    ocrPollJobId = null;
  }

  function resetOcr(): void {
    ocrGeneration++;
    clearOcrPoll();
    ocrStatus = "idle";
    ocrError = null;
    ocrObservations = [];
    ocrProviderLabel = null;
    ocrVisible = false;
  }

  function applyOcr(gen: number, sourceFrame: FrameDto, data: OcrLoadResult): void {
    if (gen !== ocrGeneration) return; // superseded (frame changed / reloaded)
    ocrStatus = data.status;
    ocrError = data.error;
    ocrObservations = data.observations;
    ocrProviderLabel = data.providerLabel;
    if (data.status === "running" && data.job) {
      scheduleOcrPoll(gen, sourceFrame, data.job.id);
    } else {
      clearOcrPoll();
    }
  }

  function scheduleOcrPoll(gen: number, sourceFrame: FrameDto, jobId: number): void {
    if (gen !== ocrGeneration) return;
    if (ocrPollTimer && ocrPollJobId === jobId) return;
    clearOcrPoll();
    ocrPollJobId = jobId;
    ocrPollTimer = setTimeout(() => {
      ocrPollTimer = null;
      ocrPollJobId = null;
      void pollOcr(gen, sourceFrame, jobId);
    }, OCR_POLL_INTERVAL_MS);
  }

  async function pollOcr(gen: number, sourceFrame: FrameDto, jobId: number): Promise<void> {
    if (gen !== ocrGeneration) return;
    try {
      const job = await invoke<ProcessingJobDto | null>("get_processing_job", {
        request: { jobId } satisfies GetProcessingJobRequest,
      });
      if (gen !== ocrGeneration) return;
      if (!job) {
        applyOcr(gen, sourceFrame, {
          status: "error", observations: [], providerLabel: null, error: "OCR job not found", job: null,
        });
        return;
      }
      applyOcr(gen, sourceFrame, await loadOcrFromJob(job, invoke));
    } catch (err) {
      if (gen !== ocrGeneration) return;
      applyOcr(gen, sourceFrame, {
        status: "error", observations: [], providerLabel: null, error: humanizeError(err), job: null,
      });
    }
  }

  async function loadOcr(sourceFrame: FrameDto): Promise<void> {
    const gen = ++ocrGeneration; // drop any prior in-flight load
    clearOcrPoll();
    ocrStatus = "running";
    ocrError = null;
    ocrObservations = [];
    ocrProviderLabel = null;
    try {
      applyOcr(gen, sourceFrame, await loadOcrForFrame(sourceFrame, invoke));
    } catch (err) {
      applyOcr(gen, sourceFrame, {
        status: "error", observations: [], providerLabel: null, error: humanizeError(err), job: null,
      });
    }
  }

  // idle → load; anything else → just show/hide the loaded surface.
  function toggleOcr(): void {
    if (!frame) return;
    if (ocrVisible) {
      ocrVisible = false;
      return;
    }
    ocrVisible = true;
    if (ocrStatus === "idle") void loadOcr(frame);
  }

  // ---- OCR overlay geometry --------------------------------------------
  // The hero <img> is object-fit:contain, so the painted rect is the contained
  // box of the img's natural dims inside the hero client box (same min-scale
  // math the Timeline runs against its background-image). Measured on img load
  // and whenever the hero resizes; box percentages then map 1:1 inside it.
  type Rect = { left: number; top: number; width: number; height: number };
  const ZERO_RECT: Rect = { left: 0, top: 0, width: 0, height: 0 };
  let heroEl = $state<HTMLElement | null>(null);
  let heroImgEl = $state<HTMLImageElement | null>(null);
  let renderedRect = $state<Rect>(ZERO_RECT);

  function measureOcrRect(): void {
    const img = heroImgEl;
    const hero = heroEl;
    const iw = img?.naturalWidth ?? 0;
    const ih = img?.naturalHeight ?? 0;
    const sw = hero?.clientWidth ?? 0;
    const sh = hero?.clientHeight ?? 0;
    if (!iw || !ih || !sw || !sh) {
      renderedRect = ZERO_RECT;
      return;
    }
    const scale = Math.min(sw / iw, sh / ih);
    const width = iw * scale;
    const height = ih * scale;
    renderedRect = { left: (sw - width) / 2, top: (sh - height) / 2, width, height };
  }

  $effect(() => {
    const hero = heroEl;
    if (!hero) return;
    measureOcrRect();
    if (typeof ResizeObserver === "undefined") {
      const onResize = () => measureOcrRect();
      window.addEventListener("resize", onResize);
      return () => window.removeEventListener("resize", onResize);
    }
    const ro = new ResizeObserver(() => measureOcrRect());
    ro.observe(hero);
    return () => ro.disconnect();
  });

  const ocrBoxStyleLocal = (obs: OcrObservation): string =>
    ocrBoxStyle(obs, renderedRect.height);

  // Drop the poll timer on destroy so no setTimeout dangles past unmount.
  $effect(() => clearOcrPoll);

  function openFrameUrl(): void {
    if (frameUrl) void openUrl(frameUrl);
  }

  function openInTimeline(): void {
    onClose();
    onOpenInTimeline?.();
  }

  // ---- Chrome: backdrop close + focus handoff ---------------------------
  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return;
    onClose();
  }

  let panelEl = $state<HTMLDivElement | null>(null);
  let opener: HTMLElement | null = null;
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      opener = document.activeElement as HTMLElement | null;
      panelEl?.focus();
    } else if (!open && wasOpen) {
      const trigger = opener;
      opener = null;
      void tick().then(() => trigger?.focus());
    }
    wasOpen = open;
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (!open) return;
    if (trapTabKey(e, panelEl)) return;
    if (e.key !== "Escape") return;
    onClose();
  }}
/>

{#if open}
  <div class="frame-modal" role="presentation" onpointerdown={onBackdropPointerDown}>
    <div
      bind:this={panelEl}
      class="frame-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-label="Captured frame"
      tabindex="-1"
    >
      <header class="frame-modal__header">
        <span class="frame-modal__eyebrow">frame</span>
        <h1 class="frame-modal__title" title={displayTitle ?? undefined}>
          {displayTitle ?? "Captured frame"}
        </h1>
        <button
          type="button"
          class="frame-modal__close"
          aria-label="Close frame"
          onclick={onClose}>×</button
        >
      </header>

      <div class="frame-modal__body">
        <figure class="frame-hero" bind:this={heroEl}>
          {#if imgSrc}
            <img
              bind:this={heroImgEl}
              src={imgSrc}
              alt={displayTitle ? `Captured frame: ${displayTitle}` : "Captured frame"}
              onload={measureOcrRect}
            />
          {:else if loadError}
            <p class="frame-hero__placeholder">Frame unavailable</p>
          {:else}
            <p class="frame-hero__placeholder frame-hero__placeholder--loading">Loading frame…</p>
          {/if}

          {#if preview?.hasSecretRedactions}
            <span class="frame-hero__redactions">
              {preview.secretRedactionCount}
              {preview.secretRedactionCount === 1 ? "redaction" : "redactions"} applied
            </span>
          {/if}

          {#if ocrVisible}
            <FrameOcrOverlay
              status={ocrStatus}
              error={ocrError}
              observations={ocrObservations}
              rect={renderedRect}
              boxStyle={ocrBoxStyleLocal}
            />
          {/if}
        </figure>
      </div>

      <div class="frame-meta">
        {#if displayApp}
          <span class="frame-meta__app">
            <span class="frame-meta__avatar" aria-hidden="true">
              {#if appIconSrc}
                <img src={appIconSrc} alt="" />
              {:else}
                <span class="frame-meta__avatar-fallback">{appFallback}</span>
              {/if}
            </span>
            <span class="frame-meta__app-name">{displayApp}</span>
          </span>
        {/if}
        {#if displayApp && displayTitle}
          <span class="frame-meta__sep">·</span>
        {/if}
        {#if displayTitle}
          <span class="frame-meta__title">{displayTitle}</span>
        {/if}
        {#if displayTime}
          <span class="frame-meta__sep">·</span>
          <time class="frame-meta__time">{displayTime}</time>
        {/if}
        {#if dims}
          <span class="frame-meta__sep">·</span>
          <span class="frame-meta__dims">{dims}</span>
        {/if}
      </div>

      <div class="frame-actions">
        <button
          type="button"
          class="frame-act"
          onclick={copyImage}
          disabled={!preview || copyState === "copying"}
        >
          {#if copyState === "copied"}
            copied
          {:else if copyState === "failed"}
            copy failed
          {:else if copyState === "copying"}
            copying…
          {:else}
            copy image
          {/if}
        </button>

        {#if frameUrl}
          <button type="button" class="frame-act frame-act--url" title={frameUrl} onclick={openFrameUrl}>
            <span>{frameUrl}</span>
          </button>
        {/if}

        <button
          type="button"
          class="frame-act frame-act--ocr"
          class:frame-act--ocr-active={ocrVisible}
          onclick={toggleOcr}
          disabled={!frame}
          aria-pressed={ocrVisible}
        >
          show ocr{#if ocrStatus === "success"}<span class="frame-act__count"> · {ocrObservations.length}</span>{/if}
        </button>

        <span class="frame-actions__spacer"></span>

        {#if onOpenInTimeline}
          <button type="button" class="frame-escape" onclick={openInTimeline}>
            show this frame in the timeline →
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .frame-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--app-overlay-bg);
    backdrop-filter: blur(10px);
  }
  .frame-modal__panel {
    width: min(1060px, 100%);
    max-height: min(calc(100vh - 100px), calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface);
    box-shadow: var(--app-shadow-popover);
    overflow: hidden;
  }

  .frame-modal__header {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 14px 16px 10px;
  }
  .frame-modal__eyebrow {
    flex: 0 0 auto;
    font-size: 10px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-accent);
  }
  .frame-modal__title {
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .frame-modal__close {
    flex: 0 0 auto;
    align-self: center;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    line-height: 1;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--app-text-subtle);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .frame-modal__close:hover,
  .frame-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border);
    color: var(--app-text-strong);
    outline: none;
  }
  .frame-modal__close:focus-visible {
    box-shadow: var(--app-ring);
  }

  .frame-modal__body {
    display: flex;
    flex: 1 1 auto;
    min-height: 0;
    margin: 0 16px;
  }
  .frame-hero {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    overflow: hidden;
    background: var(--app-bg);
  }
  .frame-hero img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .frame-hero__placeholder {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    min-height: 220px;
    margin: 0;
    font-size: 11px;
    color: var(--app-text-subtle);
  }
  .frame-hero__placeholder--loading {
    color: var(--app-text-muted);
  }
  .frame-hero__redactions {
    position: absolute;
    top: 8px;
    right: 8px;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 3px 7px;
    font-size: 10px;
    color: var(--app-warn);
    background: var(--app-overlay-bg);
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    backdrop-filter: blur(4px);
  }
  .frame-hero__redactions::before {
    content: "";
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-warn);
  }

  .frame-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px 4px;
    font-size: 11px;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
  }
  .frame-meta__app {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--app-text-strong);
  }
  .frame-meta__avatar {
    flex: 0 0 auto;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--app-surface-hover);
  }
  .frame-meta__avatar img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .frame-meta__avatar-fallback {
    font-size: 9px;
    font-weight: 600;
    line-height: 1;
    color: var(--app-text-muted);
  }
  .frame-meta__title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .frame-meta__sep {
    flex: 0 0 auto;
    color: var(--app-text-subtle);
  }
  .frame-meta__time {
    flex: 0 0 auto;
    color: var(--app-accent);
  }
  .frame-meta__dims {
    flex: 0 0 auto;
    font-size: 10px;
    color: var(--app-text-subtle);
  }

  .frame-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px 14px;
  }
  .frame-act {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-family: inherit;
    font-size: 11px;
    color: var(--app-text-muted);
    background: none;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 5px 10px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .frame-act:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
  }
  .frame-act:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .frame-act--url {
    max-width: 260px;
  }
  .frame-act--url span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* OCR toggle: pressed state borrows the accent tokens (mirrors the mockup's
     .act-ocr:checked). */
  .frame-act--ocr {
    user-select: none;
  }
  .frame-act__count {
    color: var(--app-text-subtle);
  }
  .frame-act--ocr-active,
  .frame-act--ocr-active:hover:not(:disabled) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }
  .frame-act--ocr-active .frame-act__count {
    color: var(--app-accent);
  }
  .frame-actions__spacer {
    flex: 1 1 auto;
  }
  .frame-escape {
    font-family: inherit;
    font-size: 10px;
    color: var(--app-text-subtle);
    background: none;
    border: none;
    padding: 5px 4px;
    border-radius: 4px;
    cursor: pointer;
  }
  .frame-escape:hover {
    color: var(--app-text-muted);
    text-decoration: underline;
  }

  @media (prefers-reduced-motion: reduce) {
    .frame-modal__close,
    .frame-act {
      transition: none;
    }
  }
</style>
