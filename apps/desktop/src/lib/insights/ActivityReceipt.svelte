<script lang="ts">
  // ActivityReceipt — bounded evidence playback for one Journal activity card.
  // It plays back the real captured frames over the card's time range as a
  // scrubbable "timelapse": no video is ever encoded — playback swaps frame
  // previews on requestAnimationFrame. Evidence ticks mark engine-cited frames
  // (the headline doubles as the poster), a wall-clock playhead reads WHEN each
  // frame happened, and "Open in Timeline →" hands the current frame off to the
  // raw Timeline surface.
  //
  // Per ADR 0049 it ALSO plays cited *audio*: a lavender tick per cited spoken
  // segment plays that segment's real audio at 1× while the frame viewer runs
  // the same window, clocked by the <audio> element (bounded → the segment IS
  // the bound). Audio-only Activities become a plain bounded audio player, never
  // a false "footage expired". It DISPLAYS read-only speaker attribution, late-
  // bound by id (resolved live from person profiles, never frozen). The boundary
  // is now bounded→unbounded: Timeline still owns unbounded audio scrub, OCR
  // copy/download, export, cross-Activity nav, and speaker naming/merging.
  // "Open in Timeline" remains the answer for anything wider than this Activity.

  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import Segmented from "$lib/components/Segmented.svelte";
  import { tip } from "$lib/components/tooltip";
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
  import {
    type AudioCitation,
    audioCurrentView,
    audioFooterLeft,
    audioSpeakerSummary,
    audioTickViews,
    clipRateLabelOf,
    frameIndexForMs,
    partitionEvidence,
    receiptViewState,
  } from "$lib/insights/receipt-audio";
  import { ReceiptAudioLoader } from "$lib/insights/receipt-audio-loader";
  import type { Activity } from "$lib/types/recording";
  import type {
    FrameDto,
    FramePreviewDto,
    FrameSummaryDto,
    PersonProfileDto,
  } from "$lib/types/app-infra";

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

  // ── Cited-audio state (ADR 0049) ─────────────────────────────────────
  let audioCitations = $state<AudioCitation[]>([]); // hydrated, sorted by start
  let profiles = $state<PersonProfileDto[]>([]); // for live name resolution
  let clipMode = $state(false); // a bounded 1× clip is running (audio clocks)
  let clipPlaying = $state(false); // the <audio> element's play/pause state
  let clipStartMs = $state(0); // active clip's segment start (wall-clock)
  let activeClipId = $state<number | null>(null);
  let audioEl = $state<HTMLAudioElement | null>(null);
  let clipToken = 0; // guards the async media fetch; a new clip/activity drops it

  // ── Non-reactive machinery ───────────────────────────────────────────
  // All invoke-touching fetch work (preview prefetch pump, thumbnail queue,
  // frame meta) lives in the loader (receipt-frames.ts); it reports back into
  // the reactive state through these three callbacks.
  const loader = new ReceiptFrameLoader({
    onPreview: () => cacheBump++,
    onThumb: (fid, url) => (thumbUrls[fid] = url),
    onMeta: (meta) => (currentMeta = meta),
  });
  // Cited-audio hydration (profiles + per-segment source/turns) + clip media.
  const audioLoader = new ReceiptAudioLoader({
    onProfiles: (p) => (profiles = p),
    onCitations: (c) => (audioCitations = c),
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
  const catColorVar = $derived(activity.category ? CATEGORY_COLOR[activity.category] : UNCATEGORIZED_COLOR);
  const catLabel = $derived(activity.category ? categoryLabel(activity.category) : "Uncategorized");
  const rangeLabel = $derived(
    `${clock(activity.startedAtMs)} – ${clock(activity.endedAtMs)} · ${humanizeMs(activity.endedAtMs - activity.startedAtMs)}`,
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
  const currentUrl = $derived(currentPreview ? framePreviewAssetUrl(currentPreview.filePath) : null);

  const metaApp = $derived(currentMeta?.appName ?? null);
  const metaTitle = $derived(currentMeta?.windowTitle ?? null);
  const hasOcr = $derived((currentMeta?.ocrText ?? "").trim().length > 0);

  // Frame-subject evidence refs are the cited frames; isHeadline is the poster.
  // Audio-subject refs are the cited spoken segments (ADR 0049).
  const evidenceSplit = $derived(partitionEvidence(activity.evidence));
  const frameEvidence = $derived(evidenceSplit.frames);
  const audioEvidence = $derived(evidenceSplit.audio);

  // Which viewer to render: frames win; else audio if any spoken evidence
  // survives; else the honest expired panel.
  const viewState = $derived(receiptViewState(strip.length, audioEvidence.length));

  // Audio view models (assembled by pure helpers so this block stays one-liners).
  const stripMs = $derived(strip.map((f) => f.ms));
  const audioTicks = $derived(audioTickViews(audioCitations, profiles, activity.startedAtMs, activity.endedAtMs));
  const audioCurrent = $derived(audioCurrentView(audioCitations, profiles, activeClipId));
  const clipRateLabel = $derived(clipRateLabelOf(audioCurrent.citation, clockSec));
  const isAudioOnly = $derived(viewState === "audio-only");
  const audible = $derived(isAudioOnly || clipMode); // the <audio> owns the playhead
  const playIcon = $derived(audible ? (clipPlaying ? "⏸" : "▶") : playing ? "⏸" : "▶");
  const audioHeadMs = $derived(audioCurrent.citation?.capturedAtMs ?? null);
  // The wall-clock playhead: audio segment start when audio-only, else the frame.
  const headPos = $derived(isAudioOnly ? (audioHeadMs == null ? 0 : posFor(audioHeadMs)) : currentPos);
  const headClock = $derived(isAudioOnly ? (audioHeadMs == null ? "" : clock(audioHeadMs)) : currentMs == null ? "" : clock(currentMs));
  const headlineFrameId = $derived(frameEvidence.find((e) => e.isHeadline)?.subjectId ?? null);
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
    return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", hour12: true });
  }
  function clockSec(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit", hour12: true });
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
      const summaries = await invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
        request: {
          capturedAtStart: new Date(activity.startedAtMs).toISOString(),
          capturedAtEnd: new Date(activity.endedAtMs).toISOString(),
        },
      });
      if (gen !== loadGen) return;
      // Date.parse of the RFC3339 capturedAt → epoch ms (matches journal-day.ts).
      const sorted = summaries
        .map((s) => ({ id: s.id, ms: Date.parse(s.capturedAt) }))
        .filter((f) => Number.isFinite(f.ms))
        .sort((a, b) => a.ms - b.ms);
      strip = sorted;
      index = initialPosterIndex(sorted.map((f) => f.id), headlineFrameId);
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
    return { destroy: () => thumbObserver?.unobserve(node) };
  }

  // ── Playback loop (frame-swap timelapse) ─────────────────────────────
  function tick(ts: number): void {
    if (!playing || clipMode) {
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
    if (clipMode || strip.length === 0) return;
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

  // Play/Pause routes to whichever clock owns the surface: the <audio> element
  // while a clip runs, the frame timelapse while frames play, or (audio-only)
  // starting the current segment's clip.
  function togglePlay(): void {
    if (clipMode) {
      if (!audioEl) return;
      audioEl.paused ? void audioEl.play().catch(() => {}) : audioEl.pause();
      return;
    }
    if (strip.length > 0) {
      playing ? pause() : play();
      return;
    }
    const id = audioCurrent.citation?.audioSegmentId;
    if (id != null) void playClip(id);
  }

  function seek(i: number): void {
    if (clipMode) stopClip();
    pause();
    index = clampIndex(i, strip.length);
  }

  function step(delta: number): void {
    if (clipMode) stopClip();
    pause();
    index = clampIndex(index + delta, strip.length);
  }

  function onSpeedChange(v: string): void {
    if (clipMode) return; // speed is inert during a 1× clip
    speed = Number(v) as Speed;
  }

  // ── Bounded, synchronized audio+screen clip (ADR 0049) ───────────────
  // Clicking a cited audio tick plays that segment's real audio at 1×; on each
  // timeupdate the frame viewer jumps to the strip frame at/just-before the
  // audio's wall-clock position, so one playhead drives both.
  async function playClip(id: number): Promise<void> {
    const c = audioCitations.find((x) => x.audioSegmentId === id);
    if (!c || c.capturedAtMs == null || !audioEl) return;
    pause(); // stop the rAF timelapse; the audio clocks from here
    const token = ++clipToken;
    const src = await audioLoader.fetchMediaSrc(id);
    if (token !== clipToken || !src || !audioEl) return; // superseded / failed
    clipStartMs = c.capturedAtMs;
    activeClipId = id;
    clipMode = true;
    audioEl.src = src;
    void audioEl.play().catch(() => {
      clipMode = false;
    });
  }

  function onAudioTimeUpdate(): void {
    if (!clipMode || !audioEl) return;
    const targetMs = clipStartMs + audioEl.currentTime * 1000;
    if (stripMs.length > 0) index = frameIndexForMs(stripMs, targetMs);
  }

  // Clip finished: drop out of clip mode but leave the viewer on the last frame
  // / the segment just heard (keep activeClipId so its caption stays shown).
  function onAudioEnded(): void {
    clipMode = false;
    clipPlaying = false;
  }

  // Stop any running clip and drop an in-flight media fetch (new activity or a
  // manual frame move preempts the audio clock).
  function stopClip(): void {
    clipToken++;
    audioEl?.pause();
    clipMode = false;
    clipPlaying = false;
    activeClipId = null;
  }

  // Hydrate the cited spoken segments for the current activity.
  function loadAudio(): void {
    stopClip();
    audioCitations = [];
    void audioLoader.load(audioEvidence);
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
    if (clipMode) stopClip();
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
    } catch { /* pointer already released */ }
  }

  // ── Open in Timeline handoff (frontend-only, no backend command) ─────
  // Hand off the current frame; for an audio-only receipt (no frame) hand off
  // the current/headline audio segment instead.
  function openInTimeline(): void {
    const audioSegmentId = audioCurrent.citation?.audioSegmentId;
    if (currentFrameId != null) setPendingTimelineFocus({ frameId: currentFrameId });
    else if (audioSegmentId != null) setPendingTimelineFocus({ audioSegmentId });
    else return;
    void goto("/");
    onClose();
  }

  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return; // only the backdrop itself closes
    onClose();
  }

  // ── Effects ──────────────────────────────────────────────────────────
  // Reload the strip AND re-hydrate cited audio when the activity changes
  // (also runs on mount).
  $effect(() => {
    activity.id;
    void loadStrip();
    loadAudio();
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
      const inRadioGroup = !!(e.target as HTMLElement | null)?.closest?.('[role="radiogroup"]');
      const arrow = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); }
      else if (arrow !== 0 && !inRadioGroup) { e.preventDefault(); e.stopPropagation(); step(arrow); }
      else if (e.key === " " || e.key === "Spacebar") { e.preventDefault(); e.stopPropagation(); togglePlay(); }
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  });

  // Keep the current frame's cell in view as playback/scrubbing advances.
  $effect(() => {
    const cell = filmEl?.children[index] as HTMLElement | undefined;
    cell?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  // Cancel any dangling rAF, stop the clip audio, and drop the thumb observer.
  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    audioEl?.pause();
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
      <h2 class="m-title" use:tip={activity.title}>{activity.title}</h2>
      <span class="when">{rangeLabel}</span>
      <button type="button" class="m-close" aria-label="Close receipt" onclick={onClose}>✕</button>
    </div>

    <!-- The compact journal rows show no summary, so the receipt is where the
         description lives; it also survives footage expiry (ADR 0029). -->
    {#if activity.summary}
      <p class="m-summary">{activity.summary}</p>
    {/if}

    <!-- One hidden <audio> clocks every bounded clip (ADR 0049); JS-driven, so
         visibility is irrelevant. Present in both frames and audio-only states. -->
    <audio
      bind:this={audioEl}
      onplay={() => (clipPlaying = true)}
      onpause={() => (clipPlaying = false)}
      ontimeupdate={onAudioTimeUpdate}
      onended={onAudioEnded}
      style="display:none"
    ></audio>

    <!-- Lavender ticks below the spine: one per cited spoken segment, click →
         bounded 1× clip. Hovering reveals the mockup's .tip-a (ADR 0049):
         attribution label + cited transcript + play hint — CSS-shown, not a
         plain-text `title`, so it matches the receipt's audio channel. -->
    {#snippet audioTickRow()}
      {#each audioTicks as t (t.id)}
        <button
          type="button"
          class="ev-a"
          class:ev-a--hl={t.headline}
          style="left:{t.pos * 100}%"
          aria-label={`Play spoken moment — ${t.speaker}, screen + audio, 1×`}
          onpointerdown={(e) => e.stopPropagation()}
          onclick={() => playClip(t.id)}
        ></button>
        <!-- Clamp the centered tooltip to [half-width, track − half-width] so an
             edge tick doesn't push it past the modal's overflow-clip. -->
        <div
          class="tip-a"
          style="left: clamp(180px, {t.pos * 100}%, calc(100% - 180px))"
          aria-hidden="true"
        >
          <div class="lbl">◆ {t.label}</div>
          {#if t.caption}<div class="snip">“{t.caption}”</div>{/if}
          <div class="go">▶ Play this moment — screen + audio, 1×</div>
        </div>
      {/each}
    {/snippet}

    {#if loading}
      <div class="viewer"><div class="skeleton" aria-hidden="true"></div></div>
      <div class="m-foot"><span>Loading footage…</span></div>
    {:else if viewState === "expired"}
      <!-- Retention removes frames while the card is kept (ADR 0029) AND nothing
           spoken was cited, so this expired state is honest, not an edge case. -->
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
      <!-- One surface, two viewers (ADR 0049): the frame timelapse, or a bounded
           audio player when no frames survive. Scrub/controls/footer are shared. -->
      {#if isAudioOnly}
        <div class="viewer viewer--audio">
          <button
            type="button"
            class="big-play"
            aria-label={clipPlaying ? "Pause spoken evidence" : "Play spoken evidence"}
            disabled={audioCurrent.citation == null}
            onclick={togglePlay}>{clipPlaying ? "⏸" : "▶"}</button
          >
          {#if audioCurrent.citation}
            <div class="a-spk">
              {#if audioCurrent.source && audioCurrent.source !== audioCurrent.name}{audioCurrent.source} · {/if}<b
                >{audioCurrent.name}</b
              ><span class="rb">{audioCurrent.readable} · name resolved live by id</span>
            </div>
            {#if audioCurrent.citation.caption}<div class="a-cap">{audioCurrent.citation.caption}</div>{/if}
            <div class="a-when">
              segment {audioCurrent.ordinal} of {audioCurrent.total}{#if audioCurrent.citation.startMs != null} · {clock(
                  audioCurrent.citation.startMs,
                )}{#if audioCurrent.citation.endMs != null}–{clock(audioCurrent.citation.endMs)}{/if}{/if} · captured
              as audio
            </div>
          {:else}
            <div class="a-when">Loading spoken evidence…</div>
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
            {#if hasOcr}<span class="frame-meta__chip">OCR ✓</span>{/if}
          </div>
          {#if clipMode && audioCurrent.citation}
            <!-- "reliving a cited moment": one playhead, real audio at 1× -->
            <div class="clip-bar">
              <span class="live"><span class="d"></span>{audioCurrent.source}{audioCurrent.readable
                  ? ` · ${audioCurrent.readable}`
                  : ""}</span
              >
              <span class="cap">{audioCurrent.citation.caption}</span>
              <span class="rate">{clipRateLabel}</span>
            </div>
          {/if}
        </div>
      {/if}

      <div class="scrub">
        <div
          class="track"
          bind:this={trackEl}
          role="slider"
          aria-label="Scrub"
          aria-valuemin={1}
          aria-valuemax={Math.max(1, strip.length)}
          aria-valuenow={index + 1}
          tabindex="-1"
          onpointerdown={onTrackPointerDown}
          onpointermove={onTrackPointerMove}
          onpointerup={onTrackPointerUp}
        >
          {#if !isAudioOnly}
            {#each ticks as t, i (i)}
              <span class="ev" class:ev--hl={t.headline} style="left:{t.pos * 100}%"></span>
            {/each}
            <div class="fill" class:fill--audio={clipMode} style="width:{currentPos * 100}%"></div>
          {/if}
          {@render audioTickRow()}
          <div class="head" class:head--audio={audible} style="left:{headPos * 100}%">{headClock}</div>
        </div>
        <div class="scrub-caps">
          <span>{clock(activity.startedAtMs)}</span><span>{clock(activity.endedAtMs)}</span>
        </div>
      </div>

      {#if !isAudioOnly}
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
              {#if thumbUrls[f.id]}<img class="film__img" src={thumbUrls[f.id]} alt="" />{/if}
            </button>
          {/each}
        </div>
      {/if}

      <div class="controls">
        <button
          type="button"
          class="play"
          class:play--audio={audible}
          aria-label={audible ? (clipPlaying ? "Pause" : "Play") : playing ? "Pause" : "Play"}
          disabled={isAudioOnly && audioCurrent.citation == null}
          onclick={togglePlay}>{playIcon}</button
        >
        {#if !isAudioOnly}
          {#if clipMode}
            <span class="speed-audio">1× · audio</span>
          {:else}
            <Segmented
              options={speedOptions}
              value={String(speed)}
              onValueChange={onSpeedChange}
              ariaLabel="Playback speed"
              compact
            />
          {/if}
        {/if}
        <span class="counter">
          {#if isAudioOnly}
            spoken segment {audioCurrent.ordinal} / {audioCurrent.total}{#if headClock} · {headClock}{/if}
          {:else if clipMode}
            <span class="counter__now">{currentMs != null ? clockSec(currentMs) : ""}</span> · spoken segment {audioCurrent.ordinal}
            / {audioCurrent.total}
          {:else}
            <span class="counter__now">frame {index + 1}</span> / {strip.length}{#if currentMs != null} · {clockSec(
                currentMs,
              )}{/if}
          {/if}
        </span>
        <span class="ctl-spacer"></span>
        <button type="button" class="open-tl" onclick={openInTimeline}>Open in Timeline →</button>
      </div>

      <div class="m-foot">
        {#if isAudioOnly}
          <span>{audioFooterLeft(frameEvidence.length)}</span><span class="sep">·</span>
          <span>{audioSpeakerSummary(audioCitations, profiles)}</span>
        {:else}
          <span>
            {strip.length}
            {strip.length === 1 ? "frame" : "frames"} across {segmentCount} capture
            {segmentCount === 1 ? "segment" : "segments"}
          </span>
          <span class="sep">·</span>
          <span>{frameEvidence.length} frames + {audioEvidence.length} spoken segments cited</span>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  /* One declaration per line to keep this component under the 800-line ceiling
     (repo rule); tokens + structure mirror docs/mockups/dayflow/04-timelapse.html.
     Audio channel (ADR 0049) uses --cat-communication (lavender) for voice. */
  .receipt { position: fixed; inset: 0; z-index: 2000; display: grid; place-items: center; padding: 16px; background: var(--app-overlay-bg); backdrop-filter: blur(4px); }
  .modal-card { width: 82vw; height: 90vh; display: flex; flex-direction: column; overflow: hidden; background: var(--app-surface); border: 1px solid var(--app-border-strong); border-radius: 12px; box-shadow: var(--app-shadow-popover); }
  .m-head { display: flex; align-items: center; gap: 10px; padding: 13px 16px; border-bottom: 1px solid var(--app-border); }
  .chip { flex: 0 0 auto; display: inline-flex; align-items: center; gap: 6px; font-size: 10px; letter-spacing: 0.06em; text-transform: uppercase; color: var(--app-text-muted); }
  .sw { flex: 0 0 auto; width: 8px; height: 8px; border-radius: 50%; }
  .m-title { flex: 1 1 auto; min-width: 0; margin: 0; font-size: 14px; font-weight: 600; color: var(--app-text-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .when { flex: 0 0 auto; font-size: 11px; color: var(--app-text-subtle); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .m-close { flex: 0 0 auto; display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; font: inherit; cursor: pointer; color: var(--app-text-subtle); background: transparent; border: 1px solid var(--app-border); border-radius: 5px; }
  .m-close:hover { color: var(--app-text-strong); border-color: var(--app-border-hover); }
  .m-summary { flex: 0 0 auto; margin: 0; padding: 10px 16px; border-bottom: 1px solid var(--app-border); font-size: 12px; line-height: 1.65; color: var(--app-text-muted); }

  /* Viewer — no transition on the img: instant frame swaps are the video feel. */
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
  .exp__glyph { margin-bottom: 10px; font-size: 16px; letter-spacing: 0.3em; color: var(--app-text-faint); }
  .exp h4 { margin: 0 0 6px; font-size: 13px; font-weight: 600; color: var(--app-text-strong); }
  .exp p { margin: 0; font-size: 11.5px; line-height: 1.7; color: var(--app-text-muted); }

  /* Audio-only viewer — a bounded audio player, never a false "footage expired". */
  .viewer--audio { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; text-align: center; }
  .big-play { width: 48px; height: 48px; display: inline-flex; align-items: center; justify-content: center; font-size: 17px; cursor: pointer; border-radius: 50%; color: var(--cat-communication); background: var(--app-accent-bg); border: 1px solid var(--cat-communication); }
  .big-play:disabled { opacity: 0.5; cursor: default; }
  .a-spk { font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--cat-communication); }
  .a-spk b { color: var(--app-text-strong); }
  .a-spk .rb { margin-left: 6px; font-size: 9px; letter-spacing: 0; text-transform: none; color: var(--app-text-subtle); }
  .a-cap { max-width: 470px; padding: 0 20px; font-size: 12px; line-height: 1.6; color: var(--app-text); }
  .a-when { font-size: 10.5px; color: var(--app-text-subtle); font-variant-numeric: tabular-nums; }

  /* Clip caption band on the viewer while a bounded 1× clip runs. */
  .clip-bar { position: absolute; left: 0; right: 0; bottom: 0; z-index: 3; display: flex; align-items: center; gap: 10px; padding: 9px 16px; backdrop-filter: blur(4px); background: var(--app-overlay-bg); border-top: 1px solid var(--cat-communication); }
  .clip-bar .live { flex: none; display: inline-flex; align-items: center; gap: 6px; font-size: 10px; letter-spacing: 0.1em; text-transform: uppercase; color: var(--cat-communication); }
  .clip-bar .live .d { width: 6px; height: 6px; border-radius: 50%; background: var(--cat-communication); box-shadow: 0 0 5px var(--cat-communication); }
  .clip-bar .cap { flex: 1; min-width: 0; font-size: 11px; color: var(--app-text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .clip-bar .rate { flex: none; font-size: 10px; color: var(--app-text-subtle); font-variant-numeric: tabular-nums; }

  /* Scrubber — frame ticks above the spine, lavender audio ticks below it. */
  .scrub { padding: 14px 16px 6px; }
  .track { position: relative; height: 6px; border-radius: 3px; background: var(--app-surface-hover); cursor: pointer; touch-action: none; }
  .fill { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 3px; background: var(--app-accent-strong); pointer-events: none; }
  .fill--audio { background: var(--cat-communication); }
  .head { position: absolute; top: 50%; transform: translate(-50%, -50%); padding: 2px 8px; font-size: 9px; font-weight: 700; font-variant-numeric: tabular-nums; color: var(--app-bg); background: var(--app-accent); border-radius: 999px; white-space: nowrap; pointer-events: none; }
  .head--audio { background: var(--cat-communication); color: var(--app-bg); }
  .ev { position: absolute; top: -4px; width: 2px; height: 14px; background: var(--app-accent); border-radius: 1px; opacity: 0.5; pointer-events: none; }
  .ev--hl { opacity: 1; box-shadow: 0 0 6px var(--app-accent); }
  /* Sits below the spine, nudged down + above the head pill (z-index) so a cited
     dot stays visible when the scrubber head parks on top of it. */
  .ev-a { position: absolute; bottom: -11px; left: 0; width: 9px; height: 9px; padding: 0; transform: translateX(-50%); border-radius: 50%; cursor: pointer; opacity: 0.8; background: var(--cat-communication); border: 1.5px solid var(--app-bg); z-index: 2; }
  .ev-a:hover, .ev-a:focus-visible { opacity: 1; }
  .ev-a--hl { opacity: 1; box-shadow: 0 0 7px var(--cat-communication); }
  /* Snippet tooltip for an audio tick — sits BELOW the track (arrow up), shown
     on hover/focus of its adjacent tick. Non-interactive (pointer-events:none),
     so play still happens on the tick click. Mirrors .tip-a in the mockup. */
  .tip-a { display: none; position: absolute; bottom: calc(100% + 12px); transform: translateX(-50%); width: max-content; max-width: 360px; padding: 9px 12px; border: 1px solid var(--cat-communication); border-radius: 7px; background: var(--app-surface-raised); box-shadow: var(--app-shadow-popover); pointer-events: none; text-align: left; z-index: 3; }
  .ev-a:hover + .tip-a, .ev-a:focus-visible + .tip-a { display: block; }
  .tip-a::after { content: ""; position: absolute; top: 100%; left: 50%; transform: translateX(-50%); border: 5px solid transparent; border-top-color: var(--cat-communication); }
  .tip-a .lbl { font-size: 9px; letter-spacing: 0.1em; text-transform: uppercase; color: var(--cat-communication); margin-bottom: 4px; }
  .tip-a .snip { font-size: 11px; line-height: 1.55; color: var(--app-text); }
  /* Play hint styled as the audio channel's chip (not clickable — the tick is
     the target; pointer-events stay off), so it reads as the action. */
  .tip-a .go { margin-top: 9px; display: inline-flex; align-items: center; padding: 4px 10px; font-size: 10px; font-weight: 500; white-space: nowrap; color: var(--cat-communication); background: color-mix(in srgb, var(--cat-communication) 12%, transparent); border: 1px solid var(--cat-communication); border-radius: 6px; }
  .scrub-caps { display: flex; justify-content: space-between; margin-top: 6px; font-size: 10px; font-variant-numeric: tabular-nums; color: var(--app-text-faint); }

  /* Filmstrip — a scroll container's auto min-height is 0, so flex:0 0 auto pins
     it to its natural cell-aspect height instead of getting crushed. */
  .film { flex: 0 0 auto; display: grid; grid-auto-flow: column; gap: 5px; padding: 8px 16px 8px; grid-auto-columns: calc((100% - 55px) / 12); overflow-x: auto; overflow-y: hidden; }
  .film__cell { position: relative; aspect-ratio: 16 / 10; padding: 0; cursor: pointer; background: linear-gradient(160deg, var(--app-surface-raised), var(--app-bg) 70%); border: 1px solid var(--app-border); border-radius: 4px; overflow: hidden; }
  .film__cell::after { content: ""; position: absolute; inset: 25% 18% 30%; background: var(--app-surface-hover); border-radius: 2px; }
  .film__img { position: absolute; inset: 0; z-index: 1; width: 100%; height: 100%; object-fit: cover; }
  .film__cell.cur { border-color: var(--app-accent); box-shadow: 0 0 0 1px var(--app-accent); }
  .film__cell.cited::before { content: ""; position: absolute; top: 3px; right: 3px; z-index: 2; width: 5px; height: 5px; background: var(--app-accent); border-radius: 50%; }

  /* Controls */
  .controls { display: flex; align-items: center; gap: 10px; padding: 12px 16px 14px; }
  .play { display: inline-flex; align-items: center; justify-content: center; width: 32px; height: 32px; font-size: 13px; cursor: pointer; color: var(--app-accent); background: var(--app-accent-bg); border: 1px solid var(--app-accent-border); border-radius: 7px; }
  .play:hover, .open-tl:hover { border-color: var(--app-accent); }
  .play--audio { color: var(--cat-communication); background: var(--app-accent-bg); border-color: var(--cat-communication); }
  .speed-audio { padding: 3px 9px; font-size: 10px; border-radius: 6px; color: var(--cat-communication); background: var(--app-accent-bg); border: 1px solid var(--cat-communication); }
  .counter { font-size: 11px; font-variant-numeric: tabular-nums; color: var(--app-text-muted); }
  .counter__now { color: var(--app-text-strong); }
  .ctl-spacer { flex: 1 1 auto; }
  .open-tl { display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px; font: inherit; font-size: 11px; cursor: pointer; color: var(--app-accent); background: var(--app-accent-bg); border: 1px solid var(--app-accent-border); border-radius: 6px; }

  /* Footer */
  .m-foot { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 16px; font-size: 10.5px; color: var(--app-text-subtle); border-top: 1px dashed var(--app-border); }
  .m-foot .sep { color: var(--app-text-faint); }

  @media (prefers-reduced-motion: reduce) { .skeleton { animation: none; } }
</style>
