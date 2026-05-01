<script lang="ts">
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Calendar } from "bits-ui";
  import {
    CalendarDate,
    type DateValue,
  } from "@internationalized/date";
  import { developerOptions } from "$lib/developer-options.svelte";
  import type {
    AudioSegmentDto,
    AudioSegmentMediaDto,
    FrameDto,
    FramePreviewDto,
    FrameRangeRequest,
    FrameSummaryDto,
    ListAudioSegmentsRequest,
    ListFramesRequest,
  } from "$lib/types";

  // ─── Timeline browser ─────────────────────────────────────────────────────
  // Scroll-driven, frame-by-frame browser backed by `list_frames` pagination
  // across ALL sessions (no session anchoring). The horizontal rail uses
  // fixed-width slots so the active frame can be derived purely from the
  // rail's scroll position. New pages are fetched as the user nears the end
  // of what's been loaded — no need to load all frames up front.
  //
  // Backend `list_frames` returns newest-first; we page using `beforeId`
  // (smallest id seen) so pagination stays stable even as new frames arrive
  // at the head between pages. Preview pixels come from
  // `get_frame_preview`; decoded data URLs are cached in-memory for the
  // lifetime of the page.
  //
  // The rail is presented with the NEWEST frame anchored to the right edge
  // and older frames flowing leftward, per design. Implementation keeps the
  // rail in normal LTR direction and positions each slot with `right: i *
  // SLOT_WIDTH` against the track. To advance toward older frames the user
  // scrolls leftward (scrollLeft decreases). Track has symmetric viewport-
  // sized margins so the active frame at the static center caret maps to
  // `idx = (maxScrollLeft - scrollLeft) / SLOT_WIDTH`. This avoids relying
  // on any browser-specific RTL `scrollLeft` convention.

  const TIMELINE_SLOT_WIDTH = 8; // px, must match CSS `.timeline-rail__slot`
  const TIMELINE_PAGE_SIZE = 100;
  // Distance (in frames) from the loaded tail at which we trigger the next
  // `beforeId` page. Sized generously relative to `TIMELINE_PAGE_SIZE` so a
  // fast scrub doesn't visibly stall at the temporary tail before the next
  // page lands. After a page completes we re-check this threshold and chain
  // another load if the user is still inside it (see `loadTimelinePage`'s
  // tail-prefetch follow-up below). `timelineExhausted` continues to gate
  // pagination at the true end.
  const TIMELINE_PREFETCH_AHEAD = 60;
  // Realtime head poll: how often we ask the backend for the newest page so
  // freshly captured frames appear in the rail without a manual refresh, and
  // how many frames we ask for per poll. The page-size cap is intentionally
  // small — we only need to catch up the head, not re-page history — but if
  // a single interval produces more than `TIMELINE_POLL_PAGE_SIZE` new frames
  // we walk older pages with `beforeId` until we reach the current head. The
  // page budget below bounds that walk so a backend that never returns the
  // current head can't spin the poll forever.
  const TIMELINE_POLL_INTERVAL_MS = 1500;
  const TIMELINE_POLL_PAGE_SIZE = 50;
  // Padding (ms) added to the loaded frame time-window when querying the DB
  // for audio segments. Audio segments are emitted on their own cadence
  // (typically tens of seconds) and may straddle the edges of the loaded
  // frame window; padding the window keeps the lane bars visible at those
  // edges instead of being filtered out by an overly tight range query.
  const AUDIO_SEGMENT_RANGE_PADDING_MS = 60_000;
  // Safety cap on pages walked per poll while chasing the current head. At
  // 50 frames/page this catches up bursts of a few thousand frames between
  // polls. If the cap is hit before reaching the head we fall back to a full
  // reset rather than splice in a partial prefix that would leave a hole.
  const TIMELINE_POLL_PAGE_BUDGET = 20;
  // Pixel slack from the track's right edge within which we consider the user
  // to be "following newest" — i.e. close enough that a freshly arrived frame
  // should pull the rail along instead of being silently prepended.
  const TIMELINE_FOLLOW_SLACK_PX = 4;
  // Safety cap on the pages we'll auto-load while chasing a jump target so a
  // mis-typed range can't hang the UI on a runaway pagination loop.
  const TIMELINE_JUMP_PAGE_BUDGET = 500;
  // Render only the slots near the viewport plus a buffer on each side, so
  // large recordings don't tax the DOM. The track itself keeps its full width
  // so scroll-position-based active-index math is unaffected. The rail is
  // dense (8px ticks) so a generous buffer is cheap and avoids visible churn.
  const TIMELINE_VIEWPORT_BUFFER = 80;
  // Conservative fallback viewport width (px) used only on the very first
  // render before ResizeObserver measures the rail. Picked to comfortably
  // cover wide displays (~2560px) so the centered window includes every
  // visible slot from frame one. Once the observer fires, the real measured
  // width takes over. Bounded fallback keeps virtualization in effect — the
  // window is still half-of-this plus the fixed overscan buffer, never the
  // full frame list.
  const TIMELINE_FALLBACK_VIEWPORT_WIDTH = 2560;

  type AudioSegmentSource = "microphone" | "systemAudio";
  type AudioSegmentRecord = {
    id: number;
    source: AudioSegmentSource;
    sessionId: string;
    segmentIndex: number;
    fileName: string;
    filePath: string;
    startUnixMs: number;
    endUnixMs: number;
    durationSeconds: number;
  };

  let timelineFrames = $state<FrameDto[]>([]);
  let timelineActiveIndex = $state(0);
  let timelineLoading = $state(false);
  let timelineLoadingMore = $state(false);
  let timelineExhausted = $state(false);
  let timelineError = $state<string | null>(null);
  let timelineRail: HTMLDivElement | null = $state(null);
  // Current rail scrollLeft (LTR, always >= 0). The "advance" distance —
  // how far past slot 0 (newest) the user has scrolled toward older frames —
  // is `maxScrollLeft - scrollLeft` because slot 0 is anchored to the track's
  // right edge.
  let timelineScrollLeft = $state(0);
  let timelineViewportWidth = $state(0);
  // DB-sourced audio segments overlapping the loaded frame window. The DB is
  // the source of truth for the timeline lane: rows arrive from
  // `list_audio_segments` and are mapped to UI records keyed by row id. We
  // don't filter by screen/frame session — segments are placed by their
  // captured time range against the rail.
  let audioSegments = $state<AudioSegmentRecord[]>([]);
  let audioSegmentsLoading = $state(false);
  let audioSegmentsError = $state<string | null>(null);
  // Monotonic token to discard stale `list_audio_segments` responses when a
  // newer fetch supersedes one in flight (e.g. timeline reset, head poll).
  let audioSegmentsGeneration = 0;
  // Currently selected audio segment for the inline player. The user picks a
  // segment by clicking its bar in the timeline rail; the selection survives
  // refreshes as long as the segment row is still in `audioSegments`. If a
  // window/range refresh drops the row we clear the selection (see effect
  // below) so the player doesn't keep pointing at a stale path.
  let selectedAudioSegmentId = $state<number | null>(null);
  // Monotonic token used to discard stale `list_frames` responses. A reset
  // bumps this so any in-flight page request resolves into a no-op rather
  // than appending mismatched frames.
  let timelineGeneration = 0;

  // Decoded `data:` URLs keyed by frame id. Reactive so the rail re-renders as
  // previews stream in without any extra plumbing.
  let previewCache = $state<Map<number, string>>(new Map());
  // Tracks the in-flight requests so concurrent scrolls don't fan out a
  // request per slot per scroll tick for the same id.
  const previewInFlight = new Set<number>();

  const timelineActive = $derived(timelineFrames[timelineActiveIndex] ?? null);

  const audioSegmentCounts = $derived.by(() => ({
    microphone: audioSegments.filter((s) => s.source === "microphone").length,
    systemAudio: audioSegments.filter((s) => s.source === "systemAudio").length,
  }));
  const latestAudioSegment = $derived(audioSegments[audioSegments.length - 1] ?? null);
  const audioSummaryTitle = $derived(
    latestAudioSegment
      ? `Latest audio segment: ${audioSourceLabel(latestAudioSegment.source)} ${latestAudioSegment.segmentIndex} (${formatUnixMs(latestAudioSegment.startUnixMs)} – ${formatUnixMs(latestAudioSegment.endUnixMs)})`
      : audioSegmentsError ?? "No audio segments found in the loaded timeline range",
  );

  // Selected audio segment for the inline player. Resolved from the current
  // `audioSegments` list each render so the selection auto-clears when a
  // refresh drops the row (see `$effect` below). Audio media bytes are fetched
  // by id from Tauri so playback does not depend on asset protocol scope or on
  // accepting arbitrary frontend-provided file paths.
  const selectedAudioSegment = $derived(
    selectedAudioSegmentId == null
      ? null
      : audioSegments.find((s) => s.id === selectedAudioSegmentId) ?? null,
  );
  let selectedAudioSrc = $state<string | null>(null);
  let selectedAudioMediaLoading = $state(false);
  let selectedAudioMediaError = $state<string | null>(null);
  let selectedAudioMediaGeneration = 0;
  // Latest webview-side load error from the <audio> element, if any. Lets us
  // show an inline error instead of a silent broken player when decoded bytes
  // were returned but the webview still couldn't load/play them.
  let selectedAudioLoadError = $state<string | null>(null);
  $effect(() => {
    // Clear the prior error whenever the selected segment changes.
    void selectedAudioSegmentId;
    selectedAudioLoadError = null;
  });

  $effect(() => {
    const id = selectedAudioSegmentId;
    selectedAudioMediaGeneration += 1;
    const gen = selectedAudioMediaGeneration;
    selectedAudioSrc = null;
    selectedAudioMediaError = null;

    if (id == null) {
      selectedAudioMediaLoading = false;
      return;
    }

    selectedAudioMediaLoading = true;
    void loadSelectedAudioSegmentMedia(id, gen);
  });

  async function loadSelectedAudioSegmentMedia(id: number, gen: number): Promise<void> {
    try {
      const media = await invoke<AudioSegmentMediaDto>("get_audio_segment_media", {
        request: { audioSegmentId: id },
      });
      if (gen !== selectedAudioMediaGeneration || selectedAudioSegmentId !== id) return;
      selectedAudioSrc = `data:${media.mimeType};base64,${media.dataBase64}`;
      selectedAudioMediaError = null;
    } catch (err) {
      if (gen !== selectedAudioMediaGeneration || selectedAudioSegmentId !== id) return;
      selectedAudioMediaError = typeof err === "string" ? err : JSON.stringify(err);
      selectedAudioSrc = null;
    } finally {
      if (gen === selectedAudioMediaGeneration && selectedAudioSegmentId === id) {
        selectedAudioMediaLoading = false;
      }
    }
  }

  function onSelectedAudioError() {
    selectedAudioLoadError =
      "Failed to play audio. The media bytes were loaded, but the browser could not decode this segment.";
  }

  // Drop a stale selection if the segment no longer appears in the loaded
  // window. We compare ids rather than object identity because `audioSegments`
  // is rebuilt on every refresh.
  $effect(() => {
    if (selectedAudioSegmentId == null) return;
    if (!audioSegments.some((s) => s.id === selectedAudioSegmentId)) {
      selectedAudioSegmentId = null;
    }
  });

  // ─── Audio overlay alignment ─────────────────────────────────────────────
  // Audio segment bars need to be **time-proportional** so a 60s system-audio
  // bar reads as ~6.7× wider than a 9s mic bar. The frame rail itself is
  // *frame-indexed*, not time-indexed: each frame occupies a fixed-width slot
  // regardless of capture cadence, and frame cadence is non-uniform (frames
  // are sampled by the OCR/processing pipeline based on activity, so dense
  // active periods get many frames while idle periods get few). Mapping
  // audio segments through fractional frame indices therefore makes bar
  // *width* track frame count, not real time — a 9s active-period segment
  // can render wider than a 60s idle-period segment.
  //
  // We instead derive a single global `pixelsPerMs` from the loaded frame
  // window (total inter-tick pixel distance divided by total wall-clock span)
  // and lay every segment out in real time off the newest frame's tick. Bars
  // perfectly align with frame ticks only when frame cadence is uniform; when
  // cadence varies the bar still represents real duration (which is what the
  // lane is for) and only the per-tick alignment drifts within the segment.
  //
  // `timelineFrames` is newest-first; capturedAt is an ISO-ish string. We
  // pre-compute a parallel array of millisecond timestamps so the alignment
  // helpers don't re-parse on every segment.
  const timelineFrameTimes = $derived.by<number[]>(() => {
    const out = new Array<number>(timelineFrames.length);
    for (let i = 0; i < timelineFrames.length; i++) {
      const f = timelineFrames[i];
      out[i] = f ? parseCapturedAt(f.capturedAt).getTime() : NaN;
    }
    return out;
  });

  /**
   * Pixels-per-millisecond used to size and position audio-segment bars in
   * real time. Returns `null` when the loaded window can't define a stable
   * ratio (zero/one frame, or all timestamps collapsed to the same instant);
   * callers fall back to the first/last frame spacing or to an empty lane.
   *
   * The denominator is the total time span between the newest and oldest
   * loaded frame (`times[0] - times[n-1]`); the numerator is the total
   * pixel distance between their tick centers (`(n - 1) * SLOT_WIDTH`).
   */
  const audioLanePixelsPerMs = $derived.by<number | null>(() => {
    const times = timelineFrameTimes;
    const n = times.length;
    if (n < 2) return null;
    const newest = times[0]!;
    const oldest = times[n - 1]!;
    const spanMs = newest - oldest;
    if (!Number.isFinite(spanMs) || spanMs <= 0) return null;
    return ((n - 1) * TIMELINE_SLOT_WIDTH) / spanMs;
  });

  type PositionedAudioSegment = AudioSegmentRecord & {
    /** Right offset in px (matches `.timeline-rail__slot`'s right offset). */
    rightPx: number;
    /** Width in px of the segment bar along the rail. */
    widthPx: number;
    /** Whether the segment overlaps the loaded frame window at all. */
    visible: boolean;
  };

  const positionedAudioSegments = $derived.by<PositionedAudioSegment[]>(() => {
    const times = timelineFrameTimes;
    const n = times.length;
    if (n === 0 || audioSegments.length === 0) return [];
    const newestMs = times[0]!;
    const oldestMs = times[n - 1]!;
    const pxPerMs = audioLanePixelsPerMs;
    if (pxPerMs == null) {
      // Single-frame (or zero-span) window: we can't pick a meaningful
      // pixels-per-ms. Render every overlapping segment as a hairline at
      // the newest tick so the lane still surfaces *that* a segment exists
      // without faking a duration.
      const halfSlot = TIMELINE_SLOT_WIDTH / 2;
      const out: PositionedAudioSegment[] = [];
      for (const seg of audioSegments) {
        out.push({ ...seg, rightPx: halfSlot, widthPx: 2, visible: true });
      }
      return out;
    }
    const halfSlot = TIMELINE_SLOT_WIDTH / 2;
    // Bars rendered to the left of newer frames: track right edge ↔ newest
    // frame's tick center sits at `right = halfSlot`. A segment's newer end
    // (segEndUnixMs) is offset (newestMs - segEndMs) ms older than newest,
    // i.e. `(newestMs - segEndMs) * pxPerMs` to the left of the newest tick
    // center. Width is the segment's full duration in pixels.
    //
    // Out-of-window segments are kept (no clamping) so a segment ending
    // after the newest loaded frame still gets a width matching its real
    // duration; the lane viewport's `overflow: hidden` clips any portion
    // that lies outside the visible track. Visibility filters segments
    // whose entire pixel extent is outside the visible track range so they
    // don't get rendered at all.
    const trackPxSpan = (n - 1) * TIMELINE_SLOT_WIDTH;
    const out: PositionedAudioSegment[] = [];
    for (const seg of audioSegments) {
      const widthMs = Math.max(0, seg.endUnixMs - seg.startUnixMs);
      const widthPx = Math.max(2, widthMs * pxPerMs);
      const rightPx = halfSlot + (newestMs - seg.endUnixMs) * pxPerMs;
      // Visible when the bar's [rightPx, rightPx + widthPx] interval
      // overlaps [halfSlot, halfSlot + trackPxSpan] (the tick-to-tick
      // span). Equivalent to checking the segment's time interval overlaps
      // the loaded window.
      const leftEdge = rightPx;
      const rightEdge = rightPx + widthPx;
      const trackLeft = halfSlot;
      const trackRight = halfSlot + trackPxSpan;
      const visible = !(rightEdge < trackLeft || leftEdge > trackRight) ||
        // Always show segments that fully envelop the loaded window.
        (seg.startUnixMs <= oldestMs && seg.endUnixMs >= newestMs);
      out.push({ ...seg, rightPx, widthPx, visible });
    }
    return out;
  });

  // Custom tooltip state for the rail. `hoveredFrameId` tracks which slot the
  // pointer is currently over; `hoveredX` is the pointer x relative to the
  // rail-wrap so the tooltip can follow the cursor. When nothing is hovered we
  // fall back to showing the active frame's tooltip pinned at the center
  // cursor, so the user always has a readable timestamp readout for the frame
  // they're parked on.
  let timelineRailWrap: HTMLDivElement | null = $state(null);
  let hoveredFrameId = $state<number | null>(null);
  let hoveredX = $state<number | null>(null);

  const tooltipFrame = $derived(
    hoveredFrameId != null
      ? (timelineFrames.find((f) => f.id === hoveredFrameId) ?? timelineActive)
      : timelineActive,
  );
  const tooltipIsHovered = $derived(
    hoveredFrameId != null && hoveredX != null,
  );

  // Maximum scrollLeft for the rail. Track width = N * SLOT; rail has
  // symmetric viewport-sized margins on each side (`50cqi - 4px`) so the
  // first/last slot can sit under the centered cursor. That makes the total
  // scrollable width equal to `N*SLOT + (V - 8)`, hence `maxScroll = N*SLOT - 8`.
  // Clamped non-negative for the empty/short-list case.
  const timelineMaxScroll = $derived(
    Math.max(0, timelineFrames.length * TIMELINE_SLOT_WIDTH - 8),
  );

  // Positive "advance" distance: how far the user has scrolled past slot 0
  // (the newest frame, anchored to the right edge) toward older frames.
  const timelineAdvance = $derived(
    Math.max(0, timelineMaxScroll - timelineScrollLeft),
  );

  // The active frame sits centered under the static cursor at the rail's
  // horizontal midpoint. The visible viewport therefore extends roughly
  // half a viewport's worth of slots on EACH side of the active index:
  // newer frames (lower indices) to the right of the cursor, older frames
  // (higher indices) to the left. Earlier revisions of this math computed
  // the window from the active index forward by a full viewport width,
  // which omitted the newer-side slots and left part of the rail visually
  // empty. Center the window on the active index, add buffer overscan on
  // each side, and clamp into [0, length].
  const timelineHalfViewportSlots = $derived(
    Math.ceil(
      (timelineViewportWidth > 0
        ? timelineViewportWidth
        : TIMELINE_FALLBACK_VIEWPORT_WIDTH) /
        2 /
        TIMELINE_SLOT_WIDTH,
    ),
  );
  const timelineWindowStart = $derived(
    Math.max(
      0,
      Math.floor(timelineAdvance / TIMELINE_SLOT_WIDTH) -
        timelineHalfViewportSlots -
        TIMELINE_VIEWPORT_BUFFER,
    ),
  );
  const timelineWindowEnd = $derived(
    Math.min(
      timelineFrames.length,
      Math.ceil(timelineAdvance / TIMELINE_SLOT_WIDTH) +
        timelineHalfViewportSlots +
        TIMELINE_VIEWPORT_BUFFER,
    ),
  );
  const timelineWindow = $derived(
    timelineFrames.slice(timelineWindowStart, timelineWindowEnd),
  );

  function parseCapturedAt(ts: string): Date {
    return new Date(ts.includes("T") ? ts : ts.replace(" ", "T"));
  }

  function formatCapturedAt(ts: string): string {
    const d = parseCapturedAt(ts);
    if (isNaN(d.getTime())) return ts;
    return d.toLocaleString();
  }

  function formatUnixMs(ms: number): string {
    const d = new Date(ms);
    if (isNaN(d.getTime())) return "unknown";
    return d.toLocaleString();
  }

  /** Compact `HH:MM:SS` (locale-aware) for the player panel header where the
   *  date is implicit from the surrounding timeline context. */
  function formatTimeOfDay(ms: number): string {
    const d = new Date(ms);
    if (isNaN(d.getTime())) return "—";
    return d.toLocaleTimeString();
  }

  function formatDurationSeconds(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "—";
    const total = Math.round(seconds);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function audioSourceLabel(source: AudioSegmentSource): string {
    return source === "microphone" ? "microphone" : "system audio";
  }

  function fileNameOf(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function mapAudioSegmentDto(dto: AudioSegmentDto): AudioSegmentRecord | null {
    const startUnixMs = parseCapturedAt(dto.startedAt).getTime();
    const endUnixMs = parseCapturedAt(dto.endedAt).getTime();
    if (!Number.isFinite(startUnixMs) || !Number.isFinite(endUnixMs)) return null;
    const source: AudioSegmentSource =
      dto.sourceKind === "microphone" ? "microphone" : "systemAudio";
    const durationSeconds = Math.max(0, (endUnixMs - startUnixMs) / 1_000);
    return {
      id: dto.id,
      source,
      sessionId: dto.sourceSessionId,
      segmentIndex: dto.segmentIndex,
      fileName: fileNameOf(dto.filePath),
      filePath: dto.filePath,
      startUnixMs,
      endUnixMs,
      durationSeconds,
    };
  }

  /**
   * Refresh the DB-sourced audio segment lane against the currently loaded
   * frame window. Uses the newest/oldest loaded `capturedAt` values as the
   * inclusive query range, padded by `AUDIO_SEGMENT_RANGE_PADDING_MS` on
   * each side so segments that start just before the oldest loaded frame
   * (or end just after the newest) still surface as bars at the rail edges.
   *
   * Concurrency: every call bumps the generation token so a newer request
   * always supersedes any in-flight one. Stale responses (whose `gen` no
   * longer matches `audioSegmentsGeneration`) are discarded without
   * touching state, so reset/scrub/window refreshes never get blocked by
   * an older request still resolving. Concurrent requests are allowed; the
   * latest gen is the only one that writes results or clears the loading
   * flag, so callers that fire repeatedly (e.g. head poll during a long
   * fetch) cannot leave stale rows on screen.
   */
  async function refreshAudioSegments(): Promise<void> {
    if (timelineFrames.length === 0) {
      audioSegmentsGeneration += 1;
      audioSegments = [];
      audioSegmentsError = null;
      audioSegmentsLoading = false;
      return;
    }
    const newest = timelineFrames[0];
    const oldest = timelineFrames[timelineFrames.length - 1];
    if (!newest || !oldest) return;
    const newestMs = parseCapturedAt(newest.capturedAt).getTime();
    const oldestMs = parseCapturedAt(oldest.capturedAt).getTime();
    if (!Number.isFinite(newestMs) || !Number.isFinite(oldestMs)) return;
    const startMs = Math.min(newestMs, oldestMs) - AUDIO_SEGMENT_RANGE_PADDING_MS;
    const endMs = Math.max(newestMs, oldestMs) + AUDIO_SEGMENT_RANGE_PADDING_MS;

    audioSegmentsGeneration += 1;
    const gen = audioSegmentsGeneration;
    audioSegmentsLoading = true;
    try {
      const request: ListAudioSegmentsRequest = {
        capturedAtStart: new Date(startMs).toISOString(),
        capturedAtEnd: new Date(endMs).toISOString(),
      };
      const dtos = await invoke<AudioSegmentDto[]>("list_audio_segments", { request });
      if (gen !== audioSegmentsGeneration) return;
      const mapped: AudioSegmentRecord[] = [];
      for (const dto of dtos) {
        const rec = mapAudioSegmentDto(dto);
        if (rec) mapped.push(rec);
      }
      mapped.sort((a, b) =>
        a.startUnixMs - b.startUnixMs ||
        a.source.localeCompare(b.source) ||
        a.segmentIndex - b.segmentIndex,
      );
      audioSegments = mapped;
      audioSegmentsError = null;
    } catch (err) {
      if (gen !== audioSegmentsGeneration) return;
      audioSegmentsError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      if (gen === audioSegmentsGeneration) {
        audioSegmentsLoading = false;
      }
    }
  }

  async function refreshTimelineAndDashboard(): Promise<void> {
    await loadTimelinePage(true);
  }

  /**
   * Fetch (and cache) a preview's `data:` URL for the given frame id. Multiple
   * concurrent callers for the same id collapse onto the in-flight request via
   * `previewInFlight`. Errors are swallowed so a single bad frame doesn't
   * break the whole rail; the slot simply renders without an image.
   */
  async function ensurePreview(frameId: number): Promise<void> {
    if (previewCache.has(frameId)) return;
    if (previewInFlight.has(frameId)) return;
    previewInFlight.add(frameId);
    try {
      const dto = await invoke<FramePreviewDto>("get_frame_preview", {
        request: { frameId },
      });
      const url = `data:${dto.mimeType};base64,${dto.dataBase64}`;
      // Reassign the Map so Svelte's reactivity picks the change up.
      const next = new Map(previewCache);
      next.set(frameId, url);
      previewCache = next;
    } catch {
      // Best-effort: leave the cache untouched so a retry on next render is possible.
    } finally {
      previewInFlight.delete(frameId);
    }
  }

  async function loadTimelinePage(reset = false) {
    // A reset must always be able to supersede an in-flight page request, so
    // only "load more" is gated on the loading flags. The generation token
    // below ensures the older response is discarded if a reset bumps it.
    if (!reset && (timelineLoading || timelineLoadingMore)) return;
    if (!reset && timelineExhausted) return;

    if (reset) {
      timelineGeneration += 1;
      timelineLoading = true;
      timelineLoadingMore = false;
      timelineExhausted = false;
    } else {
      timelineLoadingMore = true;
    }
    const gen = timelineGeneration;

    // No `sessionId` filter: this view spans every session, ordered newest
    // first by the backend.
    const request: ListFramesRequest = {
      limit: TIMELINE_PAGE_SIZE,
    };
    // For "load more", page using a stable cursor (`beforeId`) anchored to
    // the smallest id we've already loaded. This keeps pagination correct
    // even when new frames arrive at the head between requests, which would
    // shift offset-based windows. On reset, we omit the cursor to fetch the
    // newest page.
    if (!reset && timelineFrames.length > 0) {
      const tail = timelineFrames[timelineFrames.length - 1];
      if (tail) request.beforeId = tail.id;
    }

    try {
      const page = await invoke<FrameDto[]>("list_frames", { request });
      // A newer reset has superseded this request — drop the response.
      if (gen !== timelineGeneration) return;
      if (reset) {
        timelineFrames = page;
        timelineActiveIndex = 0;
        // Newest frame (slot 0) sits at the right edge of the track; scroll
        // all the way to the right so it's centered under the static cursor.
        // Wait for the DOM to lay out the new track before reading
        // scrollWidth, else we'd just set 0 → 0.
        await tick();
        if (timelineRail) {
          const max = timelineRail.scrollWidth - timelineRail.clientWidth;
          timelineRail.scrollLeft = max;
          timelineScrollLeft = max;
        } else {
          timelineScrollLeft = 0;
        }
        // Drop cached previews from any prior generation — keeping them
        // would grow unboundedly across refreshes.
        previewCache = new Map();
        // Invalidate the date-jump picker's month/day summary cache so
        // newly captured frames show up as available dates and times. The
        // picker effect that watches `pickerPlaceholder` will re-fetch the
        // visible month on the next render if the picker is open.
        summariesByDate = new Map();
        loadedMonths = new Set();
      } else {
        // Appending older frames grows the track to the LEFT of slot 0,
        // which increases `scrollWidth` (and therefore `maxScrollLeft`).
        // Active index is derived as `(maxScrollLeft - scrollLeft) /
        // SLOT_WIDTH`, so if we leave `scrollLeft` untouched the advance
        // distance silently grows and the active frame appears to shift
        // even though the user did not scrub. Capture the previous max,
        // append, then after layout shift `scrollLeft` by the same delta
        // so `(maxScrollLeft - scrollLeft)` — and thus the active index —
        // is preserved across the page load. This keeps wheel/keyboard/
        // click/date-jump math correct because all of them read the live
        // `scrollWidth - clientWidth` afterward.
        const prevMax = timelineRail
          ? timelineRail.scrollWidth - timelineRail.clientWidth
          : 0;
        timelineFrames = timelineFrames.concat(page);
        if (timelineRail) {
          await tick();
          const newMax = timelineRail.scrollWidth - timelineRail.clientWidth;
          const delta = newMax - prevMax;
          if (delta > 0) {
            timelineRail.scrollLeft += delta;
            timelineScrollLeft = timelineRail.scrollLeft;
          }
        }
      }
      if (page.length < TIMELINE_PAGE_SIZE) {
        timelineExhausted = true;
      }
      timelineError = null;
    } catch (err) {
      if (gen !== timelineGeneration) return;
      timelineError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      // Only the request that still owns the current generation should clear
      // the loading flags; otherwise a superseding reset's flags would be
      // wiped by the stale response's finally block.
      if (gen === timelineGeneration) {
        timelineLoading = false;
        timelineLoadingMore = false;
      }
    }

    // Refresh the DB-sourced audio segment lane whenever the loaded frame
    // window changes (reset → new range; load-more → wider/older range).
    // Fire-and-forget; the function guards its own concurrency and uses a
    // generation token to discard stale responses.
    void refreshAudioSegments();

    // Chain another page if the user is still scrubbing inside the prefetch
    // zone after this response landed. Without this, a fast scrub overruns
    // the just-loaded tail before the next request fires from `onscroll`,
    // and the rail visibly stalls at a temporary end. `timelineExhausted`
    // and the loading-flag guards inside `loadTimelinePage` keep this from
    // looping past the true end or stacking concurrent requests.
    if (
      !reset &&
      !timelineExhausted &&
      !timelineLoading &&
      !timelineLoadingMore &&
      timelineFrames.length - timelineActiveIndex <= TIMELINE_PREFETCH_AHEAD
    ) {
      // Fire-and-forget; the recursive call uses the same guards.
      loadTimelinePage(false);
    }
  }

  // ─── Realtime head poll ──────────────────────────────────────────────────
  // Periodically fetch the newest page and merge any frames whose ids are
  // greater than the current head into `timelineFrames`. If a single page
  // doesn't reach the current head (a burst arrived between polls), we walk
  // older pages using `beforeId` until we hit the head or exhaust the page
  // budget. We never overlap a poll with another head request, and we yield
  // to the manual refresh / load-more paths so they remain authoritative for
  // full resets and history pagination.
  //
  // The merge has three branches:
  //   1. Rail is empty → behave like the empty→populated half of a reset:
  //      seed the frames, scroll to the right edge, and invalidate the date
  //      picker's month summary cache so the new dates show as available.
  //      Preview cache is left alone (it's empty in this branch anyway), so
  //      the active-preview effect can hydrate it normally.
  //   2. User is following newest (active is slot 0 AND rail is at the right
  //      edge within a small slack) → keep them on the new newest by leaving
  //      activeIndex at 0 and snapping scrollLeft to the new max after layout.
  //   3. User has scrubbed away → preserve the frame they're viewing. Slot 0
  //      is anchored to the track's right edge, so prepending N frames adds
  //      `N * SLOT_WIDTH` of track on the LEFT. The user's frame moves from
  //      index `i` to `i + N`; new advance = (i+N)*SLOT, new maxScroll grew
  //      by N*SLOT, so the desired scrollLeft is unchanged. We re-assert
  //      scrollLeft after layout in case the browser adjusted it on its own.
  //
  // Whenever any fresh frames are merged we also invalidate the date picker
  // month/day summary caches — newly captured frames may belong to a date or
  // minute the picker had already cached as empty.
  let timelinePolling = false;
  async function pollTimelineHead(): Promise<void> {
    // Don't fight a reset, an in-flight load-more, or a date-jump page-walk.
    // The next interval tick will catch up; missing one poll is fine.
    if (timelinePolling) return;
    if (timelineLoading || timelineLoadingMore || pickerJumping) return;
    timelinePolling = true;
    const gen = timelineGeneration;
    try {
      const firstPage = await invoke<FrameDto[]>("list_frames", {
        request: { limit: TIMELINE_POLL_PAGE_SIZE },
      });
      // A reset since we issued the request: the rail's contents are about
      // to be replaced, so anything we'd merge here is already stale.
      if (gen !== timelineGeneration) return;
      if (firstPage.length === 0) return;

      if (timelineFrames.length === 0) {
        // Empty → populated. Mirror the relevant half of the reset path
        // (without resetting the generation or clearing the preview cache,
        // which is already empty here).
        timelineFrames = firstPage;
        timelineActiveIndex = 0;
        // Picker's month-availability map was computed against an empty
        // dataset; drop it so the newly-arrived dates light up.
        summariesByDate = new Map();
        loadedMonths = new Set();
        await tick();
        if (timelineRail) {
          const max = timelineRail.scrollWidth - timelineRail.clientWidth;
          timelineRail.scrollLeft = max;
          timelineScrollLeft = max;
        }
        // Empty → populated: kick off an audio segment fetch for the new
        // window. Subsequent prepends below also refresh after merging.
        void refreshAudioSegments();
        return;
      }

      const headId = timelineFrames[0]?.id ?? -1;
      // Walk pages newest→older using `beforeId` until we either reach the
      // current head, get a short page (no more new frames at all), or
      // exhaust the page budget. Each page is newest-first; we collect the
      // prefix whose ids exceed the current head and stop the moment we
      // observe an id we already have.
      const fresh: FrameDto[] = [];
      let reachedHead = false;
      let page = firstPage;
      let pagesWalked = 1;
      while (true) {
        let crossed = false;
        for (const f of page) {
          if (f.id <= headId) {
            crossed = true;
            break;
          }
          fresh.push(f);
        }
        if (crossed) {
          reachedHead = true;
          break;
        }
        // Whole page was new and we still haven't seen the head. Either the
        // backend has more new frames older than this page's tail, or we
        // happened to fetch exactly the new prefix and the next id is the
        // head. A short page rules out the former.
        if (page.length < TIMELINE_POLL_PAGE_SIZE) {
          // Backend has nothing older to give; the local head must already
          // be present in `fresh` or older than anything we received. Treat
          // this as catching up — fall through and merge.
          reachedHead = true;
          break;
        }
        if (pagesWalked >= TIMELINE_POLL_PAGE_BUDGET) break;
        const tail = page[page.length - 1];
        if (!tail) break;
        const nextPage = await invoke<FrameDto[]>("list_frames", {
          request: { limit: TIMELINE_POLL_PAGE_SIZE, beforeId: tail.id },
        });
        if (gen !== timelineGeneration) return;
        if (nextPage.length === 0) {
          reachedHead = true;
          break;
        }
        page = nextPage;
        pagesWalked += 1;
      }

      if (!reachedHead) {
        // Page budget exhausted before reaching the local head: splicing in
        // `fresh` would leave a hole between it and the existing frames.
        // Fall back to a full reset so the rail stays internally consistent.
        // This is rare (requires a sustained burst that outpaces the budget)
        // and is preferable to a silent inconsistency.
        await loadTimelinePage(true);
        return;
      }

      if (fresh.length === 0) return;

      const followingNewest =
        timelineActiveIndex === 0 &&
        timelineRail !== null &&
        timelineRail.scrollWidth - timelineRail.clientWidth - timelineRail.scrollLeft <=
          TIMELINE_FOLLOW_SLACK_PX;

      // Capture the frame the user is currently parked on so we can find it
      // again after the prepend and keep it under the cursor.
      const anchorFrame = !followingNewest ? timelineFrames[timelineActiveIndex] : null;
      const prevScrollLeft = timelineRail?.scrollLeft ?? 0;

      timelineFrames = fresh.concat(timelineFrames);

      // New frames may belong to a date/minute the picker had cached as
      // empty (or simply hadn't seen yet). Drop the cache so the next open
      // re-fetches the visible month and the new entries light up.
      summariesByDate = new Map();
      loadedMonths = new Set();

      await tick();
      if (!timelineRail) return;
      const newMax = timelineRail.scrollWidth - timelineRail.clientWidth;
      if (followingNewest) {
        // Stay glued to the right edge as new frames arrive.
        timelineActiveIndex = 0;
        timelineRail.scrollLeft = newMax;
        timelineScrollLeft = newMax;
      } else if (anchorFrame) {
        // Re-find the anchor and shift the active index so the same frame
        // stays selected. `findIndex` is robust to either the linear shift
        // (the common case) or any future merging logic that reorders.
        const newIdx = timelineFrames.findIndex((f) => f.id === anchorFrame.id);
        if (newIdx >= 0) {
          timelineActiveIndex = newIdx;
        }
        // The math above shows scrollLeft should be unchanged to keep the
        // anchor under the cursor; re-assert in case the browser nudged it
        // when scrollWidth grew.
        timelineRail.scrollLeft = prevScrollLeft;
        timelineScrollLeft = prevScrollLeft;
      }
      // Newly merged frames extend the loaded window; refresh the audio
      // lane so segments arriving alongside the new frames appear too.
      void refreshAudioSegments();
    } catch {
      // Swallow poll errors: the manual refresh button surfaces a real
      // error path, and a transient poll failure shouldn't take over the
      // existing `timelineError` slot.
    } finally {
      timelinePolling = false;
    }
  }

  // Drive the head poll on a timer for the lifetime of the page. The effect's
  // cleanup clears the interval when the route unmounts.
  $effect(() => {
    const id = setInterval(() => {
      void pollTimelineHead();
    }, TIMELINE_POLL_INTERVAL_MS);
    return () => clearInterval(id);
  });

  function onTimelineScroll(event: Event) {
    const el = event.currentTarget as HTMLDivElement;
    timelineScrollLeft = el.scrollLeft;
    const maxScroll = el.scrollWidth - el.clientWidth;
    const advance = Math.max(0, maxScroll - el.scrollLeft);
    const idx = Math.max(
      0,
      Math.min(
        timelineFrames.length - 1,
        Math.round(advance / TIMELINE_SLOT_WIDTH),
      ),
    );
    if (idx !== timelineActiveIndex) {
      timelineActiveIndex = idx;
    }
    // Lazy-fetch the next page once the user is within `PREFETCH_AHEAD` of
    // the tail of what's already loaded.
    if (
      !timelineExhausted &&
      !timelineLoadingMore &&
      timelineFrames.length - idx <= TIMELINE_PREFETCH_AHEAD
    ) {
      loadTimelinePage(false);
    }
  }

  // Translate wheel events anywhere on the page into horizontal scroll on the
  // rail so the user can scrub from anywhere — they don't have to find and
  // hover the (now very thin) tick rail. Both deltaY (mouse wheels, vertical
  // trackpad gestures) and deltaX (horizontal trackpad gestures) feed in.
  //
  // Bail out when a modifier is held: browsers/OS layers translate
  // ctrl/meta-wheel into pinch-zoom (and similar gestures), so swallowing the
  // event here would hijack zoom for the whole page. Letting those pass means
  // pinch-zoom keeps working while the unmodified scroll path still scrubs.
  //
  // Direction: a positive wheel delta means "advance through the timeline"
  // (toward older frames). Older frames live to the LEFT of slot 0 in the
  // rail, so advancing means scrollLeft -= delta.
  function onTimelineWheel(event: WheelEvent) {
    if (!timelineRail) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    // Don't hijack wheel events that originate inside the date/time picker
    // popover — its calendar and scrollable time list need normal vertical
    // scrolling. The picker is rendered inside the same `<section>` that owns
    // this listener, so without this guard wheeling over the time list would
    // scrub the timeline instead of scrolling the list.
    const target = event.target;
    if (target instanceof Element && target.closest(".timeline__picker")) {
      return;
    }
    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY)
      ? event.deltaX
      : event.deltaY;
    if (delta === 0) return;
    event.preventDefault();
    timelineRail.scrollLeft -= delta;
  }

  // Keyboard scrubbing on the rail. Treat the rail as a slider so screen
  // readers expose position semantics and arrow keys move one frame at a
  // time. Home/End/PageUp/PageDown match common slider conventions.
  //
  // Slot 0 (newest) sits at the right edge; older frames extend leftward.
  // ArrowLeft therefore moves toward older frames (positive delta on the
  // index), ArrowRight toward newer.
  function onTimelineKeyDown(event: KeyboardEvent) {
    if (timelineFrames.length === 0) return;
    let handled = true;
    switch (event.key) {
      case "ArrowLeft":
        timelineJump(1);
        break;
      case "ArrowRight":
        timelineJump(-1);
        break;
      case "PageUp":
        timelineJump(-10);
        break;
      case "PageDown":
        timelineJump(10);
        break;
      case "Home":
        timelineJump(-timelineFrames.length);
        break;
      case "End":
        timelineJump(timelineFrames.length);
        break;
      default:
        handled = false;
    }
    if (handled) event.preventDefault();
  }

  function timelineJump(delta: number) {
    if (!timelineRail || timelineFrames.length === 0) return;
    const target = Math.max(
      0,
      Math.min(timelineFrames.length - 1, timelineActiveIndex + delta),
    );
    const max = timelineRail.scrollWidth - timelineRail.clientWidth;
    timelineRail.scrollTo({
      left: max - target * TIMELINE_SLOT_WIDTH,
      behavior: "smooth",
    });
  }

  // Click-to-seek on the rail. Ticks themselves are presentational so the
  // slider role on the rail isn't polluted by focusable descendants; instead
  // we map the click position back to a frame index here.
  //
  // The static cursor caret sits at the rail's horizontal midpoint, and the
  // active frame is whichever slot is currently centered under it. A click
  // at viewport offset X means the user wants the slot under X to become
  // active. The horizontal offset from the caret in slot units gives the
  // index delta from the current active index — older frames live to the
  // left of the caret (positive delta).
  function onTimelineRailClick(event: MouseEvent) {
    if (!timelineRail || timelineFrames.length === 0) return;
    const rect = timelineRail.getBoundingClientRect();
    const clickX = event.clientX - rect.left;
    const caretX = rect.width / 2;
    const offset = Math.round((caretX - clickX) / TIMELINE_SLOT_WIDTH);
    const idx = Math.max(
      0,
      Math.min(timelineFrames.length - 1, timelineActiveIndex + offset),
    );
    const max = timelineRail.scrollWidth - timelineRail.clientWidth;
    timelineRail.scrollTo({
      left: max - idx * TIMELINE_SLOT_WIDTH,
      behavior: "smooth",
    });
    // Move keyboard focus onto the rail so subsequent arrow-key scrubbing
    // works without the user having to Tab back to it.
    timelineRail.focus({ preventScroll: true });
  }

  // Tooltip pointer wiring. Slot `pointerenter` claims the hovered frame id;
  // `pointermove` (delegated on the rail) keeps the tooltip glued to the
  // cursor; `pointerleave` on the rail clears it so the tooltip falls back to
  // the active frame at the center cursor.
  function onSlotPointerEnter(event: PointerEvent, frameId: number) {
    hoveredFrameId = frameId;
    if (timelineRailWrap) {
      const rect = timelineRailWrap.getBoundingClientRect();
      hoveredX = event.clientX - rect.left;
    }
  }
  function onTimelineRailPointerMove(event: PointerEvent) {
    if (hoveredFrameId == null || !timelineRailWrap) return;
    const rect = timelineRailWrap.getBoundingClientRect();
    hoveredX = event.clientX - rect.left;
  }
  function onTimelineRailPointerLeave() {
    hoveredFrameId = null;
    hoveredX = null;
  }

  // Audio segment bar selection. Clicks/keydowns on bars stop propagation so
  // the rail's scrub click and arrow-key handlers don't also fire — the bar
  // selects a segment for the inline player while the rail still owns frame
  // navigation everywhere else.
  function onAudioSegmentBarClick(event: MouseEvent, id: number) {
    event.stopPropagation();
    selectedAudioSegmentId = selectedAudioSegmentId === id ? null : id;
  }
  function onAudioSegmentBarKeyDown(event: KeyboardEvent) {
    // Buttons activate on Enter/Space natively; we just need to keep the
    // rail's arrow-key scrubber from also reacting when focus is on a bar.
    if (
      event.key === "ArrowLeft" ||
      event.key === "ArrowRight" ||
      event.key === "ArrowUp" ||
      event.key === "ArrowDown" ||
      event.key === "Home" ||
      event.key === "End" ||
      event.key === "PageUp" ||
      event.key === "PageDown" ||
      event.key === "Enter" ||
      event.key === " "
    ) {
      event.stopPropagation();
    }
  }

  // One-shot initial load. No session bootstrap — the timeline browses all
  // frames across every session, newest first. Audio segments are refreshed
  // as part of `loadTimelinePage`.
  let timelineInitialized = false;
  $effect(() => {
    if (timelineInitialized) return;
    timelineInitialized = true;
    void loadTimelinePage(true);
  });

  // Track the rail's viewport width so the windowing window stays correct
  // across resizes. Only the slots near the viewport are rendered.
  $effect(() => {
    const el = timelineRail;
    if (!el) return;
    timelineViewportWidth = el.clientWidth;
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        timelineViewportWidth = entry.contentRect.width;
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Eagerly fetch a preview for whichever frame is currently active so the
  // big preview stage updates promptly on scrub.
  $effect(() => {
    const active = timelineActive;
    if (active) ensurePreview(active.id);
  });

  // ─── Date / time jump picker ──────────────────────────────────────────────
  // A custom Bits UI calendar + time list that lets the user jump the
  // timeline to a specific local date (and optionally a specific minute).
  // Strategy:
  //   - Frame summaries (id + capturedAt) are loaded per visible calendar
  //     month and grouped by LOCAL date. The calendar disables dates with
  //     no frames in months we've already loaded.
  //   - When a date is selected we expose the available minute-buckets for
  //     that day; the user can pick the latest frame of the day, or a
  //     specific minute. Either way we delegate the "latest at or before"
  //     resolution to `get_latest_frame_in_range` so the backend remains
  //     the source of truth for the jump target.
  //   - After resolving the target we page `list_frames` (older direction,
  //     using the existing `beforeId` cursor) until the target frame is
  //     present locally, then scroll the rail to its index. This keeps the
  //     preview/rail in sync with the picker without a parallel data path.

  type DateKey = string; // "YYYY-MM-DD" in local time
  type MonthKey = string; // "YYYY-MM" in local time

  let pickerOpen = $state(false);
  let pickerPlaceholder = $state<DateValue>(todayLocal());
  let pickerSelectedDate = $state<DateValue | undefined>(undefined);
  let pickerSelectedTime = $state<string | null>(null); // "HH:MM"
  let summariesByDate = $state<Map<DateKey, FrameSummaryDto[]>>(new Map());
  let loadedMonths = $state<Set<MonthKey>>(new Set());
  let pickerLoading = $state(false);
  let pickerJumping = $state(false);
  let pickerError = $state<string | null>(null);

  function todayLocal(): DateValue {
    const d = new Date();
    return new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate());
  }

  function pad2(n: number): string {
    return String(n).padStart(2, "0");
  }

  function dateKeyOf(d: { year: number; month: number; day: number }): DateKey {
    return `${d.year}-${pad2(d.month)}-${pad2(d.day)}`;
  }

  function monthKeyOf(d: { year: number; month: number }): MonthKey {
    return `${d.year}-${pad2(d.month)}`;
  }

  function localDateKeyFromTs(ts: string): DateKey {
    const d = parseCapturedAt(ts);
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  }

  async function loadMonthSummaries(value: DateValue): Promise<void> {
    const key = monthKeyOf(value);
    if (loadedMonths.has(key)) return;
    pickerLoading = true;
    try {
      // Local month bounds, converted to UTC ISO for the backend.
      const start = new Date(value.year, value.month - 1, 1, 0, 0, 0, 0);
      const end = new Date(value.year, value.month, 1, 0, 0, 0, 0);
      const req: FrameRangeRequest = {
        capturedAtStart: start.toISOString(),
        capturedAtEnd: end.toISOString(),
      };
      const summaries = await invoke<FrameSummaryDto[]>(
        "list_frame_summaries_in_range",
        { request: req },
      );
      const next = new Map(summariesByDate);
      // Drop any prior entries whose local date falls inside this month so
      // a re-load replaces rather than duplicates rows.
      for (const k of Array.from(next.keys())) {
        if (k.startsWith(`${key}-`)) next.delete(k);
      }
      for (const s of summaries) {
        const k = localDateKeyFromTs(s.capturedAt);
        const arr = next.get(k);
        if (arr) arr.push(s);
        else next.set(k, [s]);
      }
      // Ascending by capture time within each day so minute buckets resolve
      // their "latest in bucket" by simple last-write-wins below.
      for (const arr of next.values()) {
        arr.sort((a, b) => a.capturedAt.localeCompare(b.capturedAt));
      }
      summariesByDate = next;
      const nextMonths = new Set(loadedMonths);
      nextMonths.add(key);
      loadedMonths = nextMonths;
      pickerError = null;
    } catch (err) {
      pickerError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      pickerLoading = false;
    }
  }

  // Eagerly fetch the visible month whenever the placeholder lands on a new
  // month while the picker is open.
  $effect(() => {
    if (!pickerOpen) return;
    void loadMonthSummaries(pickerPlaceholder);
  });

  function isPickerDateDisabled(d: DateValue): boolean {
    // Pre-load: don't disable so the user can navigate into a month before
    // its summaries arrive. Once a month is loaded, disable any local date
    // not present in the dataset.
    if (!loadedMonths.has(monthKeyOf(d))) return false;
    return !summariesByDate.has(dateKeyOf(d));
  }

  // Distinct minute-buckets for the selected date, each carrying the LATEST
  // frame summary in that minute (so picking the bucket maps cleanly to
  // "latest at or before the end of that minute").
  type TimeBucket = { label: string; summary: FrameSummaryDto };
  const availableTimes = $derived.by<TimeBucket[]>(() => {
    if (!pickerSelectedDate) return [];
    const key = dateKeyOf(pickerSelectedDate);
    const summaries = summariesByDate.get(key) ?? [];
    const buckets = new Map<string, FrameSummaryDto>();
    for (const s of summaries) {
      const d = parseCapturedAt(s.capturedAt);
      const label = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
      // Ascending input → last write wins → latest summary in the bucket.
      buckets.set(label, s);
    }
    return Array.from(buckets.entries())
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([label, summary]) => ({ label, summary }));
  });

  async function jumpToFrame(target: FrameDto): Promise<void> {
    pickerJumping = true;
    pickerError = null;
    try {
      // Page older frames until the target is present locally. The list is
      // newest-first so older frames have smaller ids; loadTimelinePage's
      // beforeId cursor walks backward in time.
      let budget = TIMELINE_JUMP_PAGE_BUDGET;
      while (
        !timelineFrames.some((f) => f.id === target.id) &&
        !timelineExhausted &&
        budget-- > 0
      ) {
        await loadTimelinePage(false);
      }
      const idx = timelineFrames.findIndex((f) => f.id === target.id);
      if (idx < 0) {
        pickerError = "frame is outside the loaded window";
        return;
      }
      timelineActiveIndex = idx;
      if (timelineRail) {
        const max = timelineRail.scrollWidth - timelineRail.clientWidth;
        timelineRail.scrollTo({
          left: max - idx * TIMELINE_SLOT_WIDTH,
          behavior: "smooth",
        });
      }
      pickerOpen = false;
    } finally {
      pickerJumping = false;
    }
  }

  async function resolveAndJump(rangeStart: Date, rangeEnd: Date): Promise<void> {
    const req: FrameRangeRequest = {
      capturedAtStart: rangeStart.toISOString(),
      capturedAtEnd: rangeEnd.toISOString(),
    };
    try {
      const frame = await invoke<FrameDto | null>("get_latest_frame_in_range", {
        request: req,
      });
      if (!frame) {
        pickerError = "no frame in that range";
        return;
      }
      await jumpToFrame(frame);
    } catch (err) {
      pickerError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  async function jumpToSelectedDateLatest(): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
    const end = new Date(d.year, d.month - 1, d.day, 23, 59, 59, 999);
    await resolveAndJump(start, end);
  }

  async function jumpToSelectedDateTime(label: string): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    const [hh, mm] = label.split(":").map((s) => Number(s));
    const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
    // "Latest at or before the end of the picked minute" — backend treats
    // the range as inclusive, so we extend to :59.999.
    const end = new Date(d.year, d.month - 1, d.day, hh ?? 0, mm ?? 0, 59, 999);
    pickerSelectedTime = label;
    await resolveAndJump(start, end);
  }

  // ─── Picker dialog a11y ───────────────────────────────────────────────────
  // The jump picker is rendered as a non-modal `role="dialog"` popover. To
  // give keyboard and screen-reader users a baseline dialog experience we
  // wire up: focus-into-dialog on open, focus-restore on close, Escape to
  // dismiss, a Tab focus trap while open, and click-outside to dismiss.
  let pickerEl = $state<HTMLDivElement | null>(null);
  let pickerTriggerEl = $state<HTMLButtonElement | null>(null);

  function getPickerFocusable(): HTMLElement[] {
    if (!pickerEl) return [];
    const sel =
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
    return Array.from(pickerEl.querySelectorAll<HTMLElement>(sel)).filter(
      (el) => el.offsetParent !== null || el === document.activeElement,
    );
  }

  function onPickerKeydown(e: KeyboardEvent) {
    if (!pickerOpen) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      pickerOpen = false;
      return;
    }
    if (e.key !== "Tab") return;
    const focusable = getPickerFocusable();
    if (focusable.length === 0) {
      e.preventDefault();
      pickerEl?.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement as HTMLElement | null;
    if (e.shiftKey) {
      if (active === first || !pickerEl?.contains(active)) {
        e.preventDefault();
        last.focus();
      }
    } else if (active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function onPickerPointerDownOutside(e: MouseEvent) {
    if (!pickerOpen) return;
    const target = e.target as Node | null;
    if (!target) return;
    if (pickerEl?.contains(target)) return;
    if (pickerTriggerEl?.contains(target)) return; // trigger handles its own toggle
    pickerOpen = false;
  }

  // When the picker opens, move focus inside it; when it closes, restore
  // focus to the trigger so keyboard users don't get stranded.
  $effect(() => {
    if (!pickerOpen) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !pickerOpen) return;
      const focusable = getPickerFocusable();
      (focusable[0] ?? pickerEl)?.focus();
    });
    return () => {
      cancelled = true;
      // Restore focus to trigger only if focus is still inside (or has
      // landed on body) — avoids stealing focus from elsewhere on the page.
      const active = document.activeElement as HTMLElement | null;
      if (
        !active ||
        active === document.body ||
        pickerEl?.contains(active)
      ) {
        pickerTriggerEl?.focus();
      }
    };
  });

  function togglePicker() {
    if (pickerOpen) {
      pickerOpen = false;
      return;
    }
    // Sync the picker's view to whatever the rail is currently showing so
    // the user lands on the active frame's date instead of "today".
    if (timelineActive) {
      const d = parseCapturedAt(timelineActive.capturedAt);
      const cd = new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate());
      pickerPlaceholder = cd;
      pickerSelectedDate = cd;
      pickerSelectedTime = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
    }
    pickerError = null;
    pickerOpen = true;
  }

  // Display string for the picker trigger button — reflects the active
  // frame's local time so the control doubles as a "you are here" readout.
  const triggerLabel = $derived(
    timelineActive
      ? formatCapturedAt(timelineActive.capturedAt)
      : "no active frame",
  );
