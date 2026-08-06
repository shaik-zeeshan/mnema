<script lang="ts">
  // The receipt — page 08 viewport B. One activity's evidence as a TRANSPORT
  // over the river: the real captured frames played back (no video is encoded;
  // playback swaps frame previews on rAF), a scrub bar carrying one tick per
  // cited frame with the headline frame emphasized, a filmstrip that marks the
  // cited cells, and the span's spoken turns as a synced transcript. 1× is the
  // audio-clocked pass (only when audio exists); 2×/8×/16× are silent
  // frame-stepping rates.
  //
  // It is the one modal in the app, and while it is open it OWNS the keyboard:
  // ␣ play/pause, ←→ step a frame, esc closes the receipt (never the route).
  // The playback machinery is the shipping one — receipt-frames / -playback /
  // -audio / -lane / -clock, imported, not forked; only the skin is new.
  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
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
    isAudibleSpeed,
    SPEEDS,
    type Speed,
  } from "$lib/insights/receipt-playback";
  import { ReceiptFrameLoader } from "$lib/insights/receipt-frames";
  import {
    audioFooterLeft,
    clipStartOffsetSec,
    frameIndexForMs,
    partitionEvidence,
    receiptViewState,
    scheduleClipSeek,
    turnSpeakerRoster,
    type TurnView,
  } from "$lib/insights/receipt-audio";
  import { activeKeyAt, defaultSelectedKey, nextClipTurn, turnAtMs } from "$lib/insights/receipt-lane";
  import { ReceiptAudioLoader } from "$lib/insights/receipt-audio-loader";
  import { clock, clockSec } from "$lib/insights/receipt-clock";
  import ReceiptTransport from "$lib/journal/ReceiptTransport.svelte";
  import ReceiptViewer from "$lib/journal/ReceiptViewer.svelte";
  import Transcript from "$lib/journal/Transcript.svelte";
  import type { Activity } from "$lib/types/recording";
  import type { FrameDto, FramePreviewDto, FrameSummaryDto, PersonProfileDto } from "$lib/types/app-infra";

  interface Props {
    activity: Activity;
    onClose: () => void;
  }
  let { activity, onClose }: Props = $props();

  type StripFrame = { id: number; ms: number };

  // ── Playback state ───────────────────────────────────────────────────
  let strip = $state<StripFrame[]>([]);
  let index = $state(0);
  let playing = $state(false);
  let speed = $state<Speed>(8); // silent timelapse default; drops to 1× when audio exists
  let loading = $state(true);
  let cacheBump = $state(0);
  let currentMeta = $state<FrameDto | null>(null);
  let thumbUrls = $state<Record<number, string>>({});

  // ── Span-wide spoken turns + selection ───────────────────────────────
  let turns = $state<TurnView[]>([]);
  let turnsPending = $state(true);
  let selectedKey = $state<string | null>(null);
  let clipPlaying = $state(false);
  let clipStartMs = $state(0);
  let activeClipId = $state<number | null>(null);
  let clipHeadMs = $state<number | null>(null);
  let audioEl = $state<HTMLAudioElement | null>(null);
  let clipToken = 0;

  const loader = new ReceiptFrameLoader({
    onPreview: () => cacheBump++,
    onThumb: (fid, url) => (thumbUrls[fid] = url),
    onMeta: (meta) => (currentMeta = meta),
  });
  const audioLoader = new ReceiptAudioLoader({
    onProfiles: (_p: PersonProfileDto[]) => {},
    onTurns: (t) => {
      turns = t;
      turnsPending = false;
      selectedKey = defaultSelectedKey(t);
      speed = t.length > 0 ? 1 : 8;
    },
  });
  let loadGen = 0;
  let rafId: number | null = null;
  let lastTs = 0;
  let frameAccum = 0;
  let trackEl = $state<HTMLDivElement | null>(null);
  let scrubbing = false;
  let wasPlaying = false;
  let thumbObserver: IntersectionObserver | null = null;

  // ── Derived view model ───────────────────────────────────────────────
  const catColorVar = $derived(activity.category ? CATEGORY_COLOR[activity.category] : UNCATEGORIZED_COLOR);
  const catLabel = $derived(activity.category ? categoryLabel(activity.category) : "Uncategorized");
  const rangeLabel = $derived(
    `${clock(activity.startedAtMs)} – ${clock(activity.endedAtMs)} · ${humanizeMs(activity.endedAtMs - activity.startedAtMs)}`,
  );

  const stripIds = $derived(strip.map((f) => f.id));
  const currentFrameId = $derived(strip[index]?.id ?? null);
  const currentMs = $derived(strip[index]?.ms ?? null);
  const currentPos = $derived(currentMs == null ? 0 : posFor(currentMs));
  const currentPreview = $derived.by<FramePreviewDto | null>(() => {
    cacheBump;
    const id = currentFrameId;
    return id == null ? null : loader.peekPreview(id);
  });
  const currentUrl = $derived(currentPreview ? framePreviewAssetUrl(currentPreview.filePath) : null);

  const metaApp = $derived(currentMeta?.appName ?? null);
  const metaTitle = $derived(currentMeta?.windowTitle ?? null);
  const hasOcr = $derived((currentMeta?.ocrText ?? "").trim().length > 0);

  const evidenceSplit = $derived(partitionEvidence(activity.evidence));
  const frameEvidence = $derived(evidenceSplit.frames);
  const audioEvidence = $derived(evidenceSplit.audio);

  const viewState = $derived(
    receiptViewState(strip.length, audioEvidence.length, turnsPending, turns.length),
  );
  const isAudioOnly = $derived(viewState === "audio-only");

  const stripMs = $derived(strip.map((f) => f.ms));
  const selectedTurn = $derived(turns.find((t) => t.key === selectedKey) ?? null);
  const selOrdinal = $derived(selectedKey == null ? 0 : turns.findIndex((t) => t.key === selectedKey) + 1);

  const audible = $derived(isAudioOnly || isAudibleSpeed(speed));
  const clipActive = $derived(activeClipId != null);
  const isPlaying = $derived(audible ? clipPlaying : playing);
  const relivingClip = $derived(audible && selectedTurn != null && clipActive);

  const headMs = $derived(isAudioOnly ? (clipHeadMs ?? selectedTurn?.startMs ?? null) : currentMs);
  const headPos = $derived(headMs == null ? 0 : posFor(headMs));
  const headClock = $derived(headMs == null ? "" : clock(headMs));

  const clipTurns = $derived(
    activeClipId == null ? turns : turns.filter((t) => t.audioSegmentId === activeClipId),
  );
  const activeKey = $derived(activeKeyAt(clipTurns, headMs) ?? selectedKey);

  const headlineFrameId = $derived(frameEvidence.find((e) => e.isHeadline)?.subjectId ?? null);
  // One tick per cited frame; the headline frame (the card's poster) is the
  // emphasized one.
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
  // ponytail: the segment count is a 90-second-gap heuristic — FrameSummaryDto
  // carries no segment id. Real number the day it does.
  const segmentCount = $derived(countCaptureSegments(strip.map((f) => f.ms)));
  const speedOptions = SPEEDS.map((s) => ({ value: String(s), label: `${s}×` }));

  // "frame 128 / 704 · 11:14:03 AM" — the real counter, or the spoken-turn
  // ordinal for an audio-only receipt / the speaker while a 1× clip relives.
  const counter = $derived.by(() => {
    if (isAudioOnly) {
      return `spoken turn ${selOrdinal} / ${turns.length}${headClock ? ` · ${headClock}` : ""}`;
    }
    if (relivingClip && selectedTurn) {
      return `${currentMs != null ? clockSec(currentMs) : ""} · ${selectedTurn.speaker} · 1×`;
    }
    return `frame ${index + 1} / ${strip.length}${currentMs != null ? ` · ${clockSec(currentMs)}` : ""}`;
  });

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

  // ponytail: no cell virtualization — one cell per frame; virtualize if a
  // multi-hour activity ever gets janky.
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

  // ── The silent frame timelapse (2×/8×/16×) ───────────────────────────
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
        pause();
        return;
      }
      index = next;
    }
    rafId = requestAnimationFrame(tick);
  }

  function play(): void {
    if (strip.length === 0) return;
    if (index >= strip.length - 1) index = 0;
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
    if (audible) {
      if (!audioEl) return;
      if (activeClipId == null) {
        if (selectedTurn) void playClip(selectedTurn, headMs ?? undefined);
        return;
      }
      audioEl.paused ? void audioEl.play().catch(() => {}) : audioEl.pause();
      return;
    }
    if (strip.length > 0) playing ? pause() : play();
  }

  function seek(i: number): void {
    if (strip.length === 0) return;
    stopClip();
    pause();
    index = clampIndex(i, strip.length);
  }

  function step(delta: number): void {
    if (strip.length === 0) return;
    stopClip();
    pause();
    index = clampIndex(index + delta, strip.length);
  }

  function onSelectTurn(key: string): void {
    selectedKey = key;
    const turn = turns.find((t) => t.key === key);
    if (!turn) return;
    speed = 1;
    void playClip(turn);
  }

  function onSpeedChange(v: string): void {
    speed = Number(v) as Speed;
    if (isAudibleSpeed(speed) && selectedTurn) void playClip(selectedTurn);
    else stopClip();
  }

  // ── Bounded, synchronized audio+screen clip (ADR 0049) ───────────────
  async function playClip(turn: TurnView, seekToMs?: number, autoplay = true): Promise<void> {
    if (!audioEl) return;
    pause();
    const offsetSec = clipStartOffsetSec(turn, seekToMs);
    // Same segment already loaded → seek in place; reassigning an identical
    // data: URL can reset readyState in WKWebView and strand the seek.
    if (activeClipId === turn.audioSegmentId && audioEl.src) {
      scheduleClipSeek(audioEl, offsetSec);
      if (autoplay) void audioEl.play().catch(() => {});
      return;
    }
    clipHeadMs = null;
    const token = ++clipToken;
    const src = await audioLoader.fetchMediaSrc(turn.audioSegmentId);
    if (token !== clipToken || !src || !audioEl) return;
    clipStartMs = turn.segmentStartMs;
    activeClipId = turn.audioSegmentId;
    audioEl.src = src;
    scheduleClipSeek(audioEl, offsetSec);
    if (autoplay) void audioEl.play().catch(() => {});
  }

  function onAudioTimeUpdate(): void {
    if (activeClipId == null || !audioEl) return;
    const targetMs = clipStartMs + audioEl.currentTime * 1000;
    clipHeadMs = targetMs;
    if (stripMs.length > 0) index = frameIndexForMs(stripMs, targetMs);
  }

  function onAudioEnded(): void {
    clipPlaying = false;
    const next = nextClipTurn(turns, activeClipId);
    if (!next) return;
    selectedKey = next.key;
    void playClip(next);
  }

  function stopClip(): void {
    clipToken++;
    audioEl?.pause();
    clipPlaying = false;
    activeClipId = null;
    clipHeadMs = null;
  }

  function loadAudio(): void {
    stopClip();
    turns = [];
    turnsPending = true;
    selectedKey = null;
    void audioLoader.loadSpan(activity.startedAtMs, activity.endedAtMs, audioEvidence);
  }

  // ── Scrubbing ────────────────────────────────────────────────────────
  function scrubToClientX(clientX: number): void {
    const el = trackEl;
    if (!el || strip.length === 0) return;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    index = Math.round(frac * (strip.length - 1));
  }
  function msForClientX(clientX: number): number | null {
    const el = trackEl;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return activity.startedAtMs + frac * (activity.endedAtMs - activity.startedAtMs);
  }
  function seekAudioAt(clientX: number, shouldPlay: boolean): void {
    if (turns.length === 0) return;
    const ms = msForClientX(clientX);
    if (ms == null) return;
    const turn = turnAtMs(turns, ms);
    if (!turn) {
      stopClip();
      return;
    }
    selectedKey = turn.key;
    speed = 1;
    void playClip(turn, ms, shouldPlay);
  }
  function onTrackPointerDown(e: PointerEvent): void {
    if (strip.length === 0) return;
    wasPlaying = !!audioEl && !audioEl.paused;
    pause();
    audioEl?.pause();
    scrubbing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubToClientX(e.clientX);
  }
  function onTrackPointerMove(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubToClientX(e.clientX);
  }
  function onTrackPointerUp(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* already released */
    }
    seekAudioAt(e.clientX, wasPlaying);
  }
  function onTrackPointerCancel(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* already released */
    }
  }

  // ── Open in Timeline (frontend-only handoff, no backend command) ─────
  function openInTimeline(): void {
    const audioSegmentId = selectedTurn?.audioSegmentId ?? null;
    if (currentFrameId != null) setPendingTimelineFocus({ frameId: currentFrameId });
    else if (audioSegmentId != null) setPendingTimelineFocus({ audioSegmentId });
    else return;
    void goto("/");
    onClose();
  }

  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return;
    onClose();
  }

  // ── Effects ──────────────────────────────────────────────────────────
  $effect(() => {
    activity.id;
    void loadStrip();
    loadAudio();
  });

  $effect(() => {
    loader.pump(stripIds, index);
  });

  $effect(() => {
    const id = currentFrameId;
    if (id != null) void loader.loadMeta(id);
  });

  // The modal owns the keyboard while it is open. Capture phase + stopPropagation
  // so esc closes the RECEIPT, never the route underneath (and the app-wide
  // shortcut handler doesn't also see the key). WKWebView doesn't focus <button>
  // on click, so element focus is not a reliable seam here.
  $effect(() => {
    function onKey(e: KeyboardEvent): void {
      const inRadioGroup = !!(e.target as HTMLElement | null)?.closest?.('[role="radiogroup"]');
      const arrow = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (arrow !== 0 && !inRadioGroup) {
        e.preventDefault();
        e.stopPropagation();
        step(arrow);
      } else if (e.key === " " || e.key === "Spacebar") {
        e.preventDefault();
        e.stopPropagation();
        togglePlay();
      }
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  });

  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    audioEl?.pause();
    audioLoader.reset();
    thumbObserver?.disconnect();
  });
