<script lang="ts">
  // ReceiptViewer — one activity's receipt, inline under its Journal card
  // (insights.html `.in-receipt`; it replaced the ActivityReceipt modal). Plays
  // the real captured frames over the card's span as a scrubbable timelapse (no
  // video is encoded — playback swaps frame previews on rAF); ticks mark
  // engine-cited frames. Per ADR 0049 it also plays cited *audio*: each spoken
  // turn is a click-first row; selecting one plays that segment's real audio at
  // 1× while the frame viewer follows the same clock, then auto-advances so the
  // span plays through. 8×/16× is the silent frame timelapse. Audio-only
  // activities become a bounded audio player, never a false "footage expired".
  // "Open in Timeline" handles anything wider (OCR copy, export, scrub).

  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import IconArrowRight from "~icons/lucide/arrow-right";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import IconInbox from "~icons/lucide/inbox";
  import IconInfo from "~icons/lucide/info";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import Segmented from "$lib/components/Segmented.svelte";
  import { setPendingTimelineFocus } from "$lib/timeline/pending-focus";
  import {
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
  import { hhmm } from "$lib/insights/journal-view";
  import { retentionVerdict } from "$lib/insights/retention";
  import { captureControls } from "$lib/capture-controls.svelte";
  import type { Activity } from "$lib/types/recording";
  import type { FrameDto, FrameSummaryDto } from "$lib/types/app-infra";

  let { activity }: { activity: Activity } = $props();

  type StripFrame = { id: number; ms: number };
  const hhmmss = (ms: number) =>
    new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

  // ── Reactive playback state ──────────────────────────────────────────
  let strip = $state<StripFrame[]>([]); // frames over the span, ascending
  let index = $state(0); // current frame index
  let playing = $state(false);
  let speed = $state<Speed>(8); // silent-timelapse default; onTurns drops to 1× when audio exists
  let loading = $state(true);
  let cacheBump = $state(0); // bumped when a preview lands (display dep)
  let currentMeta = $state<FrameDto | null>(null);

  // ── Span-wide turns + selection (ADR 0049) ───────────────────────────
  let turns = $state<TurnView[]>([]);
  let turnsPending = $state(true); // span hydration in flight; false once onTurns lands (even empty)
  let loadFailed = $state(false); // a frame or audio listing rejected (never "expired")
  let transcribing = $state<{ done: number; total: number } | null>(null);
  let selectedKey = $state<string | null>(null);
  let clipPlaying = $state(false);
  let clipStartMs = $state(0); // active clip's segment start (wall-clock epoch)
  let activeClipId = $state<number | null>(null); // the segment whose audio is loaded
  let clipHeadMs = $state<number | null>(null); // live audio wall-clock while a clip plays
  let audioEl = $state<HTMLAudioElement | null>(null);
  let clipToken = 0; // guards the async media fetch; a new clip/activity drops it

  // ── Non-reactive machinery ───────────────────────────────────────────
  const loader = new ReceiptFrameLoader({
    onPreview: () => cacheBump++,
    onThumb: () => {}, // no filmstrip inline — the Timeline is the frame browser
    onMeta: (meta) => (currentMeta = meta),
  });
  // Cached previews are object URLs: collapsing the receipt must revoke them.
  $effect(() => () => loader.dispose());
  const audioLoader = new ReceiptAudioLoader({
    onProfiles: () => {},
    onTranscribing: (p) => (transcribing = p.done < p.total ? p : null),
    onError: () => (loadFailed = true),
    onTurns: (t) => {
      turns = t;
      turnsPending = false;
      selectedKey = defaultSelectedKey(t);
      // Audio available → 1× relives the spoken moment; frame-only keeps 8×.
      speed = t.length > 0 ? 1 : 8;
    },
  });
  let loadGen = 0;
  let rafId: number | null = null;
  let lastTs = 0;
  let frameAccum = 0;
  let trackEl = $state<HTMLDivElement | null>(null);
  let scrubbing = false;
  let wasPlaying = false; // audio play state at scrub start; resume on release iff true

  // ── Derived view model ───────────────────────────────────────────────
  const stripIds = $derived(strip.map((f) => f.id));
  const stripMs = $derived(strip.map((f) => f.ms));
  const currentFrameId = $derived(strip[index]?.id ?? null);
  const currentMs = $derived(strip[index]?.ms ?? null);
  const currentUrl = $derived.by<string | null>(() => {
    cacheBump; // recompute when a preview lands
    return currentFrameId == null ? null : loader.peekPreviewUrl(currentFrameId);
  });

  const evidenceSplit = $derived(partitionEvidence(activity.evidence));
  const frameEvidence = $derived(evidenceSplit.frames);
  const audioEvidence = $derived(evidenceSplit.audio);
  const viewState = $derived(
    receiptViewState(strip.length, audioEvidence.length, turnsPending, turns.length, transcribing ? 1 : 0, loadFailed),
  );
  // "Expired" is only true when retention removed this span (JR-22).
  const retention = $derived(
    retentionVerdict(activity.startedAtMs, activity.endedAtMs, captureControls.recordingSettings?.retentionPolicy),
  );
  const isAudioOnly = $derived(viewState === "audio-only");
  const selectedTurn = $derived(turns.find((t) => t.key === selectedKey) ?? null);

  const audible = $derived(isAudioOnly || isAudibleSpeed(speed));
  const isPlaying = $derived(audible ? clipPlaying : playing);
  const headMs = $derived(isAudioOnly ? (clipHeadMs ?? selectedTurn?.startMs ?? null) : currentMs);
  const headPos = $derived(headMs == null ? 0 : posFor(headMs));
  // While a clip plays, only its segment's turns can light up (an overlapping
  // mic/system turn never steals the highlight).
  const clipTurns = $derived(activeClipId == null ? turns : turns.filter((t) => t.audioSegmentId === activeClipId));
  const activeKey = $derived(activeKeyAt(clipTurns, headMs) ?? selectedKey);

  const headlineFrameId = $derived(frameEvidence.find((e) => e.isHeadline)?.subjectId ?? null);
  const ticks = $derived(
    frameEvidence.flatMap((e) => {
      const ms = strip.find((f) => f.id === e.subjectId)?.ms ?? e.capturedAtMs ?? null;
      return ms == null ? [] : [posFor(ms)];
    }),
  );
  const segmentCount = $derived(countCaptureSegments(stripMs));

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
    loadFailed = false;
    strip = [];
    currentMeta = null;
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
      // A load failure is an error with Try again, never "expired" (JR-21).
      if (gen === loadGen) {
        strip = [];
        loadFailed = true;
      }
    } finally {
      if (gen === loadGen) loading = false;
    }
  }

  // ── Playback loop (frame-swap timelapse; silent speeds only) ─────────
  function tick(ts: number): void {
    if (!playing) {
      rafId = null;
      return;
    }
    if (lastTs === 0) lastTs = ts;
    frameAccum += ((ts - lastTs) / 1000) * framesPerSecond(speed);
    lastTs = ts;
    const advance = Math.floor(frameAccum);
    if (advance > 0) {
      frameAccum -= advance;
      if (index + advance >= strip.length - 1) {
        index = strip.length - 1;
        pause(); // stop at the end
        return;
      }
      index += advance;
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

  // Play/Pause routes to whichever clock owns the receipt: the <audio> element
  // when audible (starting the selected turn's clip if none is loaded), else
  // the silent frame timelapse.
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

  // Selection = play (ADR 0049): clicking a turn relives it at 1×.
  function onSelect(turn: TurnView): void {
    selectedKey = turn.key;
    speed = 1;
    void playClip(turn);
  }
  function onSpeedChange(v: string): void {
    speed = Number(v) as Speed;
    if (isAudibleSpeed(speed) && selectedTurn) void playClip(selectedTurn);
    else stopClip(); // silent timelapse; leave paused
  }

  // ── Bounded, synchronized audio+screen clip (ADR 0049) ───────────────
  async function playClip(turn: TurnView, seekToMs?: number, autoplay = true): Promise<void> {
    if (!audioEl) return;
    pause(); // the audio clocks from here
    const offsetSec = clipStartOffsetSec(turn, seekToMs);
    // Same segment already loaded → seek in place. Reassigning an identical
    // data: URL can reset readyState in WKWebView, so never touch src here.
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
    scheduleClipSeek(audioEl, offsetSec); // deferred to loadedmetadata
    if (autoplay) void audioEl.play().catch(() => {});
  }
  function onAudioTimeUpdate(): void {
    if (activeClipId == null || !audioEl) return;
    const targetMs = clipStartMs + audioEl.currentTime * 1000;
    clipHeadMs = targetMs;
    if (stripMs.length > 0) index = frameIndexForMs(stripMs, targetMs);
  }
  // Clip finished: auto-advance to the next segment; a manual pause fires
  // onpause, not onended, so pausing never auto-advances.
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
    transcribing = null;
    selectedKey = null;
    void audioLoader.loadSpan(activity.startedAtMs, activity.endedAtMs, audioEvidence);
  }
  const speedOptions = SPEEDS.filter((s) => s !== 2).map((s) => ({ value: String(s), label: `${s}×` }));

  // ── Scrubbing (frames follow the pointer; on release the audio lands there) ──
  function fracAt(clientX: number): number | null {
    if (!trackEl) return null;
    const r = trackEl.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width));
  }
  function scrubTo(clientX: number): void {
    const f = fracAt(clientX);
    if (f != null && strip.length > 0) index = Math.round(f * (strip.length - 1));
  }
  function onTrackPointerDown(e: PointerEvent): void {
    if (strip.length === 0) return; // audio-only track stays a read-only playhead
    wasPlaying = !!audioEl && !audioEl.paused;
    pause();
    audioEl?.pause();
    scrubbing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubTo(e.clientX);
  }
  function onTrackPointerMove(e: PointerEvent): void {
    if (scrubbing) scrubTo(e.clientX);
  }
  function onTrackPointerUp(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* already released */ }
    const f = fracAt(e.clientX);
    if (turns.length === 0 || f == null) return;
    const ms = activity.startedAtMs + f * (activity.endedAtMs - activity.startedAtMs);
    const turn = turnAtMs(turns, ms);
    if (!turn) return stopClip(); // released over a silent gap — the frame already moved
    selectedKey = turn.key;
    speed = 1;
    void playClip(turn, ms, wasPlaying);
  }

  // Hand off the current frame; audio-only hands off the selected segment.
  function openInTimeline(): void {
    const audioSegmentId = selectedTurn?.audioSegmentId ?? null;
    if (currentFrameId != null) setPendingTimelineFocus({ frameId: currentFrameId });
    else if (audioSegmentId != null) setPendingTimelineFocus({ audioSegmentId });
    else return;
    void goto("/");
  }

  // ── Effects ──────────────────────────────────────────────────────────
  function reload(): void {
    void loadStrip();
    loadAudio();
  }
  $effect(() => {
    activity.id;
    reload();
  });
  $effect(() => {
    loader.pump(stripIds, index);
  });
  $effect(() => {
    if (currentFrameId != null) void loader.loadMeta(currentFrameId);
  });
  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    audioEl?.pause();
    audioLoader.reset();
  });
