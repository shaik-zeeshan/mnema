<script lang="ts">
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { Calendar } from "bits-ui";
  import {
    CalendarDate,
    type DateValue,
  } from "@internationalized/date";
  import { developerOptions } from "$lib/developer-options.svelte";
  import { captureSession, setSession } from "$lib/session.svelte";
  import type {
    AudioSegmentDto,
    AudioSegmentMediaDto,
    CaptureSession,
    FrameDto,
    FramePreviewDto,
    FrameRangeRequest,
    FrameSummaryDto,
    FocusedFrameWindowDto,
    GetFirstMatchingEarlierFrameByFingerprintRequest,
    GetPermissionsResponse,
    GetProcessingResultRequest,
    GetTimelineWindowAroundFrameRequest,
    ListAudioSegmentsRequest,
    ListFramesRequest,
    OcrObservation,
    OcrStructuredPayload,
    ProcessingJobDto,
    ProcessingResultDto,
    RecordingSettings,
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
  const TIMELINE_PAGE_SIZE = 200;
  // Distance (in frames) from the loaded tail at which we trigger the next
  // `beforeId` page. Sized generously relative to `TIMELINE_PAGE_SIZE` so a
  // fast scrub doesn't visibly stall at the temporary tail before the next
  // page lands. After a page completes we re-check this threshold and chain
  // another load if the user is still inside it (see `loadTimelinePage`'s
  // tail-prefetch follow-up below). `timelineExhausted` continues to gate
  // pagination at the true end.
  const TIMELINE_PREFETCH_AHEAD = 120;
  const TIMELINE_MAX_LOADED_FRAMES = 5_000;
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
  // Focused jump window size around a picked historical frame. Keep it aligned
  // with the normal page size so the rail density and subsequent older-history
  // pagination behave like the standard timeline flow.
  const TIMELINE_JUMP_WINDOW_NEWER_LIMIT = Math.floor((TIMELINE_PAGE_SIZE - 1) / 2);
  const TIMELINE_JUMP_WINDOW_OLDER_LIMIT = TIMELINE_PAGE_SIZE -
    TIMELINE_JUMP_WINDOW_NEWER_LIMIT -
    1;
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
  type TrimmedTimelineFrames = {
    frames: FrameDto[];
    activeIndex: number;
    trimmedHead: boolean;
    trimmedTail: boolean;
  };
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
  let timelineHasNewer = $state(false);
  let timelineError = $state<string | null>(null);
  let timelineShowingHistoricalWindow = $state(false);
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
  const timelineHasMore = $derived(timelineHasNewer || !timelineExhausted);

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

  // ─── Custom audio player state ───────────────────────────────────────────
  // The visible drawer renders a bespoke transport instead of `<audio
  // controls>`, but a hidden `<audio>` element under the hood still owns
  // decoding/playback. UI state mirrors the element via `timeupdate`,
  // `loadedmetadata`, `play`, `pause`, and `ended` events. Scrubbing writes
  // back to `audio.currentTime`. State resets whenever the selected segment
  // changes (see `$effect` further down) so a new segment always begins
  // paused at 0 with a fresh duration readout.
  let audioEl = $state<HTMLAudioElement | null>(null);
  let audioIsPlaying = $state(false);
  let audioCurrentTime = $state(0);
  let audioDuration = $state(0);
  // While the user drags the scrub thumb we hold UI updates from `timeupdate`
  // events so the indicator doesn't fight the drag. Commit on release.
  let audioScrubbing = $state(false);

  $effect(() => {
    // Reset transport readouts whenever the selection (and therefore the
    // underlying media) changes, so the prior segment's progress doesn't
    // briefly appear before the new metadata loads.
    void selectedAudioSegmentId;
    audioIsPlaying = false;
    audioCurrentTime = 0;
    audioDuration = 0;
    audioScrubbing = false;
  });

  function togglePlayPause() {
    const el = audioEl;
    if (!el) return;
    if (el.paused) {
      void el.play().catch(() => {
        // Surface decode/play failures through the existing error path.
        onSelectedAudioError();
      });
    } else {
      el.pause();
    }
  }

  function onAudioTimeUpdate() {
    if (!audioEl || audioScrubbing) return;
    audioCurrentTime = audioEl.currentTime;
  }
  function onAudioLoadedMetadata() {
    if (!audioEl) return;
    audioDuration = Number.isFinite(audioEl.duration) ? audioEl.duration : 0;
  }
  function onAudioPlay() {
    audioIsPlaying = true;
  }
  function onAudioPause() {
    audioIsPlaying = false;
  }
  function onAudioEnded() {
    audioIsPlaying = false;
    audioCurrentTime = audioEl?.duration ?? audioCurrentTime;
  }
  function onScrubInput(e: Event) {
    const t = Number((e.currentTarget as HTMLInputElement).value);
    audioScrubbing = true;
    audioCurrentTime = t;
  }
  function onScrubChange(e: Event) {
    const t = Number((e.currentTarget as HTMLInputElement).value);
    audioScrubbing = false;
    if (audioEl && Number.isFinite(t)) {
      audioEl.currentTime = t;
      audioCurrentTime = t;
    }
  }

  /** `M:SS` for the player transport. Distinct from the segment-duration
   *  helper above because the transport ticks per second and a leading dash
   *  while metadata is still loading reads better as `0:00`. */
  function formatPlayerTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
    const total = Math.floor(seconds);
    const m = Math.floor(total / 60);
    const s = total % 60;
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  // ─── Outside-click dismissal ─────────────────────────────────────────────
  // While the drawer is open, a pointerdown anywhere outside the drawer
  // dismisses it. Timeline audio bars are outside the drawer too, but when
  // the drawer is already open a click on any bar should close only — not
  // immediately re-open/switch the drawer on the trailing `click`. Pointerdown
  // — not click — because `click` doesn't fire on every dismiss target (e.g.
  // dragging a scrollbar) and we want the close to feel immediate.
  let suppressNextAudioSegmentBarClick = false;
  let suppressNextAudioSegmentBarClickResetTimer: ReturnType<typeof setTimeout> | null =
    null;

  function clearPendingSuppressedAudioSegmentBarClick() {
    if (suppressNextAudioSegmentBarClickResetTimer != null) {
      clearTimeout(suppressNextAudioSegmentBarClickResetTimer);
      suppressNextAudioSegmentBarClickResetTimer = null;
    }
    suppressNextAudioSegmentBarClick = false;
  }

  function rememberSuppressedAudioSegmentBarClick() {
    clearPendingSuppressedAudioSegmentBarClick();
    suppressNextAudioSegmentBarClick = true;
    suppressNextAudioSegmentBarClickResetTimer = setTimeout(() => {
      suppressNextAudioSegmentBarClickResetTimer = null;
      suppressNextAudioSegmentBarClick = false;
    }, 250);
  }

  function onAudioDrawerOutsidePointerDown(event: PointerEvent) {
    if (selectedAudioSegmentId == null) return;
    clearPendingSuppressedAudioSegmentBarClick();
    const target = event.target;
    if (!(target instanceof Node)) return;
    if (audioDrawerEl?.contains(target)) return;
    if (target instanceof Element && target.closest(".timeline-rail__audio-bar")) {
      rememberSuppressedAudioSegmentBarClick();
    }
    closeAudioDrawer();
  }

  $effect(() => {
    if (selectedAudioSegmentId == null) return;
    // Bind in capture phase so the dismissal beats any in-tree handlers
    // that might `stopPropagation` on the same event (the rail wrap stops
    // pointer/click propagation in some flows).
    document.addEventListener(
      "pointerdown",
      onAudioDrawerOutsidePointerDown,
      true,
    );
    return () => {
      document.removeEventListener(
        "pointerdown",
        onAudioDrawerOutsidePointerDown,
        true,
      );
    };
  });

  // Drop a stale selection if the segment no longer appears in the loaded
  // window. We compare ids rather than object identity because `audioSegments`
  // is rebuilt on every refresh.
  $effect(() => {
    if (selectedAudioSegmentId == null) return;
    if (!audioSegments.some((s) => s.id === selectedAudioSegmentId)) {
      selectedAudioSegmentId = null;
    }
  });

  // ─── Audio player drawer a11y ────────────────────────────────────────────
  // The audio player is rendered as a non-modal `role="dialog"` bottom sheet
  // that slides in only when an audio segment is selected. The timeline lane
  // remains interactive while the drawer is open so users can swap selection
  // by clicking another bar; the drawer reacts to selection changes by
  // refreshing its metadata + media. We wire up Escape-to-close, a Tab focus
  // trap while open, and focus restoration to the previously-selected audio
  // bar (if still present) when the drawer closes.
  let audioDrawerEl = $state<HTMLDivElement | null>(null);
  let audioDrawerCloseEl = $state<HTMLButtonElement | null>(null);
  // Capture the element that had focus immediately before the drawer opened
  // so we can return focus there on close. Recomputed on each open transition.
  let audioDrawerReturnFocusEl: HTMLElement | null = null;

  function getAudioDrawerFocusable(): HTMLElement[] {
    if (!audioDrawerEl) return [];
    const sel =
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
    return Array.from(audioDrawerEl.querySelectorAll<HTMLElement>(sel)).filter(
      (el) => el.offsetParent !== null || el === document.activeElement,
    );
  }

  function closeAudioDrawer() {
    selectedAudioSegmentId = null;
  }

  function onAudioDrawerKeydown(e: KeyboardEvent) {
    if (selectedAudioSegmentId == null) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeAudioDrawer();
      return;
    }
    if (e.key !== "Tab") return;
    const focusable = getAudioDrawerFocusable();
    if (focusable.length === 0) {
      e.preventDefault();
      audioDrawerEl?.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement as HTMLElement | null;
    if (e.shiftKey) {
      if (active === first || !audioDrawerEl?.contains(active)) {
        e.preventDefault();
        last.focus();
      }
    } else if (active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // Track the open/close transition: capture return-focus target on open,
  // move focus into the drawer's close button after mount, and restore focus
  // on close.
  $effect(() => {
    const open = selectedAudioSegmentId != null;
    if (!open) return;
    audioDrawerReturnFocusEl = document.activeElement as HTMLElement | null;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || selectedAudioSegmentId == null) return;
      (audioDrawerCloseEl ?? audioDrawerEl)?.focus();
    });
    return () => {
      cancelled = true;
      // Restore focus to the originating element only if focus has not moved
      // somewhere unrelated (e.g. user clicked into another control already).
      const active = document.activeElement as HTMLElement | null;
      if (
        !active ||
        active === document.body ||
        audioDrawerEl?.contains(active)
      ) {
        audioDrawerReturnFocusEl?.focus({ preventScroll: true });
      }
    };
  });

  // ─── Audio overlay alignment ─────────────────────────────────────────────
  // The frame rail is *frame-indexed*: each frame occupies a fixed 8px slot
  // regardless of capture cadence (the OCR/processing pipeline samples by
  // activity, so dense active periods get many frames and idle periods get
  // few). Audio bars are rendered alongside this rail and need to **cover
  // the frames captured during their segment**, not represent wall-clock
  // duration — otherwise a bar's width drifts away from the slots it sits
  // over and clicking the bar feels disconnected from the frames it owns.
  //
  // For each segment we therefore find the contiguous range of frame
  // indices whose `capturedAt` falls within `[startUnixMs, endUnixMs]` and
  // size the bar to span exactly those slots. Out-of-window endpoints are
  // clamped to the loaded edge so a segment that started before the oldest
  // loaded frame still extends to the leftmost slot. A consequence
  // accepted by design (see the dashboard convo around audio-bar/frame
  // alignment): a 60s segment covering an idle period with two frames
  // renders narrower than a 9s segment over a dense burst of frames.
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
    const out: PositionedAudioSegment[] = [];
    for (const seg of audioSegments) {
      // Segment entirely outside the loaded window — nothing to cover.
      if (seg.endUnixMs < oldestMs || seg.startUnixMs > newestMs) {
        out.push({ ...seg, rightPx: 0, widthPx: 0, visible: false });
        continue;
      }
      // `times` is sorted descending (newest first). Find the newest
      // frame whose timestamp is `<= seg.endUnixMs` (smallest matching
      // index) and the oldest whose timestamp is `>= seg.startUnixMs`
      // (largest matching index). Linear scans are fine: `n` is bounded
      // by the loaded window (a few hundred to a few thousand frames)
      // and the segment count per render is similarly small.
      let iNewest = -1;
      for (let i = 0; i < n; i++) {
        const t = times[i]!;
        if (Number.isFinite(t) && t <= seg.endUnixMs) {
          iNewest = i;
          break;
        }
      }
      let iOldest = -1;
      for (let i = n - 1; i >= 0; i--) {
        const t = times[i]!;
        if (Number.isFinite(t) && t >= seg.startUnixMs) {
          iOldest = i;
          break;
        }
      }
      // Segment overlaps the window in time but no loaded frame falls
      // inside its range (e.g. a quiet gap between two frames straddles
      // the segment). Clamp to the nearest in-range neighbour so the bar
      // still anchors to a real slot rather than disappearing.
      if (iNewest === -1) iNewest = 0;
      if (iOldest === -1) iOldest = n - 1;
      if (iOldest < iNewest) iOldest = iNewest;
      // Slot at index `i` is positioned with `right: i * SLOT_WIDTH` and
      // is `SLOT_WIDTH` wide. To cover slots `[iNewest..iOldest]` we
      // anchor the bar at the newest slot's right edge and extend left
      // by one slot per covered frame.
      const rightPx = iNewest * TIMELINE_SLOT_WIDTH;
      const widthPx = Math.max(
        2,
        (iOldest - iNewest + 1) * TIMELINE_SLOT_WIDTH,
      );
      out.push({ ...seg, rightPx, widthPx, visible: true });
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

  function trimTimelineFramesAroundActive(
    frames: FrameDto[],
    activeIndex: number,
  ): TrimmedTimelineFrames {
    if (frames.length <= TIMELINE_MAX_LOADED_FRAMES) {
      return { frames, activeIndex, trimmedHead: false, trimmedTail: false };
    }
    const safeActiveIndex = Math.max(0, Math.min(frames.length - 1, activeIndex));
    const maxStart = frames.length - TIMELINE_MAX_LOADED_FRAMES;
    const desiredStart = safeActiveIndex - Math.floor(TIMELINE_MAX_LOADED_FRAMES / 2);
    const start = Math.max(0, Math.min(maxStart, desiredStart));
    const trimmedFrames = frames.slice(start, start + TIMELINE_MAX_LOADED_FRAMES);
    return {
      frames: trimmedFrames,
      activeIndex: safeActiveIndex - start,
      trimmedHead: start > 0,
      trimmedTail: start + TIMELINE_MAX_LOADED_FRAMES < frames.length,
    };
  }

  function prunePreviewCache(frames: FrameDto[]): void {
    if (previewCache.size === 0) return;
    const keep = new Set(frames.map((frame) => frame.id));
    const next = new Map<number, string>();
    for (const [frameId, url] of previewCache) {
      if (keep.has(frameId)) next.set(frameId, url);
    }
    if (next.size !== previewCache.size) {
      previewCache = next;
    }
  }

  async function syncTimelineScrollToActiveFrame(): Promise<void> {
    await tick();
    if (!timelineRail) {
      timelineScrollLeft = 0;
      return;
    }
    const max = timelineRail.scrollWidth - timelineRail.clientWidth;
    const targetScrollLeft = Math.max(
      0,
      Math.min(max, max - timelineActiveIndex * TIMELINE_SLOT_WIDTH),
    );
    timelineRail.scrollLeft = targetScrollLeft;
    timelineScrollLeft = targetScrollLeft;
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
      timelineHasNewer = false;
      timelineShowingHistoricalWindow = false;
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
        await syncTimelineScrollToActiveFrame();
        // Drop cached previews from any prior generation — keeping them
        // would grow unboundedly across refreshes.
        previewCache = new Map();
        // Targeted picker invalidation: only invalidate months actually
        // covered by the freshly loaded frames. Wholesale clearing of
        // `summariesByDate`/`loadedMonths` would force the open picker to
        // re-fetch and remount its disabled-date map, producing a visible
        // flicker on every routine refresh — even when no new month was
        // affected. The picker effect re-loads the visible month whenever
        // `pickerPlaceholder` changes; for the case where the affected
        // month IS already visible, we trigger a reload below.
        invalidatePickerMonthsForFrames(page);
      } else {
        const anchorFrame = timelineFrames[timelineActiveIndex] ?? null;
        const mergedFrames = timelineFrames.concat(page);
        const mergedActiveIndex = anchorFrame
          ? mergedFrames.findIndex((frame) => frame.id === anchorFrame.id)
          : timelineActiveIndex;
        const trimmed = trimTimelineFramesAroundActive(
          mergedFrames,
          mergedActiveIndex >= 0 ? mergedActiveIndex : timelineActiveIndex,
        );
        timelineFrames = trimmed.frames;
        timelineActiveIndex = trimmed.activeIndex;
        if (trimmed.trimmedHead) {
          timelineHasNewer = true;
          timelineShowingHistoricalWindow = true;
        }
        if (trimmed.trimmedTail) {
          timelineExhausted = false;
        }
        prunePreviewCache(timelineFrames);
        await syncTimelineScrollToActiveFrame();
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

  async function loadTimelineNewerPage() {
    if (!timelineShowingHistoricalWindow || !timelineHasNewer) return;
    if (timelineLoading || timelineLoadingMore) return;
    const head = timelineFrames[0];
    if (!head) return;

    timelineLoadingMore = true;
    const gen = timelineGeneration;

    try {
      const window = await invoke<FocusedFrameWindowDto>(
        "get_timeline_window_around_frame",
        {
          request: {
            frameId: head.id,
            newerLimit: TIMELINE_PAGE_SIZE,
            olderLimit: 0,
          } satisfies GetTimelineWindowAroundFrameRequest,
        },
      );
      if (gen !== timelineGeneration) return;
      if (!window.frames[window.targetIndex] || window.frames[window.targetIndex]?.id !== head.id) {
        timelineError = "failed to page newer timeline frames";
        return;
      }

      const page = window.frames.slice(0, window.targetIndex);
      timelineHasNewer = window.hasNewer;
      timelineShowingHistoricalWindow = window.hasNewer;

      if (page.length === 0) {
        timelineError = null;
        return;
      }

      const anchorFrame = timelineFrames[timelineActiveIndex] ?? null;

      // Restore the anchored active index synchronously, in the same turn as
      // the prepend, so `timelineActive` (derived from
      // `timelineFrames[timelineActiveIndex]`) does not transiently point at
      // a different frame between the array assignment and the post-tick
      // re-find. Without this, the OCR reset effect observes a momentary id
      // mismatch and clears OCR state even though the user is still parked
      // on the same logical frame. Because we strictly prepend `page` to the
      // head, the anchor's new index is just `oldIndex + page.length`.
      const mergedFrames = page.concat(timelineFrames);
      const mergedActiveIndex = anchorFrame
        ? mergedFrames.findIndex((frame) => frame.id === anchorFrame.id)
        : timelineActiveIndex + page.length;
      const trimmed = trimTimelineFramesAroundActive(
        mergedFrames,
        mergedActiveIndex >= 0 ? mergedActiveIndex : timelineActiveIndex + page.length,
      );
      timelineFrames = trimmed.frames;
      timelineActiveIndex = trimmed.activeIndex;
      timelineHasNewer = window.hasNewer || trimmed.trimmedHead;
      timelineShowingHistoricalWindow = timelineHasNewer;
      if (trimmed.trimmedTail) {
        timelineExhausted = false;
      }
      invalidatePickerMonthsForFrames(page);
      prunePreviewCache(timelineFrames);
      await syncTimelineScrollToActiveFrame();

      if (anchorFrame) {
        // Defensive re-find in case the prepended page somehow contained the
        // anchor (shouldn't happen — `page` is strictly newer than head — but
        // keep the previous correctness guarantee).
        const newIdx = timelineFrames.findIndex((frame) => frame.id === anchorFrame.id);
        if (newIdx >= 0) timelineActiveIndex = newIdx;
      }
      await syncTimelineScrollToActiveFrame();

      timelineError = null;
    } catch (err) {
      if (gen !== timelineGeneration) return;
      timelineError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      if (gen === timelineGeneration) {
        timelineLoadingMore = false;
      }
    }

    void refreshAudioSegments();

    if (
      timelineShowingHistoricalWindow &&
      timelineHasNewer &&
      !timelineLoading &&
      !timelineLoadingMore &&
      timelineActiveIndex <= TIMELINE_PREFETCH_AHEAD
    ) {
      loadTimelineNewerPage();
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
  // Outside historical-window mode, the merge has three branches:
  //   1. Rail is empty → behave like the empty→populated half of a reset:
  //      seed the frames, scroll to the right edge, and invalidate the date
  //      picker's month summary cache so the new dates show as available.
  //      Preview cache is left alone (it's empty in this branch anyway), so
  //      the active-preview effect can hydrate it normally.
  //   2. Rail already has data → preserve the exact frame the user is viewing,
  //      even if that frame is the current latest. Slot 0 is anchored to the
  //      track's right edge, so prepending N frames adds
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
        // Targeted picker invalidation for the months the new frames
        // belong to. Avoids flickering the open picker by leaving cached
        // months untouched when they don't overlap the freshly arrived
        // frames; the picker's own effect re-loads the visible month if
        // its placeholder month was invalidated below.
        invalidatePickerMonthsForFrames(firstPage);
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
        if (timelineShowingHistoricalWindow) {
          if (fresh.length > 0) {
            invalidatePickerMonthsForFrames(fresh);
          }
          return;
        }
        // Page budget exhausted before reaching the local head: splicing in
        // `fresh` would leave a hole between it and the existing frames.
        // Fall back to a full reset so the rail stays internally consistent.
        // This is rare (requires a sustained burst that outpaces the budget)
        // and is preferable to a silent inconsistency.
        await loadTimelinePage(true);
        return;
      }

      if (timelineShowingHistoricalWindow) {
        if (fresh.length > 0) {
          invalidatePickerMonthsForFrames(fresh);
        }
        return;
      }

      if (fresh.length === 0) {
        return;
      }

      // Capture the frame the user is currently parked on so we can find it
      // again after the prepend and keep it under the cursor. When the
      // follow-live setting is enabled we intentionally reselect the newest
      // frame instead.
      const anchorFrame = followTimelineLive ? null : (timelineFrames[timelineActiveIndex] ?? null);
      const mergedFrames = fresh.concat(timelineFrames);

      // Restore the anchored active index synchronously, in the same turn as
      // the prepend, so `timelineActive` (derived from
      // `timelineFrames[timelineActiveIndex]`) does not transiently point at
      // a different frame between the array assignment and the post-tick
      // re-find below. Without this, the OCR reset effect observes a brief
      // id mismatch and clears OCR state/visibility even though the user is
      // still parked on the same logical frame. `fresh` is strictly newer
      // than the prior head, so the anchor's new index is just
      // `oldIndex + fresh.length`.
      const mergedActiveIndex = anchorFrame
        ? mergedFrames.findIndex((frame) => frame.id === anchorFrame.id)
        : 0;
      const trimmed = trimTimelineFramesAroundActive(
        mergedFrames,
        mergedActiveIndex >= 0 ? mergedActiveIndex : 0,
      );
      timelineFrames = trimmed.frames;
      timelineActiveIndex = trimmed.activeIndex;
      if (trimmed.trimmedHead) {
        timelineHasNewer = true;
        timelineShowingHistoricalWindow = true;
      }
      if (trimmed.trimmedTail) {
        timelineExhausted = false;
      }
      prunePreviewCache(timelineFrames);

      // Targeted picker invalidation for months touched by the freshly
      // merged frames only. A blanket reset of `summariesByDate` /
      // `loadedMonths` on every poll would force the picker (if open) to
      // re-fetch its visible month each tick, causing the calendar's
      // disabled-date map to flicker and stealing focus on bits-ui
      // rebuilds. Months unrelated to the new frames keep their cached
      // summaries.
      invalidatePickerMonthsForFrames(fresh);

      await syncTimelineScrollToActiveFrame();
      if (followTimelineLive) {
        timelineActiveIndex = 0;
        await syncTimelineScrollToActiveFrame();
      } else if (anchorFrame) {
        // Re-find the anchor and shift the active index so the same frame
        // stays selected. `findIndex` is robust to either the linear shift
        // (the common case) or any future merging logic that reorders.
        const newIdx = timelineFrames.findIndex((f) => f.id === anchorFrame.id);
        if (newIdx >= 0) {
          timelineActiveIndex = newIdx;
        }
        await syncTimelineScrollToActiveFrame();
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
      timelineShowingHistoricalWindow &&
      timelineHasNewer &&
      !timelineLoadingMore &&
      idx <= TIMELINE_PREFETCH_AHEAD
    ) {
      loadTimelineNewerPage();
      return;
    }
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
    if (suppressNextAudioSegmentBarClick) {
      clearPendingSuppressedAudioSegmentBarClick();
      selectedAudioSegmentId = null;
      return;
    }
    clearPendingSuppressedAudioSegmentBarClick();
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

  // ─── On-demand OCR for the active frame ──────────────────────────────────
  // The "Run OCR" header button reprocesses the active frame and polls the
  // resulting processing job until it reaches a terminal state. On success we
  // parse the structured payload (Apple Vision: normalised coords with
  // lower-left origin) and overlay each observation as a translucent box +
  // text label on the preview. The overlay is positioned against the
  // *rendered* image bounds inside the stage (object-fit: contain), not the
  // full stage rect, so boxes align with what the user actually sees.
  //
  // State machine:
  //   "idle"     — no fetch requested for the current frame
  //   "running"  — an OCR job exists for this frame but has not yet
  //                terminated on the backend (queued or running). We do NOT
  //                poll; the user can click again to re-check.
  //   "success"  — completed job with at least one observation.
  //   "empty"    — completed job with zero observations.
  //   "missing"  — no OCR job/result has ever been recorded for this frame.
  //   "error"    — fetch failed, the existing job is in failed state, or
  //                its result payload is missing/invalid.
  //
  // Switching active frame clears any prior OCR state so a stale overlay
  // never sits on the wrong preview. A monotonic generation token prevents a
  // late response for an old frame from writing into the new frame's state.
  type OcrStatus = "idle" | "running" | "success" | "empty" | "missing" | "error";
  const FRAME_SUBJECT_TYPE = "frame";
  const OCR_PROCESSOR = "ocr";

  let ocrStatus = $state<OcrStatus>("idle");
  let ocrError = $state<string | null>(null);
  let ocrObservations = $state<OcrObservation[]>([]);
  let ocrFrameId = $state<number | null>(null);
  let ocrSourceFrame = $state<FrameDto | null>(null);
  let ocrGeneration = 0;
  // Whether the OCR overlay/status surface is currently shown for the active
  // frame. Tracked separately from `ocrStatus` so the button can hide a
  // loaded result without discarding it (re-show is a no-op fetch) and so
  // non-success statuses (running/missing/empty/error) still respect the
  // user's intent to view or dismiss the OCR surface.
  let ocrVisible = $state(false);

  // Clear stale overlay state whenever the active frame id changes.
  $effect(() => {
    const id = timelineActive?.id ?? null;
    if (id !== ocrFrameId) {
      ocrGeneration += 1;
      ocrStatus = "idle";
      ocrError = null;
      ocrObservations = [];
      ocrFrameId = null;
      ocrSourceFrame = null;
      ocrVisible = false;
    }
  });

  function parseOcrPayload(json: string | null | undefined): OcrObservation[] | null {
    if (!json) return null;
    try {
      const parsed = JSON.parse(json) as Partial<OcrStructuredPayload>;
      const obs = Array.isArray(parsed?.observations) ? parsed.observations : null;
      if (!obs) return null;
      // Defensive normalisation: keep only entries whose bounding box is a
      // sane numeric rectangle so a malformed observation can't crash the
      // overlay render.
      const out: OcrObservation[] = [];
      for (const o of obs) {
        const bb = o?.boundingBox;
        if (
          !bb ||
          typeof bb.x !== "number" ||
          typeof bb.y !== "number" ||
          typeof bb.width !== "number" ||
          typeof bb.height !== "number"
        )
          continue;
        out.push({
          text: typeof o.text === "string" ? o.text : "",
          confidence: typeof o.confidence === "number" ? o.confidence : 0,
          boundingBox: {
            x: bb.x,
            y: bb.y,
            width: bb.width,
            height: bb.height,
          },
        });
      }
      return out;
    } catch {
      return null;
    }
  }

  async function loadOcrForFrame(sourceFrame: FrameDto): Promise<{
    status: OcrStatus;
    observations: OcrObservation[];
    error: string | null;
  }> {
    const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
      request: { subjectType: FRAME_SUBJECT_TYPE, subjectId: sourceFrame.id },
    });

    const ocrJobs = jobs.filter((j) => j.processor === OCR_PROCESSOR);
    if (ocrJobs.length === 0) {
      return { status: "missing", observations: [], error: null };
    }

    const completed = ocrJobs
      .filter((j) => j.status === "completed")
      .sort((a, b) => b.id - a.id);
    const job = completed[0] ?? ocrJobs.sort((a, b) => b.id - a.id)[0];

    if (job.status === "queued" || job.status === "running") {
      return { status: "running", observations: [], error: null };
    }
    if (job.status === "failed") {
      return {
        status: "error",
        observations: [],
        error: job.lastError ?? "OCR job failed",
      };
    }

    const result = await invoke<ProcessingResultDto | null>("get_processing_result", {
      request: { jobId: job.id } satisfies GetProcessingResultRequest,
    });

    const observations = parseOcrPayload(result?.structuredPayloadJson);
    if (observations === null) {
      return {
        status: "error",
        observations: [],
        error: result ? "OCR result payload is missing or invalid" : "OCR result not available",
      };
    }

    return {
      status: observations.length === 0 ? "empty" : "success",
      observations,
      error: null,
    };
  }

  async function loadOcrForActiveFrame(): Promise<void> {
    const frame = timelineActive;
    if (!frame) return;
    // Bump the generation so any in-flight fetch for a prior call is dropped
    // when its response checks the token.
    ocrGeneration += 1;
    const gen = ocrGeneration;
    const frameId = frame.id;
    ocrFrameId = frameId;
    ocrStatus = "running";
    ocrError = null;
    ocrObservations = [];
    ocrSourceFrame = frame;
    ocrVisible = true;

    try {
      if (gen !== ocrGeneration) return;

      let sourceFrame = frame;
      let ocrData = await loadOcrForFrame(sourceFrame);
      if (gen !== ocrGeneration) return;

      if (ocrData.status === "missing" && frame.contentFingerprint) {
        const fallbackFrame = await invoke<FrameDto | null>(
          "get_first_matching_earlier_frame_by_fingerprint",
          {
            request: {
              sessionId: frame.sessionId,
              beforeFrameId: frame.id,
              contentFingerprint: frame.contentFingerprint,
            } satisfies GetFirstMatchingEarlierFrameByFingerprintRequest,
          },
        );
        if (gen !== ocrGeneration) return;

        if (fallbackFrame) {
          sourceFrame = fallbackFrame;
          ocrData = await loadOcrForFrame(sourceFrame);
          if (gen !== ocrGeneration) return;
        }
      }

      ocrSourceFrame = sourceFrame;
      ocrStatus = ocrData.status;
      ocrError = ocrData.error;
      ocrObservations = ocrData.observations;
    } catch (err) {
      if (gen !== ocrGeneration) return;
      ocrStatus = "error";
      ocrError = typeof err === "string" ? err : (err as Error)?.message ?? JSON.stringify(err);
      ocrSourceFrame = frame;
    }
  }

  /**
   * Toggle the OCR surface for the active frame.
   *
   * - If the overlay is currently visible for the active frame, hide it
   *   without re-fetching. Loaded observations are kept around so a
   *   subsequent show is instant.
   * - Otherwise, if a prior load already produced data for this frame
   *   (`ocrFrameId` matches and we're not in the `idle` reset state), just
   *   re-show the existing surface — no network round-trip.
   * - Otherwise kick off a fresh load via `loadOcrForActiveFrame`, which
   *   handles running/missing/empty/error statuses as before.
   */
  function toggleOcrForActiveFrame(): void {
    const frame = timelineActive;
    if (!frame) return;
    if (ocrVisible) {
      ocrVisible = false;
      return;
    }
    if (ocrFrameId === frame.id && ocrStatus !== "idle") {
      ocrVisible = true;
      return;
    }
    void loadOcrForActiveFrame();
  }

  // ─── OCR overlay geometry ────────────────────────────────────────────────
  // The preview is painted as a `background-image` with `background-size:
  // contain` on a stage-filling div. There's no `<img>` element to measure,
  // so we derive the visible image rect from the stage's client size plus
  // the active frame's intrinsic width/height (FrameDto.width/height). The
  // contain rule scales by the smaller of (stageW/imgW, stageH/imgH) and
  // centers the result, so we replicate that math here. The OCR overlay is
  // a child of the stage positioned to that rect with `overflow: hidden`,
  // so any out-of-bounds OCR box gets clipped to the visible image.
  let stageEl = $state<HTMLDivElement | null>(null);
  let stageWidth = $state(0);
  let stageHeight = $state(0);

  type RenderedImageRect = { left: number; top: number; width: number; height: number };

  // Painted background-image rect derived from stage size + active frame's
  // intrinsic dimensions. Falls back to a zero rect when either side is
  // unknown (no active frame yet, missing dims, stage not measured) so the
  // OCR overlay's render gate (`width > 0 && height > 0`) hides cleanly.
  const renderedImageRect = $derived.by<RenderedImageRect>(() => {
    const sw = stageWidth;
    const sh = stageHeight;
    const iw = timelineActive?.width ?? null;
    const ih = timelineActive?.height ?? null;
    if (!sw || !sh || !iw || !ih || iw <= 0 || ih <= 0) {
      return { left: 0, top: 0, width: 0, height: 0 };
    }
    const scale = Math.min(sw / iw, sh / ih);
    const width = iw * scale;
    const height = ih * scale;
    const left = (sw - width) / 2;
    const top = (sh - height) / 2;
    return { left, top, width, height };
  });

  // Track the stage's content-box size so the derived rect updates as the
  // window/layout resizes around it.
  $effect(() => {
    const stage = stageEl;
    if (!stage) {
      stageWidth = 0;
      stageHeight = 0;
      return;
    }
    const measure = () => {
      stageWidth = stage.clientWidth;
      stageHeight = stage.clientHeight;
    };
    measure();
    if (typeof ResizeObserver === "undefined") {
      const onWindowResize = () => measure();
      window.addEventListener("resize", onWindowResize);
      return () => window.removeEventListener("resize", onWindowResize);
    }
    const ro = new ResizeObserver(() => measure());
    ro.observe(stage);
    return () => ro.disconnect();
  });

  // OCR box styles are expressed in PERCENTAGES of the overlay wrapper.
  // The wrapper itself is sized/positioned to match the measured image
  // rect (see template), so percentage coordinates inside it map 1:1 onto
  // image-space coordinates. The lower-left origin of the source space
  // means y must be flipped to CSS top.
  function ocrBoxStyle(obs: OcrObservation): string {
    const bb = obs.boundingBox;
    const leftPct = bb.x * 100;
    const topPct = (1 - bb.y - bb.height) * 100;
    const widthPct = bb.width * 100;
    const heightPct = bb.height * 100;
    return `left: ${leftPct}%; top: ${topPct}%; width: ${widthPct}%; height: ${heightPct}%;`;
  }

  const ocrButtonLabel = $derived(
    ocrStatus === "running"
      ? "loading OCR…"
      : ocrVisible
        ? "hide OCR"
        : "show OCR",
  );

  const ocrUsingEarlierFrame = $derived(
    !!timelineActive && !!ocrSourceFrame && ocrSourceFrame.id !== timelineActive.id,
  );

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
  //   - After resolving the target we load a focused newest-first window
  //     around that frame in one request, then scroll the rail to the
  //     returned target index. From there the rail can page both directions:
  //     newer frames back toward the live head from the loaded start, and
  //     older history from the loaded tail via the normal `beforeId` path.

  type DateKey = string; // "YYYY-MM-DD" in local time
  type MonthKey = string; // "YYYY-MM" in local time

  let pickerOpen = $state(false);
  let pickerPlaceholder = $state<DateValue>(todayLocal());
  let pickerSelectedDate = $state<DateValue | undefined>(undefined);
  let pickerSelectedTime = $state<string | null>(null); // "HH:MM"
  let summariesByDate = $state<Map<DateKey, FrameSummaryDto[]>>(new Map());
  let loadedMonths = $state<Set<MonthKey>>(new Set());
  // Months whose cached summaries are known to be out-of-date because new
  // frames have arrived in them. Kept SEPARATE from `loadedMonths` /
  // `summariesByDate` so the open picker keeps rendering the existing
  // disabled-date map while a background revalidation is in flight — a
  // stale-while-revalidate strategy that avoids the visible flicker that
  // came from deleting a month's cache before its replacement landed.
  let staleMonths = $state<Set<MonthKey>>(new Set());
  // In-flight month fetches. Dedupes concurrent revalidations triggered by
  // the picker effect, manual refresh, and head poll all racing to refresh
  // the same month.
  const monthsInFlight = new Set<MonthKey>();
  let pickerLoading = $state(false);
  let pickerJumping = $state(false);
  let pickerError = $state<string | null>(null);
  let pickerStyle = $state("");

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

  /**
   * Targeted invalidation of the date-jump picker's per-month summary
   * cache. Given a set of newly-arrived frames, compute the LOCAL months
   * they belong to and MARK those months stale. We deliberately do NOT
   * delete `loadedMonths` entries or drop rows from `summariesByDate`:
   * doing so during a routine refresh / head poll would unmount the open
   * picker's disabled-date map for the visible month and produce a
   * visible flicker every poll, even when no UI-visible change exists.
   *
   * Reactivity: `staleMonths` is itself reactive state, and the picker's
   * `$effect` reads it via `loadMonthSummaries`, so marking the visible
   * month stale here re-triggers a background revalidation. The existing
   * cached data stays mounted until the replacement response lands, at
   * which point `loadMonthSummaries` swaps the affected month's rows in
   * one assignment and clears the stale flag (see below).
   */
  function invalidatePickerMonthsForFrames(frames: { capturedAt: string }[]): void {
    if (frames.length === 0) return;
    const affectedMonths = new Set<MonthKey>();
    for (const f of frames) {
      const d = parseCapturedAt(f.capturedAt);
      if (isNaN(d.getTime())) continue;
      affectedMonths.add(`${d.getFullYear()}-${pad2(d.getMonth() + 1)}`);
    }
    if (affectedMonths.size === 0) return;
    let changed = false;
    const next = new Set(staleMonths);
    for (const m of affectedMonths) {
      if (!next.has(m)) {
        next.add(m);
        changed = true;
      }
    }
    if (changed) staleMonths = next;
  }

  async function loadMonthSummaries(value: DateValue): Promise<void> {
    const key = monthKeyOf(value);
    const isStale = staleMonths.has(key);
    // Already up-to-date and loaded — nothing to do.
    if (loadedMonths.has(key) && !isStale) return;
    // Another caller is already revalidating this month; let its response
    // be the one that swaps the data in. Prevents fetch storms when the
    // picker effect, head poll, and manual refresh all race.
    if (monthsInFlight.has(key)) return;
    monthsInFlight.add(key);
    // Only show the spinner when there's nothing to render yet. Stale
    // revalidations happen quietly so the existing disabled-date map keeps
    // rendering until replacement data arrives.
    const isFirstLoad = !loadedMonths.has(key);
    if (isFirstLoad) pickerLoading = true;
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
      // Atomically swap this month's rows: drop any prior entries whose
      // local date falls inside this month, then insert the fresh ones.
      // Doing this in one assignment means the picker never observes an
      // intermediate "month exists in loadedMonths but has no rows" state.
      const next = new Map(summariesByDate);
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
      if (!loadedMonths.has(key)) {
        const nextMonths = new Set(loadedMonths);
        nextMonths.add(key);
        loadedMonths = nextMonths;
      }
      if (staleMonths.has(key)) {
        const nextStale = new Set(staleMonths);
        nextStale.delete(key);
        staleMonths = nextStale;
      }
      pickerError = null;
    } catch (err) {
      pickerError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      monthsInFlight.delete(key);
      if (isFirstLoad) pickerLoading = false;
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
    timelineGeneration += 1;
    const gen = timelineGeneration;
    timelineLoading = true;
    timelineLoadingMore = false;
    try {
      const request: GetTimelineWindowAroundFrameRequest = {
        frameId: target.id,
        newerLimit: TIMELINE_JUMP_WINDOW_NEWER_LIMIT,
        olderLimit: TIMELINE_JUMP_WINDOW_OLDER_LIMIT,
      };
      const window = await invoke<FocusedFrameWindowDto>(
        "get_timeline_window_around_frame",
        { request },
      );
      if (gen !== timelineGeneration) return;
      if (!window.frames[window.targetIndex] || window.frames[window.targetIndex]?.id !== target.id) {
        pickerError = "failed to focus selected frame";
        return;
      }
      timelineFrames = window.frames;
      timelineActiveIndex = window.targetIndex;
      timelineExhausted = !window.hasOlder;
      timelineHasNewer = window.hasNewer;
      timelineError = null;
      timelineShowingHistoricalWindow = window.hasNewer;
      previewCache = new Map();
      await syncTimelineScrollToActiveFrame();
      void refreshAudioSegments();
      pickerOpen = false;
    } catch (err) {
      if (gen !== timelineGeneration) return;
      pickerError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      if (gen === timelineGeneration) {
        timelineLoading = false;
        timelineLoadingMore = false;
      }
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

  function updatePickerPosition(): void {
    if (!pickerEl || !pickerTriggerEl) return;
    const viewportMargin = 12;
    const triggerGap = 6;
    const triggerRect = pickerTriggerEl.getBoundingClientRect();
    const pickerRect = pickerEl.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const pickerWidth = Math.min(
      pickerRect.width,
      Math.max(0, viewportWidth - viewportMargin * 2),
    );

    let left = triggerRect.left;
    if (triggerRect.left + pickerWidth > viewportWidth - viewportMargin) {
      left = triggerRect.right - pickerWidth;
    }
    left = Math.min(
      Math.max(viewportMargin, left),
      Math.max(viewportMargin, viewportWidth - viewportMargin - pickerWidth),
    );

    const availableBelow = Math.max(
      160,
      viewportHeight - triggerRect.bottom - triggerGap - viewportMargin,
    );
    const availableAbove = Math.max(160, triggerRect.top - triggerGap - viewportMargin);
    const maxHeight = Math.min(420, Math.max(availableBelow, availableAbove));
    const openAbove = availableBelow < 260 && availableAbove > availableBelow;
    const top = openAbove
      ? Math.max(
          viewportMargin,
          triggerRect.top - triggerGap - Math.min(pickerRect.height, maxHeight),
        )
      : Math.min(
          triggerRect.bottom + triggerGap,
          viewportHeight - viewportMargin - Math.min(pickerRect.height, maxHeight),
        );

    pickerStyle = `left: ${left}px; top: ${top}px; max-height: ${maxHeight}px;`;
  }

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
      updatePickerPosition();
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

  $effect(() => {
    if (!pickerOpen) {
      pickerStyle = "";
      return;
    }

    let frame = 0;
    const scheduleUpdate = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => updatePickerPosition());
    };

    void tick().then(scheduleUpdate);

    const ro = new ResizeObserver(scheduleUpdate);
    if (pickerEl) ro.observe(pickerEl);
    if (pickerTriggerEl) ro.observe(pickerTriggerEl);

    window.addEventListener("resize", scheduleUpdate);
    window.addEventListener("scroll", scheduleUpdate, true);

    return () => {
      cancelAnimationFrame(frame);
      ro.disconnect();
      window.removeEventListener("resize", scheduleUpdate);
      window.removeEventListener("scroll", scheduleUpdate, true);
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

  // ─── Recording controls ──────────────────────────────────────────────────
  // Mirrors the debug page: bootstrap the shared `captureSession` store via
  // `get_capture_permissions`, load `recording_settings` so a fresh start
  // honours the user's persisted source toggles, and toggle between
  // `start_native_capture` / `stop_native_capture`. A monotonic generation
  // token prevents a slow `get_capture_permissions` reconciliation response
  // from clobbering an authoritative start/stop write that landed first.
  const captureSessionValue = $derived(captureSession.value);
  const isCapturing = $derived(captureSessionValue?.isRunning === true);
  const isInactivityPaused = $derived(
    captureSessionValue?.isInactivityPaused === true,
  );
  let recordingSettings = $state<RecordingSettings | null>(null);
  let captureLoadingStart = $state(false);
  let captureLoadingStop = $state(false);
  let captureLoadingSettings = $state(false);
  let captureBootstrapped = $state(false);
  let captureError = $state<string | null>(null);
  let captureSessionGeneration = 0;
  const followTimelineLive = $derived(recordingSettings?.followTimelineLive === true);

  const captureStatusLabel = $derived(
    isCapturing
      ? isInactivityPaused
        ? "Paused"
        : "Recording"
      : captureSessionValue?.isRunning === false
        ? "Stopped"
        : "Idle",
  );
  const captureStatusModifier = $derived(
    isCapturing
      ? isInactivityPaused
        ? "paused"
        : "running"
      : "idle",
  );

  async function bootstrapCaptureControls(): Promise<void> {
    captureLoadingSettings = true;
    const gen = captureSessionGeneration;
    try {
      const [perm, settings] = await Promise.all([
        invoke<GetPermissionsResponse>("get_capture_permissions"),
        invoke<RecordingSettings>("get_recording_settings"),
      ]);
      // Don't overwrite a session that a start/stop produced while the
      // bootstrap call was in-flight.
      if (perm.session && captureSessionGeneration === gen) {
        setSession(perm.session);
      }
      recordingSettings = settings;
      captureError = null;
    } catch (err) {
      captureError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      captureLoadingSettings = false;
      captureBootstrapped = true;
    }
  }

  async function startCapture(): Promise<void> {
    if (captureLoadingStart || isCapturing) return;
    captureLoadingStart = true;
    captureError = null;
    try {
      const result = await invoke<{ session: CaptureSession }>(
        "start_native_capture",
        {
          request: {
            captureScreen: recordingSettings?.captureScreen ?? true,
            captureMicrophone: recordingSettings?.captureMicrophone ?? false,
            captureSystemAudio:
              recordingSettings?.captureSystemAudio ?? false,
          },
        },
      );
      captureSessionGeneration += 1;
      setSession(result.session);
    } catch (err) {
      captureError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      captureLoadingStart = false;
    }
  }

  async function stopCapture(): Promise<void> {
    if (captureLoadingStop || !isCapturing) return;
    captureLoadingStop = true;
    captureError = null;
    try {
      const result = await invoke<{ session: CaptureSession }>(
        "stop_native_capture",
      );
      captureSessionGeneration += 1;
      setSession(result.session);
    } catch (err) {
      captureError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      captureLoadingStop = false;
    }
  }

  // Fire-and-forget bootstrap so the control reflects an already-running
  // recording started from the debug page or a prior session restored by
  // the backend.
  $effect(() => {
    if (captureBootstrapped) return;
    void bootstrapCaptureControls();
  });

  // ─── Wake/visibility resync ──────────────────────────────────────────────
  // After macOS sleep/wake (or any prolonged background interval) the native
  // capture pipeline may have been torn down and restarted by the backend
  // while the webview slept. The shared `captureSession` store would then
  // reflect a stale "running" state. The backend-emitted `system_did_wake`
  // event is the primary reliable trigger; foreground/drift heuristics remain
  // as backstops. Every resync snapshots the generation so a wake-triggered
  // refresh can never overwrite a newer authoritative start/stop write.
  //
  // Tauri/macOS does not reliably flip `document.visibilityState` on every
  // wake (the webview can stay "visible" while the system slept), so we
  // listen to a small union of triggers in addition to `visibilitychange`:
  //   - window `focus` and `pageshow` — fire when the webview/window becomes
  //     active again, even if visibility never changed.
  //   - `online` — heuristic for resumed activity after a network stall.
  //   - a 1Hz wall-clock drift watchdog — if `setInterval`'s tick lands far
  //     later than expected, the process was suspended (sleep/throttle) and
  //     we must re-fetch even though no DOM event fired. This is the
  //     backstop for wakes that produce no other signal.
  //
  // The drift threshold is intentionally generous (5s) so normal jank or GC
  // pauses don't trigger a resync; a real sleep is typically tens of seconds
  // or more.
  const WAKE_DRIFT_THRESHOLD_MS = 5_000;
  const WAKE_DRIFT_TICK_MS = 1_000;
  $effect(() => {
    if (typeof document === "undefined") return;
    async function resyncCaptureSession() {
      const gen = captureSessionGeneration;
      try {
        const r = await invoke<GetPermissionsResponse>("get_capture_permissions");
        if (captureSessionGeneration !== gen) return; // superseded by start/stop
        if (r.session) setSession(r.session);
      } catch {
        // Best-effort: a transient IPC error here shouldn't surface; the
        // existing reconcile/bootstrap paths still cover steady-state drift.
      }
    }
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      void resyncCaptureSession();
    };
    const onFocus = () => { void resyncCaptureSession(); };
    let unlistenSystemDidWake: (() => void) | undefined;
    let destroyed = false;

    listen("system_did_wake", () => {
      void resyncCaptureSession();
      void pollTimelineHead();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenSystemDidWake = fn;
    });

    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", onFocus);
    window.addEventListener("pageshow", onFocus);
    window.addEventListener("online", onFocus);

    let lastTick = Date.now();
    const driftTimer = setInterval(() => {
      const now = Date.now();
      const drift = now - lastTick - WAKE_DRIFT_TICK_MS;
      lastTick = now;
      if (drift >= WAKE_DRIFT_THRESHOLD_MS) {
        // Wall-clock jumped — process was suspended. Treat as a wake.
        void resyncCaptureSession();
        // Also nudge the timeline back into sync; visibility may not change.
        void pollTimelineHead();
      }
    }, WAKE_DRIFT_TICK_MS);

    return () => {
      destroyed = true;
      unlistenSystemDidWake?.();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("pageshow", onFocus);
      window.removeEventListener("online", onFocus);
      clearInterval(driftTimer);
    };
  });
</script>

<!-- ── Timeline browser ──────────────────────────────────────────────────── -->
<svelte:window onpointerdown={onPickerPointerDownOutside} />
<section class="timeline" onwheel={onTimelineWheel}>
  <header class="timeline__bar">
    <div class="timeline__bar-group timeline__bar-group--primary">
      <div
        class="timeline__capture"
        role="group"
        aria-label="Recording controls"
      >
        <span
          class="timeline__capture-status timeline__capture-status--{captureStatusModifier}"
          aria-live="polite"
        >
          <span class="timeline__capture-dot" aria-hidden="true"></span>
          <span class="timeline__capture-status-label">{captureStatusLabel}</span>
        </span>
        {#if isCapturing}
          <button
            type="button"
            class="btn btn--sm timeline__capture-btn timeline__capture-btn--stop"
            onclick={stopCapture}
            disabled={captureLoadingStop}
            title="Stop recording"
            aria-label="Stop recording"
          >
            <span
              class="timeline__capture-glyph timeline__capture-glyph--square"
              aria-hidden="true"
            ></span>
            <span>{captureLoadingStop ? "Stopping…" : "Stop"}</span>
          </button>
        {:else}
          <button
            type="button"
            class="btn btn--sm timeline__capture-btn timeline__capture-btn--start"
            onclick={startCapture}
            disabled={captureLoadingStart || captureLoadingSettings}
            title="Start recording"
            aria-label="Start recording"
          >
            <span class="timeline__capture-glyph" aria-hidden="true">●</span>
            <span>{captureLoadingStart ? "Starting…" : "Record"}</span>
          </button>
        {/if}
      </div>
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
            style={pickerStyle}
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
    </div>

    <div class="timeline__bar-group timeline__bar-group--secondary">
      <button
        class="btn btn--ghost btn--sm timeline__ocr-btn"
        class:timeline__ocr-btn--running={ocrStatus === "running"}
        class:timeline__ocr-btn--error={ocrStatus === "error"}
        class:timeline__ocr-btn--success={ocrStatus === "success"}
        onclick={toggleOcrForActiveFrame}
        disabled={!timelineActive}
        title={ocrError ??
          (ocrVisible
            ? "Hide OCR data for the active frame"
            : ocrStatus === "success"
              ? `${ocrObservations.length} text region${ocrObservations.length === 1 ? "" : "s"} detected${ocrUsingEarlierFrame ? ` (reused from frame ${ocrSourceFrame?.id})` : ""}`
              : ocrStatus === "empty"
                ? ocrUsingEarlierFrame
                  ? `no text detected (reused from frame ${ocrSourceFrame?.id})`
                  : "no text detected"
                : ocrStatus === "missing"
                  ? "no OCR data for this frame"
                  : "Show OCR data for the active frame")}
        aria-label={ocrVisible
          ? "Hide OCR data for active frame"
          : "Show OCR data for active frame"}
        aria-pressed={ocrVisible}
      >
        <span class="timeline__ocr-glyph" aria-hidden="true">⌖</span>
        <span>{ocrButtonLabel}</span>
        {#if ocrStatus === "success" && ocrObservations.length > 0}
          <span class="timeline__ocr-count">{ocrObservations.length}</span>
        {/if}
      </button>
      <button
        class="btn btn--ghost btn--sm"
        onclick={refreshTimelineAndDashboard}
        disabled={timelineLoading || timelineLoadingMore || audioSegmentsLoading}
      >refresh</button>
      <a
        class="btn btn--ghost btn--sm timeline__menu-link"
        href="/menu"
        aria-label="Open menu"
        title="Menu"
      ><span class="timeline__menu-icon" aria-hidden="true">⚙</span></a>
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

  <!-- Audio segment player drawer. Rendered as a non-modal bottom sheet
       that slides in only when an audio segment is selected. The timeline
       rail stays interactive while the drawer is open so the user can pick
       a different segment without dismissing first; selecting null (or
       pressing Escape / clicking close) hides the drawer entirely. The
       audio lane bars themselves remain visible above the rail so audio
       presence/discovery is unaffected. -->

  <div class="timeline__stage" bind:this={stageEl}>
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
        <div
          class="timeline__preview"
          role="img"
          aria-label={`frame ${timelineActive.id}`}
          style={`background-image: url("${previewUrl}");`}
        ></div>
        <!-- OCR overlay: anchored to the painted background-image rect
             (background-size: contain, centered) inside the stage. The
             rect is derived from stage size + the active frame's intrinsic
             width/height since there's no <img> element to measure.
             Pointer-events stay disabled so the overlay never blocks
             scrub/click on the stage. Boxes and labels only render once
             an OCR run has produced observations for the currently active
             frame. -->
        {#if ocrVisible && ocrStatus === "success" && ocrFrameId === timelineActive.id && ocrObservations.length > 0 && renderedImageRect.width > 0 && renderedImageRect.height > 0}
          <div
            class="timeline__ocr-overlay"
            aria-hidden="true"
            style={`left: ${renderedImageRect.left}px; top: ${renderedImageRect.top}px; width: ${renderedImageRect.width}px; height: ${renderedImageRect.height}px;`}
          >
            {#each ocrObservations as obs, i (i)}
              <div
                class="timeline__ocr-box"
                style={ocrBoxStyle(obs)}
                title={`${obs.text} · ${(obs.confidence * 100).toFixed(0)}%`}
              >
                <span class="timeline__ocr-label">{obs.text}</span>
              </div>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="timeline__preview-pending">decoding preview…</div>
      {/if}
    {/if}

    {#if ocrVisible && timelineActive && ocrFrameId === timelineActive.id && ocrStatus !== "idle" && ocrStatus !== "success"}
      <div
        class="timeline__ocr-status timeline__ocr-status--{ocrStatus}"
        role="status"
        aria-live="polite"
      >
        {#if ocrStatus === "running"}
          <span class="timeline__ocr-spinner" aria-hidden="true"></span>
          <span>loading OCR data…</span>
        {:else if ocrStatus === "empty"}
          <span class="timeline__ocr-status-glyph" aria-hidden="true">∅</span>
          <span>
            {ocrUsingEarlierFrame
              ? `no text detected (reused from frame ${ocrSourceFrame?.id})`
              : "no text detected on this frame"}
          </span>
        {:else if ocrStatus === "missing"}
          <span class="timeline__ocr-status-glyph" aria-hidden="true">∅</span>
          <span>no OCR data for this frame</span>
        {:else if ocrStatus === "error"}
          <span class="timeline__ocr-status-glyph" aria-hidden="true">!</span>
          <span class="timeline__ocr-status-msg">{ocrError ?? "OCR failed"}</span>
        {/if}
      </div>
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
          ? `Frame ${timelineActiveIndex + 1} of ${timelineFrames.length}${timelineHasMore ? "+" : ""} — captured ${formatCapturedAt(timelineActive.capturedAt)}`
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
           the rail does not resize the stage. The audio lane shell stays
           visible so users always have a clear audio surface — it just
           shows an empty/instructional state until segments arrive. -->
      <div class="timeline-rail timeline-rail--placeholder" aria-hidden="true"></div>
      <div
        class="timeline-rail__audio-lane-wrap"
        aria-label="Audio segments"
      >
        <div class="timeline-rail__audio-lane-labels" aria-hidden="true">
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--microphone">mic</span>
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--systemAudio">sys</span>
        </div>
        <div class="timeline-rail__audio-lane-viewport">
          <span class="timeline-rail__audio-lane-empty">
            {#if audioSegmentsLoading}
              loading audio…
            {:else if audioSegmentsError}
              audio unavailable
            {:else if timelineLoading}
              waiting for frames…
            {:else}
              no frames loaded
            {/if}
          </span>
        </div>
      </div>
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

<!-- Audio player drawer. Lives outside the timeline section so its fixed
     positioning is not affected by the section's `overflow: hidden`. The
     drawer is non-modal: the timeline rail/lane stay interactive, so the
     user can swap segments by clicking another bar without dismissing first.
     `selectedAudioSegmentId` is the open/closed signal — clearing it (close
     button, Escape, or `audioSegments` losing the row) collapses the drawer
     and the surrounding effects clear/refresh media bytes accordingly. -->
{#if selectedAudioSegment}
  <div
    class="audio-drawer"
    role="dialog"
    aria-modal="false"
    aria-label={`Audio segment player — ${audioSourceLabel(selectedAudioSegment.source)} #${selectedAudioSegment.segmentIndex}`}
    tabindex="-1"
    bind:this={audioDrawerEl}
    onkeydown={onAudioDrawerKeydown}
  >
    <div class="audio-drawer__handle" aria-hidden="true"></div>
    <div class="audio-drawer__meta">
      <span
        class="audio-drawer__source audio-drawer__source--{selectedAudioSegment.source}"
        aria-label={`Source: ${audioSourceLabel(selectedAudioSegment.source)}`}
      >
        <span class="audio-drawer__swatch" aria-hidden="true"></span>
        {audioSourceLabel(selectedAudioSegment.source)}
      </span>
      <span class="audio-drawer__index" aria-label="Segment index">
        #{selectedAudioSegment.segmentIndex}
      </span>
      <span
        class="audio-drawer__time"
        title={`${formatUnixMs(selectedAudioSegment.startUnixMs)} – ${formatUnixMs(selectedAudioSegment.endUnixMs)}`}
      >
        {formatTimeOfDay(selectedAudioSegment.startUnixMs)}
        <span class="audio-drawer__time-sep" aria-hidden="true">→</span>
        {formatTimeOfDay(selectedAudioSegment.endUnixMs)}
        <span class="audio-drawer__duration"
          >· {formatDurationSeconds(selectedAudioSegment.durationSeconds)}</span
        >
      </span>
      <span class="audio-drawer__file" title={selectedAudioSegment.filePath}
        >{selectedAudioSegment.fileName}</span
      >
      <button
        type="button"
        class="audio-drawer__close"
        onclick={closeAudioDrawer}
        bind:this={audioDrawerCloseEl}
        aria-label="Close audio player"
      >×</button>
    </div>
    {#if selectedAudioMediaLoading}
      <div class="audio-drawer__status">
        <span class="audio-drawer__status-glyph" aria-hidden="true">…</span>
        <span>loading audio segment…</span>
      </div>
    {:else if selectedAudioMediaError}
      <div class="audio-drawer__error" role="alert">
        <span class="audio-drawer__error-label">playback unavailable</span>
        <span class="audio-drawer__error-msg">{selectedAudioMediaError}</span>
      </div>
    {:else if selectedAudioSrc}
      <!-- `src` is reactive: switching segments swaps the audio element's
           source via Svelte's binding. Using a keyed block forces a fresh
           <audio> element per segment so the browser doesn't keep playing
           the previous file while the new metadata loads. The native
           `<audio>` element below stays hidden — it owns decoding/playback
           — and the visible UI is a bespoke transport (play/pause, scrub,
           current/duration) so the player matches the surrounding deck. -->
      {#key selectedAudioSegment.id}
        <audio
          class="audio-drawer__audio-native"
          preload="metadata"
          src={selectedAudioSrc}
          bind:this={audioEl}
          onerror={onSelectedAudioError}
          ontimeupdate={onAudioTimeUpdate}
          onloadedmetadata={onAudioLoadedMetadata}
          ondurationchange={onAudioLoadedMetadata}
          onplay={onAudioPlay}
          onpause={onAudioPause}
          onended={onAudioEnded}
          aria-hidden="true"
        ></audio>
      {/key}
      <div class="audio-drawer__player" role="group" aria-label="Audio playback controls">
        <button
          type="button"
          class="audio-drawer__play"
          onclick={togglePlayPause}
          aria-label={audioIsPlaying ? "Pause" : "Play"}
          aria-pressed={audioIsPlaying}
        >
          {#if audioIsPlaying}
            <svg
              viewBox="0 0 16 16"
              width="14"
              height="14"
              aria-hidden="true"
              focusable="false"
            >
              <rect x="3.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
              <rect x="9.5" y="2.5" width="3" height="11" rx="0.5" fill="currentColor" />
            </svg>
          {:else}
            <svg
              viewBox="0 0 16 16"
              width="14"
              height="14"
              aria-hidden="true"
              focusable="false"
            >
              <path d="M4.5 2.5 L13 8 L4.5 13.5 Z" fill="currentColor" />
            </svg>
          {/if}
        </button>
        <span class="audio-drawer__time-readout audio-drawer__time-readout--current"
          >{formatPlayerTime(audioCurrentTime)}</span
        >
        <input
          type="range"
          class="audio-drawer__scrub"
          min="0"
          max={audioDuration > 0 ? audioDuration : 0}
          step="0.05"
          value={audioCurrentTime}
          disabled={!(audioDuration > 0)}
          oninput={onScrubInput}
          onchange={onScrubChange}
          aria-label="Seek"
          aria-valuemin={0}
          aria-valuemax={audioDuration > 0 ? audioDuration : 0}
          aria-valuenow={audioCurrentTime}
          aria-valuetext={`${formatPlayerTime(audioCurrentTime)} of ${formatPlayerTime(audioDuration)}`}
          style:--audio-progress={audioDuration > 0
            ? `${Math.min(100, (audioCurrentTime / audioDuration) * 100)}%`
            : "0%"}
        />
        <span class="audio-drawer__time-readout audio-drawer__time-readout--duration"
          >{formatPlayerTime(audioDuration)}</span
        >
      </div>
    {/if}
    {#if selectedAudioLoadError}
      <div class="audio-drawer__error" role="alert">
        <span class="audio-drawer__error-label">playback error</span>
        <span class="audio-drawer__error-msg">{selectedAudioLoadError}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* ── Page layout ──────────────────────────────────────────── */
  .timeline {
    /* Fill the route shell instead of guessing with a viewport subtraction.
       This keeps the rail flush to the bottom even when the app shell height
       differs slightly from the old hard-coded 44px assumption. */
    flex: 1 1 auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 8px 6px;
    background: #0c0c0e;
    /* Allow the stage child (flex: 1, min-height: 0) to actually shrink so
       the bottom rail stays in view regardless of preview intrinsic size. */
    min-height: 0;
    overflow: hidden;
  }

  .timeline__bar,
  .timeline__audio,
  .timeline__error,
  .timeline__rail-wrap {
    flex: 0 0 auto;
  }

  /* Header bar: two clearly-separated control groups (recording + jump on
     the left, frame actions + menu on the right) that wrap onto a second
     row on narrow viewports instead of cramming together. */
  .timeline__bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px 16px;
  }

  .timeline__bar-group {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    min-width: 0;
  }

  .timeline__bar-group--secondary {
    margin-left: auto;
  }

  /* ── Recording control cluster ─────────────────────────────── */
  /* Reads the shared capture session and drives start/stop via the same
     Tauri commands as the debug page. The status pill on the left of the
     cluster mirrors `captureSession.value`; the button on the right toggles
     between record (idle) and stop (running) and reflects loading states. */
  .timeline__capture {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px 4px 8px;
    background: #0a0a10;
    border: 1px solid #161624;
    border-radius: 6px;
  }

  .timeline__capture-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #555574;
    font-variant-numeric: tabular-nums;
  }

  .timeline__capture-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #2a2a3a;
    box-shadow: 0 0 0 0 rgba(0, 0, 0, 0);
    flex: 0 0 auto;
  }

  .timeline__capture-status--running {
    color: #ff5d6c;
  }
  .timeline__capture-status--running :global(.timeline__capture-dot) {
    background: #ff3148;
    box-shadow: 0 0 0 3px rgba(255, 49, 72, 0.18);
    animation: timeline-capture-pulse 1.4s ease-in-out infinite;
  }

  .timeline__capture-status--paused {
    color: #d6a14a;
  }
  .timeline__capture-status--paused :global(.timeline__capture-dot) {
    background: #d6a14a;
    box-shadow: 0 0 0 3px rgba(214, 161, 74, 0.16);
  }

  @keyframes timeline-capture-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
  }

  .timeline__capture-status-label {
    line-height: 1;
  }

  .timeline__capture-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    font-size: 9px;
    border-radius: 4px;
  }

  .timeline__capture-btn--start {
    background: #1a0f12;
    color: #ff8a96;
    border-color: #3a1820;
  }
  .timeline__capture-btn--start:not(:disabled):hover {
    background: #2a1218;
    color: #ffb0b9;
    border-color: #5a2030;
  }

  .timeline__capture-btn--stop {
    background: #170d0f;
    color: #f0f0f5;
    border-color: #4a1c26;
  }
  .timeline__capture-btn--stop:not(:disabled):hover {
    background: #2a1218;
    border-color: #6a2434;
  }

  .timeline__capture-glyph {
    display: inline-block;
    width: 8px;
    height: 8px;
    line-height: 1;
    text-align: center;
    color: #ff3148;
    font-size: 12px;
  }

  .timeline__capture-btn--stop :global(.timeline__capture-glyph) {
    color: #ff8a96;
  }

  .timeline__capture-glyph--square {
    background: currentColor;
    border-radius: 1px;
    width: 7px;
    height: 7px;
  }

  .timeline__audio {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 3px 8px;
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

  /* ── Audio segment player drawer ──────────────────────────────
     Bottom-anchored sheet that slides in only when an audio segment is
     selected. Non-modal: timeline rail and audio lane bars stay
     interactive so users can swap segments without dismissing. The
     drawer's industrial vibe matches the rail (matte black surface,
     hairline border, red accent on the active segment) and lifts off
     the page with a soft shadow + blurred top edge so it's clearly
     a transient overlay rather than part of the timeline column. */
  .audio-drawer {
    position: fixed;
    left: 12px;
    right: 12px;
    bottom: 12px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px 10px;
    background: linear-gradient(180deg, #14141d 0%, #0c0c12 100%);
    border: 1px solid #25253a;
    border-radius: 8px;
    box-shadow:
      0 18px 40px rgba(0, 0, 0, 0.55),
      0 2px 0 rgba(255, 255, 255, 0.02) inset;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    /* Slide-in motion: the drawer transforms from below and fades in.
       Keeps the entrance grounded to the bottom edge so it reads as a
       rising sheet rather than a pop-up. */
    animation: audio-drawer-rise 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
    outline: none;
  }

  .audio-drawer:focus-visible {
    border-color: rgba(255, 68, 85, 0.5);
    box-shadow:
      0 18px 40px rgba(0, 0, 0, 0.55),
      0 0 0 2px rgba(255, 68, 85, 0.35);
  }

  @keyframes audio-drawer-rise {
    from {
      transform: translateY(12px);
      opacity: 0;
    }
    to {
      transform: translateY(0);
      opacity: 1;
    }
  }

  /* Centered grab-handle pill. Purely decorative — signals the drawer
     nature of the surface without claiming it's draggable. */
  .audio-drawer__handle {
    align-self: center;
    width: 36px;
    height: 3px;
    border-radius: 2px;
    background: #2a2a3a;
    margin-bottom: 2px;
  }

  .audio-drawer__meta {
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

  .audio-drawer__source {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid #242438;
    border-radius: 999px;
    color: #d6d6ea;
    font-weight: 700;
  }

  .audio-drawer__swatch {
    width: 10px;
    height: 3px;
    border-radius: 1.5px;
  }

  .audio-drawer__source--microphone .audio-drawer__swatch {
    background: linear-gradient(
      90deg,
      rgba(120, 200, 255, 0.95),
      rgba(80, 160, 230, 0.95)
    );
  }

  .audio-drawer__source--systemAudio .audio-drawer__swatch {
    background: linear-gradient(
      90deg,
      rgba(255, 180, 100, 0.95),
      rgba(220, 130, 60, 0.95)
    );
  }

  .audio-drawer__index {
    color: #ff5566;
    font-weight: 700;
  }

  .audio-drawer__time {
    color: #8e8eb0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .audio-drawer__time-sep {
    color: #444462;
  }

  .audio-drawer__duration {
    color: #5e5e80;
  }

  .audio-drawer__file {
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

  .audio-drawer__close {
    appearance: none;
    background: transparent;
    border: 1px solid #242438;
    border-radius: 4px;
    width: 24px;
    height: 24px;
    color: #8a8aae;
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    transition:
      color 0.12s,
      border-color 0.12s,
      background 0.12s;
  }

  .audio-drawer__close:hover,
  .audio-drawer__close:focus-visible {
    color: #ff5566;
    border-color: #ff4455;
    background: rgba(255, 68, 85, 0.08);
    outline: none;
  }

  .audio-drawer__audio-native {
    display: none;
  }

  /* Custom transport: a play/pause button, a thin scrub bar that fills
     to a brand-red as it advances, and tabular-numeric time readouts on
     either side. Built to read as part of the deck — no native chrome. */
  .audio-drawer__player {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 2px 2px;
  }

  .audio-drawer__play {
    appearance: none;
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: rgba(255, 68, 85, 0.1);
    border: 1px solid #3a1a26;
    border-radius: 50%;
    color: #ff5566;
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s,
      transform 0.08s;
  }

  .audio-drawer__play:hover {
    background: rgba(255, 68, 85, 0.18);
    border-color: #ff4455;
  }

  .audio-drawer__play:focus-visible {
    outline: none;
    border-color: #ff4455;
    box-shadow: 0 0 0 2px rgba(255, 68, 85, 0.35);
  }

  .audio-drawer__play:active {
    transform: scale(0.96);
  }

  .audio-drawer__time-readout {
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    font-variant-numeric: tabular-nums;
    color: #8a8aae;
    min-width: 36px;
  }

  .audio-drawer__time-readout--current {
    color: #d6d6ea;
    text-align: right;
  }

  /* Scrub: a thin track that fills to brand-red up to the current time,
     with a small thumb that grows on hover/focus. Track and thumb are
     restyled across WebKit/Firefox so the bar reads identically with no
     native chrome. The fill ratio comes from `--audio-progress` set
     inline so the change is purely declarative. */
  .audio-drawer__scrub {
    flex: 1 1 auto;
    appearance: none;
    -webkit-appearance: none;
    height: 18px;
    margin: 0;
    background: transparent;
    cursor: pointer;
    color: #ff5566;
  }

  .audio-drawer__scrub:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .audio-drawer__scrub::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      #ff4455 0%,
      #ff4455 var(--audio-progress, 0%),
      #1f1f2e var(--audio-progress, 0%),
      #1f1f2e 100%
    );
  }

  .audio-drawer__scrub::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: #1f1f2e;
  }

  .audio-drawer__scrub::-moz-range-progress {
    height: 4px;
    border-radius: 2px;
    background: #ff4455;
  }

  .audio-drawer__scrub::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #ff5566;
    border: 2px solid #14141d;
    margin-top: -3px;
    box-shadow: 0 0 0 0 rgba(255, 68, 85, 0);
    transition:
      transform 0.12s,
      box-shadow 0.12s;
  }

  .audio-drawer__scrub::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #ff5566;
    border: 2px solid #14141d;
    box-shadow: 0 0 0 0 rgba(255, 68, 85, 0);
    transition:
      transform 0.12s,
      box-shadow 0.12s;
  }

  .audio-drawer__scrub:hover::-webkit-slider-thumb,
  .audio-drawer__scrub:focus-visible::-webkit-slider-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px rgba(255, 68, 85, 0.18);
  }

  .audio-drawer__scrub:hover::-moz-range-thumb,
  .audio-drawer__scrub:focus-visible::-moz-range-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px rgba(255, 68, 85, 0.18);
  }

  .audio-drawer__scrub:focus-visible {
    outline: none;
  }

  .audio-drawer__status {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 2px;
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: #6a6a88;
  }

  .audio-drawer__status-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid #242438;
    border-radius: 50%;
    color: #ff5566;
    font-size: 9px;
  }

  .audio-drawer__error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 10px;
    background: #1a0e10;
    border: 1px solid #3a1a20;
    border-radius: 4px;
    font-size: 11px;
    color: #c08080;
  }

  .audio-drawer__error-label {
    flex: 0 0 auto;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: #80505a;
    padding-top: 1px;
  }

  .audio-drawer__error-msg {
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

  /* Anchors styled as buttons (e.g. the timeline → menu link) need the
     same ghost-button colour reset; without this the global `a` colour
     would override `.btn--ghost`. */
  a.btn--ghost {
    color: #7a7a9a;
  }
  a.btn--ghost:hover {
    color: #a0a0c0;
  }
  .timeline__menu-link {
    text-decoration: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    /* Keep the icon button the same height as adjacent ghost buttons (refresh)
       while shrinking horizontal padding to read as a square icon control. */
    padding: 3px 6px;
    line-height: 1;
  }
  .timeline__menu-icon {
    display: inline-block;
    font-size: 12px;
    line-height: 1;
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
    position: fixed;
    z-index: 20;
    display: grid;
    grid-template-columns: auto 200px;
    width: min(520px, calc(100vw - 24px));
    gap: 12px;
    padding: 12px;
    box-sizing: border-box;
    overflow: auto;
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

  @media (max-width: 640px) {
    .timeline__picker {
      grid-template-columns: minmax(0, 1fr);
      width: min(320px, calc(100vw - 24px));
    }
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
    position: absolute;
    inset: 0;
    background-repeat: no-repeat;
    background-position: center center;
    background-size: contain;
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

  /* ── OCR header button + overlay ───────────────────────────── */
  /* The button sits in the right-side cluster next to refresh. Its colour
     mirrors the OCR run state: muted when idle, amber while running, green
     on success, red on error — so the user can read OCR availability at a
     glance without opening the tooltip. */
  .timeline__ocr-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-variant-numeric: tabular-nums;
  }

  .timeline__ocr-glyph {
    display: inline-block;
    font-size: 11px;
    line-height: 1;
    color: #7a7a9a;
    transform: translateY(-1px);
  }

  .timeline__ocr-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 16px;
    padding: 0 4px;
    height: 14px;
    border-radius: 7px;
    background: rgba(120, 220, 160, 0.12);
    color: #7adfa0;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }

  .timeline__ocr-btn--running {
    color: #d6a14a;
    border-color: #3a2a18;
    background: rgba(214, 161, 74, 0.06);
  }
  .timeline__ocr-btn--running .timeline__ocr-glyph {
    color: #d6a14a;
    animation: timeline-ocr-pulse 1.2s ease-in-out infinite;
  }

  .timeline__ocr-btn--success {
    color: #7adfa0;
    border-color: #1f3a28;
  }
  .timeline__ocr-btn--success .timeline__ocr-glyph {
    color: #7adfa0;
  }

  .timeline__ocr-btn--error {
    color: #ff8a96;
    border-color: #3a1820;
  }
  .timeline__ocr-btn--error .timeline__ocr-glyph {
    color: #ff8a96;
  }

  @keyframes timeline-ocr-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  /* Overlay wrapper sized & positioned to match the actual rendered image
     rect (measured from the DOM each layout). `overflow: hidden` clips any
     OCR box whose normalized bounds slightly extend past the image edges
     so visuals never spill into the surrounding stage letterbox area.
     pointer-events stays off so the stage continues to receive
     clicks/drags. */
  .timeline__ocr-overlay {
    position: absolute;
    overflow: hidden;
    pointer-events: none;
  }

  .timeline__ocr-box {
    position: absolute;
    border: 1px solid rgba(120, 220, 160, 0.85);
    background: rgba(120, 220, 160, 0.08);
    border-radius: 2px;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.45),
      inset 0 0 0 1px rgba(255, 255, 255, 0.04);
    /* Allow zero-width/height edge cases to remain visible as a hairline. */
    min-width: 1px;
    min-height: 1px;
  }

  .timeline__ocr-label {
    position: absolute;
    left: 0;
    bottom: 100%;
    margin-bottom: 2px;
    max-width: max(160px, 100%);
    padding: 1px 5px;
    background: rgba(8, 14, 10, 0.92);
    border: 1px solid rgba(120, 220, 160, 0.6);
    border-radius: 2px;
    color: #d8f5e2;
    font-family:
      ui-monospace,
      SFMono-Regular,
      Menlo,
      monospace;
    font-size: 9px;
    line-height: 1.3;
    letter-spacing: 0.02em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }

  /* Compact inline status pill for non-success OCR states (running / empty /
     error). Pinned to the bottom-left of the stage so it never competes with
     the metadata overlay in the top-left corner. */
  .timeline__ocr-status {
    position: absolute;
    left: 10px;
    bottom: 10px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    background: rgba(10, 10, 16, 0.78);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 4px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #b8b8d0;
    max-width: calc(100% - 20px);
  }

  .timeline__ocr-status--running {
    color: #e8c98a;
    border-color: rgba(214, 161, 74, 0.35);
  }

  .timeline__ocr-status--empty {
    color: #7a7a9a;
  }

  .timeline__ocr-status--missing {
    color: #7a7a9a;
  }

  .timeline__ocr-status--error {
    color: #ff8a96;
    border-color: rgba(255, 90, 110, 0.4);
  }

  .timeline__ocr-status-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid currentColor;
    font-size: 9px;
    font-weight: 700;
  }

  .timeline__ocr-status-msg {
    text-transform: none;
    letter-spacing: 0;
    font-family:
      ui-monospace,
      SFMono-Regular,
      Menlo,
      monospace;
    word-break: break-word;
    max-width: 360px;
  }

  .timeline__ocr-spinner {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid rgba(214, 161, 74, 0.3);
    border-top-color: #d6a14a;
    animation: timeline-ocr-spin 0.9s linear infinite;
  }

  @keyframes timeline-ocr-spin {
    to { transform: rotate(360deg); }
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
    gap: 4px;
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