</script>

<!-- ── Timeline browser ──────────────────────────────────────────────────── -->
<svelte:window onpointerdown={onPickerPointerDownOutside} />
<section class="timeline" onwheel={onTimelineWheel}>
  <header class="timeline__bar">
    <div class="timeline__bar-left">
      <h1 class="timeline__title">Timeline</h1>
      <span class="timeline__hint">scroll anywhere to scrub · newest first · all sessions</span>
    </div>
    <div class="timeline__bar-right">
      <div class="timeline__jump">
        <button
          class="btn btn--ghost btn--sm timeline__jump-trigger"
          onclick={togglePicker}
          bind:this={pickerTriggerEl}
          aria-haspopup="dialog"
          aria-expanded={pickerOpen}
        >
          <span class="timeline__jump-icon">▣</span>
          <span class="timeline__jump-label">{triggerLabel}</span>
        </button>
        {#if pickerOpen}
          <div
            class="timeline__picker"
            role="dialog"
            aria-modal="true"
            aria-label="Jump to date and time"
            tabindex="-1"
            bind:this={pickerEl}
            onkeydown={onPickerKeydown}
          >
            <Calendar.Root
              type="single"
              bind:value={pickerSelectedDate}
              bind:placeholder={pickerPlaceholder}
              isDateDisabled={isPickerDateDisabled}
              weekdayFormat="short"
              class="cal"
            >
              {#snippet children({ months, weekdays })}
                <header class="cal__header">
                  <Calendar.PrevButton class="cal__nav">‹</Calendar.PrevButton>
                  <Calendar.Heading class="cal__heading" />
                  <Calendar.NextButton class="cal__nav">›</Calendar.NextButton>
                </header>
                {#each months as month (month.value)}
                  <Calendar.Grid class="cal__grid">
                    <Calendar.GridHead>
                      <Calendar.GridRow class="cal__row">
                        {#each weekdays as wd (wd)}
                          <Calendar.HeadCell class="cal__weekday">{wd}</Calendar.HeadCell>
                        {/each}
                      </Calendar.GridRow>
                    </Calendar.GridHead>
                    <Calendar.GridBody>
                      {#each month.weeks as weekDates, weekIdx (weekIdx)}
                        <Calendar.GridRow class="cal__row">
                          {#each weekDates as date (date.toString())}
                            <Calendar.Cell {date} month={month.value} class="cal__cell">
                              <Calendar.Day class="cal__day" />
                            </Calendar.Cell>
                          {/each}
                        </Calendar.GridRow>
                      {/each}
                    </Calendar.GridBody>
                  </Calendar.Grid>
                {/each}
              {/snippet}
            </Calendar.Root>

            <div class="timeline__picker-side">
              {#if pickerLoading}
                <div class="timeline__picker-pending">loading month…</div>
              {/if}
              {#if pickerError}
                <div class="timeline__picker-error">{pickerError}</div>
              {/if}
              {#if pickerSelectedDate}
                <div class="timeline__picker-row">
                  <span class="timeline__picker-key">date</span>
                  <span class="timeline__picker-val">{dateKeyOf(pickerSelectedDate)}</span>
                </div>
                <button
                  class="btn btn--ghost btn--sm"
                  onclick={jumpToSelectedDateLatest}
                  disabled={pickerJumping || availableTimes.length === 0}
                >jump to latest of day</button>
                <div class="timeline__picker-key">times</div>
                {#if availableTimes.length === 0}
                  <div class="timeline__picker-pending">no frames on this day</div>
                {:else}
                  <div class="timeline__picker-times">
                    {#each availableTimes as t (t.label)}
                      <button
                        type="button"
                        class="timeline__picker-time"
                        class:timeline__picker-time--active={pickerSelectedTime === t.label}
                        onclick={() => jumpToSelectedDateTime(t.label)}
                        disabled={pickerJumping}
                      >{t.label}</button>
                    {/each}
                  </div>
                {/if}
              {:else}
                <div class="timeline__picker-pending">pick a date</div>
              {/if}
            </div>
          </div>
        {/if}
      </div>

      {#if timelineActive}
        <span class="timeline__counter">
          <span class="timeline__counter-strong">{timelineActiveIndex + 1}</span>
          <span class="timeline__counter-dim">/ {timelineFrames.length}{timelineExhausted ? "" : "+"}</span>
        </span>
      {/if}
      <button
        class="btn btn--ghost btn--sm"
        onclick={refreshTimelineAndDashboard}
        disabled={timelineLoading || timelineLoadingMore || audioSegmentsLoading}
      >refresh</button>
    </div>
  </header>

  <div class="timeline__audio" title={audioSummaryTitle} aria-label="Audio segments summary">
    <span class="timeline__audio-label">audio segments</span>
    <span class="timeline__audio-pill timeline__audio-pill--microphone" aria-label={`${audioSegmentCounts.microphone} microphone segments`}>
      <span class="timeline__audio-swatch" aria-hidden="true"></span>
      mic {audioSegmentCounts.microphone}
    </span>
    <span class="timeline__audio-pill timeline__audio-pill--systemAudio" aria-label={`${audioSegmentCounts.systemAudio} system audio segments`}>
      <span class="timeline__audio-swatch" aria-hidden="true"></span>
      system {audioSegmentCounts.systemAudio}
    </span>
    {#if latestAudioSegment}
      <span class="timeline__audio-latest">
        latest {audioSourceLabel(latestAudioSegment.source)} #{latestAudioSegment.segmentIndex}
      </span>
    {:else if audioSegmentsLoading}
      <span class="timeline__audio-muted">loading…</span>
    {:else if audioSegmentsError}
      <span class="timeline__audio-error">timeline audio unavailable</span>
    {:else}
      <span class="timeline__audio-muted">none in loaded range</span>
    {/if}
  </div>

  {#if timelineError}
    <div class="timeline__error">
      <span class="timeline__error-label">load error</span>
      <span class="timeline__error-msg">{timelineError}</span>
    </div>
  {/if}

  {#if audioSegments.length > 0}
    <!-- Inline audio segment player. Compact deck panel that sits above the
         stage so it's adjacent to the rail where bars are selected. The
         panel renders an empty/instruction state until a bar is clicked. -->
    <div class="timeline__player" aria-label="Audio segment player">
      {#if selectedAudioSegment}
        <div class="timeline__player-meta">
          <span
            class="timeline__player-source timeline__player-source--{selectedAudioSegment.source}"
            aria-label={`Source: ${audioSourceLabel(selectedAudioSegment.source)}`}
          >
            <span class="timeline__player-swatch" aria-hidden="true"></span>
            {audioSourceLabel(selectedAudioSegment.source)}
          </span>
          <span class="timeline__player-index" aria-label="Segment index">
            #{selectedAudioSegment.segmentIndex}
          </span>
          <span class="timeline__player-time" title={`${formatUnixMs(selectedAudioSegment.startUnixMs)} – ${formatUnixMs(selectedAudioSegment.endUnixMs)}`}>
            {formatTimeOfDay(selectedAudioSegment.startUnixMs)}
            <span class="timeline__player-time-sep" aria-hidden="true">→</span>
            {formatTimeOfDay(selectedAudioSegment.endUnixMs)}
            <span class="timeline__player-duration">· {formatDurationSeconds(selectedAudioSegment.durationSeconds)}</span>
          </span>
          <span
            class="timeline__player-file"
            title={selectedAudioSegment.filePath}
          >{selectedAudioSegment.fileName}</span>
          <button
            type="button"
            class="timeline__player-close"
            onclick={() => (selectedAudioSegmentId = null)}
            aria-label="Close audio player"
          >×</button>
        </div>
        {#if selectedAudioMediaLoading}
          <div class="timeline__player-empty">
            <span class="timeline__player-empty-glyph" aria-hidden="true">…</span>
            <span>loading audio segment…</span>
          </div>
        {:else if selectedAudioMediaError}
          <div class="timeline__player-error" role="alert">
            <span class="timeline__player-error-label">playback unavailable</span>
            <span class="timeline__player-error-msg">{selectedAudioMediaError}</span>
          </div>
        {:else if selectedAudioSrc}
          <!-- `src` is reactive: switching segments swaps the audio element's
               source via Svelte's binding. Using a keyed block forces a fresh
               <audio> element per segment so the browser doesn't keep playing
               the previous file while the new metadata loads. -->
          {#key selectedAudioSegment.id}
            <audio
              class="timeline__player-audio"
              controls
              preload="metadata"
              src={selectedAudioSrc}
              onerror={onSelectedAudioError}
            ></audio>
          {/key}
        {/if}
        {#if selectedAudioLoadError}
          <div class="timeline__player-error" role="alert">
            <span class="timeline__player-error-label">playback error</span>
            <span class="timeline__player-error-msg">{selectedAudioLoadError}</span>
          </div>
        {/if}
      {:else}
        <div class="timeline__player-empty">
          <span class="timeline__player-empty-glyph" aria-hidden="true">▶</span>
          <span>select an audio segment to play</span>
          <span class="timeline__player-empty-hint">click any bar on the timeline rail</span>
        </div>
      {/if}
    </div>
  {/if}

  <div class="timeline__stage">
    {#if timelineLoading && timelineFrames.length === 0}
      <div class="timeline__preview-pending">loading frames…</div>
    {:else if timelineFrames.length === 0}
      <div class="timeline__empty">
        <span>no frames yet</span>
        <span class="timeline__empty-hint">capture a session to populate the timeline</span>
      </div>
    {:else if timelineActive}
      {@const previewUrl = previewCache.get(timelineActive.id)}
      {#if previewUrl}
        <img
          class="timeline__preview"
          src={previewUrl}
          alt={`frame ${timelineActive.id}`}
          draggable="false"
        />
      {:else}
        <div class="timeline__preview-pending">decoding preview…</div>
      {/if}
    {/if}

    {#if timelineActive && developerOptions.value}
      <!-- Compact, overlaid metadata so the preview itself dominates. Gated
           behind the developer-options flag — non-dev users only see the
           preview and the rail. -->
      <aside class="timeline__overlay" aria-label="frame metadata">
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">id</span>
          <span class="timeline__overlay-val">{timelineActive.id}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">captured</span>
          <span class="timeline__overlay-val">{formatCapturedAt(timelineActive.capturedAt)}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">session</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.sessionId}</span>
        </div>
        {#if timelineActive.width != null && timelineActive.height != null}
          <div class="timeline__overlay-row">
            <span class="timeline__overlay-key">dims</span>
            <span class="timeline__overlay-val">{timelineActive.width}×{timelineActive.height}</span>
          </div>
        {/if}
        {#if timelineActive.contentFingerprint}
          <div class="timeline__overlay-row">
            <span class="timeline__overlay-key">fp</span>
            <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.contentFingerprint}</span>
          </div>
        {/if}
      </aside>
    {/if}
  </div>

  <!-- Rail-wrap is always rendered (even when there are no frames) so the
       stage's flex size is stable across the empty → populated transition.
       The rail itself is locked to a fixed height and the loading indicator
       lives outside the rail so neither pagination loads nor the
       loading→loaded swap can change page/stage/rail height. -->
  <div class="timeline__rail-wrap" bind:this={timelineRailWrap}>
    {#if timelineFrames.length > 0}
      <div
        class="timeline-rail"
        bind:this={timelineRail}
        onscroll={onTimelineScroll}
        onkeydown={onTimelineKeyDown}
        onclick={onTimelineRailClick}
        onpointermove={onTimelineRailPointerMove}
        onpointerleave={onTimelineRailPointerLeave}
        role="slider"
        tabindex="0"
        aria-label="Timeline scrubber"
        aria-valuemin={1}
        aria-valuemax={Math.max(1, timelineFrames.length)}
        aria-valuenow={timelineActiveIndex + 1}
        aria-describedby="timeline-rail-readout"
        aria-valuetext={timelineActive
          ? `Frame ${timelineActiveIndex + 1} of ${timelineFrames.length}${timelineExhausted ? "" : "+"} — captured ${formatCapturedAt(timelineActive.capturedAt)}`
          : undefined}
      >
        <div
          class="timeline-rail__track"
          style="width: {timelineFrames.length * TIMELINE_SLOT_WIDTH}px"
        >
          {#each timelineWindow as frame, j (frame.id)}
            {@const i = timelineWindowStart + j}
            {@const isActive = i === timelineActiveIndex}
            {@const isMajor = i % 50 === 0}
            <!-- Ticks are intentionally presentational (no role, not
                 focusable) so the parent's role="slider" is valid. The slider
                 itself owns position semantics via aria-valuenow/text, and
                 click-to-seek is handled by the rail's onclick. Slot 0
                 (newest) is anchored to the right of the track via `right:`. -->
            <div
              class="timeline-rail__slot"
              class:timeline-rail__slot--active={isActive}
              class:timeline-rail__slot--major={isMajor}
              style="right: {i * TIMELINE_SLOT_WIDTH}px"
              onpointerenter={(e) => onSlotPointerEnter(e, frame.id)}
              aria-hidden="true"
            >
              <span class="timeline-rail__tick"></span>
            </div>
          {/each}
        </div>
         <span class="timeline-rail__cursor" aria-hidden="true"></span>
      </div>
      <!-- Audio segment lane. Lives as a sibling of the slider rail (not
           inside it, and not on top of it) so:
             1. Interactive segment buttons aren't nested inside an element
                with role="slider" — that nesting is invalid a11y.
             2. Bars don't visually overlap the frame tick rail, which made
                clicking either the rail or a bar ambiguous.
           The lane mirrors the rail's scrolled track by reusing the same
           per-frame slot width, the same `cqi`-based viewport margins (the
           lane re-establishes a `container-type: inline-size` context with
           the same width as the rail), and a `translateX(-scrollLeft)` that
           keeps bars aligned with the frame tick centers as the user scrubs.
           Two stacked sub-rows split microphone (top) from system audio
           (bottom) so dense overlap stays readable. -->
      <div
        class="timeline-rail__audio-lane-wrap"
        aria-label="Audio segments"
      >
        <div class="timeline-rail__audio-lane-labels" aria-hidden="true">
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--microphone">mic</span>
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--systemAudio">sys</span>
        </div>
        <div class="timeline-rail__audio-lane-viewport">
          {#if positionedAudioSegments.length > 0}
            <div
              class="timeline-rail__audio-lane-track"
              style="width: {timelineFrames.length *
                TIMELINE_SLOT_WIDTH}px; transform: translateX({-timelineScrollLeft}px)"
            >
              <div class="timeline-rail__audio-row timeline-rail__audio-row--microphone" role="presentation">
                {#each positionedAudioSegments as seg (seg.id)}
                  {#if seg.visible && seg.source === "microphone"}
                    <button
                      type="button"
                      class="timeline-rail__audio-bar timeline-rail__audio-bar--microphone"
                      class:timeline-rail__audio-bar--selected={seg.id === selectedAudioSegmentId}
                      style="right: {seg.rightPx}px; width: {seg.widthPx}px"
                      title={`${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} · ${seg.fileName} · ${formatUnixMs(seg.startUnixMs)} – ${formatUnixMs(seg.endUnixMs)}`}
                      aria-label={`Play ${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} from ${formatTimeOfDay(seg.startUnixMs)} to ${formatTimeOfDay(seg.endUnixMs)}`}
                      aria-pressed={seg.id === selectedAudioSegmentId}
                      onclick={(e) => onAudioSegmentBarClick(e, seg.id)}
                      onkeydown={onAudioSegmentBarKeyDown}
                      onpointerdown={(e) => e.stopPropagation()}
                    ></button>
                  {/if}
                {/each}
              </div>
              <div class="timeline-rail__audio-row timeline-rail__audio-row--systemAudio" role="presentation">
                {#each positionedAudioSegments as seg (seg.id)}
                  {#if seg.visible && seg.source === "systemAudio"}
                    <button
                      type="button"
                      class="timeline-rail__audio-bar timeline-rail__audio-bar--systemAudio"
                      class:timeline-rail__audio-bar--selected={seg.id === selectedAudioSegmentId}
                      style="right: {seg.rightPx}px; width: {seg.widthPx}px"
                      title={`${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} · ${seg.fileName} · ${formatUnixMs(seg.startUnixMs)} – ${formatUnixMs(seg.endUnixMs)}`}
                      aria-label={`Play ${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} from ${formatTimeOfDay(seg.startUnixMs)} to ${formatTimeOfDay(seg.endUnixMs)}`}
                      aria-pressed={seg.id === selectedAudioSegmentId}
                      onclick={(e) => onAudioSegmentBarClick(e, seg.id)}
                      onkeydown={onAudioSegmentBarKeyDown}
                      onpointerdown={(e) => e.stopPropagation()}
                    ></button>
                  {/if}
                {/each}
              </div>
            </div>
          {:else}
            <span class="timeline-rail__audio-lane-empty">
              {#if audioSegmentsLoading}
                loading audio…
              {:else if audioSegmentsError}
                audio unavailable
              {:else}
                no audio in range
              {/if}
            </span>
          {/if}
        </div>
      </div>
    {:else}
      <!-- Empty placeholder reserves the rail's height so removing/adding
           the rail does not resize the stage. The audio-lane placeholder
           reserves matching height for the lane below. -->
      <div class="timeline-rail timeline-rail--placeholder" aria-hidden="true"></div>
      <div
        class="timeline-rail__audio-lane-wrap timeline-rail__audio-lane-wrap--placeholder"
        aria-hidden="true"
      ></div>
    {/if}
    {#if timelineLoadingMore}
      <div class="timeline-rail__loading">loading…</div>
    {/if}
    {#if timelineFrames.length > 0 && tooltipFrame}
      <div
        id="timeline-rail-readout"
        class="timeline-rail__tooltip"
        class:timeline-rail__tooltip--pinned={!tooltipIsHovered}
        style={tooltipIsHovered && hoveredX != null
          ? `left: ${hoveredX}px; transform: translate(-50%, -100%);`
          : "left: 50%; transform: translate(-50%, -100%);"}
        role="tooltip"
      >
        {formatCapturedAt(tooltipFrame.capturedAt)}
      </div>
    {/if}
  </div>
</section>

<style>
  /* ── Page layout ──────────────────────────────────────────── */
  .timeline {
    /* Fill the viewport below the 44px sticky nav. */
    height: calc(100vh - 44px);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 16px 16px;
    background: #0c0c0e;
    /* Allow the stage child (flex: 1, min-height: 0) to actually shrink so
       the bottom rail stays in view regardless of preview intrinsic size. */
    min-height: 0;
    overflow: hidden;
  }

  .timeline__bar,
  .timeline__audio,
  .timeline__error,
  .timeline__player,
  .timeline__rail-wrap {
    flex: 0 0 auto;
  }

  .timeline__bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .timeline__bar-left {
    display: flex;
    align-items: baseline;
    gap: 12px;
    min-width: 0;
  }

  .timeline__bar-right {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .timeline__title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #f0f0f5;
  }

  .timeline__hint {
    font-size: 9px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #44445a;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .timeline__counter {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    letter-spacing: 0.04em;
  }

  .timeline__counter-strong {
    color: #f0f0f5;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .timeline__counter-dim {
    color: #44445a;
    font-variant-numeric: tabular-nums;
    margin-left: 4px;
  }

  .timeline__audio {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 6px 8px;
    background: #0a0a10;
    border: 1px solid #161624;
    border-radius: 4px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #555574;
  }

  .timeline__audio-label {
    color: #6a6a88;
  }

  .timeline__audio-pill {
    padding: 1px 6px;
    border: 1px solid #242438;
    border-radius: 999px;
    color: #a0a0c0;
    font-variant-numeric: tabular-nums;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .timeline__audio-swatch {
    width: 8px;
    height: 3px;
    border-radius: 1.5px;
    display: inline-block;
  }

  .timeline__audio-pill--microphone .timeline__audio-swatch {
    background: linear-gradient(
      90deg,
      rgba(120, 200, 255, 0.95),
      rgba(80, 160, 230, 0.95)
    );
  }

  .timeline__audio-pill--systemAudio .timeline__audio-swatch {
    background: linear-gradient(
      90deg,
      rgba(255, 180, 100, 0.95),
      rgba(220, 130, 60, 0.95)
    );
  }

  .timeline__audio-latest {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #7a7a9a;
  }

  .timeline__audio-muted {
    color: #3a3a52;
  }

  .timeline__audio-error {
    color: #a06068;
  }

  /* ── Audio segment player deck ─────────────────────────────────
     Compact panel that pairs with the rail: source pill, segment index,
     time range, file name, and a native <audio controls> element. The
     industrial vibe matches the rail (matte black surface, hairline
     border, red accent on the live segment). The empty state nudges the
     user toward the rail when audio segments exist but none is selected. */
  .timeline__player {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    background: linear-gradient(180deg, #111118 0%, #0c0c12 100%);
    border: 1px solid #1f1f30;
    border-radius: 5px;
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.02);
  }

  .timeline__player-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #b8b8d0;
    font-variant-numeric: tabular-nums;
    min-width: 0;
  }

  .timeline__player-source {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid #242438;
    border-radius: 999px;
    color: #d6d6ea;
    font-weight: 700;
  }

  .timeline__player-swatch {
    width: 10px;
    height: 3px;
    border-radius: 1.5px;
  }

  .timeline__player-source--microphone .timeline__player-swatch {
    background: linear-gradient(
      90deg,
      rgba(120, 200, 255, 0.95),
      rgba(80, 160, 230, 0.95)
    );
  }

  .timeline__player-source--systemAudio .timeline__player-swatch {
    background: linear-gradient(
      90deg,
      rgba(255, 180, 100, 0.95),
      rgba(220, 130, 60, 0.95)
    );
  }

  .timeline__player-index {
    color: #ff5566;
    font-weight: 700;
  }

  .timeline__player-time {
    color: #8e8eb0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .timeline__player-time-sep {
    color: #444462;
  }

  .timeline__player-duration {
    color: #5e5e80;
  }

  .timeline__player-file {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #6e6e90;
    text-transform: none;
    letter-spacing: 0;
    font-family:
      ui-monospace,
      SFMono-Regular,
      Menlo,
      monospace;
    font-size: 11px;
    text-align: right;
  }

  .timeline__player-close {
    appearance: none;
    background: transparent;
    border: 1px solid #242438;
    border-radius: 4px;
    width: 22px;
    height: 22px;
    color: #8a8aae;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition: color 0.12s, border-color 0.12s, background 0.12s;
  }

  .timeline__player-close:hover,
  .timeline__player-close:focus-visible {
    color: #ff5566;
    border-color: #ff4455;
    background: rgba(255, 68, 85, 0.08);
    outline: none;
  }

  .timeline__player-audio {
    width: 100%;
    height: 32px;
    /* Tone the native player down so it reads as part of the deck. */
    filter: invert(0.88) hue-rotate(180deg) saturate(0.85);
    border-radius: 4px;
  }

  .timeline__player-empty {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 4px;
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #6a6a88;
  }

  .timeline__player-empty-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid #242438;
    border-radius: 50%;
    color: #ff5566;
    font-size: 9px;
    padding-left: 1px;
  }

  .timeline__player-empty-hint {
    color: #3e3e58;
    text-transform: none;
    letter-spacing: 0;
    font-size: 11px;
  }

  .timeline__player-error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 10px;
    margin-top: 6px;
    background: #1a0e10;
    border: 1px solid #3a1a20;
    border-radius: 4px;
    font-size: 11px;
    color: #c08080;
  }

  .timeline__player-error-label {
    flex: 0 0 auto;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #80505a;
    padding-top: 1px;
  }

  .timeline__player-error-msg {
    flex: 1 1 auto;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    word-break: break-word;
    line-height: 1.4;
  }

  /* ── Buttons (subset used by the timeline) ─────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .btn--ghost {
    background: transparent;
    color: #7a7a9a;
    border-color: #2a2a3a;
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: #1a1a2a;
    color: #a0a0c0;
    border-color: #3a3a5a;
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  /* ── Date jump picker ──────────────────────────────────────── */
  .timeline__jump {
    position: relative;
  }

  .timeline__jump-trigger {
    gap: 6px;
    font-variant-numeric: tabular-nums;
    max-width: 220px;
  }

  .timeline__jump-icon {
    font-size: 10px;
    color: #5a5a7a;
  }

  .timeline__jump-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .timeline__picker {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    display: grid;
    grid-template-columns: auto 200px;
    gap: 12px;
    padding: 12px;
    background: #0e0e16;
    border: 1px solid #1e1e2e;
    border-radius: 6px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  .timeline__picker-side {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .timeline__picker-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }

  .timeline__picker-key {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #44445a;
  }

  .timeline__picker-val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    color: #c0c0d8;
  }

  .timeline__picker-pending {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #44445a;
  }

  .timeline__picker-error {
    font-size: 10px;
    color: #c08080;
    word-break: break-word;
  }

  .timeline__picker-times {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(56px, 1fr));
    gap: 4px;
    max-height: 180px;
    overflow-y: auto;
    padding-right: 2px;
  }

  .timeline__picker-time {
    padding: 4px 6px;
    background: transparent;
    border: 1px solid #1e1e2e;
    border-radius: 3px;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #8a8aa8;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .timeline__picker-time:hover:not(:disabled) {
    background: #1a1a2a;
    color: #d0d0e8;
    border-color: #3a3a5a;
  }

  .timeline__picker-time:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .timeline__picker-time--active {
    color: #ff4455;
    border-color: rgba(255, 68, 85, 0.4);
    background: rgba(255, 68, 85, 0.08);
  }

  /* Bits UI calendar — narrow themed shell. */
  :global(.cal) {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: #c0c0d8;
  }

  :global(.cal__header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 2px 4px;
  }

  :global(.cal__nav) {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid #1e1e2e;
    border-radius: 3px;
    color: #8a8aa8;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }

  :global(.cal__nav:hover) {
    background: #1a1a2a;
    color: #d0d0e8;
    border-color: #3a3a5a;
  }

  :global(.cal__heading) {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #c0c0d8;
  }

  :global(.cal__grid) {
    border-collapse: collapse;
  }

  :global(.cal__row) {
    display: grid;
    grid-template-columns: repeat(7, 28px);
  }

  :global(.cal__weekday) {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #44445a;
    text-align: center;
    padding: 4px 0;
  }

  :global(.cal__cell) {
    padding: 1px;
  }

  :global(.cal__day) {
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 3px;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    color: #c0c0d8;
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  :global(.cal__day:hover:not([data-disabled])) {
    background: #1a1a2a;
    border-color: #3a3a5a;
  }

  :global(.cal__day[data-disabled]),
  :global(.cal__day[data-outside-month]) {
    color: #2a2a3a;
    cursor: not-allowed;
  }

  :global(.cal__day[data-selected]) {
    background: rgba(255, 68, 85, 0.12);
    border-color: rgba(255, 68, 85, 0.5);
    color: #ffb0b8;
  }

  :global(.cal__day[data-today]:not([data-selected])) {
    border-color: #2a2a3a;
    color: #f0f0f5;
  }

  /* ── Error / empty ─────────────────────────────────────────── */
  .timeline__error {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    background: #1a0e10;
    border: 1px solid #3a1a20;
    border-radius: 4px;
    font-size: 11px;
    color: #c08080;
  }

  .timeline__error-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #80505a;
  }

  .timeline__error-msg {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    word-break: break-word;
  }

  .timeline__empty {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
    justify-content: center;
    color: #44445a;
    font-size: 11px;
    letter-spacing: 0.06em;
  }

  .timeline__empty-hint {
    font-size: 10px;
    color: #2a2a3a;
  }

  /* ── Stage (preview dominates) ─────────────────────────────── */
  .timeline__stage {
    position: relative;
    flex: 1 1 0;
    min-height: 0; /* allow the flex child to actually shrink as needed */
    background: linear-gradient(135deg, #0a0a10 0%, #0e0e16 100%);
    border: 1px solid #1e1e2e;
    border-radius: 6px;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .timeline__preview {
    max-width: 100%;
    max-height: 100%;
    width: auto;
    height: auto;
    object-fit: contain;
    image-rendering: -webkit-optimize-contrast;
    user-select: none;
  }

  .timeline__preview-pending {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #33334a;
  }

  /* Compact metadata pinned to the corner of the stage so the preview
     remains the visual anchor. Translucent panel with backdrop blur keeps
     it legible across both light and dark frames. */
  .timeline__overlay {
    position: absolute;
    top: 10px;
    left: 10px;
    max-width: min(40%, 360px);
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 10px;
    padding: 8px 10px;
    background: rgba(10, 10, 16, 0.72);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 4px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    pointer-events: none;
  }

  .timeline__overlay-row {
    display: contents;
  }

  .timeline__overlay-key {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #44445a;
    align-self: center;
  }

  .timeline__overlay-val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: #c0c0d8;
    min-width: 0;
  }

  .timeline__overlay-truncate {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  /* ── Rail (bottom dock) ────────────────────────────────────── */
  .timeline__rail-wrap {
    /* Reserve a fixed footprint so the stage's flex height never reflows
       when the rail toggles between empty and populated, and so the
       absolutely-positioned loading indicator has a predictable anchor. */
    position: relative;
    flex: 0 0 auto;
    /* Stretch across the full width of the timeline column. Flex column
       children stretch on the cross axis by default, but stating it
       explicitly — and zeroing `min-width` — guarantees the rail always
       fills available width even if a future ancestor changes
       `align-items` or introduces an intrinsic-width sibling. */
    align-self: stretch;
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .timeline-rail {
    position: relative;
    /* Fill the wrapper's full inline size. Without an explicit width, an
       overflow-x scroll container with extremely wide content can collapse
       to its content's intrinsic width in some flex contexts; pinning
       width: 100% (and min-width: 0 so flex can still shrink it) keeps the
       scrollable viewport spanning the entire row. */
    width: 100%;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    /* Track is 22px + 1px top/bottom border = 24px. Locking the rail's
       height (rather than letting it derive from content) ensures that
       transient in-flow children (e.g. previous sticky loader, future
       overlays) cannot grow the rail and ripple height into the stage. */
    height: 24px;
    flex: 0 0 24px;
    box-sizing: border-box;
    /* Slot 0 (newest) is anchored to the right edge of the track via
       `right: i * SLOT_WIDTH` on each slot, with symmetric viewport-sized
       margins (`50cqi - 4px`) so the first/last slot can sit under the
       static cursor caret at the rail's center. To advance toward older
       frames the user scrolls leftward (scrollLeft decreases from
       `maxScrollLeft`). The rail stays in normal LTR direction so all
       scrollLeft math is straightforward and browser-portable. */
    background: #0a0a10;
    border: 1px solid #1e1e2e;
    border-radius: 4px;
    padding: 0;
    scrollbar-width: none;
    /* Establish a containment context so the track's spacer margins can be
       sized in `cqi` (rail's visible inline size) rather than the track's
       own width — necessary because the track itself is much wider than the
       viewport at high frame counts. */
    container-type: inline-size;
    cursor: pointer;
  }

  .timeline-rail:focus {
    outline: none;
  }

  .timeline-rail:focus-visible {
    outline: none;
    border-color: #ff4455;
    box-shadow: 0 0 0 2px rgba(255, 68, 85, 0.35);
  }

  .timeline-rail::-webkit-scrollbar {
    display: none;
  }

  .timeline-rail__track {
    position: relative;
    height: 22px;
    /* Symmetric viewport-relative spacers so the first/last frames can sit
       under the centered cursor caret. Using `cqi` (rail's inline size)
       rather than `%` (which resolves against the track's own width and
       drifts wildly with frame count) makes both centering and click
       positioning reliable. Margin — not padding — is required because slot
       ticks are absolutely positioned and would ignore padding offsets. */
    margin-left: calc(50cqi - 4px);
    margin-right: calc(50cqi - 4px);
  }

  .timeline-rail__slot {
    position: absolute;
    top: 0;
    width: 8px;
    height: 22px;
    margin: 0;
    padding: 0;
    background: transparent;
    border: 0;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
  }

  /* ── Audio segment lane ────────────────────────────────────────
     A sibling of the slider rail (kept outside `role="slider"` so
     interactive segment buttons aren't nested inside slider semantics)
     and visually placed BELOW the rail — not on top of frame ticks — so
     bars don't compete with the rail for clicks. The lane re-establishes
     the same `container-type: inline-size` context as `.timeline-rail`,
     mirrors the same track width and the same `cqi`-based viewport
     margins, and is translated horizontally to follow the rail's
     scrollLeft so bars stay locked to frame tick centers. Two stacked
     sub-rows split microphone (top) from system audio (bottom), each
     with a fixed-width gutter label so the lane's purpose reads at a
     glance. */
  .timeline-rail__audio-lane-wrap {
    flex: 0 0 auto;
    display: flex;
    align-items: stretch;
    gap: 6px;
    width: 100%;
    min-width: 0;
    /* Subtle inset background so the lane reads as a distinct surface
       from the rail above without drawing a hard border. */
    padding: 4px 0 4px 0;
    background: linear-gradient(
      180deg,
      rgba(10, 10, 16, 0) 0%,
      rgba(10, 10, 16, 0.55) 30%,
      rgba(10, 10, 16, 0.55) 70%,
      rgba(10, 10, 16, 0) 100%
    );
    border-radius: 4px;
  }

  .timeline-rail__audio-lane-labels {
    flex: 0 0 28px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    align-items: flex-end;
    padding: 1px 4px 1px 0;
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    user-select: none;
  }

  .timeline-rail__audio-lane-label--microphone {
    color: rgba(120, 200, 255, 0.85);
  }

  .timeline-rail__audio-lane-label--systemAudio {
    color: rgba(255, 180, 100, 0.85);
  }

  .timeline-rail__audio-lane-viewport {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    height: 26px;
    overflow: hidden;
    /* Same container context the rail uses, so the inner track-mirror's
       `cqi` margins resolve to the same pixel width and bars line up
       with the in-rail ticks. */
    container-type: inline-size;
    border-radius: 3px;
    background: rgba(8, 8, 14, 0.6);
    box-shadow: inset 0 0 0 1px #15151f;
  }

  .timeline-rail__audio-lane-track {
    position: relative;
    height: 26px;
    /* Same spacers as `.timeline-rail__track` so a bar at `right: N*8px`
       from the track-mirror's right edge sits over the same screen
       position as the slot tick at `right: N*8px` inside the rail. */
    margin-left: calc(50cqi - 4px);
    margin-right: calc(50cqi - 4px);
    /* `transform` is set inline based on `timelineScrollLeft`. */
    will-change: transform;
  }

  .timeline-rail__audio-row {
    position: absolute;
    left: 0;
    right: 0;
    height: 11px;
  }

  .timeline-rail__audio-row--microphone {
    top: 1px;
  }

  .timeline-rail__audio-row--systemAudio {
    top: 14px;
  }

  .timeline-rail__audio-lane-empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #3a3a52;
    pointer-events: none;
  }

  .timeline-rail__audio-bar {
    position: absolute;
    top: 0;
    height: 11px;
    border-radius: 2px;
    padding: 0;
    border: 0;
    appearance: none;
    cursor: pointer;
    /* Larger hit area than the visual rectangle: a transparent ::before
       extends 3px above and below so even narrow bars are easy to grab,
       without shifting visual layout. */
    box-shadow: 0 0 0 0.5px rgba(0, 0, 0, 0.5);
    transition:
      filter 90ms ease,
      box-shadow 90ms ease,
      transform 90ms ease;
  }

  .timeline-rail__audio-bar::before {
    content: "";
    position: absolute;
    inset: -3px -1px;
  }

  .timeline-rail__audio-bar:hover {
    filter: brightness(1.15);
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.55),
      0 0 0 1px rgba(255, 255, 255, 0.3);
  }

  .timeline-rail__audio-bar:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.6),
      0 0 0 2px rgba(255, 208, 96, 0.85);
    z-index: 2;
  }

  .timeline-rail__audio-bar--selected {
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.6),
      0 0 0 1.5px #ff4455,
      0 0 8px rgba(255, 68, 85, 0.45);
    z-index: 1;
  }

  .timeline-rail__audio-bar--microphone {
    background: linear-gradient(
      180deg,
      rgba(140, 215, 255, 0.95),
      rgba(70, 150, 220, 0.9)
    );
  }

  .timeline-rail__audio-bar--systemAudio {
    background: linear-gradient(
      180deg,
      rgba(255, 195, 120, 0.95),
      rgba(215, 125, 55, 0.9)
    );
  }

  /* Extend the pointer hit area beyond the visual 8px tick so dense ticks
     are easier to grab without widening the rail itself. The pseudo overlays
     adjacent slots; the topmost (most recently painted) slot wins, which is
     fine for click — the visual tick still anchors which frame is targeted. */
  .timeline-rail__slot::before {
    content: "";
    position: absolute;
    inset: -2px -3px;
  }

  .timeline-rail__slot:focus-visible {
    z-index: 1;
  }

  .timeline-rail__slot:focus-visible .timeline-rail__tick {
    width: 2px;
    height: 18px;
    background: #ffd060;
    box-shadow: 0 0 0 2px rgba(255, 208, 96, 0.35);
  }

  .timeline-rail__tick {
    display: block;
    width: 1px;
    height: 8px;
    background: #2a2a3a;
    border-radius: 0.5px;
    transition: height 0.12s ease-out, background 0.12s;
  }

  .timeline-rail__slot--major .timeline-rail__tick {
    height: 14px;
    background: #3a3a52;
  }

  .timeline-rail__slot:hover .timeline-rail__tick {
    background: #5a5a7a;
    height: 12px;
  }

  .timeline-rail__slot--active .timeline-rail__tick,
  .timeline-rail__slot--active.timeline-rail__slot--major .timeline-rail__tick {
    width: 2px;
    height: 22px;
    background: #ff4455;
    box-shadow: 0 0 6px rgba(255, 68, 85, 0.7);
  }

  /* Static center indicator — the rail scrolls beneath it, so the active
     frame is always whichever tick is centered under this caret. */
  .timeline-rail__cursor {
    position: absolute;
    top: -1px;
    bottom: -1px;
    left: 50%;
    width: 1px;
    background: rgba(255, 68, 85, 0.35);
    pointer-events: none;
  }

  .timeline-rail__cursor::before,
  .timeline-rail__cursor::after {
    content: "";
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    width: 0;
    height: 0;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
  }

  .timeline-rail__cursor::before {
    top: -1px;
    border-top: 4px solid #ff4455;
  }

  .timeline-rail__cursor::after {
    bottom: -1px;
    border-bottom: 4px solid #ff4455;
  }

  .timeline-rail--placeholder {
    /* Visually identical empty rail used to reserve layout space before any
       frames have loaded, so the stage's flex height is the same in the
       empty and populated states. */
    cursor: default;
    pointer-events: none;
  }

  .timeline-rail__audio-lane-wrap--placeholder {
    /* Reserve the same vertical footprint as the populated lane so the
       stage doesn't reflow when the first frames + segments arrive. */
    height: 34px;
    background: none;
    pointer-events: none;
  }

  .timeline-rail__loading {
    /* Absolutely anchored to the rail-wrap rather than living inside the
       (horizontally scrolling) rail, so showing/hiding the loader during
       pagination cannot push the rail's height. Pinned to the LEFT/TOP of
       the rail row to stay clear of both the newest-frame anchor on the
       right AND the audio lane that now sits below the rail. */
    position: absolute;
    left: 8px;
    top: 4px;
    width: fit-content;
    padding: 2px 6px;
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #44445a;
    background: rgba(13, 13, 20, 0.9);
    border: 1px solid #1e1e2e;
    border-radius: 3px;
    pointer-events: none;
  }

  /* Custom hover/active tooltip for the rail. Anchored to the rail-wrap (not
     the scrolling rail) so it never scrolls horizontally with the track.
     Positioned via inline `left`/`transform` from script: when the user is
     hovering a slot it follows the cursor; otherwise it pins above the center
     caret to surface the active frame's timestamp without a hover gesture. */
  .timeline-rail__tooltip {
    position: absolute;
    top: -6px;
    z-index: 2;
    padding: 3px 7px;
    font-size: 10px;
    font-weight: 600;
    line-height: 1.2;
    letter-spacing: 0.02em;
    color: #f0f0f6;
    background: rgba(20, 20, 28, 0.96);
    border: 1px solid #2a2a3a;
    border-radius: 4px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    white-space: nowrap;
    pointer-events: none;
    /* Subtle pointer hint below the bubble. */
  }

  .timeline-rail__tooltip::after {
    content: "";
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%);
    width: 0;
    height: 0;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 4px solid rgba(20, 20, 28, 0.96);
  }

  .timeline-rail__tooltip--pinned {
    /* Pinned-to-active variant: tinted to match the active caret accent so
       it's clear the readout corresponds to the frame under the center cursor. */
    border-color: rgba(255, 68, 85, 0.5);
    color: #ffd5d9;
  }

  .timeline-rail__tooltip--pinned::after {
    border-top-color: rgba(20, 20, 28, 0.96);
  }
</style>