</script>

<!-- One hidden <audio> clocks every bounded clip (ADR 0049). -->
<audio
  bind:this={audioEl}
  onplay={() => (clipPlaying = true)}
  onpause={() => (clipPlaying = false)}
  ontimeupdate={onAudioTimeUpdate}
  onended={onAudioEnded}
  style="display:none"
></audio>

{#snippet turnList()}
  <div class="in-turns">
    {#each turns as turn (turn.key)}
      <button
        type="button"
        class="in-turn"
        aria-pressed={turn.key === activeKey}
        style="--c: var({turn.colorVar})"
        onclick={() => onSelect(turn)}
      >
        <time class="num">{hhmm(turn.startMs)}</time>
        <span><b>{turn.speaker}</b>{turn.text || "—"}</span>
      </button>
    {/each}
    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm in-open" onclick={openInTimeline}>
      Open in Timeline<IconArrowRight width="13" height="13" aria-hidden="true" />
    </button>
  </div>
{/snippet}

{#snippet controls()}
  <div
    class="in-scrub"
    class:in-scrub--ro={isAudioOnly}
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
    onpointercancel={() => (scrubbing = false)}
  >
    {#if !isAudioOnly}
      {#each ticks as pos, i (i)}<i style="left:{pos * 100}%"></i>{/each}
    {/if}
    <span class="head" style="left:calc({headPos * 100}% - 1px)"></span>
  </div>
  <div class="in-playrow">
    <button
      type="button"
      class="mx-btn mx-btn--sm"
      disabled={isAudioOnly && selectedTurn == null}
      onclick={togglePlay}
    >
      {#if isPlaying}<IconPause width="11" height="11" aria-hidden="true" />Pause{:else}<IconPlay width="11" height="11" aria-hidden="true" />Play{/if}
    </button>
    {#if !isAudioOnly}
      <Segmented options={speedOptions} value={String(speed)} onValueChange={onSpeedChange} ariaLabel="Playback speed" mono compact />
    {/if}
    <span class="num">{headMs != null ? hhmmss(headMs) : ""}</span>
  </div>
{/snippet}

<div class="mx-panel mx-panel--inset in-receipt">
  {#if loading}
    <div aria-busy="true">
      <span class="mx-sr">Loading footage</span>
      <div class="mx-skel mx-skel--thumb"></div>
      <div class="mx-skel-lines"><div class="mx-skel"></div></div>
    </div>
    <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
    <div class="in-rfoot">Loading footage…</div>
  {:else if viewState === "error"}
    <div class="in-wide">
      <div class="mx-empty mx-empty--compact" data-tone="danger" role="alert">
        <span class="mx-empty__glyph"><IconAlert width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">Couldn't load footage</b>
        <p class="mx-empty__text">Reading this activity's frames failed this time. The summary above isn't affected.</p>
        <div class="mx-empty__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={reload}><IconRetry width="13" height="13" aria-hidden="true" />Try again</button>
        </div>
      </div>
    </div>
    <div class="in-rfoot">Footage didn't load · summary shown above</div>
  {:else if viewState === "expired"}
    <!-- "Expired" only when retention removed the span (ADR 0029); otherwise the
         screen simply wasn't captured. -->
    <div class="in-wide">
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconInbox width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">{retention ? "Footage expired" : "No screen capture"}</b>
        <p class="mx-empty__text">
          {retention
            ? `Removed by your ${retention.days}-day retention. The card, its summary and its evidence list are kept — only the pixels age out.`
            : "No screen frames were captured during this activity. The card and its summary are kept."}
        </p>
      </div>
    </div>
    <div class="in-rfoot">{retention ? "0 frames still on disk" : "No screen capture"} · summary retained</div>
  {:else if isAudioOnly}
    <div>
      <div class="in-audio">
        <button
          type="button"
          class="mx-btn mx-btn--icon"
          aria-label={isPlaying ? "Pause spoken evidence" : "Play spoken evidence"}
          disabled={selectedTurn == null}
          onclick={togglePlay}
        >{#if isPlaying}<IconPause width="14" height="14" />{:else}<IconPlay width="14" height="14" />{/if}</button>
        {#if selectedTurn}
          <b style="--c: var({selectedTurn.colorVar})"><i></i>{selectedTurn.speaker}
            {#if selectedTurn.sourceMeta}<span class="mx-label">via {selectedTurn.sourceMeta}</span>{/if}</b>
          <span class="mx-label num">spoken segment · {hhmm(selectedTurn.startMs)}–{hhmm(selectedTurn.endMs)} · captured as audio</span>
        {:else if turnsPending}
          <span class="mx-label">Loading spoken evidence…</span>
        {:else if transcribing}
          <span class="mx-label">Transcribing this audio… {transcribing.done} of {transcribing.total} segments done</span>
        {:else}
          <span class="mx-label">No readable speech in the cited audio</span>
        {/if}
      </div>
      {#if selectedTurn}{@render controls()}{/if}
    </div>
    {#if turns.length > 0}
      {@render turnList()}
    {:else}
      <div>
        <div class="mx-empty mx-empty--compact">
          <span class="mx-empty__glyph"><IconInfo width="14" height="14" aria-hidden="true" /></span>
          <b class="mx-empty__title">Nothing to play</b>
          <p class="mx-empty__text">The cited audio was silent, or transcription found no words.</p>
        </div>
      </div>
    {/if}
    <div class="in-rfoot">
      {audioFooterLeft(retention != null)} · {transcribing ? "transcript on its way" : turnSpeakerRoster(turns) || `${audioEvidence.length} spoken segment${audioEvidence.length === 1 ? "" : "s"} cited`}
    </div>
  {:else}
    <div>
      <div class="in-screen">
        {#if currentUrl}
          <img src={currentUrl} alt={currentMeta?.windowTitle ?? "Captured frame"} />
        {:else}
          <div class="mx-skel"></div>
        {/if}
        {#if currentMeta?.appName}<span class="in-screen__app">{currentMeta.appName}{currentMeta.windowTitle ? ` — ${currentMeta.windowTitle}` : ""}</span>{/if}
      </div>
      {@render controls()}
    </div>
    {@render turnList()}
    <div class="in-rfoot">
      {strip.length} frame{strip.length === 1 ? "" : "s"} across {segmentCount} capture segment{segmentCount === 1 ? "" : "s"}
      · {frameEvidence.length} frames + {audioEvidence.length} spoken segments cited
    </div>
  {/if}
</div>

<style>
  /* insights.html `.in-receipt` and its parts (kit tokens only). */
  .in-receipt { margin-top: var(--s-3); padding: var(--s-4); display: grid; grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.1fr); gap: var(--s-5); animation: mx-rise var(--t-slow) var(--ease-expo) both; }
  .in-wide { grid-column: 1 / -1; }
  .in-receipt .mx-empty { padding: var(--s-4) 0; max-width: none; }
  .in-screen { position: relative; aspect-ratio: 16 / 9; border-radius: var(--r-md); border: 1px solid var(--app-border); background: var(--app-bg); overflow: hidden; display: grid; }
  .in-screen img { width: 100%; height: 100%; object-fit: contain; }
  .in-screen .mx-skel { width: 100%; height: 100%; border-radius: 0; }
  .in-screen__app { position: absolute; left: 6px; bottom: 6px; max-width: calc(100% - 12px); padding: 2px 6px; border-radius: var(--r-xs); background: var(--app-overlay-bg); font: 500 var(--text-xs)/1.3 var(--font-mono); color: var(--app-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .in-audio { display: grid; gap: var(--s-2); align-content: center; justify-items: start; aspect-ratio: 16 / 9; padding: var(--s-4); border-radius: var(--r-md); border: 1px solid var(--app-border); background: var(--app-bg); }
  .in-audio b { font: 600 var(--text-md)/1.2 var(--font-sans); color: var(--app-text-strong); display: flex; align-items: center; gap: 7px; }
  .in-audio b i { width: 8px; height: 8px; border-radius: 50%; background: var(--c); }
  .in-scrub { position: relative; height: 22px; margin-top: var(--s-2); cursor: pointer; touch-action: none; }
  .in-scrub--ro { cursor: default; }
  .in-scrub::before { content: ""; position: absolute; left: 0; right: 0; top: 10px; height: 2px; border-radius: 1px; background: var(--app-border-strong); }
  .in-scrub i { position: absolute; top: 6px; width: 2px; height: 10px; border-radius: 1px; background: var(--cat-meetings); }
  .in-scrub .head { position: absolute; top: 2px; width: 2px; height: 18px; background: var(--app-accent); border-radius: 1px; box-shadow: 0 0 0 3px var(--app-accent-glow); pointer-events: none; }
  .in-playrow { display: flex; align-items: center; gap: var(--s-2); margin-top: 4px; }
  .in-playrow .num { font-size: var(--text-xs); color: var(--app-text-subtle); margin-left: auto; }
  .in-turns { display: grid; gap: 2px; align-content: start; max-height: 320px; overflow-y: auto; }
  .in-turn { display: grid; grid-template-columns: 40px 1fr; gap: var(--s-2); padding: 6px 8px; border: 0; border-radius: var(--r-sm); background: none; text-align: left; font: 400 var(--text-base)/1.5 var(--font-sans); color: var(--app-text); cursor: pointer; transition: background-color var(--t-fast); }
  .in-turn:hover { background: var(--app-surface-hover); }
  .in-turn[aria-pressed="true"] { background: var(--app-surface-active); }
  .in-turn:focus-visible { outline: 0; box-shadow: var(--app-ring); }
  .in-turn time { font-size: var(--text-xs); color: var(--app-text-subtle); line-height: 1.7; }
  .in-turn b { display: block; font: 600 var(--text-sm)/1.4 var(--font-sans); color: var(--c); }
  .in-open { margin: 6px 0 0 -1px; justify-self: start; }
  .in-rfoot { grid-column: 1 / -1; display: flex; gap: var(--s-2); font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); padding-top: var(--s-2); border-top: 1px solid var(--mx-hairline); }
</style>
