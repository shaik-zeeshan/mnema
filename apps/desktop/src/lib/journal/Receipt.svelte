<script lang="ts">
  // The receipt, open (mockup 08) — bounded evidence playback for one Journal
  // activity, dressed as a bento of its own: the frame (2×2, media bleeding to
  // all four edges), the transcript (the turns inside this activity, the one
  // under the playhead lit in its speaker's colour), and the filmstrip (bleeding
  // into the bottom radius, its last cell half-cut). The scrub track carries one
  // tick per cited frame, the headline tick full strength.
  //
  // The playback machinery is the shipping receipt's, unchanged: the frame
  // timelapse is a rAF frame-swap (no video is encoded), 1× is audio-clocked so
  // releasing over a turn plays that turn's real audio (ADR 0049), and the pure
  // helpers all come from `$lib/insights/receipt-*`. Only the surface is new.
  import { invoke } from "@tauri-apps/api/core";
  import IconClose from "~icons/lucide/x";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
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
    type TurnView,
  } from "$lib/insights/receipt-audio";
  import { activeKeyAt, defaultSelectedKey, nextClipTurn, turnAtMs } from "$lib/insights/receipt-lane";
  import { ReceiptAudioLoader } from "$lib/insights/receipt-audio-loader";
  import { clock, clockSec } from "$lib/insights/receipt-clock";
  import type { Activity } from "$lib/types/recording";
  import type { FrameDto, FramePreviewDto, FrameSummaryDto, PersonProfileDto } from "$lib/types/app-infra";
  import ReceiptControls from "./ReceiptControls.svelte";
  import ReceiptFrame from "./ReceiptFrame.svelte";
  import ReceiptTurns from "./ReceiptTurns.svelte";

  let { activity, onClose }: { activity: Activity; onClose: () => void } = $props();

  type StripFrame = { id: number; ms: number };

  // ── Playback state ───────────────────────────────────────────────────
  let strip = $state<StripFrame[]>([]); // frames over the span, ascending
  let index = $state(0);
  let playing = $state(false);
  let speed = $state<Speed>(8); // silent-timelapse default; drops to 1× when audio exists
  let loading = $state(true);
  let cacheBump = $state(0); // bumped when a preview lands (display dep)
  let currentMeta = $state<FrameDto | null>(null);
  let thumbUrls = $state<Record<number, string>>({});

  // ── Span-wide turns + selection (ADR 0049) ───────────────────────────
  let turns = $state<TurnView[]>([]);
  let turnsPending = $state(true); // false once onTurns lands, even empty
  let selectedKey = $state<string | null>(null);
  let clipPlaying = $state(false);
  let clipStartMs = $state(0);
  let activeClipId = $state<number | null>(null);
  let clipHeadMs = $state<number | null>(null); // live audio wall-clock; null when idle
  let audioEl = $state<HTMLAudioElement | null>(null);
  let clipToken = 0;

  // All invoke-touching fetch work lives in the loaders; they report back here.
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
      // Audio available → 1× so Play relives the spoken moment with real audio.
      speed = t.length > 0 ? 1 : 8;
    },
  });
  let loadGen = 0;
  let rafId: number | null = null;
  let lastTs = 0;
  let frameAccum = 0;
  let trackEl = $state<HTMLDivElement | null>(null);
  let filmEl = $state<HTMLDivElement | null>(null);
  let scrubbing = false;
  let wasPlaying = false;
  let thumbObserver: IntersectionObserver | null = null;

  // ── Derived view model ───────────────────────────────────────────────
  const catColorVar = $derived(activity.category ? CATEGORY_COLOR[activity.category] : UNCATEGORIZED_COLOR);
  const catLabel = $derived(activity.category ? categoryLabel(activity.category) : "Uncategorized");
  const rangeLabel = $derived(
    `${clock(activity.startedAtMs)} – ${clock(activity.endedAtMs)} · ${humanizeMs(activity.endedAtMs - activity.startedAtMs)}`,
  );

  // Frame ids are stable for the loaded strip — derive once so the per-tick pump
  // effect doesn't rebuild an O(strip) array on every playhead move.
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

  const evidenceSplit = $derived(partitionEvidence(activity.evidence));
  const frameEvidence = $derived(evidenceSplit.frames);
  const audioEvidence = $derived(evidenceSplit.audio);

  // Which viewer: frames win; else audio if spoken evidence survives; else the
  // honest expired panel.
  const viewState = $derived(
    receiptViewState(strip.length, audioEvidence.length, turnsPending, turns.length),
  );
  const isAudioOnly = $derived(viewState === "audio-only");
  const isExpired = $derived(!loading && viewState === "expired");

  const stripMs = $derived(strip.map((f) => f.ms));
  const selectedTurn = $derived(turns.find((t) => t.key === selectedKey) ?? null);
  const selOrdinal = $derived(selectedKey == null ? 0 : turns.findIndex((t) => t.key === selectedKey) + 1);

  const audible = $derived(isAudioOnly || isAudibleSpeed(speed));
  const clipActive = $derived(activeClipId != null);
  const isPlaying = $derived(audible ? clipPlaying : playing);
  const relivingClip = $derived(audible && selectedTurn != null && clipActive);

  // The wall-clock playhead: the live audio head while a clip plays (so
  // consecutive same-segment turns light in sequence), else the current frame.
  const headMs = $derived(isAudioOnly ? (clipHeadMs ?? selectedTurn?.startMs ?? null) : currentMs);
  const headPos = $derived(headMs == null ? 0 : posFor(headMs));
  const headClock = $derived(headMs == null ? "" : clock(headMs));

  // While a clip plays, restrict the lit row to that segment's turns so an
  // overlapping mic/system turn can't steal the highlight.
  const clipTurns = $derived(
    activeClipId == null ? turns : turns.filter((t) => t.audioSegmentId === activeClipId),
  );
  const activeKey = $derived(activeKeyAt(clipTurns, headMs) ?? selectedKey);

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
  const segmentCount = $derived(countCaptureSegments(stripMs));

  // What the transport row reads out. Both lines are counts of real things —
  // frames on disk, segments inferred from inter-frame gaps, cited refs.
  const counterLabel = $derived.by(() => {
    if (isAudioOnly) {
      const head = headClock ? ` · ${headClock}` : "";
      return `spoken turn ${selOrdinal} / ${turns.length}${head}`;
    }
    if (relivingClip && selectedTurn) {
      const now = currentMs != null ? clockSec(currentMs) : "";
      return `${now} · ${selectedTurn.speaker} · 1×`;
    }
    const when = currentMs != null ? ` · ${clockSec(currentMs)}` : "";
    return `frame ${index + 1} / ${strip.length}${when}`;
  });
  const footerParts = $derived.by(() => {
    if (isAudioOnly) {
      const turnWord = turns.length === 1 ? "turn" : "turns";
      return [
        audioFooterLeft(frameEvidence.length),
        `${turns.length} spoken ${turnWord} · ${audioEvidence.length} cited`,
      ];
    }
    const frameWord = strip.length === 1 ? "frame" : "frames";
    const segWord = segmentCount === 1 ? "segment" : "segments";
    return [
      `${strip.length} ${frameWord} across ${segmentCount} capture ${segWord}`,
      `${frameEvidence.length} frames + ${audioEvidence.length} spoken segments cited`,
    ];
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
      const sorted = (summaries ?? [])
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

  // An IntersectionObserver queues a cell's preview as it scrolls into view; the
  // loader's bounded pump does the fetching. ponytail: no cell virtualization —
  // a multi-hour activity renders one button per frame; virtualize if janky.
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

  // ── Playback loop (frame-swap timelapse; silent 2×/8×/16× only) ──────
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

  // Play/Pause routes to whichever clock owns the surface: <audio> when audible,
  // else the silent frame timelapse.
  function togglePlay(): void {
    if (audible) {
      if (!audioEl) return;
      if (activeClipId == null) {
        // Start under the pill, not the segment head, so audio and playhead
        // share one clock.
        if (selectedTurn) void playClip(selectedTurn, headMs ?? undefined);
        return;
      }
      audioEl.paused ? void audioEl.play().catch(() => {}) : audioEl.pause();
      return;
    }
    if (strip.length > 0) playing ? pause() : play();
  }

  function seek(i: number): void {
    if (strip.length === 0) return; // audio-only: nothing to move, don't halt the clip
    stopClip(); // a manual frame move preempts the audio clock
    pause();
    index = clampIndex(i, strip.length);
  }

  function step(delta: number): void {
    if (strip.length === 0) return;
    stopClip();
    pause();
    index = clampIndex(index + delta, strip.length);
  }

  // Selection = play (ADR 0049).
  function onSelect(key: string): void {
    selectedKey = key;
    const turn = turns.find((t) => t.key === key);
    if (!turn) return;
    speed = 1;
    void playClip(turn);
  }

  function onSpeedChange(v: Speed): void {
    speed = v;
    if (isAudibleSpeed(speed) && selectedTurn) void playClip(selectedTurn);
    else stopClip(); // silent timelapse; leave paused
  }

  // ── Bounded, synchronized audio+screen clip (ADR 0049) ───────────────
  // The turn's segment audio at 1×; each timeupdate jumps the frame viewer to
  // the strip frame at/just-before the audio's wall clock, so one playhead
  // drives both. Plays through, then auto-advances (onended).
  async function playClip(turn: TurnView, seekToMs?: number, autoplay = true): Promise<void> {
    if (!audioEl) return;
    pause(); // stop the rAF timelapse; the audio clocks from here
    const offsetSec = clipStartOffsetSec(turn, seekToMs);
    // Same segment already loaded → seek in place. Reassigning an identical
    // data: URL can reset readyState to 0 in WKWebView (the audio then never
    // moves), so a same-segment re-seek must NOT touch src.
    if (activeClipId === turn.audioSegmentId && audioEl.src) {
      scheduleClipSeek(audioEl, offsetSec);
      if (autoplay) void audioEl.play().catch(() => {});
      return;
    }
    clipHeadMs = null;
    const token = ++clipToken;
    const src = await audioLoader.fetchMediaSrc(turn.audioSegmentId);
    if (token !== clipToken || !src || !audioEl) return; // superseded / failed
    clipStartMs = turn.segmentStartMs;
    activeClipId = turn.audioSegmentId;
    audioEl.src = src;
    // New src: metadata isn't in yet, so scheduleClipSeek defers the seek to
    // loadedmetadata via a single-slot property (a superseding clip drops any
    // stale seek; a {once} listener would leak across the src swap).
    scheduleClipSeek(audioEl, offsetSec);
    if (autoplay) void audioEl.play().catch(() => {});
  }

  function onAudioTimeUpdate(): void {
    if (activeClipId == null || !audioEl) return;
    const targetMs = clipStartMs + audioEl.currentTime * 1000;
    clipHeadMs = targetMs;
    if (stripMs.length > 0) index = frameIndexForMs(stripMs, targetMs);
  }

  // Auto-advance so the span plays through; a manual pause fires onpause, not
  // onended, so pausing never advances.
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

  // ── Scrubbing: frames follow the pointer; on release the audio lands there ──
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
  // Release lands the audio at that instant; a release over a silent gap clears
  // the clip — the frame scrub already moved there.
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
    if (strip.length === 0) return; // audio-only track stays a read-only playhead
    wasPlaying = !!audioEl && !audioEl.paused;
    pause();
    audioEl?.pause();
    scrubbing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubToClientX(e.clientX); // a click is a 0-length drag
  }
  function onTrackPointerMove(e: PointerEvent): void {
    if (scrubbing) scrubToClientX(e.clientX);
  }
  function onTrackPointerUp(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* released */ }
    seekAudioAt(e.clientX, wasPlaying);
  }
  function onTrackPointerCancel(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* released */ }
    // Gesture stolen: keep the scrubbed frame, never resume.
  }

  // Hand the current frame (or, audio-only, the selected segment) to the
  // Timeline through the same broker seam Quick Recall and the Overview use;
  // the shell layout routes the window to `/` when the event lands.
  async function openInTimeline(): Promise<void> {
    const frameId = currentFrameId;
    const audioSegmentId = selectedTurn?.audioSegmentId ?? null;
    if (frameId == null && audioSegmentId == null) return;
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: frameId != null ? "frame" : "audio",
        frameId,
        audioSegmentId: frameId != null ? null : audioSegmentId,
        spanStartMs: null,
        alignedFrameId: null,
      });
    } catch {
      // Best-effort hand-off: a failure leaves the receipt open.
      return;
    }
    onClose();
  }

  function onBackdropPointerDown(e: PointerEvent): void {
    if (e.target !== e.currentTarget) return; // only the backdrop itself closes
    onClose();
  }

  // ── Effects ──────────────────────────────────────────────────────────
  $effect(() => {
    activity.id;
    void loadStrip();
    loadAudio();
  });

  // Re-pump the preview lookahead when the strip loads or the playhead moves.
  $effect(() => {
    loader.pump(stripIds, index);
  });

  $effect(() => {
    const id = currentFrameId;
    if (id != null) void loader.loadMeta(id);
  });

  // Window capture-phase keyboard — WKWebView doesn't focus <button> on click,
  // so element focus is unreliable; a window listener is the seam.
  $effect(() => {
    function onKey(e: KeyboardEvent): void {
      const arrow = e.key === "ArrowLeft" ? -1 : e.key === "ArrowRight" ? 1 : 0;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (arrow !== 0) {
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

  // Keep the current cell in view as playback/scrubbing advances.
  $effect(() => {
    const cell = filmEl?.children[index] as HTMLElement | undefined;
    cell?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    audioEl?.pause();
    audioLoader.reset();
    thumbObserver?.disconnect();
  });
</script>

<div class="dim" role="presentation" onpointerdown={onBackdropPointerDown}>
  <div class="rcp" role="dialog" aria-modal="true" aria-label={`Activity receipt: ${activity.title}`}>
    <div class="rcp__h">
      <span class="dot" style="background:var({catColorVar})"></span>
      <span class="t-label">{catLabel}</span>
      <span class="t-ui title">{activity.title}</span>
      <span class="t-meta is-mono is-num when">{rangeLabel}</span>
      <button type="button" class="btn btn--ghost btn--icon btn--sm close" aria-label="Close receipt" onclick={onClose}>
        <IconClose />
      </button>
    </div>

    <div class="rcp__b">
      {#if activity.summary}<p class="rcp__sum">{activity.summary}</p>{/if}

      <!-- One hidden <audio> clocks every bounded clip (ADR 0049). -->
      <audio
        bind:this={audioEl}
        onplay={() => (clipPlaying = true)}
        onpause={() => (clipPlaying = false)}
        ontimeupdate={onAudioTimeUpdate}
        onended={onAudioEnded}
        style="display:none"
      ></audio>

      <div class="bento bento--3 grid" class:grid--flat={isAudioOnly} class:grid--solo={isExpired}>
        <ReceiptFrame
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
          frameEvidenceCount={frameEvidence.length}
          onTogglePlay={togglePlay}
        />

        {#if !isExpired && turns.length > 0}
          <ReceiptTurns {turns} {activeKey} {onSelect} />
        {/if}

        {#if !isExpired && !isAudioOnly}
          <div class="tile tile--w3 tile--static">
            <div class="tile__h">
              <span class="t-label">Frames</span>
              <span class="tile__more is-num">{strip.length} · {frameEvidence.length} cited</span>
            </div>
            <div class="pay pay--bleed">
              <div class="film scroll" bind:this={filmEl}>
                {#each strip as f, ti (f.id)}
                  <button
                    type="button"
                    class="film__c"
                    class:cur={ti === index}
                    class:cited={citedIds.has(f.id)}
                    aria-label={`Seek to ${clock(f.ms)}`}
                    use:thumbCell={f.id}
                    onclick={() => seek(ti)}
                  >
                    {#if thumbUrls[f.id]}<img src={thumbUrls[f.id]} alt="" />{/if}
                  </button>
                {/each}
              </div>
            </div>
          </div>
        {/if}
      </div>

      {#if loading}
        <div class="rfoot"><span>Loading footage…</span></div>
      {:else if isExpired}
        <div class="rfoot">
          <span>0 frames still on disk</span><span>·</span><span>summary retained</span>
        </div>
      {:else}
        <ReceiptControls
          startMs={activity.startedAtMs}
          endMs={activity.endedAtMs}
          frameCount={strip.length}
          {index}
          {ticks}
          {currentPos}
          {headPos}
          {headClock}
          {isAudioOnly}
          canPlay={!isAudioOnly || selectedTurn != null}
          {isPlaying}
          {speed}
          counter={counterLabel}
          footer={footerParts}
          bind:trackEl
          onTogglePlay={togglePlay}
          {onSpeedChange}
          onOpenInTimeline={() => void openInTimeline()}
          {onTrackPointerDown}
          {onTrackPointerMove}
          {onTrackPointerUp}
          {onTrackPointerCancel}
        />
      {/if}
    </div>
  </div>
</div>

<style>
  /* The receipt floats over the day — the one place this direction spends a
     shadow, because the thing genuinely floats. */
  /* The loading / expired footer (the transport row owns its own). */
  .rfoot {
    display: flex;
    gap: var(--s-8);
    padding-top: var(--s-8);
    border-top: var(--hairline) dashed var(--tile-sep);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
  }
  .dim {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    background: var(--mat-dim);
  }
  .rcp {
    display: flex;
    flex-direction: column;
    width: min(900px, calc(100vw - 48px));
    height: min(620px, calc(100vh - 72px));
    border-radius: var(--r-xl);
    background: var(--app-bg);
    box-shadow: var(--app-shadow-modal, 0 24px 64px rgba(0, 0, 0, 0.48)), 0 0 0 var(--hairline) var(--app-border-strong);
    overflow: hidden;
  }
  .rcp__h {
    flex: 0 0 44px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--s-12) 0 var(--pad-window);
    box-shadow: 0 var(--hairline) 0 var(--app-border);
  }
  .dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .rcp__h .t-label {
    color: var(--app-text-subtle);
  }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .when {
    flex: 0 0 auto;
    white-space: nowrap;
  }
  .close {
    flex: 0 0 auto;
    cursor: pointer;
  }
  .close :global(svg) {
    width: 12px;
    height: 12px;
  }

  .rcp__b {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--cell-gutter);
    padding: var(--s-12) var(--pad-window) var(--pad-window);
  }
  .rcp__sum {
    margin: 0;
    font: var(--w-regular) var(--t-meta) / 1.55 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  /* The receipt's own bento: frame 2×2, transcript 1×2, filmstrip 3×1. */
  .grid {
    flex: 1 1 auto;
    min-height: 0;
    grid-template-rows: 1fr 1fr 116px;
  }
  /* No filmstrip (audio-only) — the two payload rows take the height. */
  .grid--flat {
    grid-template-rows: 1fr 1fr;
  }
  /* Expired: one panel, and it fills the receipt rather than floating in a
     third of it. */
  .grid--solo {
    grid-template-rows: 1fr;
  }

  /* Filmstrip — bleeds into the bottom radius; the last cell is half-cut
     whenever the strip overflows, which is the real overflow signifier. */
  .film {
    display: flex;
    gap: 5px;
    height: 100%;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .film__c {
    position: relative;
    flex: 0 0 122px;
    padding: 0;
    border: 0;
    border-radius: var(--tile-r-in);
    background: var(--media-void);
    overflow: hidden;
    cursor: pointer;
  }
  .film__c img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .film__c.cur {
    box-shadow: 0 0 0 2px var(--app-accent);
  }
  .film__c.cited::after {
    content: "";
    position: absolute;
    right: 4px;
    top: 4px;
    z-index: 4;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  .film__c:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px var(--app-accent);
  }

  /* Scrub track — one tick per cited frame, the headline tick full strength. */


</style>