</script>

<div class="scrim" role="presentation" onpointerdown={onBackdropPointerDown}>
  <div class="rcpt" role="dialog" aria-modal="true" aria-label={`Receipt: ${activity.title}`}>
    <div class="rcpt__h">
      <span class="jchip" style="--cat: var({catColorVar});"><i></i>{catLabel}</span>
      <span class="rcpt__ttl">{activity.title}</span>
      <span class="t-meta is-mono is-num rcpt__when">{rangeLabel}</span>
      <span class="rcpt__close">
        <span class="hint"><span class="kbd">esc</span><span>close</span></span>
        <button type="button" class="btn btn--icon btn--sm" aria-label="Close receipt" onclick={onClose}>
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
        </button>
      </span>
    </div>

    {#if activity.summary}
      <p class="rcpt__sum">{activity.summary}</p>
    {/if}

    <!-- One hidden <audio> clocks every bounded clip (ADR 0049). -->
    <audio
      bind:this={audioEl}
      onplay={() => (clipPlaying = true)}
      onpause={() => (clipPlaying = false)}
      ontimeupdate={onAudioTimeUpdate}
      onended={onAudioEnded}
      style="display:none"
    ></audio>

    <ReceiptViewer
      {loading}
      {turnsPending}
      {viewState}
      {isPlaying}
      {selectedTurn}
      {currentUrl}
      {metaApp}
      {metaTitle}
      {currentMs}
      {hasOcr}
      {currentPreview}
      onTogglePlay={togglePlay}
    />

    {#if !loading && viewState !== "expired"}
      <ReceiptTransport
        {strip}
        {index}
        {ticks}
        {citedIds}
        {thumbUrls}
        {currentPos}
        {headPos}
        {headClock}
        startLabel={clock(activity.startedAtMs)}
        endLabel={clock(activity.endedAtMs)}
        {isAudioOnly}
        {isPlaying}
        playDisabled={isAudioOnly && selectedTurn == null}
        {speed}
        {speedOptions}
        {counter}
        clockOf={clock}
        {thumbCell}
        bind:trackEl
        {onTrackPointerDown}
        {onTrackPointerMove}
        {onTrackPointerUp}
        {onTrackPointerCancel}
        onSeek={seek}
        onTogglePlay={togglePlay}
        {onSpeedChange}
        onOpenTimeline={openInTimeline}
      />

      {#if turns.length > 0}
        <Transcript {turns} {activeKey} onSelect={onSelectTurn} />
      {/if}

      <div class="rcpt__ft">
        {#if isAudioOnly}
          <span class="t-meta is-mono is-num">{audioFooterLeft(frameEvidence.length)}</span>
          <span class="rcpt__sep"></span>
          <span class="t-meta is-mono is-num">{turns.length} spoken turns</span>
          <span class="t-meta rcpt__roster">{turnSpeakerRoster(turns)}</span>
        {:else}
          <span class="t-meta is-mono is-num">
            {strip.length.toLocaleString()}
            {strip.length === 1 ? "frame" : "frames"} across {segmentCount} capture
            {segmentCount === 1 ? "segment" : "segments"}
          </span>
          <span class="rcpt__sep"></span>
          <span class="t-meta is-mono is-num">
            {frameEvidence.length} frames + {audioEvidence.length} spoken segments cited
          </span>
          {#if turns.length > 0}
            <span class="t-meta rcpt__roster">{turnSpeakerRoster(turns)}</span>
          {/if}
        {/if}
      </div>
    {:else if viewState === "expired"}
      <div class="rcpt__ft">
        <span class="t-meta is-mono is-num">0 frames still on disk</span>
        <span class="rcpt__sep"></span>
        <span class="t-meta is-mono is-num">summary retained</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--s-16);
    background: rgba(0, 0, 0, 0.5);
  }
  /* The scrim is ink in both themes; light needs far less of it to read as
     "behind". (`light-dark()` needs a `color-scheme`, which this app doesn't
     set — the theme is `[data-theme]`.) */
  :global([data-theme="light"]) .scrim {
    background: rgba(21, 28, 38, 0.28);
  }
  .rcpt {
    width: min(900px, 100%);
    height: min(648px, 100%);
    display: flex;
    flex-direction: column;
    border-radius: var(--r-xl);
    overflow: hidden;
    background: var(--app-surface);
    box-shadow:
      var(--shadow-modal),
      0 0 0 var(--hairline) var(--app-border-strong);
  }

  .rcpt__h {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: var(--s-12) var(--s-12) var(--s-8);
  }
  .jchip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--cat);
  }
  .jchip i {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: currentColor;
  }
  .rcpt__ttl {
    flex: 1 1 auto;
    min-width: 0;
    font: var(--w-semi) var(--t-ui) / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rcpt__when {
    flex: 0 0 auto;
    color: var(--app-text-muted);
  }
  .rcpt__close {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-8);
  }
  .rcpt__close svg {
    width: 11px;
    height: 11px;
  }
  .rcpt__sum {
    flex: 0 0 auto;
    margin: 0;
    padding: 0 var(--s-12) var(--s-12);
    max-width: 92ch;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
  }


  .rcpt__ft {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: var(--s-8) var(--s-12);
    box-shadow: inset 0 1px 0 var(--app-border);
  }
  .rcpt__sep {
    width: var(--hairline);
    height: 14px;
    background: var(--app-border-strong);
  }
  .rcpt__roster {
    margin-left: auto;
    color: var(--app-text-subtle);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
