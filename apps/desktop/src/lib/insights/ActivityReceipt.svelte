<script lang="ts">
  // ActivityReceipt — bounded evidence playback for one Journal activity card.
  // It plays back the real captured frames over the card's time range as a
  // scrubbable "timelapse": no video is ever encoded — playback swaps frame
  // previews on requestAnimationFrame. Evidence ticks mark engine-cited frames
  // (the headline doubles as the poster), a wall-clock playhead reads WHEN each
  // frame happened, and "Open in Timeline →" hands the current frame off to the
  // raw Timeline surface. Deliberately never grows OCR copy/download, audio,
  // export, or navigation past the activity's span (grill Jul 3 / CONTEXT.md);
  // "Open in Timeline" is always the answer for anything more.

  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import Segmented from "$lib/components/Segmented.svelte";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { setPendingTimelineFocus } from "$lib/timeline/pending-focus";
  import {
    CATEGORY_COLOR,
    UNCATEGORIZED_COLOR,
    categoryLabel,
    humanizeMs,
  } from "$lib/insights/activity-helpers";
  import {
    clampIndex,
    countCaptureSegments,
    framesPerSecond,
    initialPosterIndex,
    SPEEDS,
    type Speed,
  } from "$lib/insights/receipt-playback";
  import { ReceiptFrameLoader } from "$lib/insights/receipt-frames";
  import type { Activity } from "$lib/types/recording";
  import type { FrameDto, FramePreviewDto, FrameSummaryDto } from "$lib/types/app-infra";

  interface Props {
    activity: Activity;
    onClose: () => void;
  }
  let { activity, onClose }: Props = $props();

  type StripFrame = { id: number; ms: number };

  // ── Reactive playback state ──────────────────────────────────────────
  let strip = $state<StripFrame[]>([]); // frames over the span, ascending
  let index = $state(0); // current frame index
  let playing = $state(false);
  let speed = $state<Speed>(8); // 8× per mockup default
  let loading = $state(true);
  let cacheBump = $state(0); // bumped when a preview lands (display dep)
  let currentMeta = $state<FrameDto | null>(null);
  let thumbUrls = $state<Record<number, string>>({}); // frameId → preview URL

  // ── Non-reactive machinery ───────────────────────────────────────────
  // All invoke-touching fetch work (preview prefetch pump, thumbnail queue,
  // frame meta) lives in the loader (receipt-frames.ts); it reports back into
  // the reactive state through these three callbacks.
  const loader = new ReceiptFrameLoader({
    onPreview: () => {
      cacheBump++;
    },
    onThumb: (fid, url) => {
      thumbUrls[fid] = url;
    },
    onMeta: (meta) => {
      currentMeta = meta;
    },
  });
  let loadGen = 0; // bumped per activity load; a stale strip fetch drops
  let rafId: number | null = null;
  let lastTs = 0;
  let frameAccum = 0;
  let trackEl = $state<HTMLDivElement | null>(null);
  let filmEl = $state<HTMLDivElement | null>(null);
  let scrubbing = false;
  let thumbObserver: IntersectionObserver | null = null;

  // ── Derived view model ───────────────────────────────────────────────
  const catColorVar = $derived(
    activity.category ? CATEGORY_COLOR[activity.category] : UNCATEGORIZED_COLOR,
  );
  const catLabel = $derived(
    activity.category ? categoryLabel(activity.category) : "Uncategorized",
  );
  const rangeLabel = $derived(
    `${clock(activity.startedAtMs)} – ${clock(activity.endedAtMs)} · ${humanizeMs(
      activity.endedAtMs - activity.startedAtMs,
    )}`,
  );

  // Frame ids are stable for the loaded strip — derive them once so the
  // per-tick pump effect below doesn't rebuild an O(strip) array on every
  // playhead move (index changes ~speed× per second during playback and on
  // every scrub pointermove; a long activity is thousands of frames).
  const stripIds = $derived(strip.map((f) => f.id));

  const currentFrameId = $derived(strip[index]?.id ?? null);
  const currentMs = $derived(strip[index]?.ms ?? null);
  const currentPos = $derived(currentMs == null ? 0 : posFor(currentMs));
  const currentPreview = $derived.by<FramePreviewDto | null>(() => {
    cacheBump; // recompute when a preview lands
    const id = currentFrameId;
    return id == null ? null : loader.peekPreview(id);
  });
  const currentUrl = $derived(
    currentPreview ? framePreviewAssetUrl(currentPreview.filePath) : null,
  );

  const metaApp = $derived(currentMeta?.appName ?? null);
  const metaTitle = $derived(currentMeta?.windowTitle ?? null);
  const hasOcr = $derived((currentMeta?.ocrText ?? "").trim().length > 0);

  // Frame-subject evidence refs are the cited frames; isHeadline is the poster.
  const frameEvidence = $derived(
    activity.evidence.filter((e) => e.subjectType === "frame"),
  );
  const headlineFrameId = $derived(
    frameEvidence.find((e) => e.isHeadline)?.subjectId ?? null,
  );
  const citedCount = $derived(frameEvidence.length);
  const ticks = $derived.by(() => {
    const out: { pos: number; headline: boolean }[] = [];
    for (const e of frameEvidence) {
      const sf = strip.find((f) => f.id === e.subjectId);
      const ms = sf?.ms ?? e.capturedAtMs ?? null;
      if (ms == null) continue;
      out.push({ pos: posFor(ms), headline: e.isHeadline });
    }
    return out;
  });

  const citedIds = $derived(new Set(frameEvidence.map((e) => e.subjectId)));
  const segmentCount = $derived(countCaptureSegments(strip.map((f) => f.ms)));

  // ── Wall-clock formatters (evidence answers WHEN, not elapsed) ────────
  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    });
  }
  function clockSec(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
      hour12: true,
    });
  }
  function posFor(ms: number): number {
    const span = activity.endedAtMs - activity.startedAtMs;
    if (span <= 0) return 0;
    return Math.min(1, Math.max(0, (ms - activity.startedAtMs) / span));
  }

  // ── Load the frame strip over the activity span ──────────────────────
  async function loadStrip(): Promise<void> {
    pause();
    const gen = ++loadGen;
    loading = true;
    strip = [];
    currentMeta = null;
    thumbUrls = {};
    index = 0;
    loader.reset();
    try {
      const summaries = await invoke<FrameSummaryDto[]>(
        "list_frame_summaries_in_range",
        {
          request: {
            capturedAtStart: new Date(activity.startedAtMs).toISOString(),
            capturedAtEnd: new Date(activity.endedAtMs).toISOString(),
          },
        },
      );
      if (gen !== loadGen) return;
      // Date.parse of the RFC3339 capturedAt → epoch ms (matches journal-day.ts).
      const sorted = summaries
        .map((s) => ({ id: s.id, ms: Date.parse(s.capturedAt) }))
        .filter((f) => Number.isFinite(f.ms))
        .sort((a, b) => a.ms - b.ms);
      strip = sorted;
      index = initialPosterIndex(
        sorted.map((f) => f.id),
        headlineFrameId,
      );
      cacheBump++;
    } catch {
      // 0 frames (retention) and a load failure both render the expired panel.
      if (gen === loadGen) strip = [];
    } finally {
      if (gen === loadGen) loading = false;
    }
  }

  // Filmstrip thumbnails: every frame gets a cell; an IntersectionObserver
  // queues a cell's preview as it scrolls into view and the loader's bounded
  // pump does the fetching.
  // ponytail: no cell virtualization — a multi-hour activity renders one
  // <button> per frame; virtualize the strip if that ever gets janky.
  function thumbCell(node: HTMLElement, fid: number) {
    node.dataset.fid = String(fid);
    thumbObserver ??= new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        thumbObserver?.unobserve(entry.target);
        loader.requestThumb(Number((entry.target as HTMLElement).dataset.fid));
      }
    });
    thumbObserver.observe(node);
    return {
      destroy() {
        thumbObserver?.unobserve(node);
      },
    };
  }

  // ── Playback loop (frame-swap timelapse) ─────────────────────────────
  function tick(ts: number): void {
    if (!playing) {
      rafId = null;
      return;
    }
    if (lastTs === 0) lastTs = ts;
    const dt = (ts - lastTs) / 1000;
    lastTs = ts;
    frameAccum += dt * framesPerSecond(speed);
    const advance = Math.floor(frameAccum);
    if (advance > 0) {
      frameAccum -= advance;
      const next = index + advance;
      if (next >= strip.length - 1) {
        index = strip.length - 1;
        pause(); // stop at the end
        return;
      }
      index = next;
    }
    rafId = requestAnimationFrame(tick);
  }

  function play(): void {
    if (strip.length === 0) return;
    if (index >= strip.length - 1) index = 0; // replay from the top
    playing = true;
    lastTs = 0;
    frameAccum = 0;
    rafId = requestAnimationFrame(tick);
  }

  function pause(): void {
    playing = false;
    if (rafId != null) cancelAnimationFrame(rafId);
    rafId = null;
  }

  function togglePlay(): void {
    playing ? pause() : play();
  }

  function seek(i: number): void {
    pause();
    index = clampIndex(i, strip.length);
  }

  function step(delta: number): void {
    pause();
    index = clampIndex(index + delta, strip.length);
  }

  function onSpeedChange(v: string): void {
    speed = Number(v) as Speed;
  }
  const speedOptions = SPEEDS.map((s) => ({ value: String(s), label: `${s}×` }));

  // ── Scrubbing ────────────────────────────────────────────────────────
  function scrubToClientX(clientX: number): void {
    const el = trackEl;
    if (!el || strip.length === 0) return;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    index = Math.round(frac * (strip.length - 1));
  }
  function onTrackPointerDown(e: PointerEvent): void {
    pause();
    scrubbing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubToClientX(e.clientX);
  }
  function onTrackPointerMove(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubToClientX(e.clientX);
  }
  function onTrackPointerUp(e: PointerEvent): void {
    scrubbing = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* pointer already released */
    }
  }

  // ── Open in Timeline handoff (frontend-only, no backend command) ─────
  function openInTimeline(): void {
    const id = currentFrameId;
    if (id == null) return;
    setPendingTimelineFocus(id);
    void goto("/");
    onClose();
  }

  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return; // only the backdrop itself closes
    onClose();
  }

  // ── Effects ──────────────────────────────────────────────────────────
  // Reload the strip when the activity changes (also runs on mount).
  $effect(() => {
    activity.id;
    void loadStrip();
  });

  // Re-pump the preview lookahead whenever the strip loads or the playhead moves.
  // `stripIds` only recomputes when the strip changes, so a playback tick / scrub
  // move here is O(lookahead), not O(strip).
  $effect(() => {
    loader.pump(stripIds, index);
  });

  // Load display metadata for the current frame.
  $effect(() => {
    const id = currentFrameId;
    if (id != null) void loader.loadMeta(id);
  });

  // Window capture-phase keyboard — WKWebView doesn't focus <button> on click,
  // so element focus is unreliable; a window listener is the seam. Lives only
  // while the receipt is mounted (Slice 3 renders it conditionally).
  $effect(() => {
    function onKey(e: KeyboardEvent): void {
      // Let the speed Segmented keep its own arrow-key nav when it's focused;
      // otherwise arrows step frames. stopPropagation isolates handled keys from
      // the Insights page underneath while the modal is open.
      const inRadioGroup = !!(e.target as HTMLElement | null)?.closest?.(
        '[role="radiogroup"]',
      );
      switch (e.key) {
        case "Escape":
          e.preventDefault();
          e.stopPropagation();
          onClose();
          break;
        case "ArrowLeft":
          if (inRadioGroup) return;
          e.preventDefault();
          e.stopPropagation();
          step(-1);
          break;
        case "ArrowRight":
          if (inRadioGroup) return;
          e.preventDefault();
          e.stopPropagation();
          step(1);
          break;
        case " ":
        case "Spacebar":
          e.preventDefault();
          e.stopPropagation();
          togglePlay();
          break;
      }
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  });

  // Keep the current frame's cell in view as playback/scrubbing advances.
  $effect(() => {
    const cell = filmEl?.children[index] as HTMLElement | undefined;
    cell?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  // Cancel any dangling rAF and the thumb observer on unmount.
  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    thumbObserver?.disconnect();
  });
</script>

<div class="receipt" role="presentation" onpointerdown={onBackdropPointerDown}>
  <div
    class="modal-card"
    role="dialog"
    aria-modal="true"
    aria-label={`Activity receipt: ${activity.title}`}
  >
    <div class="m-head">
      <span class="chip">
        <span class="sw" style="background:var({catColorVar})"></span>{catLabel}
      </span>
      <h2 class="m-title" title={activity.title}>{activity.title}</h2>
      <span class="when">{rangeLabel}</span>
      <button type="button" class="m-close" aria-label="Close receipt" onclick={onClose}>✕</button>
    </div>

    <!-- The compact journal rows show no summary, so the receipt is where the
         description lives; it also survives footage expiry (ADR 0029). -->
    {#if activity.summary}
      <p class="m-summary">{activity.summary}</p>
    {/if}

    {#if loading}
      <div class="viewer">
        <div class="skeleton" aria-hidden="true"></div>
      </div>
      <div class="m-foot"><span>Loading footage…</span></div>
    {:else if strip.length === 0}
      <!-- Retention removes frames while the card is kept (ADR 0029), so this
           expired state is guaranteed, not an edge case. -->
      <div class="viewer viewer--expired">
        <div class="exp">
          <div class="exp__glyph">▸ ▸ ▸</div>
          <h4>Footage expired</h4>
          <p>
            The raw frames behind this card were removed by Retention Cleanup. The
            card, its summary, and its evidence list are kept — only the pixels age
            out.
          </p>
        </div>
      </div>
      <div class="m-foot">
        <span>0 frames still on disk</span><span class="sep">·</span>
        <span>summary retained</span>
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
          {#if metaApp}
            <span class="frame-meta__chip frame-meta__chip--app">{metaApp}</span>
          {/if}
          {#if metaTitle}
            <span class="frame-meta__chip">{metaTitle}</span>
          {/if}
          {#if currentMs != null}
            <span class="frame-meta__chip">{clockSec(currentMs)}</span>
          {/if}
          {#if hasOcr}
            <span class="frame-meta__chip">OCR ✓</span>
          {/if}
        </div>
      </div>

      <div class="scrub">
        <div
          class="track"
          bind:this={trackEl}
          role="slider"
          aria-label="Scrub frames"
          aria-valuemin={1}
          aria-valuemax={strip.length}
          aria-valuenow={index + 1}
          tabindex="-1"
          onpointerdown={onTrackPointerDown}
          onpointermove={onTrackPointerMove}
          onpointerup={onTrackPointerUp}
        >
          {#each ticks as t, i (i)}
            <span class="ev" class:ev--hl={t.headline} style="left:{t.pos * 100}%"></span>
          {/each}
          <div class="fill" style="width:{currentPos * 100}%"></div>
          <div class="head" style="left:{currentPos * 100}%">
            {currentMs != null ? clock(currentMs) : ""}
          </div>
        </div>
        <div class="scrub-caps">
          <span>{clock(activity.startedAtMs)}</span><span>{clock(activity.endedAtMs)}</span>
        </div>
      </div>

      <div class="film" bind:this={filmEl}>
        {#each strip as f, ti (f.id)}
          <button
            type="button"
            class="film__cell"
            class:cur={ti === index}
            class:cited={citedIds.has(f.id)}
            aria-label={`Seek to ${clock(f.ms)}`}
            use:thumbCell={f.id}
            onclick={() => seek(ti)}
          >
            {#if thumbUrls[f.id]}
              <img class="film__img" src={thumbUrls[f.id]} alt="" />
            {/if}
          </button>
        {/each}
      </div>

      <div class="controls">
        <button
          type="button"
          class="play"
          aria-label={playing ? "Pause" : "Play"}
          onclick={togglePlay}>{playing ? "⏸" : "▶"}</button
        >
        <Segmented
          options={speedOptions}
          value={String(speed)}
          onValueChange={onSpeedChange}
          ariaLabel="Playback speed"
          compact
        />
        <span class="counter">
          <span class="counter__now">frame {index + 1}</span> / {strip.length}{#if currentMs != null}
            · {clockSec(currentMs)}{/if}
        </span>
        <span class="ctl-spacer"></span>
        <button type="button" class="open-tl" onclick={openInTimeline}>Open in Timeline →</button>
      </div>

      <div class="m-foot">
        <span>
          {strip.length}
          {strip.length === 1 ? "frame" : "frames"} across {segmentCount} capture
          {segmentCount === 1 ? "segment" : "segments"}
        </span>
        <span class="sep">·</span>
        <span>{citedCount} cited as evidence</span>
      </div>
    {/if}
  </div>
</div>

<style>
  /* Dense declarations to keep this component under the 800-line ceiling;
     tokens + structure mirror docs/mockups/dayflow/04-timelapse.html. */
  .receipt {
    position: fixed; inset: 0; z-index: 2000;
    display: grid; place-items: center; padding: 16px;
    background: var(--app-overlay-bg); backdrop-filter: blur(4px);
  }
  .modal-card {
    width: 82vw; height: 90vh; display: flex; flex-direction: column;
    overflow: hidden; background: var(--app-surface);
    border: 1px solid var(--app-border-strong); border-radius: 12px;
    box-shadow: var(--app-shadow-popover);
  }

  /* Header */
  .m-head {
    display: flex; align-items: center; gap: 10px;
    padding: 13px 16px; border-bottom: 1px solid var(--app-border);
  }
  .chip {
    flex: 0 0 auto; display: inline-flex; align-items: center; gap: 6px;
    font-size: 10px; letter-spacing: 0.06em; text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .sw { flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; }
  .m-title {
    flex: 1 1 auto; min-width: 0; margin: 0;
    font-size: 14px; font-weight: 600; color: var(--app-text-strong);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .when {
    flex: 0 0 auto; font-size: 11px; color: var(--app-text-subtle);
    font-variant-numeric: tabular-nums; white-space: nowrap;
  }
  .m-close {
    flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center;
    width: 24px; height: 24px; font: inherit; cursor: pointer;
    color: var(--app-text-subtle); background: transparent;
    border: 1px solid var(--app-border); border-radius: 5px;
  }
  .m-close:hover { color: var(--app-text-strong); border-color: var(--app-border-hover); }
  .m-summary {
    flex: 0 0 auto; margin: 0; padding: 10px 16px;
    border-bottom: 1px solid var(--app-border);
    font-size: 12px; line-height: 1.65; color: var(--app-text-muted);
  }

  /* Viewer — no transition on the img: instant frame swaps are the video feel. */
  .viewer {
    position: relative; flex: 1 1 auto; min-height: 0; overflow: hidden;
    background: var(--app-bg); border-bottom: 1px solid var(--app-border);
  }
  .viewer__img { display: block; width: 100%; height: 100%; object-fit: contain; }
  .skeleton {
    position: absolute; inset: 18px 22px;
    background: linear-gradient(160deg, var(--app-surface-raised), var(--app-bg) 70%);
    border: 1px solid var(--app-border); border-radius: 8px;
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse { 0%, 100% { opacity: 0.55; } 50% { opacity: 0.85; } }
  .viewer__redactions {
    position: absolute; top: 8px; right: 8px; padding: 3px 7px;
    font-size: 10px; color: var(--app-text-muted); background: var(--app-overlay-bg);
    border: 1px solid var(--app-border-strong); border-radius: 5px; backdrop-filter: blur(4px);
  }
  .frame-meta {
    position: absolute; left: 16px; bottom: 12px;
    display: flex; gap: 8px; max-width: calc(100% - 32px); overflow: hidden;
  }
  .frame-meta__chip {
    padding: 2px 8px; font-size: 10px; color: var(--app-text-muted);
    background: var(--app-overlay-bg); border: 1px solid var(--app-border-strong);
    border-radius: 4px; white-space: nowrap; overflow: hidden;
    text-overflow: ellipsis; backdrop-filter: blur(4px);
  }
  .frame-meta__chip--app { flex: 0 0 auto; color: var(--app-text); }

  /* Expired */
  .viewer--expired { aspect-ratio: 16 / 6; display: flex; align-items: center; justify-content: center; }
  .exp { max-width: 440px; padding: 24px; text-align: center; }
  .exp__glyph { margin-bottom: 10px; font-size: 16px; letter-spacing: 0.3em; color: var(--app-text-faint); }
  .exp h4 { margin: 0 0 6px; font-size: 13px; font-weight: 600; color: var(--app-text-strong); }
  .exp p { margin: 0; font-size: 11.5px; line-height: 1.7; color: var(--app-text-muted); }

  /* Scrubber */
  .scrub { padding: 14px 16px 6px; }
  .track {
    position: relative; height: 6px; border-radius: 3px;
    background: var(--app-surface-hover); cursor: pointer; touch-action: none;
  }
  .fill {
    position: absolute; left: 0; top: 0; bottom: 0; border-radius: 3px;
    background: var(--app-accent-strong); pointer-events: none;
  }
  .head {
    position: absolute; top: 50%; transform: translate(-50%, -50%);
    padding: 2px 8px; font-size: 9px; font-weight: 700; font-variant-numeric: tabular-nums;
    color: var(--app-bg); background: var(--app-accent); border-radius: 999px;
    white-space: nowrap; pointer-events: none;
  }
  .ev {
    position: absolute; top: -4px; width: 2px; height: 14px;
    background: var(--app-accent); border-radius: 1px; opacity: 0.5; pointer-events: none;
  }
  .ev--hl { opacity: 1; box-shadow: 0 0 6px var(--app-accent); }
  .scrub-caps {
    display: flex; justify-content: space-between; margin-top: 6px;
    font-size: 10px; font-variant-numeric: tabular-nums; color: var(--app-text-faint);
  }

  /* Filmstrip */
  .film {
    /* A scroll container's automatic min-height is 0, so the modal's flex
       column would crush it — pin it to its natural (cell aspect) height. */
    flex: 0 0 auto;
    display: grid; grid-auto-flow: column; gap: 5px; padding: 8px 16px 8px;
    grid-auto-columns: calc((100% - 55px) / 12); /* 12 cells in view, rest scroll (55px = 11 gaps) */
    overflow-x: auto; overflow-y: hidden;
  }
  .film__cell {
    position: relative; aspect-ratio: 16 / 10; padding: 0; cursor: pointer;
    background: linear-gradient(160deg, var(--app-surface-raised), var(--app-bg) 70%);
    border: 1px solid var(--app-border); border-radius: 4px; overflow: hidden;
  }
  .film__cell::after {
    content: ""; position: absolute; inset: 25% 18% 30%;
    background: var(--app-surface-hover); border-radius: 2px;
  }
  .film__img {
    position: absolute; inset: 0; z-index: 1; /* above the ::after placeholder */
    width: 100%; height: 100%; object-fit: cover;
  }
  .film__cell.cur { border-color: var(--app-accent); box-shadow: 0 0 0 1px var(--app-accent); }
  .film__cell.cited::before {
    content: ""; position: absolute; top: 3px; right: 3px; z-index: 2;
    width: 5px; height: 5px; background: var(--app-accent); border-radius: 50%;
  }

  /* Controls */
  .controls { display: flex; align-items: center; gap: 10px; padding: 12px 16px 14px; }
  .play {
    display: inline-flex; align-items: center; justify-content: center;
    width: 32px; height: 32px; font-size: 13px; cursor: pointer;
    color: var(--app-accent); background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border); border-radius: 7px;
  }
  .play:hover, .open-tl:hover { border-color: var(--app-accent); }
  .counter { font-size: 11px; font-variant-numeric: tabular-nums; color: var(--app-text-muted); }
  .counter__now { color: var(--app-text-strong); }
  .ctl-spacer { flex: 1 1 auto; }
  .open-tl {
    display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px;
    font: inherit; font-size: 11px; cursor: pointer;
    color: var(--app-accent); background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border); border-radius: 6px;
  }

  /* Footer */
  .m-foot {
    display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
    padding: 10px 16px; font-size: 10.5px; color: var(--app-text-subtle);
    border-top: 1px dashed var(--app-border);
  }
  .m-foot .sep { color: var(--app-text-faint); }

  @media (prefers-reduced-motion: reduce) { .skeleton { animation: none; } }
</style>
