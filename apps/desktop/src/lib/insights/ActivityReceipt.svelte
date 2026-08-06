<script lang="ts">
  // ActivityReceipt — bounded evidence playback for one Journal activity card.
  // Plays the real captured frames over the card's span as a scrubbable
  // "timelapse" (no video is encoded — playback swaps frame previews on rAF);
  // ticks mark engine-cited frames (headline = poster) and a wall-clock playhead
  // reads WHEN each frame happened. Per ADR 0049 it ALSO plays cited *audio* via
  // a synced transcript reader: each spoken turn is a click-first row colored per
  // speaker; selecting one plays that segment's real audio at 1× while the frame
  // viewer runs the same window, clocked by the <audio> element, then auto-
  // advances to the next segment so the span plays through. Clicking the scrub
  // bar lands playback at that instant. The reader highlights the turn under the
  // current playhead.
  // 2×/8×/16× is
  // the silent frame timelapse. Audio-only Activities become a bounded audio
  // player, never a false "footage expired". Attribution is read-only, late-bound
  // by id. "Open in Timeline" handles anything wider (OCR copy, export, scrub).

  import { invoke } from "@tauri-apps/api/core";
  import { goto } from "$app/navigation";
  import IconArrowRight from "~icons/lucide/arrow-right";
  import IconClose from "~icons/lucide/x";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
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
  import { clock, clockSec, clockShort } from "$lib/insights/receipt-clock";
  import ReceiptTranscript from "$lib/insights/ReceiptTranscript.svelte";
  import ReceiptViewer from "$lib/insights/ReceiptViewer.svelte";
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
  let speed = $state<Speed>(8); // 8× silent-timelapse default; onTurns drops to 1× when audio exists
  let loading = $state(true);
  let cacheBump = $state(0); // bumped when a preview lands (display dep)
  let currentMeta = $state<FrameDto | null>(null);
  let thumbUrls = $state<Record<number, string>>({}); // frameId → preview URL

  // ── Span-wide turns + selection (ADR 0049 redesign) ──────────────────
  let turns = $state<TurnView[]>([]); // every spoken turn over the span, ordered
  let turnsPending = $state(true); // span hydration in flight; false once onTurns lands (even empty)
  let selectedKey = $state<string | null>(null); // the one selection the lane + reader share
  let profiles = $state<PersonProfileDto[]>([]); // for live name resolution
  let clipPlaying = $state(false); // the <audio> element's play/pause state
  let clipStartMs = $state(0); // active clip's segment start (wall-clock epoch)
  let activeClipId = $state<number | null>(null); // the segment whose audio is loaded
  let clipHeadMs = $state<number | null>(null); // live audio wall-clock while a clip plays; null when idle (drives audio-only highlight)
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
  // Cited-audio hydration: shared profiles + the span's ordered TurnView[].
  const audioLoader = new ReceiptAudioLoader({
    onProfiles: (p) => (profiles = p),
    onTurns: (t) => {
      turns = t;
      turnsPending = false;
      selectedKey = defaultSelectedKey(t);
      // Audio available → default to 1× so Play relives the spoken moment with
      // real audio; a silent frame-only activity keeps the 8× timelapse default.
      speed = t.length > 0 ? 1 : 8;
    },
  });
  let loadGen = 0; // bumped per activity load; a stale strip fetch drops
  let rafId: number | null = null;
  let lastTs = 0;
  let frameAccum = 0;
  let trackEl = $state<HTMLDivElement | null>(null);
  let filmEl = $state<HTMLDivElement | null>(null);
  let scrubbing = false;
  let wasPlaying = false; // audio play state captured at scrub start; resume on release iff true
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
  const viewState = $derived(
    receiptViewState(strip.length, audioEvidence.length, turnsPending, turns.length),
  );
  const isAudioOnly = $derived(viewState === "audio-only");

  const stripMs = $derived(strip.map((f) => f.ms));
  const selectedTurn = $derived(turns.find((t) => t.key === selectedKey) ?? null);
  const selOrdinal = $derived(selectedKey == null ? 0 : turns.findIndex((t) => t.key === selectedKey) + 1);

  // 1× relives the selected turn (its real audio clocks the frames); 2×/8×/16×
  // is a silent frame timelapse. Audio-only is always audible.
  const audible = $derived(isAudioOnly || isAudibleSpeed(speed));
  const clipActive = $derived(activeClipId != null); // a segment's audio is loaded
  const isPlaying = $derived(audible ? clipPlaying : playing);
  const relivingClip = $derived(audible && selectedTurn != null && clipActive);

  // The wall-clock playhead: while an audio-only clip plays, the live audio head
  // (so consecutive same-segment turns light up in sequence, same as frames mode);
  // idle audio-only falls back to the selected turn's start; frames mode uses the
  // current frame (which the clip drives via onAudioTimeUpdate while reliving).
  const headMs = $derived(
    isAudioOnly ? (clipHeadMs ?? selectedTurn?.startMs ?? null) : currentMs,
  );
  const headPos = $derived(headMs == null ? 0 : posFor(headMs));
  const headClock = $derived(headMs == null ? "" : clock(headMs));

  // The transcript row to light up: the turn under the playhead. While a clip
  // plays, restrict to that segment's turns so consecutive same-segment turns
  // light up in sequence as their audio plays (and an overlapping mic/system turn
  // never steals the highlight); otherwise track across every turn — idle at the
  // poster, scrubbing, or the silent timelapse.
  const clipTurns = $derived(activeClipId == null ? turns : turns.filter((t) => t.audioSegmentId === activeClipId));
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
  const segmentCount = $derived(countCaptureSegments(strip.map((f) => f.ms)));

  // Wall-clock formatters (clock/clockSec/clockShort) live in receipt-clock.ts.
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

  // Play/Pause routes to whichever clock owns the surface: the <audio> element
  // when audible (audio-only, or 1× reliving) — starting the selected turn's clip
  // if none is loaded — else the silent frame timelapse.
  function togglePlay(): void {
    if (audible) {
      if (!audioEl) return;
      if (activeClipId == null) {
        // Start under the pill, not the segment head: seek to the current head
        // (poster frame / selected turn) so audio and playhead share one clock.
        if (selectedTurn) void playClip(selectedTurn, headMs ?? undefined);
        return;
      }
      audioEl.paused ? void audioEl.play().catch(() => {}) : audioEl.pause();
      return;
    }
    if (strip.length > 0) playing ? pause() : play();
  }

  function seek(i: number): void {
    if (strip.length === 0) return; // audio-only: no frames to move, don't halt the clip
    stopClip(); // a manual frame move preempts the audio clock
    pause();
    index = clampIndex(i, strip.length);
  }

  function step(delta: number): void {
    if (strip.length === 0) return; // audio-only: arrows have no frame to step, don't halt the clip
    stopClip();
    pause();
    index = clampIndex(index + delta, strip.length);
  }

  // ── Selection = play (ADR 0049): a lane block and its transcript row are one
  // selection; clicking either plays that spoken moment at 1×. ────────────────
  function onSelect(key: string): void {
    selectedKey = key;
    const turn = turns.find((t) => t.key === key);
    if (!turn) return;
    speed = 1;
    void playClip(turn);
  }

  function onSpeedChange(v: string): void {
    speed = Number(v) as Speed;
    if (isAudibleSpeed(speed) && selectedTurn) void playClip(selectedTurn);
    else stopClip(); // silent timelapse; leave paused
  }

  // ── Bounded, synchronized audio+screen clip (ADR 0049) ───────────────
  // Play the selected turn's segment audio at 1×; on each timeupdate the frame
  // viewer jumps to the strip frame at/just-before the audio's wall-clock
  // position, so one playhead drives both. It plays the segment through, then
  // auto-advances to the next (onended). Pass `seekToMs` to start at a chosen
  // wall-clock instant within the segment (scrub-bar click) instead of its head.
  async function playClip(turn: TurnView, seekToMs?: number, autoplay = true): Promise<void> {
    if (!audioEl) return;
    pause(); // stop the rAF timelapse; the audio clocks from here
    // 0 when seekToMs falls outside this turn's segment window — e.g. the frame
    // playhead in a DIFFERENT segment when Play is pressed — so an out-of-segment
    // head can't seek the clip past its end and auto-advance-skip the turn.
    const offsetSec = clipStartOffsetSec(turn, seekToMs);
    // Same segment already loaded → seek in place. Reassigning an identical data:
    // URL can reset readyState to 0 in WKWebView and re-arm the metadata-defer
    // path (the audio never moves), so a same-segment re-seek must NOT touch src.
    if (activeClipId === turn.audioSegmentId && audioEl.src) {
      scheduleClipSeek(audioEl, offsetSec);
      if (autoplay) void audioEl.play().catch(() => {});
      return;
    }
    clipHeadMs = null; // fall back to the new turn's start until its audio plays
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
    clipHeadMs = targetMs; // live head for the audio-only highlight (frames mode ignores it)
    if (stripMs.length > 0) index = frameIndexForMs(stripMs, targetMs);
  }

  // Clip finished: auto-advance to the next segment's clip so the whole span
  // plays continuously (ADR 0049 amendment 2026-07-06); stop at the last
  // segment. A manual pause fires onpause, not onended, so pausing never
  // auto-advances.
  function onAudioEnded(): void {
    clipPlaying = false;
    const next = nextClipTurn(turns, activeClipId);
    if (!next) return; // last segment — stop at the span's end
    selectedKey = next.key;
    void playClip(next);
  }

  // Stop any running clip and drop an in-flight media fetch (new activity, a
  // manual frame move, or a switch to a silent speed preempts the audio clock).
  function stopClip(): void {
    clipToken++;
    audioEl?.pause();
    clipPlaying = false;
    activeClipId = null;
    clipHeadMs = null;
  }

  // Hydrate the span's spoken turns for the current activity.
  function loadAudio(): void {
    stopClip();
    turns = [];
    turnsPending = true;
    selectedKey = null;
    void audioLoader.loadSpan(activity.startedAtMs, activity.endedAtMs, audioEvidence);
  }
  const speedOptions = SPEEDS.map((s) => ({ value: String(s), label: `${s}×` }));

  // ── Scrubbing (frames follow the pointer; on release the audio lands there) ──
  function scrubToClientX(clientX: number): void {
    const el = trackEl;
    if (!el || strip.length === 0) return;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    index = Math.round(frac * (strip.length - 1));
  }
  // Scrub-bar x → the wall-clock instant it points at over the activity span.
  function msForClientX(clientX: number): number | null {
    const el = trackEl;
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return activity.startedAtMs + frac * (activity.endedAtMs - activity.startedAtMs);
  }
  // Release on the timeline lands the audio at that instant: load the segment
  // covering it and play from the chosen offset (ADR 0049 amendment). A release
  // over a silent gap clears the clip — the frame scrub already moved there.
  function seekAudioAt(clientX: number, shouldPlay: boolean): void {
    if (turns.length === 0) return;
    const ms = msForClientX(clientX);
    if (ms == null) return;
    const turn = turnAtMs(turns, ms);
    if (!turn) { stopClip(); return; } // released over a silent gap — the frame already moved
    selectedKey = turn.key;
    speed = 1;
    void playClip(turn, ms, shouldPlay); // seek there; resume playback only if it was playing
  }
  function onTrackPointerDown(e: PointerEvent): void {
    if (strip.length === 0) return; // audio-only track stays a read-only playhead
    wasPlaying = !!audioEl && !audioEl.paused; // resume on release only if we interrupt playback
    pause(); // stop the silent rAF timelapse
    audioEl?.pause(); // silence the running clip while dragging; release re-lands it
    scrubbing = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrubToClientX(e.clientX); // frame/pill jump to the pressed instant (a click is a 0-length drag)
  }
  function onTrackPointerMove(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubToClientX(e.clientX); // frame/pill follow the cursor
  }
  function onTrackPointerUp(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* already released */ }
    seekAudioAt(e.clientX, wasPlaying); // land audio at the released instant; resume iff it was playing
  }
  function onTrackPointerCancel(e: PointerEvent): void {
    if (!scrubbing) return;
    scrubbing = false;
    try { (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId); } catch { /* already released */ }
    // Cancelled (gesture stolen): keep the scrubbed frame where it landed, never resume.
  }

  // ── Open in Timeline handoff (frontend-only, no backend command) ─────
  // Hand off the current frame; for an audio-only receipt (no frame) hand off
  // the selected spoken segment instead.
  function openInTimeline(): void {
    const audioSegmentId = selectedTurn?.audioSegmentId ?? null;
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
  // Reload the strip AND re-hydrate the spoken turns when the activity changes
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
  // while the receipt is mounted.
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

  // Cancel any dangling rAF, stop the clip audio, drop the thumb observer, and
  // invalidate any in-flight span hydration so a late onTurns can't fire into
  // the unmounted component.
  $effect(() => () => {
    if (rafId != null) cancelAnimationFrame(rafId);
    audioEl?.pause();
    audioLoader.reset();
    thumbObserver?.disconnect();
  });
</script>

<!-- The receipt is a floating SHEET, so it may wear material — and every region
     inside it that carries information (stage, scrub, filmstrip, transcript) is
     an opaque plate. That is the direction's whole rule stated in one
     component. -->
<div class="dim" role="presentation" onpointerdown={onBackdropPointerDown}>
  <div
    class="rsheet"
    role="dialog"
    aria-modal="true"
    aria-label={`Activity receipt: ${activity.title}`}
  >
    <div class="rhead">
      <span class="cchip">
        <em style="background:var({catColorVar})"></em>{catLabel}
      </span>
      <h2 class="rhead__t" use:tip={activity.title}>{activity.title}</h2>
      <span class="t-meta is-num rhead__when">{rangeLabel}</span>
      <button
        type="button"
        class="btn btn--ghost btn--icon btn--sm rhead__x"
        aria-label="Close receipt"
        onclick={onClose}><IconClose /></button
      >
    </div>

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

    <!-- The compact journal rows show no summary, so the receipt is where the
         description lives; it also survives footage expiry (ADR 0029). Prose,
         so it lands on a plate, never on the sheet's material. -->
    {#if activity.summary}
      <p class="plate rsummary">{activity.summary}</p>
    {/if}

    <div class="rbody" class:rbody--wide={turns.length === 0}>
      <div class="rleft">
        <!-- The stage (loading / expired / audio-only / frame) — the one elastic
             region; ReceiptViewer.svelte owns its markup + styles. -->
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
          <!-- Evidence ticks live ON the axis; the playhead carries the clock. -->
          <div class="plate scrub">
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
              onpointercancel={onTrackPointerCancel}
            >
              {#if !isAudioOnly}
                <div class="fill" class:fill--audio={clipActive} style="width:{currentPos * 100}%"></div>
                <!-- Ticks paint AFTER the fill: an evidence mark the playhead has
                     already passed must still be visible. -->
                {#each ticks as t, i (i)}
                  <span class="ev" class:ev--hl={t.headline} style="left:{t.pos * 100}%"></span>
                {/each}
              {/if}
              <div class="head is-num" class:head--audio={isAudioOnly || clipActive} style="left:{headPos * 100}%">{headClock}</div>
            </div>
            <div class="scrub-caps">
              <span class="is-num">{clock(activity.startedAtMs)}</span>
              <span class="is-num">{clock(activity.endedAtMs)}</span>
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
        {/if}
      </div>

      <!-- Prose, so an opaque plate — never the sheet's material. The active row
           tracks the playhead; clicking one relives that spoken moment at 1×. -->
      {#if turns.length > 0}
        <ReceiptTranscript {turns} selectedKey={activeKey} {onSelect} clock={clockShort} />
      {/if}
    </div>

    {#if !loading && viewState !== "expired"}
      <div class="rctrl">
        <button
          type="button"
          class="btn btn--icon"
          class:play--audio={audible}
          aria-label={isPlaying ? "Pause" : "Play"}
          disabled={isAudioOnly && selectedTurn == null}
          onclick={togglePlay}
        >{#if isPlaying}<IconPause />{:else}<IconPlay />{/if}</button>
        {#if !isAudioOnly}
          <Segmented
            options={speedOptions}
            value={String(speed)}
            onValueChange={onSpeedChange}
            ariaLabel="Playback speed"
            compact
          />
        {/if}
        <span class="t-meta is-mono is-num counter">
          {#if isAudioOnly}
            spoken turn {selOrdinal} / {turns.length}{#if headClock} · {headClock}{/if}
          {:else if relivingClip && selectedTurn}
            <span class="counter__now">{currentMs != null ? clockSec(currentMs) : ""}</span> · {selectedTurn.speaker} · 1×
          {:else}
            <span class="counter__now">frame {index + 1}</span> / {strip.length}{#if currentMs != null} · {clockSec(currentMs)}{/if}
          {/if}
        </span>
        <button type="button" class="btn btn--ghost btn--sm open-tl" onclick={openInTimeline}
          >Open in Timeline <IconArrowRight /></button
        >
      </div>
    {/if}

    <div class="rfoot">
      <span class="t-meta is-num">
        {#if loading}
          Loading footage…
        {:else if viewState === "expired"}
          0 frames still on disk · summary retained
        {:else if isAudioOnly}
          {audioFooterLeft(frameEvidence.length)} · {turnSpeakerRoster(turns)}
        {:else}
          {strip.length}
          {strip.length === 1 ? "frame" : "frames"} across {segmentCount} capture
          {segmentCount === 1 ? "segment" : "segments"} · {frameEvidence.length} frames
          + {audioEvidence.length} spoken segments cited
        {/if}
      </span>
      <span class="t-meta rfoot__keys">Esc closes · ←/→ steps a frame · Space plays</span>
    </div>
  </div>
</div>

<style>
  /* One selector per line to keep this component under the 800-line ceiling
     (repo rule). Page 08: the receipt is a floating glass SHEET over a dimmed
     pane, and every content region inside it is an opaque `.plate`. Audio
     channel (ADR 0049) uses --cat-communication (lavender) for voice. */
  /* The sheet floats over a dimmed PANE — the title bar stays lit, because the
     chrome is not what you are reading past. */
  .dim { position: fixed; inset: var(--h-titlebar) 0 0 0; z-index: 2000; display: grid; place-items: center; padding: 16px; background: rgba(12, 12, 18, 0.44); -webkit-backdrop-filter: blur(2px); backdrop-filter: blur(2px); }
  .rsheet { width: min(900px, 94vw); height: min(610px, 92vh); padding: 12px; display: flex; flex-direction: column; gap: 10px; border-radius: 16px; background: var(--glass-hud); -webkit-backdrop-filter: var(--glass-blur); backdrop-filter: var(--glass-blur); box-shadow: var(--sh-hud), inset 0 0 0 var(--hairline) var(--glass-line), inset 0 1px 0 var(--glass-hi); }

  /* Header — chip, title, span, close. Sits directly on the sheet: these are
     labels, not prose. */
  .rhead { flex: 0 0 auto; display: flex; align-items: center; gap: 10px; padding: 0 2px; }
  .cchip { flex: 0 0 auto; display: inline-flex; align-items: center; gap: 5px; height: 19px; padding: 0 8px; border-radius: var(--r-pill); background: var(--glass-tint); font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans); color: var(--app-text-muted); box-shadow: inset 0 0 0 var(--hairline) var(--glass-line); }
  .cchip em { width: 7px; height: 7px; border-radius: 2px; display: block; font-style: normal; }
  .rhead__t { flex: 1 1 auto; min-width: 0; margin: 0; font: var(--w-semi) var(--t-title) / 1.2 var(--app-font-sans); letter-spacing: var(--ls-title); color: var(--app-text-strong); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .rhead__when { flex: 0 0 auto; color: var(--app-text-subtle); white-space: nowrap; }
  .rhead__x { flex: 0 0 auto; }
  .rhead__x :global(svg) { width: 13px; height: 13px; }
  .rsummary { flex: 0 0 auto; margin: 0; padding: 8px 12px; font: var(--w-regular) var(--t-meta) / 1.55 var(--app-font-sans); color: var(--app-text-muted); }

  /* Body — stage column beside the transcript rail (the rail collapses when the
     span holds no spoken turns). */
  .rbody { flex: 1 1 auto; min-height: 0; display: grid; grid-template-columns: 1fr 268px; gap: 10px; }
  /* Grid items default to min-height:auto — without this the stage refuses to
     shrink and the filmstrip pushes the controls out of the sheet. */
  .rbody > :global(*) { min-height: 0; }
  .rbody--wide { grid-template-columns: 1fr; }
  .rleft { display: flex; flex-direction: column; gap: 8px; min-width: 0; min-height: 0; }

  /* Scrub — evidence ticks live ON the track; the playhead carries the clock. */
  .scrub { flex: 0 0 auto; padding: 28px 10px 8px; }
  .track { position: relative; height: 6px; border-radius: 3px; background: var(--app-surface-hover); cursor: pointer; touch-action: none; }
  .fill { position: absolute; left: 0; top: 0; bottom: 0; border-radius: 3px; background: var(--app-accent); opacity: 0.55; pointer-events: none; }
  .fill--audio { background: var(--cat-communication); opacity: 0.7; }
  .head { position: absolute; top: -24px; transform: translateX(-50%); display: inline-flex; align-items: center; height: 20px; padding: 0 8px; border-radius: var(--r-pill); background: var(--app-accent); color: var(--app-accent-contrast); font: var(--w-medium) var(--t-meta) / 1 var(--app-font-mono); white-space: nowrap; pointer-events: none; }
  .head--audio { background: var(--cat-communication); color: var(--app-bg); }
  .ev { position: absolute; top: -3px; width: 2px; height: 12px; border-radius: 1px; background: var(--chart-grey-4); pointer-events: none; }
  .ev--hl { top: -5px; height: 16px; background: var(--app-accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-accent) 20%, transparent); }
  .scrub-caps { display: flex; justify-content: space-between; margin-top: 7px; font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono); color: var(--app-text-faint); }

  /* Filmstrip — plates in a row; a scroll container's auto min-height is 0, so
     flex:0 0 auto pins it to its natural height instead of getting crushed. */
  .film { flex: 0 0 auto; display: grid; grid-template-rows: 52px; grid-auto-flow: column; grid-auto-columns: calc((100% - 42px) / 8); gap: 6px; overflow-x: auto; overflow-y: hidden; scrollbar-width: none; }
  .film__cell { position: relative; padding: 0; border: 0; border-radius: 6px; overflow: hidden; cursor: pointer; background: var(--app-surface-subtle); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2); }
  .film__img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .film__cell.cited::after { content: ""; position: absolute; inset: 0; border-radius: 6px; box-shadow: inset 0 0 0 1.5px var(--app-accent); }
  .film__cell.cur::after { content: ""; position: absolute; inset: 0; border-radius: 6px; box-shadow: inset 0 0 0 2px var(--app-text-strong); }

  /* Controls + footer sit on the sheet: machine labels, not prose. */
  .rctrl { flex: 0 0 auto; display: flex; align-items: center; gap: 10px; padding: 0 2px; }
  .rctrl .play--audio { color: var(--cat-communication); }
  .rctrl :global(svg) { width: 14px; height: 14px; }
  .counter { color: var(--app-text-muted); }
  .counter__now { color: var(--app-text-strong); }
  .open-tl { margin-left: auto; }
  .rfoot { flex: 0 0 auto; display: flex; align-items: center; gap: 8px; padding: 0 4px; }
  .rfoot :global(.t-meta) { color: var(--app-text-subtle); }
  .rfoot__keys { margin-left: auto; }
</style>
