<script lang="ts">
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { Image } from "@tauri-apps/api/image";
  import { writeImage } from "@tauri-apps/plugin-clipboard-manager";
  import { BaseDirectory, writeFile } from "@tauri-apps/plugin-fs";
  import { Calendar } from "bits-ui";
  import {
    CalendarDate,
    type DateValue,
  } from "@internationalized/date";
  import {
    bootstrapCaptureControls,
    captureControls,
    resyncCaptureSession,
  } from "$lib/capture-controls.svelte";
  import { developerOptions } from "$lib/developer-options.svelte";
  import { framePreviewAssetUrl, readFramePreviewBytes } from "$lib/frame-preview";
  import type {
    AudioSegmentDto,
    AudioSegmentMediaDto,
    AudioSegmentTranscriptionReprocessingResultDto,
    CapturedFrameReprocessingResultDto,
    FrameDto,
    FramePreviewDto,
    FrameRangeRequest,
    FrameSummaryDto,
    FocusedFrameWindowDto,
    GetEarliestEarlierEquivalentFrameRequest,
    GetProcessingJobRequest,
    GetProcessingResultRequest,
    GetTimelineWindowAroundFrameRequest,
    ListAudioSegmentsRequest,
    ListFramesRequest,
    OcrObservation,
    OcrStructuredPayload,
    ProcessingJobDto,
    ProcessingResultDto,
    ReprocessAudioSegmentTranscriptionRequest,
    ReprocessCapturedFrameOcrRequest,
    TranscriptionSegment,
    TranscriptionStructuredPayload,
    TranscriptionWord,
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
  const AUDIO_SEGMENT_REFRESH_DEBOUNCE_MS = 100;
  const AUDIO_TRANSCRIPT_POLL_INTERVAL_MS = 1000;
  const OCR_POLL_INTERVAL_MS = 1000;
  const ACTIVE_PREVIEW_FETCH_FAST_SCRUB_DEBOUNCE_MS = 40;
  const ACTIVE_PREVIEW_PREFETCH_RADIUS = 8;
  const ACTIVE_PREVIEW_FAST_SCRUB_RADIUS = 2;
  const ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS = 2;
  const PREVIEW_CACHE_MAX_ENTRIES = 192;
  const PREVIEW_FAILURE_CACHE_TTL_MS = 5_000;
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
  type AudioTranscriptStatus =
    | "idle"
    | "loading"
    | "success"
    | "empty"
    | "missing"
    | "running"
    | "error";

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
  // Last `clientWidth` of the rail acknowledged by the `ResizeObserver`. Used
  // by `onTimelineScroll` to discriminate user-driven scroll events from
  // resize-induced ones: when the window grows, the rail's `cqi`-based track
  // margins recompute non-atomically with `clientWidth`, momentarily shrinking
  // `maxScroll = scrollWidth - clientWidth`. The browser then clamps
  // `scrollLeft` and fires `scroll` against that transient state. Without
  // this guard, `onTimelineScroll` would happily commit a newer
  // `timelineActiveIndex` derived from the clamped scroll position, jumping
  // the active frame on every window resize. We treat `timelineActiveIndex`
  // as the source of truth across resizes and restore `scrollLeft` from it
  // in the resize observer below.
  let lastTimelineRailClientWidth = 0;
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

  // Preview file paths keyed by frame id. Reactive so the rail re-renders as
  // previews stream in without any extra plumbing.
  let previewCache = $state<Map<number, string>>(new Map());
  let previewMimeTypeCache = $state<Map<number, string>>(new Map());
  let previewFailedAt = $state<Map<number, number>>(new Map());
  // Tracks the in-flight requests so concurrent scrolls don't fan out a
  // request per slot per scroll tick for the same id.
  const previewInFlight = new Set<number>();
  type FrameActionStatus = {
    message: string;
    detail: string | null;
    tone: "neutral" | "error";
  };

  let frameActionStatus = $state<FrameActionStatus | null>(null);
  let frameActionStatusTimer: ReturnType<typeof setTimeout> | null = null;
  let frameActionStatusHovered = $state(false);
  let stageActionsMenuOpen = $state(false);
  let activePreviewFetchGeneration = 0;
  let activePreviewFetchTimer: ReturnType<typeof setTimeout> | null = null;
  let lastTimelineScrollSample = { left: 0, at: 0 };
  let previewScrubVelocityPxPerMs = $state(0);
  let previewCacheReuseCount = $state(0);
  let previewCacheHitCount = $state(0);
  let previewCacheMissCount = $state(0);
  let previewFailureCacheHitCount = $state(0);
  let previewInFlightJoinCount = $state(0);
  let previewDirectPathCount = $state(0);
  let previewGeneratedPathCount = $state(0);
  let previewStaleRetryCount = $state(0);
  let activePreviewLoadErrorFrameId = $state<number | null>(null);

  function handleActivePreviewLoadError(frameId: number): void {
    if (activePreviewLoadErrorFrameId === frameId) return;
    activePreviewLoadErrorFrameId = frameId;
    previewStaleRetryCount += 1;
    dropPreviewCacheEntry(frameId);
    clearPreviewFailure(frameId);
    void ensurePreview(frameId);
  }

  const timelineActive = $derived(timelineFrames[timelineActiveIndex] ?? null);
  const timelineHasMore = $derived(timelineHasNewer || !timelineExhausted);
  let lastPreviewReuseFrameId = $state<number | null>(null);
  let timelineActiveDuplicateOf = $state<FrameDto | null>(null);
  let timelineActiveDuplicateLookupGeneration = 0;

  // Preview load/error state belongs to the currently selected frame only.
  // Clear it on frame switches so a stale message from an older request does
  // not sit over the next frame while its own preview is still loading.
  $effect(() => {
    const activeId = timelineActive?.id ?? null;
    if (activeId == null) {
      setFrameActionStatus(null);
      return;
    }
    if (activePreviewLoadErrorFrameId !== activeId) {
      setFrameActionStatus(null);
    }
  });

  $effect(() => {
    const activeId = timelineActive?.id ?? null;
    if (activeId == null) {
      lastPreviewReuseFrameId = null;
      return;
    }
    if (lastPreviewReuseFrameId === activeId) return;
    lastPreviewReuseFrameId = activeId;
    if (previewCache.has(activeId)) {
      previewCacheReuseCount += 1;
    }
  });

  $effect(() => {
    const active = timelineActive;
    const shouldLookup = !!active && developerOptions.value;
    timelineActiveDuplicateLookupGeneration += 1;
    const gen = timelineActiveDuplicateLookupGeneration;
    timelineActiveDuplicateOf = null;

    if (!shouldLookup || !active) {
      return;
    }

    void (async () => {
      try {
        const duplicateOf = await invoke<FrameDto | null>(
          "get_earliest_earlier_equivalent_frame",
          {
            request: {
              frameId: active.id,
            } satisfies GetEarliestEarlierEquivalentFrameRequest,
          },
        );
        if (gen !== timelineActiveDuplicateLookupGeneration) return;
        timelineActiveDuplicateOf = duplicateOf;
      } catch {
        if (gen !== timelineActiveDuplicateLookupGeneration) return;
        timelineActiveDuplicateOf = null;
      }
    })();
  });

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
  let selectedAudioTranscriptStatus = $state<AudioTranscriptStatus>("idle");
  let selectedAudioTranscriptText = $state<string | null>(null);
  let selectedAudioTranscriptSegments = $state<TranscriptionSegment[]>([]);
  let selectedAudioTranscriptModelLabel = $state<string | null>(null);
  let selectedAudioTranscriptError = $state<string | null>(null);
  let selectedAudioTranscriptRerunLoading = $state(false);
  let selectedAudioTranscriptRerunError = $state<string | null>(null);
  let selectedAudioTranscriptGeneration = 0;
  let selectedAudioTranscriptPollTimer: ReturnType<typeof setTimeout> | null = null;
  let selectedAudioTranscriptPollJobId: number | null = null;
  // Latest webview-side load error from the <audio> element, if any. Lets us
  // show an inline error instead of a silent broken player when decoded bytes
  // were returned but the webview still couldn't load/play them.
  let selectedAudioLoadError = $state<string | null>(null);
  $effect(() => {
    // Clear the prior errors whenever the selected segment changes.
    void selectedAudioSegmentId;
    selectedAudioLoadError = null;
    selectedAudioTranscriptRerunError = null;
    selectedAudioTranscriptRerunLoading = false;
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

  $effect(() => {
    const id = selectedAudioSegmentId;
    selectedAudioTranscriptGeneration += 1;
    const gen = selectedAudioTranscriptGeneration;
    clearSelectedAudioTranscriptPoll();
    selectedAudioTranscriptText = null;
    selectedAudioTranscriptSegments = [];
    selectedAudioTranscriptModelLabel = null;
    selectedAudioTranscriptError = null;

    if (id == null) {
      selectedAudioTranscriptStatus = "idle";
      return () => clearSelectedAudioTranscriptPoll();
    }

    selectedAudioTranscriptStatus = "loading";
    void loadSelectedAudioSegmentTranscript(id, gen);

    return () => {
      if (gen === selectedAudioTranscriptGeneration) {
        clearSelectedAudioTranscriptPoll();
      }
    };
  });

  function selectedAudioTranscriptIsCurrent(id: number, gen: number): boolean {
    return gen === selectedAudioTranscriptGeneration && selectedAudioSegmentId === id;
  }

  function clearSelectedAudioTranscriptPoll(): void {
    if (selectedAudioTranscriptPollTimer) {
      clearTimeout(selectedAudioTranscriptPollTimer);
      selectedAudioTranscriptPollTimer = null;
    }
    selectedAudioTranscriptPollJobId = null;
  }

  function scheduleSelectedAudioTranscriptPoll(id: number, jobId: number, gen: number): void {
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
    if (selectedAudioTranscriptPollTimer && selectedAudioTranscriptPollJobId === jobId) return;
    clearSelectedAudioTranscriptPoll();
    selectedAudioTranscriptPollJobId = jobId;
    selectedAudioTranscriptPollTimer = setTimeout(() => {
      selectedAudioTranscriptPollTimer = null;
      selectedAudioTranscriptPollJobId = null;
      void pollSelectedAudioSegmentTranscriptJob(id, jobId, gen);
    }, AUDIO_TRANSCRIPT_POLL_INTERVAL_MS);
  }

  async function pollSelectedAudioSegmentTranscriptJob(
    id: number,
    jobId: number,
    gen: number,
  ): Promise<void> {
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
    try {
      const job = await invoke<ProcessingJobDto | null>("get_processing_job", {
        request: { jobId } satisfies GetProcessingJobRequest,
      });
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      if (!job) {
        selectedAudioTranscriptStatus = "error";
        selectedAudioTranscriptText = null;
        selectedAudioTranscriptSegments = [];
        selectedAudioTranscriptError = "Transcription job not found";
        return;
      }
      await applySelectedAudioTranscriptJob(id, gen, job);
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  type AudioTranscriptionJobPayloadShape = {
    provider?: string;
    modelId?: string | null;
    language?: string;
  };

  function parseTranscriptionStructuredPayload(
    json: string | null | undefined,
  ): Partial<TranscriptionStructuredPayload> | null {
    if (!json) return null;
    try {
      return JSON.parse(json) as Partial<TranscriptionStructuredPayload>;
    } catch {
      return null;
    }
  }

  function parseAudioTranscriptionJobPayload(
    json: string | null | undefined,
  ): AudioTranscriptionJobPayloadShape | null {
    if (!json) return null;
    try {
      return JSON.parse(json) as AudioTranscriptionJobPayloadShape;
    } catch {
      return null;
    }
  }

  function formatAudioTranscriptionProviderLabel(provider: string): string {
    switch (provider) {
      case "local_whisper":
        return "Local Whisper";
      case "apple_speech_on_device":
        return "Apple Speech (on-device)";
      case "parakeet":
        return "Parakeet";
      default:
        return provider;
    }
  }

  function resolveAudioTranscriptionModelLabel(
    jobPayloadJson: string | null | undefined,
    resultPayloadJson: string | null | undefined,
  ): string | null {
    const jobPayload = parseAudioTranscriptionJobPayload(jobPayloadJson);
    const resultPayload = parseTranscriptionStructuredPayload(resultPayloadJson);
    const provider =
      typeof resultPayload?.provider === "string"
        ? resultPayload.provider
        : typeof jobPayload?.provider === "string"
          ? jobPayload.provider
          : null;
    if (!provider) return null;
    const providerLabel = formatAudioTranscriptionProviderLabel(provider);
    const modelId =
      typeof resultPayload?.modelId === "string"
        ? resultPayload.modelId
        : typeof jobPayload?.modelId === "string"
          ? jobPayload.modelId
          : null;
    return modelId ? `${providerLabel} · ${modelId}` : providerLabel;
  }

  function normalizeTranscriptionTimedRuns(
    runs: Array<Partial<TranscriptionSegment> | Partial<TranscriptionWord>>,
  ): TranscriptionSegment[] {
    const normalized: TranscriptionSegment[] = [];
    for (const run of runs) {
      if (
        !run ||
        typeof run.startMs !== "number" ||
        typeof run.endMs !== "number" ||
        typeof run.text !== "string"
      ) {
        continue;
      }
      const text = run.text.trim();
      if (!text) continue;
      const startMs = Math.max(0, Math.round(run.startMs));
      const endMs = Math.max(startMs, Math.round(run.endMs));
      normalized.push({
        startMs,
        endMs,
        text,
        confidence:
          typeof run.confidence === "number"
            ? Math.min(1, Math.max(0, run.confidence))
            : null,
      });
    }
    normalized.sort((a, b) => a.startMs - b.startMs || a.endMs - b.endMs);
    return normalized;
  }

  function parseTranscriptionSegments(
    json: string | null | undefined,
  ): TranscriptionSegment[] {
    const parsed = parseTranscriptionStructuredPayload(json);
    const segments = normalizeTranscriptionTimedRuns(
      Array.isArray(parsed?.segments) ? parsed.segments : [],
    );
    if (segments.length > 0) return segments;
    return normalizeTranscriptionTimedRuns(
      Array.isArray(parsed?.words) ? parsed.words : [],
    );
  }

  async function applySelectedAudioTranscriptJob(
    id: number,
    gen: number,
    job: ProcessingJobDto,
  ): Promise<void> {
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return;

    selectedAudioTranscriptModelLabel = resolveAudioTranscriptionModelLabel(
      job.payloadJson,
      null,
    );

    if (job.status === "queued" || job.status === "running") {
      selectedAudioTranscriptStatus = "running";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = null;
      scheduleSelectedAudioTranscriptPoll(id, job.id, gen);
      return;
    }

    clearSelectedAudioTranscriptPoll();

    if (job.status === "failed") {
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = job.lastError ?? "Transcription job failed";
      return;
    }

    const result = await invoke<ProcessingResultDto | null>("get_processing_result", {
      request: { jobId: job.id } satisfies GetProcessingResultRequest,
    });
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return;

    if (!result) {
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = "Transcription result not available";
      return;
    }

    const segments = parseTranscriptionSegments(result.structuredPayloadJson);
    const transcript = result.resultText?.trim().length
      ? result.resultText
      : segments.map((segment) => segment.text).join(" ");
    selectedAudioTranscriptText = transcript;
    selectedAudioTranscriptSegments = segments;
    selectedAudioTranscriptModelLabel = resolveAudioTranscriptionModelLabel(
      job.payloadJson,
      result.structuredPayloadJson,
    );
    selectedAudioTranscriptError = null;
    selectedAudioTranscriptStatus = transcript.trim().length === 0 ? "empty" : "success";
  }

  async function loadSelectedAudioSegmentTranscript(id: number, gen: number): Promise<void> {
    try {
      const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
        request: { subjectType: AUDIO_SEGMENT_SUBJECT_TYPE, subjectId: id },
      });
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;

      const transcriptionJobs = jobs.filter(
        (job) => job.processor === AUDIO_TRANSCRIPTION_PROCESSOR,
      );
      if (transcriptionJobs.length === 0) {
        clearSelectedAudioTranscriptPoll();
        selectedAudioTranscriptStatus = "missing";
        selectedAudioTranscriptText = null;
        selectedAudioTranscriptSegments = [];
        selectedAudioTranscriptError = null;
        return;
      }

      const completed = transcriptionJobs
        .filter((job) => job.status === "completed")
        .sort((a, b) => b.id - a.id);
      const job = completed[0] ?? transcriptionJobs.sort((a, b) => b.id - a.id)[0];
      await applySelectedAudioTranscriptJob(id, gen, job);
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      clearSelectedAudioTranscriptPoll();
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  const selectedAudioTranscriptActionLabel = $derived(
    selectedAudioTranscriptStatus === "missing" ? "Run" : "Rerun",
  );
  const selectedAudioTranscriptActionDisabled = $derived(
    !selectedAudioSegment ||
      selectedAudioSegment.source !== "microphone" ||
      selectedAudioTranscriptRerunLoading ||
      selectedAudioTranscriptStatus === "loading" ||
      selectedAudioTranscriptStatus === "running",
  );
  const selectedAudioTranscriptActionTitle = $derived(
    selectedAudioSegment?.source !== "microphone"
      ? "Only microphone segments can be transcribed"
      : selectedAudioTranscriptStatus === "loading"
        ? "Transcript is still loading"
        : selectedAudioTranscriptStatus === "running"
          ? "Transcription is queued or still processing"
          : `${selectedAudioTranscriptActionLabel} transcription with current settings`,
  );

  async function reprocessSelectedAudioSegmentTranscript(): Promise<void> {
    const segment = selectedAudioSegment;
    if (!segment || selectedAudioTranscriptActionDisabled) return;
    const id = segment.id;

    selectedAudioTranscriptRerunLoading = true;
    selectedAudioTranscriptRerunError = null;
    try {
      const result = await invoke<AudioSegmentTranscriptionReprocessingResultDto>(
        "reprocess_audio_segment_transcription",
        {
          request: {
            audioSegmentId: id,
          } satisfies ReprocessAudioSegmentTranscriptionRequest,
        },
      );
      if (selectedAudioSegmentId !== id) return;
      selectedAudioTranscriptGeneration += 1;
      const gen = selectedAudioTranscriptGeneration;
      clearSelectedAudioTranscriptPoll();
      selectedAudioTranscriptStatus = "running";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = null;
      selectedAudioTranscriptRerunError = null;
      await applySelectedAudioTranscriptJob(id, gen, result.job);
    } catch (err) {
      if (selectedAudioSegmentId !== id) return;
      selectedAudioTranscriptRerunError = typeof err === "string" ? err : JSON.stringify(err);
    } finally {
      if (selectedAudioSegmentId === id) {
        selectedAudioTranscriptRerunLoading = false;
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
  let selectedAudioTranscriptContainerEl = $state<HTMLDivElement | null>(null);
  // While the user drags the scrub thumb we hold UI updates from `timeupdate`
  // events so the indicator doesn't fight the drag. Commit on release.
  let audioScrubbing = $state(false);

  function findActiveTranscriptSegmentIndex(
    segments: TranscriptionSegment[],
    currentTimeSeconds: number,
  ): number | null {
    if (!Number.isFinite(currentTimeSeconds) || currentTimeSeconds < 0) return null;
    const currentMs = Math.round(currentTimeSeconds * 1000);
    for (let index = segments.length - 1; index >= 0; index -= 1) {
      if (currentMs >= segments[index].startMs) return index;
    }
    return null;
  }

  const selectedAudioTranscriptActiveSegmentIndex = $derived(
    selectedAudioTranscriptSegments.length === 0 || (!audioIsPlaying && audioCurrentTime <= 0)
      ? null
      : findActiveTranscriptSegmentIndex(selectedAudioTranscriptSegments, audioCurrentTime),
  );

  $effect(() => {
    const activeIndex = selectedAudioTranscriptActiveSegmentIndex;
    const container = selectedAudioTranscriptContainerEl;
    if (activeIndex == null || !container) return;
    void tick().then(() => {
      const activeSegment = container.querySelector<HTMLElement>(
        `[data-transcript-segment-index="${activeIndex}"]`,
      );
      activeSegment?.scrollIntoView({ block: "nearest", inline: "nearest" });
    });
  });

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

  function seekAudioToTimeMs(startMs: number): void {
    const el = audioEl;
    if (!el) return;
    const nextTime = Math.max(
      0,
      Math.min(Number.isFinite(audioDuration) && audioDuration > 0 ? audioDuration : Infinity, startMs / 1000),
    );
    if (!Number.isFinite(nextTime)) return;
    el.currentTime = nextTime;
    audioCurrentTime = nextTime;
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

  function formatTranscriptSegmentTitle(segment: TranscriptionSegment): string {
    const start = formatPlayerTime(segment.startMs / 1000);
    if (segment.endMs <= segment.startMs) return start;
    return `${start}–${formatPlayerTime(segment.endMs / 1000)}`;
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

  function clearFrameActionStatusTimer() {
    if (frameActionStatusTimer) {
      clearTimeout(frameActionStatusTimer);
      frameActionStatusTimer = null;
    }
  }

  function scheduleFrameActionStatusDismiss() {
    clearFrameActionStatusTimer();
    if (!frameActionStatus || frameActionStatusHovered) return;
    frameActionStatusTimer = setTimeout(() => {
      frameActionStatus = null;
      frameActionStatusTimer = null;
    }, 2200);
  }

  function setFrameActionStatus(
    message: string | null,
    options?: {
      detail?: string | null;
      tone?: FrameActionStatus["tone"];
    },
  ) {
    frameActionStatus = message
      ? {
          message,
          detail: options?.detail ?? null,
          tone: options?.tone ?? "neutral",
        }
      : null;
    frameActionStatusHovered = false;
    clearFrameActionStatusTimer();
    if (!message) return;
    scheduleFrameActionStatusDismiss();
  }

  function isTimelineActiveFrame(frameId: number): boolean {
    return timelineActive?.id === frameId;
  }

  function clearActivePreviewFetchTimer(): void {
    if (activePreviewFetchTimer != null) {
      clearTimeout(activePreviewFetchTimer);
      activePreviewFetchTimer = null;
    }
  }

  function trimPreviewCache(): void {
    if (previewCache.size <= PREVIEW_CACHE_MAX_ENTRIES) return;
    const next = new Map(previewCache);
    while (next.size > PREVIEW_CACHE_MAX_ENTRIES) {
      const oldestFrameId = next.keys().next().value;
      if (oldestFrameId == null) break;
      next.delete(oldestFrameId);
    }
    previewCache = next;
  }

  function touchPreviewCache(frameId: number, url: string): void {
    const next = new Map(previewCache);
    next.delete(frameId);
    next.set(frameId, url);
    previewCache = next;
    trimPreviewCache();
  }

  function rememberPreviewFailure(frameId: number): void {
    const next = new Map(previewFailedAt);
    next.set(frameId, Date.now());
    previewFailedAt = next;
  }

  function clearPreviewFailure(frameId: number): void {
    if (!previewFailedAt.has(frameId)) return;
    const next = new Map(previewFailedAt);
    next.delete(frameId);
    previewFailedAt = next;
  }

  function dropPreviewCacheEntry(frameId: number): void {
    if (previewCache.has(frameId)) {
      const next = new Map(previewCache);
      next.delete(frameId);
      previewCache = next;
    }
    if (previewMimeTypeCache.has(frameId)) {
      const nextMimeTypes = new Map(previewMimeTypeCache);
      nextMimeTypes.delete(frameId);
      previewMimeTypeCache = nextMimeTypes;
    }
  }

  function recentlyFailedPreview(frameId: number): boolean {
    const failedAt = previewFailedAt.get(frameId);
    if (failedAt == null) return false;
    if (Date.now() - failedAt < PREVIEW_FAILURE_CACHE_TTL_MS) {
      return true;
    }
    clearPreviewFailure(frameId);
    return false;
  }

  function currentPreviewPrefetchRadius(): number {
    return previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS
      ? ACTIVE_PREVIEW_FAST_SCRUB_RADIUS
      : ACTIVE_PREVIEW_PREFETCH_RADIUS;
  }

  function currentActivePreviewFetchDebounceMs(): number {
    return previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS
      ? ACTIVE_PREVIEW_FETCH_FAST_SCRUB_DEBOUNCE_MS
      : 0;
  }

  function prefetchPreviewNeighbors(activeIndex: number): void {
    const radius = currentPreviewPrefetchRadius();
    for (let offset = 1; offset <= radius; offset += 1) {
      const newer = timelineFrames[activeIndex - offset];
      const older = timelineFrames[activeIndex + offset];
      if (newer && !previewCache.has(newer.id) && !previewInFlight.has(newer.id)) {
        void ensurePreview(newer.id);
      }
      if (older && !previewCache.has(older.id) && !previewInFlight.has(older.id)) {
        void ensurePreview(older.id);
      }
    }
  }

  function onFrameActionStatusPointerEnter() {
    frameActionStatusHovered = true;
    clearFrameActionStatusTimer();
  }

  function onFrameActionStatusPointerLeave() {
    frameActionStatusHovered = false;
    scheduleFrameActionStatusDismiss();
  }

  function prettifyFramePreviewError(rawMessage: string): string {
    const message = rawMessage
      .replace(/^failed to get frame preview \d+:\s*/i, "")
      .trim();

    if (/no decodable video sample found/i.test(message)) {
      return "Couldn't load the frame preview from this recording.";
    }
    if (/sample did not contain an image buffer/i.test(message)) {
      return "Couldn't decode an image from the frame preview.";
    }
    if (/failed to start reading video samples/i.test(message)) {
      return "Couldn't read the recording while generating the frame preview.";
    }
    if (/failed to read video sample/i.test(message)) {
      return "Couldn't read a video sample for the frame preview.";
    }
    if (/failed to convert preview sample .* CGImage/i.test(message)) {
      return "Couldn't convert the frame preview into an image.";
    }
    if (/failed to join video preview extraction task/i.test(message)) {
      return "Frame preview generation stopped unexpectedly.";
    }
    if (/only supported on macOS/i.test(message)) {
      return "Video fallback previews are only supported on macOS.";
    }
    if (/not found/i.test(message)) {
      return "This frame preview is no longer available.";
    }

    return "Couldn't load the frame preview.";
  }

  function previewSourceLabel(sourceKind: FramePreviewDto["sourceKind"]): string {
    switch (sourceKind) {
      case "original_frame":
        return "stored frame";
      case "segment_frame_fallback":
        return "segment frame fallback";
      case "video_fallback":
        return "video fallback";
    }
  }

  function fileExtensionForMimeType(mimeType: string | null): string {
    switch (mimeType) {
      case "image/jpeg":
        return "jpg";
      case "image/webp":
        return "webp";
      case "image/gif":
        return "gif";
      case "image/png":
      default:
        return "png";
    }
  }

  function activeFrameDownloadName(frame: FrameDto, mimeType: string | null): string {
    const capturedAt = frame.capturedAt.replace(/[^0-9A-Za-z]+/g, "-").replace(/^-+|-+$/g, "");
    const ext = fileExtensionForMimeType(mimeType);
    return `frame-${frame.id}-${capturedAt || "capture"}.${ext}`;
  }

  async function previewFilePathToClipboardImage(filePath: string): Promise<Image> {
    const blob = new Blob([await readFramePreviewBytes(filePath)]);
    const image = await createImageBitmap(blob);
    try {
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) throw new Error("2d canvas context unavailable");
      ctx.drawImage(image, 0, 0);
      const { data, width, height } = ctx.getImageData(0, 0, image.width, image.height);
      return await Image.new(new Uint8Array(data.buffer.slice(0)), width, height);
    } finally {
      image.close();
    }
  }

  async function copyActiveFrameImage(): Promise<void> {
    const frame = timelineActive;
    const previewUrl = frame ? previewCache.get(frame.id) : null;
    if (!frame || !previewUrl) {
      setFrameActionStatus("Frame preview not ready yet");
      return;
    }

    try {
      const image = await previewFilePathToClipboardImage(previewUrl);
      try {
        await writeImage(image);
      } finally {
        image.close();
      }
      setFrameActionStatus(`Copied frame ${frame.id}`);
      stageActionsMenuOpen = false;
    } catch (err) {
      setFrameActionStatus(
        `Copy failed: ${typeof err === "string" ? err : "clipboard write was rejected"}`,
      );
    }
  }

  async function downloadActiveFrameImage(): Promise<void> {
    const frame = timelineActive;
    const previewUrl = frame ? previewCache.get(frame.id) : null;
    if (!frame || !previewUrl) {
      setFrameActionStatus("Frame preview not ready yet");
      return;
    }

    try {
      await writeFile(
        activeFrameDownloadName(frame, previewMimeTypeCache.get(frame.id) ?? null),
        await readFramePreviewBytes(previewUrl),
        {
        baseDir: BaseDirectory.Download,
        },
      );
      setFrameActionStatus(`Saved frame ${frame.id} to Downloads`);
      stageActionsMenuOpen = false;
    } catch (err) {
      setFrameActionStatus(
        `Download failed: ${typeof err === "string" ? err : "file write was rejected"}`,
      );
    }
  }

  function onStageActionsToggle(event: Event) {
    stageActionsMenuOpen = (event.currentTarget as HTMLDetailsElement | null)?.open ?? false;
  }

  $effect(() => {
    return () => {
      clearFrameActionStatusTimer();
    };
  });

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

  let refreshAudioSegmentsDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleAudioSegmentsRefresh(): void {
    if (refreshAudioSegmentsDebounceTimer != null) {
      clearTimeout(refreshAudioSegmentsDebounceTimer);
    }
    refreshAudioSegmentsDebounceTimer = setTimeout(() => {
      refreshAudioSegmentsDebounceTimer = null;
      void refreshAudioSegments();
    }, AUDIO_SEGMENT_REFRESH_DEBOUNCE_MS);
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
    if (previewCache.has(frameId)) {
      previewCacheHitCount += 1;
      const url = previewCache.get(frameId);
      if (url) touchPreviewCache(frameId, url);
      return;
    }
    if (recentlyFailedPreview(frameId)) {
      previewFailureCacheHitCount += 1;
      return;
    }
    previewCacheMissCount += 1;
    if (previewInFlight.has(frameId)) {
      previewInFlightJoinCount += 1;
      return;
    }
    previewInFlight.add(frameId);
    const isActiveFrame = isTimelineActiveFrame(frameId);
    if (isActiveFrame) {
      setFrameActionStatus("Loading frame preview...");
    }
    try {
      const dto = await invoke<FramePreviewDto | null>("get_frame_preview", {
        request: { frameId },
      });
      if (!dto) {
        throw new Error(`frame preview ${frameId} not found`);
      }
      clearPreviewFailure(frameId);
      touchPreviewCache(frameId, dto.filePath);
      if (activePreviewLoadErrorFrameId === frameId) {
        activePreviewLoadErrorFrameId = null;
      }
      const nextMimeTypes = new Map(previewMimeTypeCache);
      nextMimeTypes.set(frameId, dto.mimeType);
      previewMimeTypeCache = nextMimeTypes;
      if (dto.sourceKind === "original_frame") {
        previewDirectPathCount += 1;
      } else {
        previewGeneratedPathCount += 1;
      }
      if (isTimelineActiveFrame(frameId)) {
        setFrameActionStatus(null);
      }
    } catch (error) {
      rememberPreviewFailure(frameId);
      const message = error instanceof Error ? error.message : String(error);
      if (isTimelineActiveFrame(frameId)) {
        setFrameActionStatus(prettifyFramePreviewError(message), {
          detail: message,
          tone: "error",
        });
      }
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
        previewMimeTypeCache = new Map();
        previewFailedAt = new Map();
        activePreviewLoadErrorFrameId = null;
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
    const now = performance.now();
    const deltaMs = now - lastTimelineScrollSample.at;
    if (deltaMs > 0) {
      previewScrubVelocityPxPerMs = Math.abs(el.scrollLeft - lastTimelineScrollSample.left) / deltaMs;
    }
    lastTimelineScrollSample = { left: el.scrollLeft, at: now };
    timelineScrollLeft = el.scrollLeft;
    // Resize-induced scroll guard: when the window grows, `cqi`-based track
    // margins recompute non-atomically with `clientWidth`, so the browser
    // emits a `scroll` event against a transiently inconsistent
    // `(scrollLeft, scrollWidth, clientWidth)` triple before the
    // `ResizeObserver` can re-sync us. Recomputing the active index from
    // that triple jumps the user to a different frame on every enlarge.
    // Detect this by comparing `clientWidth` to the value the observer last
    // acknowledged: load-more grows `scrollWidth` but not `clientWidth`, so
    // genuine user scrubs during pagination still pass through.
    if (el.clientWidth !== lastTimelineRailClientWidth) {
      return;
    }
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

  function isTimelineShortcutSuppressedTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(
      target.closest(
        'input, textarea, select, button, audio, video, [contenteditable="true"], [role="textbox"], [role="searchbox"], [role="spinbutton"], [role="slider"], [role="combobox"], [role="switch"], [role="menuitem"], [data-shortcuts-ignore], .timeline__picker, .audio-drawer',
      ),
    );
  }

  // Page-level timeline shortcuts: ArrowLeft/ArrowRight move the active frame
  // even when the thin rail itself does not have focus. Interactive surfaces
  // keep their own keyboard behavior (calendar navigation, buttons, audio
  // scrubbing, text selection, etc.).
  function onTimelineWindowKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
    if (isTimelineShortcutSuppressedTarget(event.target)) return;
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    onTimelineKeyDown(event);
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
  // across resizes. Only the slots near the viewport are rendered. Also acts
  // as the **resize-recovery seam**: every observer fire records the current
  // `clientWidth` and re-pins `scrollLeft` to the canonical
  // `timelineActiveIndex`. The browser preserves `scrollLeft` across resize
  // only as a number — when the window grows, `cqi`-driven margin reflow
  // briefly shrinks `maxScroll`, the browser clamps `scrollLeft`, and an
  // intervening `scroll` event would otherwise corrupt the active index.
  // `onTimelineScroll` filters those events out via the
  // `lastTimelineRailClientWidth` mismatch check above, and this observer
  // then restores the parked frame after the resize settles.
  $effect(() => {
    const el = timelineRail;
    if (!el) return;
    timelineViewportWidth = el.clientWidth;
    lastTimelineRailClientWidth = el.clientWidth;
    if (typeof ResizeObserver === "undefined") return;
    let firstFire = true;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        timelineViewportWidth = entry.contentRect.width;
      }
      // The synthetic first fire happens immediately after `observe` and
      // doesn't represent a real resize: the rail was just laid out and the
      // initial scroll position has already been set by callers like
      // `loadTimelinePage` via `syncTimelineScrollToActiveFrame`. Skipping
      // it avoids an extra programmatic scroll on mount.
      if (firstFire) {
        firstFire = false;
        lastTimelineRailClientWidth = el.clientWidth;
        return;
      }
      lastTimelineRailClientWidth = el.clientWidth;
      // Restore scroll from the canonical active index. The intermediate
      // browser-emitted `scroll` event (clamped during reflow) was filtered
      // out by `onTimelineScroll`, so `timelineActiveIndex` still points at
      // the user's parked frame.
      void syncTimelineScrollToActiveFrame();
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  // Slow scrubs should show each frame's preview promptly, so we only debounce
  // while the user is moving fast enough to outrun the preview pipeline. The
  // latest-only generation token ensures an older scheduled fetch cannot start
  // after a newer active frame supersedes it.
  $effect(() => {
    const active = timelineActive;
    const activeIndex = timelineActiveIndex;
    const debounceMs = currentActivePreviewFetchDebounceMs();
    activePreviewFetchGeneration += 1;
    const gen = activePreviewFetchGeneration;
    clearActivePreviewFetchTimer();
    if (!active || previewCache.has(active.id) || previewInFlight.has(active.id)) {
      return;
    }
    activePreviewFetchTimer = setTimeout(() => {
      activePreviewFetchTimer = null;
      if (gen !== activePreviewFetchGeneration) return;
      if (!isTimelineActiveFrame(active.id)) return;
      void ensurePreview(active.id);
      prefetchPreviewNeighbors(activeIndex);
    }, debounceMs);
    return () => {
      clearActivePreviewFetchTimer();
    };
  });

  // ─── On-demand OCR for the active frame ──────────────────────────────────
  // The "show OCR" header button loads existing OCR for the active frame,
  // and the inline rerun button reprocesses it with current settings. Queued
  // or running jobs are polled until they reach a terminal state. On success
  // we parse the structured payload (Apple Vision: normalised coords with
  // lower-left origin) and overlay each observation as a translucent box +
  // text label on the preview. The overlay is positioned against the
  // *rendered* image bounds inside the stage (object-fit: contain), not the
  // full stage rect, so boxes align with what the user actually sees.
  //
  // State machine:
  //   "idle"     — no fetch requested for the current frame
  //   "running"  — an OCR job exists for this frame but has not yet
  //                terminated on the backend (queued or running). We poll
  //                until the job reaches a terminal state.
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
  const AUDIO_SEGMENT_SUBJECT_TYPE = "audio_segment";
  const OCR_PROCESSOR = "ocr";
  const OCR_SOURCE_IMAGE_PATH_OPTION = "mnemaSourceImagePath";
  const AUDIO_TRANSCRIPTION_PROCESSOR = "audio_transcription";

  let ocrStatus = $state<OcrStatus>("idle");
  let ocrError = $state<string | null>(null);
  let ocrObservations = $state<OcrObservation[]>([]);
  let ocrProviderLabel = $state<string | null>(null);
  let ocrFrameId = $state<number | null>(null);
  let ocrSourceFrame = $state<FrameDto | null>(null);
  let ocrGeneration = 0;
  let ocrPollTimer: ReturnType<typeof setTimeout> | null = null;
  let ocrPollJobId: number | null = null;
  let ocrRerunLoading = $state(false);
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
      clearOcrPoll();
      ocrObservations = [];
      ocrProviderLabel = null;
      ocrFrameId = null;
      ocrSourceFrame = null;
      ocrVisible = false;
      ocrRerunLoading = false;
    }
  });

  type OcrPayloadShape = {
    provider?: string;
    modelId?: string | null;
    observations?: OcrObservation[];
    provenance?: {
      provider?: string;
      modelId?: string | null;
    } | null;
  };

  type OcrJobPayloadShape = {
    provider?: string;
    modelId?: string | null;
  };

  function formatOcrProviderLabel(provider: string): string {
    switch (provider) {
      case "apple_vision": return "Apple Vision";
      case "tesseract": return "Tesseract";
      case "paddle_ocr": return "PaddleOCR";
      default: return provider;
    }
  }

  function resolveOcrProviderLabel(
    jobPayloadJson: string | null | undefined,
    resultPayloadJson: string | null | undefined,
    processorVersion: string | null | undefined,
  ): string | null {
    let jobPayload: OcrJobPayloadShape | null = null;
    let resultPayload: OcrPayloadShape | null = null;
    try {
      if (jobPayloadJson) jobPayload = JSON.parse(jobPayloadJson) as OcrJobPayloadShape;
    } catch {}
    try {
      if (resultPayloadJson) resultPayload = JSON.parse(resultPayloadJson) as OcrPayloadShape;
    } catch {}

    const provider =
      typeof resultPayload?.provider === "string"
        ? resultPayload.provider
        : typeof resultPayload?.provenance?.provider === "string"
          ? resultPayload.provenance.provider
          : typeof jobPayload?.provider === "string"
            ? jobPayload.provider
            : typeof processorVersion === "string" && processorVersion.includes(":")
              ? processorVersion.split(":", 1)[0]
              : processorVersion;
    if (!provider) return null;
    const modelId =
      typeof resultPayload?.modelId === "string"
        ? resultPayload.modelId
        : typeof resultPayload?.provenance?.modelId === "string"
          ? resultPayload.provenance.modelId
          : typeof jobPayload?.modelId === "string"
            ? jobPayload.modelId
            : null;
    const providerLabel = formatOcrProviderLabel(provider);
    return modelId ? `${providerLabel} · ${modelId}` : providerLabel;
  }

  function parseOcrPayload(json: string | null | undefined): { observations: OcrObservation[]; providerLabel: string | null } | null {
    if (!json) return null;
    try {
      const parsed = JSON.parse(json) as Partial<OcrStructuredPayload>;
      const obs = Array.isArray(parsed?.observations) ? parsed.observations : null;
      if (!obs) return null;
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
      const provider =
        typeof parsed?.provider === "string"
          ? parsed.provider
          : typeof parsed?.provenance?.provider === "string"
            ? parsed.provenance.provider
            : null;
      const modelId =
        typeof parsed?.modelId === "string"
          ? parsed.modelId
          : typeof parsed?.provenance?.modelId === "string"
            ? parsed.provenance.modelId
            : null;
      return {
        observations: out,
        providerLabel: provider ? (modelId ? `${formatOcrProviderLabel(provider)} · ${modelId}` : formatOcrProviderLabel(provider)) : null,
      };
    } catch {
      return null;
    }
  }

  type OcrLoadResult = {
    status: OcrStatus;
    observations: OcrObservation[];
    providerLabel: string | null;
    error: string | null;
    job: ProcessingJobDto | null;
  };

  async function loadOcrFromJob(job: ProcessingJobDto): Promise<OcrLoadResult> {
    if (job.status === "queued" || job.status === "running") {
      return {
        status: "running",
        observations: [],
        providerLabel: resolveOcrProviderLabel(job.payloadJson, null, null),
        error: null,
        job,
      };
    }
    if (job.status === "failed") {
      return {
        status: "error",
        observations: [],
        providerLabel: resolveOcrProviderLabel(job.payloadJson, null, null),
        error: job.lastError ?? "OCR job failed",
        job,
      };
    }

    const result = await invoke<ProcessingResultDto | null>("get_processing_result", {
      request: { jobId: job.id } satisfies GetProcessingResultRequest,
    });

    const parsedPayload = parseOcrPayload(result?.structuredPayloadJson);
    if (parsedPayload === null) {
      return {
        status: "error",
        observations: [],
        providerLabel: resolveOcrProviderLabel(job.payloadJson, result?.structuredPayloadJson, result?.processorVersion),
        error: result ? "OCR result payload is missing or invalid" : "OCR result not available",
        job,
      };
    }

    return {
      status: parsedPayload.observations.length === 0 ? "empty" : "success",
      observations: parsedPayload.observations,
      providerLabel: parsedPayload.providerLabel ?? resolveOcrProviderLabel(job.payloadJson, result?.structuredPayloadJson, result?.processorVersion),
      error: null,
      job,
    };
  }

  async function loadOcrForFrame(sourceFrame: FrameDto): Promise<OcrLoadResult> {
    const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
      request: { subjectType: FRAME_SUBJECT_TYPE, subjectId: sourceFrame.id },
    });

    const ocrJobs = jobs.filter((j) => j.processor === OCR_PROCESSOR);
    if (ocrJobs.length === 0) {
      return { status: "missing", observations: [], providerLabel: null, error: null, job: null };
    }

    const completed = ocrJobs
      .filter((j) => j.status === "completed")
      .sort((a, b) => b.id - a.id);
    const job = completed[0] ?? ocrJobs.sort((a, b) => b.id - a.id)[0];
    return loadOcrFromJob(job);
  }

  function ocrIsCurrent(activeFrameId: number, gen: number): boolean {
    return gen === ocrGeneration && timelineActive?.id === activeFrameId && ocrFrameId === activeFrameId;
  }

  function clearOcrPoll(): void {
    if (ocrPollTimer) {
      clearTimeout(ocrPollTimer);
      ocrPollTimer = null;
    }
    ocrPollJobId = null;
  }

  function scheduleOcrPoll(activeFrameId: number, sourceFrame: FrameDto, jobId: number, gen: number): void {
    if (!ocrIsCurrent(activeFrameId, gen)) return;
    if (ocrPollTimer && ocrPollJobId === jobId) return;
    clearOcrPoll();
    ocrPollJobId = jobId;
    ocrPollTimer = setTimeout(() => {
      ocrPollTimer = null;
      ocrPollJobId = null;
      void pollOcrJob(activeFrameId, sourceFrame, jobId, gen);
    }, OCR_POLL_INTERVAL_MS);
  }

  function applyLoadedOcrData(
    activeFrameId: number,
    sourceFrame: FrameDto,
    gen: number,
    ocrData: OcrLoadResult,
  ): void {
    if (!ocrIsCurrent(activeFrameId, gen)) return;
    ocrSourceFrame = sourceFrame;
    ocrStatus = ocrData.status;
    ocrError = ocrData.error;
    ocrObservations = ocrData.observations;
    ocrProviderLabel = ocrData.providerLabel;
    if (ocrData.status === "running" && ocrData.job) {
      scheduleOcrPoll(activeFrameId, sourceFrame, ocrData.job.id, gen);
    } else {
      clearOcrPoll();
      ocrRerunLoading = false;
    }
  }

  async function pollOcrJob(
    activeFrameId: number,
    sourceFrame: FrameDto,
    jobId: number,
    gen: number,
  ): Promise<void> {
    if (!ocrIsCurrent(activeFrameId, gen)) return;
    try {
      const job = await invoke<ProcessingJobDto | null>("get_processing_job", {
        request: { jobId } satisfies GetProcessingJobRequest,
      });
      if (!ocrIsCurrent(activeFrameId, gen)) return;
      if (!job) {
        applyLoadedOcrData(activeFrameId, sourceFrame, gen, {
          status: "error",
          observations: [],
          providerLabel: null,
          error: "OCR job not found",
          job: null,
        });
        return;
      }
      const ocrData = await loadOcrFromJob(job);
      applyLoadedOcrData(activeFrameId, sourceFrame, gen, ocrData);
    } catch (err) {
      if (!ocrIsCurrent(activeFrameId, gen)) return;
      applyLoadedOcrData(activeFrameId, sourceFrame, gen, {
        status: "error",
        observations: [],
        providerLabel: null,
        error: typeof err === "string" ? err : (err as Error)?.message ?? JSON.stringify(err),
        job: null,
      });
    }
  }

  async function loadOcrForActiveFrame(): Promise<void> {
    const frame = timelineActive;
    if (!frame) return;
    // Bump the generation so any in-flight fetch for a prior call is dropped
    // when its response checks the token.
    ocrGeneration += 1;
    const gen = ocrGeneration;
    const frameId = frame.id;
    clearOcrPoll();
    ocrFrameId = frameId;
    ocrStatus = "running";
    ocrError = null;
    ocrObservations = [];
    ocrProviderLabel = null;
    ocrSourceFrame = frame;
    ocrVisible = true;
    ocrRerunLoading = false;

    try {
      if (!ocrIsCurrent(frameId, gen)) return;

      let sourceFrame = frame;
      let ocrData = await loadOcrForFrame(sourceFrame);
      if (!ocrIsCurrent(frameId, gen)) return;

      if (ocrData.status === "missing") {
        const fallbackFrame = await invoke<FrameDto | null>(
          "get_earliest_earlier_equivalent_frame",
          {
            request: {
              frameId: frame.id,
            } satisfies GetEarliestEarlierEquivalentFrameRequest,
          },
        );
        if (!ocrIsCurrent(frameId, gen)) return;

        if (fallbackFrame) {
          sourceFrame = fallbackFrame;
          ocrData = await loadOcrForFrame(sourceFrame);
          if (!ocrIsCurrent(frameId, gen)) return;
        }
      }

      applyLoadedOcrData(frameId, sourceFrame, gen, ocrData);
    } catch (err) {
      if (!ocrIsCurrent(frameId, gen)) return;
      applyLoadedOcrData(frameId, frame, gen, {
        status: "error",
        observations: [],
        providerLabel: null,
        error: typeof err === "string" ? err : (err as Error)?.message ?? JSON.stringify(err),
        job: null,
      });
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

  const ocrRerunButtonLabel = $derived(
    ocrRerunLoading
      ? "rerunning…"
      : ocrStatus === "missing"
        ? "run OCR"
        : "rerun OCR",
  );

  const ocrRerunDisabled = $derived(
    !timelineActive || ocrRerunLoading || ocrStatus === "running",
  );

  async function reprocessOcrForActiveFrame(): Promise<void> {
    const frame = timelineActive;
    if (!frame || ocrRerunDisabled) return;

    ocrGeneration += 1;
    const gen = ocrGeneration;
    const frameId = frame.id;
    clearOcrPoll();
    ocrFrameId = frameId;
    ocrStatus = "running";
    ocrError = null;
    ocrObservations = [];
    ocrProviderLabel = null;
    ocrSourceFrame = frame;
    ocrVisible = true;
    ocrRerunLoading = true;

    try {
      let sourceImagePath = previewCache.get(frameId) ?? null;
      if (!sourceImagePath) {
        await ensurePreview(frameId);
        sourceImagePath = previewCache.get(frameId) ?? null;
      }
      const payloadJson = sourceImagePath
        ? JSON.stringify({
            provider: "apple_vision",
            options: {
              [OCR_SOURCE_IMAGE_PATH_OPTION]: sourceImagePath,
            },
          })
        : null;
      const result = await invoke<CapturedFrameReprocessingResultDto>(
        "reprocess_captured_frame_ocr",
        {
          request: {
            frameId,
            payloadJson,
          } satisfies ReprocessCapturedFrameOcrRequest,
        },
      );
      if (!ocrIsCurrent(frameId, gen)) return;
      const ocrData = await loadOcrFromJob(result.job);
      applyLoadedOcrData(frameId, frame, gen, ocrData);
    } catch (err) {
      if (!ocrIsCurrent(frameId, gen)) return;
      applyLoadedOcrData(frameId, frame, gen, {
        status: "error",
        observations: [],
        providerLabel: null,
        error: typeof err === "string" ? err : (err as Error)?.message ?? JSON.stringify(err),
        job: null,
      });
    }
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
  //
  // Boxes are drawn quietly by default; the recognized text only appears
  // when the user hovers/focuses a single box. The reveal uses an opaque
  // chip whose font-size is derived from the box height (so it visually
  // matches the underlying glyph row and replaces — rather than doubles —
  // the pixels underneath).
  function ocrBoxStyle(obs: OcrObservation): string {
    const bb = obs.boundingBox;
    const leftPct = bb.x * 100;
    const topPct = (1 - bb.y - bb.height) * 100;
    const widthPct = bb.width * 100;
    const heightPct = bb.height * 100;
    // 0.78 ≈ cap-height ratio of common UI fonts; keeps glyphs vertically
    // centered inside the bbox without descenders escaping the bottom edge.
    const heightPx = Math.max(8, bb.height * renderedImageRect.height);
    const fontSizePx = Math.max(6, heightPx * 0.78);
    return `left: ${leftPct}%; top: ${topPct}%; width: ${widthPct}%; height: ${heightPct}%; --ocr-font-size: ${fontSizePx.toFixed(2)}px;`;
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
  // Selected hour bucket label, e.g. "1:00 AM". Null when nothing has been
  // chosen for the current selected date yet.
  let pickerSelectedTime = $state<string | null>(null);
  // Guard token that suppresses the date-change auto-jump effect for one
  // tick after we programmatically seed `pickerSelectedDate` (e.g. when the
  // picker opens, or when we sync the selection back to the resolved
  // jump-target frame). Without this, opening the picker would immediately
  // trigger a date-jump, and post-jump bookkeeping would re-jump in a loop.
  let suppressPickerDateAutoJump = false;
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

  // Hourly time buckets for the selected date. Labels use 12-hour clock
  // (e.g. "1:00 AM", "12:00 PM", "11:00 PM"). For today we stop at the
  // current hour; for any other date we render the full day through 11 PM.
  // The chosen hour resolves to the latest frame at-or-before that hour's
  // end via `get_latest_frame_in_range`, so the rail jumps near the picked
  // time even if no frame falls exactly inside that hour.
  type TimeBucket = { label: string; hour: number; disabled: boolean };
  function formatHourLabel(hour: number): string {
    const period = hour < 12 ? "AM" : "PM";
    const display = hour % 12 === 0 ? 12 : hour % 12;
    return `${display}:00 ${period}`;
  }
  const availableTimes = $derived.by<TimeBucket[]>(() => {
    if (!pickerSelectedDate) return [];
    const now = new Date();
    const isToday =
      pickerSelectedDate.year === now.getFullYear() &&
      pickerSelectedDate.month === now.getMonth() + 1 &&
      pickerSelectedDate.day === now.getDate();
    const lastHour = isToday ? now.getHours() : 23;
    // Determine which hours of the selected date have at least one frame,
    // using the already-loaded month summaries. If the month for this date
    // has not loaded yet, leave every hour enabled — disabling everything on
    // a not-yet-loaded month would block the user from time-picking before
    // background data arrives. Once the month is loaded, an absent day key
    // means the day truly has no frames, so all its hours render disabled.
    const monthLoaded = loadedMonths.has(monthKeyOf(pickerSelectedDate));
    const hoursWithFrames = new Set<number>();
    if (monthLoaded) {
      const daySummaries = summariesByDate.get(dateKeyOf(pickerSelectedDate));
      if (daySummaries) {
        for (const s of daySummaries) {
          const d = parseCapturedAt(s.capturedAt);
          if (!isNaN(d.getTime())) hoursWithFrames.add(d.getHours());
        }
      }
    }
    const out: TimeBucket[] = [];
    for (let h = 0; h <= lastHour; h++) {
      const disabled = monthLoaded && !hoursWithFrames.has(h);
      out.push({ label: formatHourLabel(h), hour: h, disabled });
    }
    return out;
  });

  async function jumpToFrame(target: FrameDto, closePicker = true): Promise<void> {
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
      // Keep picker selection state in sync with the resolved target frame
      // so the calendar/time list reflects where we actually landed. The
      // suppression flag prevents the date-change effect from re-jumping
      // in response to our own assignment.
      const resolved = parseCapturedAt(target.capturedAt);
      if (!isNaN(resolved.getTime())) {
        suppressPickerDateAutoJump = true;
        const cd = new CalendarDate(
          resolved.getFullYear(),
          resolved.getMonth() + 1,
          resolved.getDate(),
        );
        pickerPlaceholder = cd;
        pickerSelectedDate = cd;
        pickerSelectedTime = formatHourLabel(resolved.getHours());
      }
      if (closePicker) pickerOpen = false;
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

  async function resolveAndJump(
    rangeStart: Date,
    rangeEnd: Date,
    closePicker = true,
  ): Promise<void> {
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
      await jumpToFrame(frame, closePicker);
    } catch (err) {
      pickerError = typeof err === "string" ? err : JSON.stringify(err);
    }
  }

  async function jumpToSelectedDateLatest(closePicker = true): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
    const end = new Date(d.year, d.month - 1, d.day, 23, 59, 59, 999);
    await resolveAndJump(start, end, closePicker);
  }

  async function jumpToSelectedDateTime(label: string, hour: number): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    const start = new Date(d.year, d.month - 1, d.day, 0, 0, 0, 0);
    // "Latest at or before the end of the picked hour" — backend treats
    // the range as inclusive, so we extend to :59:59.999 of that hour.
    const end = new Date(d.year, d.month - 1, d.day, hour, 59, 59, 999);
    pickerSelectedTime = label;
    await resolveAndJump(start, end);
  }

  // Auto-jump when the user picks a date in the calendar. We deliberately
  // do NOT close the picker so the user can then choose a time, dismiss it
  // manually, or keep navigating. Programmatic seeding (open, post-jump
  // sync) bypasses this via `suppressPickerDateAutoJump`.
  $effect(() => {
    const d = pickerSelectedDate;
    if (!pickerOpen) return;
    if (suppressPickerDateAutoJump) {
      suppressPickerDateAutoJump = false;
      return;
    }
    if (!d) return;
    if (pickerJumping) return;
    // Once the month for this date has been loaded, we know definitively
    // whether the date has any frames. If it doesn't, skip the futile
    // backend call (and the resulting "no frame in that range" error).
    // When the month is still loading we don't over-block — the user may
    // have navigated into a freshly visible month whose summaries haven't
    // arrived yet, and the backend call is the cheapest way to land on a
    // real frame once data is available.
    if (loadedMonths.has(monthKeyOf(d)) && !summariesByDate.has(dateKeyOf(d))) {
      return;
    }
    void jumpToSelectedDateLatest(false);
  });

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
    // Always re-initialize from the active frame on open so the picker
    // reflects "you are here" rather than whatever was last selected.
    // Suppress the date-change auto-jump for this seeding so opening the
    // picker doesn't immediately re-jump to the frame already shown.
    suppressPickerDateAutoJump = true;
    if (timelineActive) {
      const d = parseCapturedAt(timelineActive.capturedAt);
      const cd = new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate());
      pickerPlaceholder = cd;
      pickerSelectedDate = cd;
      // Containing hour bucket: align with the hour the active frame
      // actually falls inside (floor), rather than rounding into the next
      // hour. Clamp to today's current hour when the active frame is on
      // today's date so we don't pre-select a future hour the list won't
      // even render.
      const candidate = d.getHours();
      const now = new Date();
      const isToday =
        d.getFullYear() === now.getFullYear() &&
        d.getMonth() === now.getMonth() &&
        d.getDate() === now.getDate();
      const maxHour = isToday ? now.getHours() : 23;
      const hour = Math.max(0, Math.min(maxHour, candidate));
      pickerSelectedTime = formatHourLabel(hour);
    } else {
      pickerSelectedDate = undefined;
      pickerSelectedTime = null;
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
  const latestFrameOffset = $derived(
    timelineFrames.length === 0 ? 0 : timelineActiveIndex + (timelineHasNewer ? 1 : 0),
  );
  const showJumpToLatestButton = $derived(latestFrameOffset > 50);

  async function jumpToLatestFrame(): Promise<void> {
    pickerOpen = false;
    pickerError = null;
    await loadTimelinePage(true);
  }

  // ─── Recording controls ──────────────────────────────────────────────────
  // Mirrors the debug page: bootstrap the shared `captureSession` store via
  // `get_capture_permissions`, load `recording_settings` so a fresh start
  // honours the user's persisted source toggles, and toggle between
  // `start_native_capture` / `stop_native_capture`. A monotonic generation
  // token prevents a slow `get_capture_permissions` reconciliation response
  // from clobbering an authoritative start/stop write that landed first.
  // The recording status indicator and start/stop button now live in the
  // app-wide title bar (see `routes/+layout.svelte`); the dashboard only
  // still needs `followTimelineLive` to drive the live-tail behaviour
  // around the rail head, plus `captureControls.bootstrapped` for the
  // idempotent bootstrap effect below.
  const followTimelineLive = $derived(captureControls.followTimelineLive);

  // Fire-and-forget bootstrap so the control reflects an already-running
  // recording started from the debug page or a prior session restored by
  // the backend.
  $effect(() => {
    if (captureControls.bootstrapped) return;
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
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      void resyncCaptureSession();
    };
    const onFocus = () => { void resyncCaptureSession(); };
    let unlistenSystemDidWake: (() => void) | undefined;
    let unlistenAudioSegmentsChanged: (() => void) | undefined;
    let destroyed = false;

    listen("system_did_wake", () => {
      void resyncCaptureSession();
      void pollTimelineHead();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenSystemDidWake = fn;
    });

    listen("audio_segments_changed", () => {
      scheduleAudioSegmentsRefresh();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenAudioSegmentsChanged = fn;
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
      unlistenAudioSegmentsChanged?.();
      if (refreshAudioSegmentsDebounceTimer != null) {
        clearTimeout(refreshAudioSegmentsDebounceTimer);
        refreshAudioSegmentsDebounceTimer = null;
      }
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("pageshow", onFocus);
      window.removeEventListener("online", onFocus);
      clearInterval(driftTimer);
    };
  });
</script>

<!-- ── Timeline browser ──────────────────────────────────────────────────── -->
<svelte:window onpointerdown={onPickerPointerDownOutside} onkeydown={onTimelineWindowKeyDown} />
<section class="timeline" onwheel={onTimelineWheel}>
  <header class="timeline__bar">
    <div class="timeline__bar-group timeline__bar-group--primary">
      <!-- Recording status indicator and start/stop controls now live in
           the app-wide title bar (see `routes/+layout.svelte`) so the
           recording affordance is visible regardless of which route is
           active. The timeline header retains only timeline-specific
           controls below (jump, OCR toggle, refresh). -->
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
        {#if showJumpToLatestButton}
          <button
            class="btn btn--ghost btn--sm timeline__jump-latest"
            onclick={jumpToLatestFrame}
            disabled={timelineLoading || timelineLoadingMore || pickerJumping}
            title="Jump to latest frame"
          >latest</button>
        {/if}
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
                  onclick={() => jumpToSelectedDateLatest()}
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
                        onclick={() => jumpToSelectedDateTime(t.label, t.hour)}
                        disabled={pickerJumping || t.disabled}
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
              ? `${ocrObservations.length} text region${ocrObservations.length === 1 ? "" : "s"} detected${ocrUsingEarlierFrame ? ` (reused from frame ${ocrSourceFrame?.id})` : ""}${ocrProviderLabel ? ` · ${ocrProviderLabel}` : ""}`
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
      {#if ocrVisible && timelineActive && ocrFrameId === timelineActive.id}
        {#if ocrProviderLabel}
          <span class="timeline__ocr-provider-chip" title={ocrProviderLabel}>{ocrProviderLabel}</span>
        {/if}
        <button
          type="button"
          class="btn btn--ghost btn--sm timeline__ocr-rerun-btn"
          onclick={reprocessOcrForActiveFrame}
          disabled={ocrRerunDisabled}
          title={ocrStatus === "running"
            ? "OCR is queued or still processing"
            : ocrStatus === "missing"
              ? "Run OCR for the active frame with current settings"
              : "Rerun OCR for the active frame with current settings"}
        >{ocrRerunButtonLabel}</button>
      {/if}
      <button
        class="btn btn--ghost btn--sm"
        onclick={refreshTimelineAndDashboard}
        disabled={timelineLoading || timelineLoadingMore || audioSegmentsLoading}
      >refresh</button>
    </div>
  </header>

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
      {@const previewPath = previewCache.get(timelineActive.id)}
      {@const previewUrl = previewPath ? framePreviewAssetUrl(previewPath) : null}
      {#if frameActionStatus}
        <div
          class="timeline__stage-status"
          class:timeline__stage-status--error={frameActionStatus.tone === "error"}
          role="status"
          aria-live="polite"
          onpointerenter={onFrameActionStatusPointerEnter}
          onpointerleave={onFrameActionStatusPointerLeave}
        >
          <div class="timeline__stage-status-summary">{frameActionStatus.message}</div>
          {#if frameActionStatus.detail}
            <div class="timeline__stage-status-detail">{frameActionStatus.detail}</div>
          {/if}
        </div>
      {/if}
      {#if previewUrl}
        <details class="timeline__stage-actions" open={stageActionsMenuOpen} ontoggle={onStageActionsToggle}>
          <summary
            class="btn btn--ghost btn--sm timeline__stage-action-trigger"
            aria-label="Frame actions"
            title="Frame actions"
          >⋯</summary>
          <div class="timeline__stage-action-menu">
            <button
              type="button"
              class="timeline__stage-action-menu-item"
              onclick={copyActiveFrameImage}
              aria-label="Copy active frame image"
              title="Copy image"
            >copy</button>
            <button
              type="button"
              class="timeline__stage-action-menu-item"
              onclick={downloadActiveFrameImage}
              aria-label="Download active frame image"
              title="Download image"
            >download</button>
          </div>
        </details>
        <div
          class="timeline__preview"
          role="img"
          aria-label={`frame ${timelineActive.id}`}
          style={`background-image: url("${previewUrl}");`}
        ></div>
        <img
          class="timeline__preview-load-sentinel"
          src={previewUrl}
          alt=""
          aria-hidden="true"
          onerror={() => handleActivePreviewLoadError(timelineActive.id)}
        />
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
                <span class="timeline__ocr-text">{obs.text}</span>
              </div>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="timeline__preview-pending">
          {frameActionStatus?.tone === "error" ? "preview unavailable" : "decoding preview…"}
        </div>
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
        {#if timelineActive.equivalenceHint}
          <div class="timeline__overlay-row">
            <span class="timeline__overlay-key">fp</span>
            <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.equivalenceHint}</span>
          </div>
        {/if}
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">preview</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">
            reuse {previewCacheReuseCount} reqHit {previewCacheHitCount} miss {previewCacheMissCount} fail {previewFailureCacheHitCount} join {previewInFlightJoinCount}
          </span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">previewSrc</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">
            direct {previewDirectPathCount} generated {previewGeneratedPathCount} retry {previewStaleRetryCount}
          </span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">prefetch</span>
          <span class="timeline__overlay-val">
            r{currentPreviewPrefetchRadius()} @ {previewScrubVelocityPxPerMs.toFixed(2)}px/ms
          </span>
        </div>
        {#if timelineActiveDuplicateOf}
          {@const duplicateOf = timelineActiveDuplicateOf}
          <div class="timeline__overlay-row">
            <span class="timeline__overlay-key">duplicateOf</span>
            <button
              type="button"
              class="timeline__overlay-link"
              onclick={() => void jumpToFrame(duplicateOf)}
            >{duplicateOf.id}</button>
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
    <section class="audio-drawer__transcript" aria-label="Audio transcription">
      <div class="audio-drawer__transcript-header">
        <div class="audio-drawer__transcript-heading">
          <span class="audio-drawer__transcript-title">Transcript</span>
          {#if selectedAudioTranscriptModelLabel}
            <span class="audio-drawer__transcript-model" title={selectedAudioTranscriptModelLabel}>
              · {selectedAudioTranscriptModelLabel}
            </span>
          {/if}
        </div>
        <div class="audio-drawer__transcript-actions">
          <button
            type="button"
            class="audio-drawer__transcript-action"
            onclick={reprocessSelectedAudioSegmentTranscript}
            disabled={selectedAudioTranscriptActionDisabled}
            title={selectedAudioTranscriptActionTitle}
          >
            {selectedAudioTranscriptRerunLoading
              ? "Starting…"
              : selectedAudioTranscriptActionLabel}
          </button>
          <span class="audio-drawer__transcript-state audio-drawer__transcript-state--{selectedAudioTranscriptStatus}">
            {#if selectedAudioTranscriptStatus === "loading"}
              loading
            {:else if selectedAudioTranscriptStatus === "running"}
              processing
            {:else if selectedAudioTranscriptStatus === "success"}
              completed
            {:else if selectedAudioTranscriptStatus === "empty"}
              no speech
            {:else if selectedAudioTranscriptStatus === "error"}
              error
            {:else}
              unavailable
            {/if}
          </span>
        </div>
      </div>
      {#if selectedAudioTranscriptRerunError}
        <p class="audio-drawer__transcript-error">{selectedAudioTranscriptRerunError}</p>
      {/if}
      {#if selectedAudioTranscriptStatus === "success"}
        {#if selectedAudioTranscriptSegments.length > 0}
          <div
            class="audio-drawer__transcript-text audio-drawer__transcript-text--segmented"
            bind:this={selectedAudioTranscriptContainerEl}
          >
            {#each selectedAudioTranscriptSegments as segment, index}
              <button
                type="button"
                class="audio-drawer__transcript-segment"
                class:audio-drawer__transcript-segment--active={selectedAudioTranscriptActiveSegmentIndex === index}
                data-transcript-segment-index={index}
                title={`Jump to ${formatTranscriptSegmentTitle(segment)}`}
                aria-label={`Jump to transcript segment at ${formatTranscriptSegmentTitle(segment)}`}
                onclick={() => seekAudioToTimeMs(segment.startMs)}
              >
                {segment.text}
              </button>
            {/each}
          </div>
        {:else}
          <p class="audio-drawer__transcript-text">{selectedAudioTranscriptText}</p>
        {/if}
      {:else if selectedAudioTranscriptStatus === "empty"}
        <p class="audio-drawer__transcript-empty">No speech detected in this segment.</p>
      {:else if selectedAudioTranscriptStatus === "loading"}
        <p class="audio-drawer__transcript-empty">Loading transcript…</p>
      {:else if selectedAudioTranscriptStatus === "running"}
        <p class="audio-drawer__transcript-empty">Transcription is queued or still processing.</p>
      {:else if selectedAudioTranscriptStatus === "error"}
        <p class="audio-drawer__transcript-error">{selectedAudioTranscriptError}</p>
      {:else}
        <p class="audio-drawer__transcript-empty">No transcript has been recorded for this segment.</p>
      {/if}
    </section>
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
    background: var(--app-bg);
    /* Allow the stage child (flex: 1, min-height: 0) to actually shrink so
       the bottom rail stays in view regardless of preview intrinsic size. */
    min-height: 0;
    overflow: hidden;
  }

  .timeline__bar,
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

  /* ── Recording control cluster ─────────────────────────────
     Recording status + start/stop now live in the app-wide title bar
     (see `routes/+layout.svelte`); the previous `.timeline__capture*`
     styles moved alongside as `.titlebar__status*` / `.titlebar__record*`. */

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
    background: linear-gradient(180deg, var(--app-surface-raised) 0%, var(--app-surface) 100%);
    border: 1px solid var(--app-border-strong);
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
    background: var(--app-border-strong);
    margin-bottom: 2px;
  }

  .audio-drawer__meta {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text);
    font-variant-numeric: tabular-nums;
    min-width: 0;
  }

  .audio-drawer__source {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    color: var(--app-text-strong);
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
    color: var(--app-danger);
    font-weight: 700;
  }

  .audio-drawer__time {
    color: var(--app-text-muted);
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .audio-drawer__time-sep {
    color: var(--app-text-subtle);
  }

  .audio-drawer__duration {
    color: var(--app-text-muted);
  }

  .audio-drawer__file {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--app-text-muted);
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
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    width: 24px;
    height: 24px;
    color: var(--app-text-muted);
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
    color: var(--app-danger);
    border-color: var(--app-danger-strong);
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
    background: color-mix(in srgb, var(--app-danger-strong) 10%, transparent);
    border: 1px solid var(--app-danger-border);
    border-radius: 50%;
    color: var(--app-danger);
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s,
      transform 0.08s;
  }

  .audio-drawer__play:hover {
    background: color-mix(in srgb, var(--app-danger-strong) 18%, transparent);
    border-color: var(--app-danger-strong);
  }

  .audio-drawer__play:focus-visible {
    outline: none;
    border-color: var(--app-danger-strong);
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
    color: var(--app-text-muted);
    min-width: 36px;
  }

  .audio-drawer__time-readout--current {
    color: var(--app-text-strong);
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
    color: var(--app-danger);
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
      var(--app-danger-strong) 0%,
      var(--app-danger-strong) var(--audio-progress, 0%),
      var(--app-surface-hover) var(--audio-progress, 0%),
      var(--app-surface-hover) 100%
    );
  }

  .audio-drawer__scrub::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: var(--app-surface-hover);
  }

  .audio-drawer__scrub::-moz-range-progress {
    height: 4px;
    border-radius: 2px;
    background: var(--app-danger-strong);
  }

  .audio-drawer__scrub::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-danger);
    border: 2px solid var(--app-surface-raised);
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
    background: var(--app-danger);
    border: 2px solid var(--app-surface-raised);
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
    color: var(--app-text-muted);
  }

  .audio-drawer__status-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1px solid var(--app-border-strong);
    border-radius: 50%;
    color: var(--app-danger);
    font-size: 9px;
  }

  .audio-drawer__error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 10px;
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    font-size: 11px;
    color: var(--app-danger-text);
  }

  .audio-drawer__error-label {
    flex: 0 0 auto;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-danger);
    padding-top: 1px;
  }

  .audio-drawer__error-msg {
    flex: 1 1 auto;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    word-break: break-word;
    line-height: 1.4;
  }

  .audio-drawer__transcript {
    display: grid;
    gap: 6px;
    margin-top: 2px;
    padding: 8px 10px;
    background: color-mix(in srgb, var(--app-surface-hover) 58%, transparent);
    border: 1px solid var(--app-border);
    border-radius: 6px;
  }

  .audio-drawer__transcript-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .audio-drawer__transcript-heading {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    flex-wrap: wrap;
  }

  .audio-drawer__transcript-title,
  .audio-drawer__transcript-state {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }

  .audio-drawer__transcript-title {
    color: var(--app-text-muted);
  }

  .audio-drawer__transcript-model {
    color: var(--app-text);
    font-size: 10px;
    font-weight: 600;
    line-height: 1.35;
    letter-spacing: 0.02em;
    word-break: break-word;
    opacity: 0.9;
  }

  .audio-drawer__transcript-actions {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }

  .audio-drawer__transcript-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 52px;
    padding: 4px 8px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    background: color-mix(in srgb, var(--app-surface-raised) 72%, transparent);
    color: var(--app-text-muted);
    font: inherit;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      border-color 0.12s,
      color 0.12s,
      background 0.12s,
      opacity 0.12s;
  }

  .audio-drawer__transcript-action:hover:not(:disabled),
  .audio-drawer__transcript-action:focus-visible:not(:disabled) {
    border-color: var(--app-accent);
    background: color-mix(in srgb, var(--app-accent) 12%, var(--app-surface-raised));
    color: var(--app-text);
    outline: none;
  }

  .audio-drawer__transcript-action:disabled {
    opacity: 0.38;
    cursor: not-allowed;
  }

  .audio-drawer__transcript-state {
    color: var(--app-text-faint);
  }

  .audio-drawer__transcript-state--success,
  .audio-drawer__transcript-state--empty {
    color: var(--app-accent);
  }

  .audio-drawer__transcript-state--running,
  .audio-drawer__transcript-state--loading {
    color: var(--app-warn);
  }

  .audio-drawer__transcript-state--error {
    color: var(--app-danger);
  }

  .audio-drawer__transcript-text,
  .audio-drawer__transcript-empty,
  .audio-drawer__transcript-error {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
  }

  .audio-drawer__transcript-text {
    max-height: 7.5em;
    overflow: auto;
    color: var(--app-text);
    white-space: pre-wrap;
  }

  .audio-drawer__transcript-text--segmented {
    white-space: normal;
  }

  .audio-drawer__transcript-segment {
    display: inline;
    margin-right: 0.28em;
    padding: 1px 3px;
    border: 0;
    border-radius: 4px;
    color: inherit;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
    box-decoration-break: clone;
    -webkit-box-decoration-break: clone;
    scroll-margin-block: 10px;
    transition:
      background 0.12s,
      color 0.12s;
  }

  .audio-drawer__transcript-segment:hover,
  .audio-drawer__transcript-segment:focus-visible {
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    color: var(--app-text);
    outline: none;
  }

  .audio-drawer__transcript-segment--active {
    background: color-mix(in srgb, var(--app-accent) 18%, transparent);
    color: var(--app-text);
  }

  .audio-drawer__transcript-empty {
    color: var(--app-text-muted);
    font-style: italic;
  }

  .audio-drawer__transcript-error {
    color: var(--app-danger-text);
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    word-break: break-word;
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
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  /* The previous dashboard-local settings/menu anchor moved into the shared
     title bar as reusable surface actions, so those local rules were removed. */

  /* ── Date jump picker ──────────────────────────────────────── */
  .timeline__jump {
    display: flex;
    align-items: center;
    gap: 6px;
    position: relative;
  }

  .timeline__jump-trigger {
    gap: 6px;
    font-variant-numeric: tabular-nums;
    max-width: 220px;
  }

  .timeline__jump-latest {
    flex: 0 0 auto;
  }

  .timeline__jump-icon {
    font-size: 10px;
    color: var(--app-text-muted);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border);
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
    color: var(--app-text-subtle);
  }

  .timeline__picker-val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 11px;
    color: var(--app-text);
  }

  .timeline__picker-pending {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .timeline__picker-error {
    font-size: 10px;
    color: var(--app-danger-text);
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
    border: 1px solid var(--app-border);
    border-radius: 3px;
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .timeline__picker-time:hover:not(:disabled) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  .timeline__picker-time:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .timeline__picker-time--active {
    color: var(--app-danger-strong);
    border-color: color-mix(in srgb, var(--app-danger-strong) 40%, transparent);
    background: color-mix(in srgb, var(--app-danger-strong) 8%, transparent);
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
    color: var(--app-text);
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
    border: 1px solid var(--app-border);
    border-radius: 3px;
    color: var(--app-text-muted);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }

  :global(.cal__nav:hover) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  :global(.cal__heading) {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text);
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
    color: var(--app-text-subtle);
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
    color: var(--app-text);
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  :global(.cal__day:hover:not([data-disabled])) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }

  :global(.cal__day[data-disabled]),
  :global(.cal__day[data-outside-month]) {
    color: var(--app-text-faint);
    cursor: not-allowed;
  }

  :global(.cal__day[data-selected]) {
    background: rgba(255, 68, 85, 0.12);
    border-color: rgba(255, 68, 85, 0.5);
    color: var(--app-danger-text);
  }

  :global(.cal__day[data-today]:not([data-selected])) {
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
  }

  /* ── Error / empty ─────────────────────────────────────────── */
  .timeline__error {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    font-size: 11px;
    color: var(--app-danger-text);
  }

  .timeline__error-label {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-danger);
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
    color: var(--app-text-subtle);
    font-size: 11px;
    letter-spacing: 0.06em;
  }

  .timeline__empty-hint {
    font-size: 10px;
    color: var(--app-text-faint);
  }

  /* ── Stage (preview dominates) ─────────────────────────────── */
  .timeline__stage {
    position: relative;
    flex: 1 1 0;
    min-height: 0; /* allow the flex child to actually shrink as needed */
    background: linear-gradient(135deg, var(--app-surface-raised) 0%, var(--app-surface) 100%);
    border: 1px solid var(--app-border);
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

  .timeline__preview-load-sentinel {
    position: absolute;
    width: 0;
    height: 0;
    opacity: 0;
    pointer-events: none;
  }

  .timeline__stage-actions {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 2;
  }

  .timeline__stage-actions[open] {
    z-index: 3;
  }

  .timeline__stage-action-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    min-height: 28px;
    padding: 0;
    background: color-mix(in srgb, var(--app-surface-raised) 82%, transparent);
    border: 1px solid color-mix(in srgb, var(--app-border-strong) 88%, transparent);
    border-radius: 999px;
    box-shadow:
      0 8px 20px rgba(0, 0, 0, 0.22),
      inset 0 1px 0 rgba(255, 255, 255, 0.04);
    color: var(--app-text-muted);
    list-style: none;
    font-size: 18px;
    font-weight: 700;
    line-height: 1;
    letter-spacing: 0;
    user-select: none;
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s,
      box-shadow 0.12s,
      transform 0.12s;
  }

  .timeline__stage-action-trigger::-webkit-details-marker {
    display: none;
  }

  .timeline__stage-action-trigger:hover {
    background: color-mix(in srgb, var(--app-surface-hover) 88%, transparent);
    border-color: var(--app-border-hover);
    color: var(--app-text);
    box-shadow:
      0 10px 24px rgba(0, 0, 0, 0.26),
      inset 0 1px 0 rgba(255, 255, 255, 0.06);
  }

  .timeline__stage-action-trigger:focus-visible {
    outline: none;
    border-color: var(--app-border-hover);
    color: var(--app-text);
    box-shadow:
      0 0 0 2px color-mix(in srgb, var(--app-border-hover) 48%, transparent),
      0 10px 24px rgba(0, 0, 0, 0.26);
  }

  .timeline__stage-actions[open] > .timeline__stage-action-trigger {
    background: color-mix(in srgb, var(--app-surface-hover) 92%, transparent);
    border-color: var(--app-border-hover);
    color: var(--app-text);
    transform: translateY(1px);
  }

  .timeline__stage-action-menu {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    display: grid;
    min-width: 112px;
    gap: 2px;
    padding: 6px;
    background: color-mix(in srgb, var(--app-surface) 94%, transparent);
    border: 1px solid var(--app-border);
    border-radius: 10px;
    box-shadow:
      0 18px 40px rgba(0, 0, 0, 0.28),
      inset 0 1px 0 rgba(255, 255, 255, 0.04);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
  }

  .timeline__stage-action-menu-item {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    width: 100%;
    padding: 8px 10px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    cursor: pointer;
  }

  .timeline__stage-action-menu-item:hover {
    background: var(--app-surface-hover);
    border-color: color-mix(in srgb, var(--app-border-hover) 70%, transparent);
    color: var(--app-text);
  }

  .timeline__stage-action-menu-item:focus-visible {
    outline: none;
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--app-border-hover) 32%, transparent);
  }

  .timeline__stage-status {
    position: absolute;
    right: 10px;
    bottom: 10px;
    z-index: 2;
    max-width: min(60%, 360px);
    padding: 6px 8px;
    display: grid;
    gap: 4px;
    background: var(--app-overlay-bg-strong);
    border: 1px solid var(--app-overlay-border);
    border-radius: 8px;
    color: var(--app-text);
    font-size: 10px;
    line-height: 1.35;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    box-shadow: 0 10px 24px color-mix(in srgb, var(--app-bg) 28%, transparent);
  }

  .timeline__stage-status-summary {
    font-weight: 700;
  }

  .timeline__stage-status-detail {
    max-height: 8.2em;
    overflow: auto;
    padding-top: 3px;
    border-top: 1px solid color-mix(in srgb, currentColor 16%, transparent);
    color: var(--app-text-subtle);
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 9px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .timeline__stage-status--error {
    color: color-mix(in srgb, var(--app-danger) 72%, var(--app-text) 28%);
    border-color: color-mix(in srgb, var(--app-danger) 40%, var(--app-overlay-border));
  }

  .timeline__preview-pending {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-faint);
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
    background: var(--app-overlay-bg);
    border: 1px solid var(--app-overlay-border);
    border-radius: 4px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
  }

  .timeline__overlay-row {
    display: contents;
  }

  .timeline__overlay-key {
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    align-self: center;
  }

  .timeline__overlay-val {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-text);
    min-width: 0;
  }

  .timeline__overlay-link {
    font-family: "SF Mono", "Fira Mono", "Courier New", monospace;
    font-size: 10px;
    color: var(--app-info);
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .timeline__overlay-link:hover {
    color: var(--app-info-strong);
    text-decoration: underline;
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
    color: var(--app-text-muted);
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
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }

  .timeline__ocr-provider-chip {
    display: inline-flex;
    align-items: center;
    min-height: 24px;
    max-width: 240px;
    padding: 0 8px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font-size: 10px;
    letter-spacing: 0.04em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .timeline__ocr-rerun-btn {
    color: var(--app-text-muted);
  }

  .timeline__ocr-rerun-btn:not(:disabled):hover {
    color: var(--app-text);
  }

  .timeline__ocr-btn--running {
    color: var(--app-warn);
    border-color: var(--app-warn-border);
    background: rgba(214, 161, 74, 0.06);
  }
  .timeline__ocr-btn--running .timeline__ocr-glyph {
    color: var(--app-warn);
    animation: timeline-ocr-pulse 1.2s ease-in-out infinite;
  }

  .timeline__ocr-btn--success {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
  }
  .timeline__ocr-btn--success .timeline__ocr-glyph {
    color: var(--app-accent);
  }

  .timeline__ocr-btn--error {
    color: var(--app-danger-text);
    border-color: var(--app-danger-border);
  }
  .timeline__ocr-btn--error .timeline__ocr-glyph {
    color: var(--app-danger-text);
  }

  @keyframes timeline-ocr-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  /* Overlay wrapper sized & positioned to match the actual rendered image
     rect (measured from the DOM each layout). `overflow: hidden` clips any
     OCR box whose normalized bounds slightly extend past the image edges
     so visuals never spill into the surrounding stage letterbox area.
     Keep the wrapper non-interactive, then opt labels back into pointer
     events so OCR text can be selected/copied without turning the whole stage
     into an overlay hit-target. */
  .timeline__ocr-overlay {
    position: absolute;
    overflow: hidden;
    pointer-events: none;
  }

  .timeline__ocr-box {
    position: absolute;
    border: 1px solid var(--app-ocr-box);
    background: transparent;
    border-radius: 2px;
    /* Allow zero-width/height edge cases to remain visible as a hairline. */
    min-width: 1px;
    min-height: 1px;
    pointer-events: auto;
    cursor: text;
    transition:
      border-color 120ms ease,
      background 120ms ease;
  }

  .timeline__ocr-box:hover,
  .timeline__ocr-box:focus-within {
    border-color: var(--app-ocr-box-hover);
    background: var(--app-ocr-box-fill);
    box-shadow:
      0 0 0 1px var(--app-ocr-hover-shadow),
      inset 0 0 0 1px var(--app-ocr-hover-inset);
    /* Lift the hovered box above its neighbours so the revealed chip can
       extend past the bbox without being clipped by adjacent siblings. */
    z-index: 2;
  }

  /* Text is hidden by default — boxes alone act as a quiet visual scan
     layer over the original pixels. On hover/focus a single chip is
     revealed; it sits flush with the bbox and is fully opaque so it
     replaces (rather than doubles) the underlying glyphs.

     `--ocr-font-size` is set inline per-observation in ocrBoxStyle so
     the chip's text height matches the recognised glyph row. */
  .timeline__ocr-text {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    padding: 0 4px;
    background: var(--app-ocr-chip-bg);
    color: var(--app-ocr-chip-text);
    text-shadow: var(--app-ocr-chip-text-shadow);
    font-family:
      ui-monospace,
      SFMono-Regular,
      Menlo,
      monospace;
    font-size: var(--ocr-font-size, 11px);
    line-height: 1;
    letter-spacing: -0.01em;
    white-space: nowrap;
    /* Let long text spill out of the bbox horizontally so it stays
       readable; the parent overlay still clips at the image edge. */
    width: max-content;
    min-width: 100%;
    max-width: none;
    border: 1px solid var(--app-ocr-chip-border);
    border-radius: 2px;
    pointer-events: none;
    user-select: text;
    opacity: 0;
    transition: opacity 80ms ease;
  }

  .timeline__ocr-box:hover .timeline__ocr-text,
  .timeline__ocr-box:focus-within .timeline__ocr-text {
    opacity: 1;
    pointer-events: auto;
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
    background: var(--app-overlay-bg-strong);
    border: 1px solid var(--app-overlay-border);
    border-radius: 4px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text);
    max-width: calc(100% - 20px);
  }

  .timeline__ocr-status--running {
    color: var(--app-warn);
    border-color: var(--app-warn-border);
  }

  .timeline__ocr-status--empty {
    color: var(--app-text-muted);
  }

  .timeline__ocr-status--missing {
    color: var(--app-text-muted);
  }

  .timeline__ocr-status--error {
    color: var(--app-danger-text);
    border-color: var(--app-danger-border);
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
    border: 1.5px solid color-mix(in srgb, var(--app-warn) 30%, transparent);
    border-top-color: var(--app-warn);
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
    background: var(--app-surface);
    border: 1px solid var(--app-border);
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
    border-color: var(--app-danger-strong);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--app-danger-strong) 35%, transparent);
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
      color-mix(in srgb, var(--app-bg) 0%, transparent) 0%,
      color-mix(in srgb, var(--app-bg) 55%, transparent) 30%,
      color-mix(in srgb, var(--app-bg) 55%, transparent) 70%,
      color-mix(in srgb, var(--app-bg) 0%, transparent) 100%
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
    background: color-mix(in srgb, var(--app-surface-raised) 72%, transparent);
    box-shadow: inset 0 0 0 1px var(--app-border);
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
    color: var(--app-text-faint);
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
      0 0 0 1.5px var(--app-danger-strong),
      0 0 8px color-mix(in srgb, var(--app-danger-strong) 45%, transparent);
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
    background: var(--app-warn);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--app-warn) 35%, transparent);
  }

  .timeline-rail__tick {
    display: block;
    width: 1px;
    height: 8px;
    background: var(--app-border-strong);
    border-radius: 0.5px;
    transition: height 0.12s ease-out, background 0.12s;
  }

  .timeline-rail__slot--major .timeline-rail__tick {
    height: 14px;
    background: var(--app-text-subtle);
  }

  .timeline-rail__slot:hover .timeline-rail__tick {
    background: var(--app-text-muted);
    height: 12px;
  }

  .timeline-rail__slot--active .timeline-rail__tick,
  .timeline-rail__slot--active.timeline-rail__slot--major .timeline-rail__tick {
    width: 2px;
    height: 22px;
    background: var(--app-danger-strong);
    box-shadow: 0 0 6px color-mix(in srgb, var(--app-danger-strong) 70%, transparent);
  }

  /* Static center indicator — the rail scrolls beneath it, so the active
     frame is always whichever tick is centered under this caret. */
  .timeline-rail__cursor {
    position: absolute;
    top: -1px;
    bottom: -1px;
    left: 50%;
    width: 1px;
    background: color-mix(in srgb, var(--app-danger-strong) 35%, transparent);
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
    border-top: 4px solid var(--app-danger-strong);
  }

  .timeline-rail__cursor::after {
    bottom: -1px;
    border-bottom: 4px solid var(--app-danger-strong);
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
    color: var(--app-text-subtle);
    background: color-mix(in srgb, var(--app-surface-raised) 90%, transparent);
    border: 1px solid var(--app-border);
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
    color: var(--app-text-strong);
    background: color-mix(in srgb, var(--app-surface-raised) 96%, transparent);
    border: 1px solid var(--app-border-strong);
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
    border-top: 4px solid color-mix(in srgb, var(--app-surface-raised) 96%, transparent);
  }

  .timeline-rail__tooltip--pinned {
    /* Pinned-to-active variant: tinted to match the active caret accent so
       it's clear the readout corresponds to the frame under the center cursor. */
    border-color: rgba(255, 68, 85, 0.5);
    color: var(--app-danger-text);
  }

  .timeline-rail__tooltip--pinned::after {
    border-top-color: color-mix(in srgb, var(--app-surface-raised) 96%, transparent);
  }

  /* ── Light theme overrides ──────────────────────────────────
     The dark palette above is the source of truth; this block flips the
     dashboard's major surfaces, borders, and text colors when
     `[data-theme="light"]` is active on the document root (driven by
     `$lib/theme.svelte`). Kept narrow on purpose: the intent is to
     re-tint surfaces and copy without restructuring layout, so any new
     dark-only rule above will simply inherit a sensible light variant
     here through the semantic-token cascade in `+layout.svelte`. */
  :global([data-theme="light"]) .timeline {
    background: var(--app-bg);
  }

  :global([data-theme="light"]) .audio-drawer {
    background: linear-gradient(180deg, var(--app-surface) 0%, var(--app-surface-raised) 100%);
    border-color: var(--app-border);
    box-shadow:
      0 18px 40px rgba(20, 28, 40, 0.12),
      0 2px 0 rgba(255, 255, 255, 0.6) inset;
  }
  :global([data-theme="light"]) .audio-drawer__handle {
    background: var(--app-border-strong);
  }
  :global([data-theme="light"]) .audio-drawer__source {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .audio-drawer__index,
  :global([data-theme="light"]) .audio-drawer__time,
  :global([data-theme="light"]) .audio-drawer__duration,
  :global([data-theme="light"]) .audio-drawer__file {
    color: var(--app-text);
  }
  :global([data-theme="light"]) .audio-drawer__time-sep {
    color: var(--app-text-faint);
  }
  :global([data-theme="light"]) .audio-drawer__close {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .audio-drawer__close:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }
  :global([data-theme="light"]) .audio-drawer__player,
  :global([data-theme="light"]) .audio-drawer__scrub {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .audio-drawer__time-readout {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .audio-drawer__time-readout--current {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .audio-drawer__status {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .audio-drawer__error-msg {
    color: var(--app-danger);
  }

  :global([data-theme="light"]) .btn {
    background: var(--app-surface);
    color: var(--app-text);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .btn:not(:disabled):hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .btn--ghost:not(:disabled):hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }

  :global([data-theme="light"]) .timeline__jump-trigger {
    background: var(--app-surface);
    color: var(--app-text);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .timeline__jump-trigger:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .timeline__jump-icon {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline__jump-label {
    color: var(--app-text);
  }

  :global([data-theme="light"]) .timeline__picker {
    background: var(--app-surface);
    border-color: var(--app-border);
    box-shadow: 0 12px 28px rgba(20, 28, 40, 0.12);
  }
  :global([data-theme="light"]) .timeline__picker-key {
    color: var(--app-text-subtle);
  }
  :global([data-theme="light"]) .timeline__picker-val {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .timeline__picker-pending {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline__picker-time {
    color: var(--app-text);
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline__picker-time:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .timeline__picker-time--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  :global([data-theme="light"]) .timeline__stage-action-trigger {
    background: color-mix(in srgb, var(--app-surface) 90%, transparent);
    border-color: color-mix(in srgb, var(--app-border-strong) 92%, transparent);
    box-shadow:
      0 10px 24px rgba(20, 28, 40, 0.14),
      inset 0 1px 0 rgba(255, 255, 255, 0.72);
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .timeline__stage-action-trigger:hover,
  :global([data-theme="light"]) .timeline__stage-action-trigger:focus-visible,
  :global([data-theme="light"]) .timeline__stage-actions[open] > .timeline__stage-action-trigger {
    background: color-mix(in srgb, var(--app-surface-hover) 94%, transparent);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    box-shadow:
      0 12px 28px rgba(20, 28, 40, 0.16),
      inset 0 1px 0 rgba(255, 255, 255, 0.84);
  }

  :global([data-theme="light"]) .timeline__stage-action-menu {
    background: color-mix(in srgb, var(--app-surface) 96%, white 4%);
    border-color: var(--app-border);
    box-shadow:
      0 18px 36px rgba(20, 28, 40, 0.14),
      inset 0 1px 0 rgba(255, 255, 255, 0.86);
  }

  :global([data-theme="light"]) .timeline__error {
    background: var(--app-danger-bg-soft);
    border-color: var(--app-danger-border);
    color: var(--app-danger);
  }
  :global([data-theme="light"]) .timeline__error-label {
    color: var(--app-danger);
  }
  :global([data-theme="light"]) .timeline__error-msg {
    color: var(--app-danger-text);
  }

  :global([data-theme="light"]) .timeline__empty {
    color: var(--app-text-muted);
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline__empty-hint {
    color: var(--app-text-subtle);
  }

  :global([data-theme="light"]) .timeline__stage {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline__preview-pending {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline__stage-status {
    color: var(--app-text-muted);
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }

  :global([data-theme="light"]) .timeline__overlay {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline__overlay-key {
    color: var(--app-text-subtle);
  }
  :global([data-theme="light"]) .timeline__overlay-val {
    color: var(--app-text);
  }
  :global([data-theme="light"]) .timeline__overlay-link {
    color: var(--app-accent-strong);
  }
  :global([data-theme="light"]) .timeline__overlay-link:hover {
    color: var(--app-accent);
  }

  :global([data-theme="light"]) .timeline__ocr-btn {
    background: var(--app-surface);
    color: var(--app-text);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .timeline__ocr-btn:hover {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
  }
  :global([data-theme="light"]) .timeline__ocr-glyph {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline__ocr-count {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .timeline__ocr-text {
    color: var(--app-text);
  }
  :global([data-theme="light"]) .timeline__ocr-status {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline__ocr-status-msg {
    color: var(--app-text-muted);
  }

  :global([data-theme="light"]) .timeline__rail-wrap {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail {
    background: var(--app-surface);
  }
  :global([data-theme="light"]) .timeline-rail__track {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail__slot {
    background: var(--app-surface);
    border-color: var(--app-border-strong);
  }
  :global([data-theme="light"]) .timeline-rail__audio-lane-wrap {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail__audio-lane-labels {
    color: var(--app-text-subtle);
  }
  :global([data-theme="light"]) .timeline-rail__audio-lane-empty {
    color: var(--app-text-faint);
  }
  :global([data-theme="light"]) .timeline-rail--placeholder {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail__loading {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip {
    background: rgba(255, 255, 255, 0.96);
    border-color: var(--app-border);
    color: var(--app-text-strong);
    box-shadow: 0 6px 16px rgba(20, 28, 40, 0.16);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip::after {
    border-top-color: rgba(255, 255, 255, 0.96);
  }
</style>
