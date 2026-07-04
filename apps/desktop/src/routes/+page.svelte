<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { tick } from "svelte";
  import { fly } from "svelte/transition";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { Image } from "@tauri-apps/api/image";
  import { writeImage, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { BaseDirectory, writeFile } from "@tauri-apps/plugin-fs";
  import {
    bootstrapCaptureControls,
    captureControls,
    resyncCaptureSession,
  } from "$lib/capture-controls.svelte";
  import { developerOptions } from "$lib/developer-options.svelte";
  import ActionSelect from "$lib/components/ActionSelect.svelte";
  import TimelineJumper from "$lib/timeline/TimelineJumper.svelte";
  import { takePendingTimelineFocus } from "$lib/timeline/pending-focus";
  import { parseCapturedAt, formatTimestampCompact } from "$lib/format-time";
  import { humanizeError } from "$lib/format-error";
  import { framePreviewAssetUrl, readFramePreviewBytes } from "$lib/frame-preview";
  import {
    loadOcrForFrame,
    loadOcrFromJob,
    ocrBoxStyle,
    type OcrLoadResult,
    type OcrStatus,
  } from "$lib/frame-ocr";
  import { openCapturedUrl } from "$lib/open-captured-url";
  import IconScanText from "~icons/lucide/scan-text";
  import IconMoreHorizontal from "~icons/lucide/ellipsis";
  import IconClapperboard from "~icons/lucide/clapperboard";
  import IconHeadphones from "~icons/lucide/headphones";
  import {
    activeExactPreviewDelayMs,
    scrubPreviewResponseShouldApply,
    timelineMovementShouldScheduleScrubPreview,
  } from "$lib/timeline-preview-state";
  import {
    detectKeyboardPlatform,
    getFocusableElements,
    isShortcutSuppressedTarget,
    matchShortcut,
    trapTabKey,
    type KeyboardPlatform,
    type ShortcutDefinition,
  } from "$lib/keyboard";
  import {
    setKeyboardHelpGroups,
    type KeyboardHelpGroup,
  } from "$lib/keyboard-help.svelte";
  import {
    getShortcutBinding,
    keyboardBindings,
    shortcutDefinitionWithBinding,
    type EditableShortcutActionId,
  } from "$lib/keyboard-bindings.svelte";
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
    FrameScrubPreviewsDto,
    GetFramePreviewRequest,
    GetAudioSegmentRequest,
    GetEarliestEarlierEquivalentFrameRequest,
    GetScrubPreviewAvailabilityRequest,
    GetProcessingJobRequest,
    GetProcessingResultRequest,
    GetTimelineWindowAroundFrameRequest,
    ListAudioSegmentsRequest,
    ListFramesRequest,
    OcrObservation,
    ProcessingJobDto,
    ProcessingResultDto,
    ReprocessAudioSegmentSpeakerAnalysisRequest,
    ReprocessAudioSegmentTranscriptionRequest,
    ReprocessCapturedFrameOcrRequest,
    PersonProfileDto,
    SpeakerAnalysisSkipReason,
    ScrubPreviewAvailabilityDto,
    ScrubPreviewAvailabilityIntervalDto,
    FramePreviewVideoScope,
    SpeakerAnalysisStructuredPayload,
    SystemAudioSpeechActivityReprocessingResultDto,
    SpeakerClusterDto,
    SpeakerTurnDto,
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
  // At the default 0.5 fps capture rate frames land ~2s apart, so this must
  // comfortably exceed one frame interval or a single skipped frame (privacy
  // exclusion, self-capture skip) splits an app run in two.
  const TIMELINE_APP_GROUP_MAX_GAP_MS = 10_000;
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
  const ACTIVE_PREVIEW_SCRUB_REQUEST_INTERVAL_MS = 160;
  const ACTIVE_PREVIEW_SCRUB_FAST_REQUEST_INTERVAL_MS = 240;
  const ACTIVE_PREVIEW_DISPLAY_SCRUB_THROTTLE_MS = 80;
  const ACTIVE_PREVIEW_DISPLAY_SETTLE_MS = 180;
  const ACTIVE_PREVIEW_SCRUB_WARM_SETTLE_MS = 1200;
  const ACTIVE_PREVIEW_EXACT_SETTLE_MS = 500;
  const ACTIVE_PREVIEW_SCRUB_RADIUS = 4;
  const ACTIVE_PREVIEW_SCRUB_MEDIUM_RADIUS = 3;
  const ACTIVE_PREVIEW_SCRUB_FAST_RADIUS = 2;
  const ACTIVE_PREVIEW_SCRUB_VERY_FAST_RADIUS = 1;
  const ACTIVE_PREVIEW_SCRUB_MAX_UNCACHED = 9;
  const ACTIVE_PREVIEW_SCRUB_MEDIUM_MAX_UNCACHED = 7;
  const ACTIVE_PREVIEW_SCRUB_FAST_MAX_UNCACHED = 5;
  const ACTIVE_PREVIEW_SCRUB_VERY_FAST_MAX_UNCACHED = 3;
  const ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS = 2;
  const ACTIVE_PREVIEW_VERY_FAST_SCRUB_PX_PER_MS = 4;
  const ACTIVE_PREVIEW_EXTREME_SCRUB_PX_PER_MS = 8;
  const ACTIVE_PREVIEW_DEFERRED_LOG_MIN_DELTA = 40;
  const PREVIEW_CACHE_MAX_ENTRIES = 192;
  const PREVIEW_FAILURE_CACHE_TTL_MS = 5_000;
  const PREVIEW_GENERATION_CANCELLED_MESSAGE = "preview generation cancelled";
  const SCRUB_PERF_LOG_PREFIX = "[DEBUG-scrub-perf]";
  const SCRUB_PERF_SCROLL_WINDOW_MS = 500;
  const SCRUB_PERF_SLOW_DERIVED_MS = 4;
  const SCRUB_PERF_SLOW_PREVIEW_MS = 25;
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
  const TIMELINE_RENDER_REANCHOR_DISTANCE = Math.max(
    1,
    Math.floor(TIMELINE_VIEWPORT_BUFFER / 2),
  );
  // Conservative fallback viewport width (px) used only on the very first
  // render before ResizeObserver measures the rail. Picked to comfortably
  // cover wide displays (~2560px) so the centered window includes every
  // visible slot from frame one. Once the observer fires, the real measured
  // width takes over. Bounded fallback keeps virtualization in effect — the
  // window is still half-of-this plus the fixed overscan buffer, never the
  // full frame list.
  const TIMELINE_FALLBACK_VIEWPORT_WIDTH = 2560;

  let scrubPerfScrollWindowStartedAt = 0;
  let scrubPerfScrollEventCount = 0;
  let scrubPerfScrollActiveChangeCount = 0;
  let scrubPerfScrollTotalMs = 0;
  let scrubPerfScrollMaxMs = 0;

  function scrubPerfEnabled(): boolean {
    return import.meta.env.DEV && developerOptions.value;
  }

  function scrubPerfField(key: string, value: string | number | boolean | null | undefined): string {
    if (typeof value === "number") {
      return `${key}=${Number.isFinite(value) ? value.toFixed(value % 1 === 0 ? 0 : 2) : "nan"}`;
    }
    return `${key}=${value ?? "null"}`;
  }

  function scrubPerfLog(event: string, fields: Record<string, string | number | boolean | null | undefined>): void {
    if (!scrubPerfEnabled()) return;
    console.debug(
      `${SCRUB_PERF_LOG_PREFIX} event=${event} ${Object.entries(fields)
        .map(([key, value]) => scrubPerfField(key, value))
        .join(" ")}`,
    );
  }

  function scrubPerfLogSlow(
    event: string,
    durationMs: number,
    fields: Record<string, string | number | boolean | null | undefined> = {},
    thresholdMs = SCRUB_PERF_SLOW_DERIVED_MS,
  ): void {
    if (durationMs < thresholdMs) return;
    scrubPerfLog(event, { durationMs, ...fields });
  }

  function isPreviewGenerationCancelled(message: string): boolean {
    return message.toLowerCase().includes(PREVIEW_GENERATION_CANCELLED_MESSAGE);
  }

  function scrubPerfRecordScroll(
    durationMs: number,
    fields: Record<string, string | number | boolean | null | undefined>,
  ): void {
    if (!scrubPerfEnabled()) return;
    const now = performance.now();
    if (scrubPerfScrollWindowStartedAt === 0) scrubPerfScrollWindowStartedAt = now;
    scrubPerfScrollEventCount += 1;
    scrubPerfScrollTotalMs += durationMs;
    scrubPerfScrollMaxMs = Math.max(scrubPerfScrollMaxMs, durationMs);
    if (fields.activeChanged) scrubPerfScrollActiveChangeCount += 1;
    if (now - scrubPerfScrollWindowStartedAt < SCRUB_PERF_SCROLL_WINDOW_MS) return;
    scrubPerfLog("scroll_window", {
      events: scrubPerfScrollEventCount,
      activeChanges: scrubPerfScrollActiveChangeCount,
      avgMs: scrubPerfScrollTotalMs / Math.max(1, scrubPerfScrollEventCount),
      maxMs: scrubPerfScrollMaxMs,
      ...fields,
    });
    scrubPerfScrollWindowStartedAt = now;
    scrubPerfScrollEventCount = 0;
    scrubPerfScrollActiveChangeCount = 0;
    scrubPerfScrollTotalMs = 0;
    scrubPerfScrollMaxMs = 0;
  }

  const DASHBOARD_SHORTCUTS = {
    olderFrame: {
      id: "dashboard.olderFrame",
      label: "Older frame",
      bindings: [{ key: "ArrowLeft" }],
      kind: "behavior",
      scope: "dashboard",
    },
    newerFrame: {
      id: "dashboard.newerFrame",
      label: "Newer frame",
      bindings: [{ key: "ArrowRight" }],
      kind: "behavior",
      scope: "dashboard",
    },
    olderFrameFast: {
      id: "dashboard.olderFrameFast",
      label: "10 frames older",
      bindings: [{ key: "ArrowLeft", shift: true }],
      kind: "behavior",
      scope: "dashboard",
    },
    newerFrameFast: {
      id: "dashboard.newerFrameFast",
      label: "10 frames newer",
      bindings: [{ key: "ArrowRight", shift: true }],
      kind: "behavior",
      scope: "dashboard",
    },
    openJumpPicker: {
      id: "dashboard.openJumpPicker",
      label: "Open jump picker",
      bindings: [{ key: "J" }],
      kind: "command",
      scope: "dashboard",
    },
    jumpLatest: {
      id: "dashboard.jumpLatest",
      label: "Jump to latest",
      bindings: [{ key: "L" }],
      kind: "command",
      scope: "dashboard",
    },
    toggleOcr: {
      id: "dashboard.toggleOcr",
      label: "Toggle OCR panel",
      bindings: [{ key: "O" }],
      kind: "command",
      scope: "dashboard",
    },
    refreshTimeline: {
      id: "dashboard.refreshTimeline",
      label: "Refresh timeline",
      bindings: [{ key: "R" }],
      kind: "command",
      scope: "dashboard",
    },
    copyFrame: {
      id: "dashboard.copyFrame",
      label: "Copy active frame image",
      bindings: [{ key: "C" }],
      kind: "command",
      scope: "dashboard",
    },
    downloadFrame: {
      id: "dashboard.downloadFrame",
      label: "Download active frame image",
      bindings: [{ key: "D" }],
      kind: "command",
      scope: "dashboard",
    },
    playMoment: {
      id: "dashboard.playMoment",
      label: "Play audio at this moment",
      bindings: [{ key: "P" }],
      kind: "command",
      scope: "dashboard",
    },
    closeSurface: {
      id: "dashboard.closeSurface",
      label: "Close the top open surface",
      bindings: [{ key: "Escape" }],
      kind: "behavior",
      scope: "dashboard",
    },
  } satisfies Record<string, ShortcutDefinition>;

  const AUDIO_DRAWER_SHORTCUTS = {
    playPause: {
      id: "audioDrawer.playPause",
      label: "Play or pause",
      bindings: [{ key: "Space" }],
      kind: "command",
      scope: "audioDrawer",
    },
    seekBack: {
      id: "audioDrawer.seekBack",
      label: "Seek back 5 seconds",
      bindings: [{ key: "ArrowLeft" }],
      kind: "command",
      scope: "audioDrawer",
    },
    seekForward: {
      id: "audioDrawer.seekForward",
      label: "Seek forward 5 seconds",
      bindings: [{ key: "ArrowRight" }],
      kind: "command",
      scope: "audioDrawer",
    },
    seekBackFast: {
      id: "audioDrawer.seekBackFast",
      label: "Seek back 30 seconds",
      bindings: [{ key: "ArrowLeft", shift: true }],
      kind: "command",
      scope: "audioDrawer",
    },
    seekForwardFast: {
      id: "audioDrawer.seekForwardFast",
      label: "Seek forward 30 seconds",
      bindings: [{ key: "ArrowRight", shift: true }],
      kind: "command",
      scope: "audioDrawer",
    },
    close: {
      id: "audioDrawer.close",
      label: "Close speaker actions or audio drawer",
      bindings: [{ key: "Escape" }],
      kind: "behavior",
      scope: "audioDrawer",
    },
    trapFocus: {
      id: "audioDrawer.trapFocus",
      label: "Move through audio drawer controls",
      bindings: [{ key: "Tab" }],
      kind: "behavior",
      scope: "audioDrawer",
    },
  } satisfies Record<string, ShortcutDefinition>;

  function effectiveShortcut(definition: ShortcutDefinition): ShortcutDefinition {
    if (definition.kind !== "command") return definition;
    return shortcutDefinitionWithBinding(
      definition,
      getShortcutBinding(keyboardBindings.settings, definition.id as EditableShortcutActionId),
    );
  }

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
  type TimelineDataChangedPayload = {
    reason: "retention" | string;
    deletedBefore: string | null;
    startedAt?: string | null;
    endedAt?: string | null;
    deletedFrameIds?: number[];
    deletedAudioSegmentIds?: number[];
  };
  type BrokerOpenCaptureResultPayload = {
    opaqueId: string;
    kind: "frame" | "audio";
    frameId: number | null;
    audioSegmentId: number | null;
    // Audio Search Result Anchor carried by the Quick Recall handoff so an audio
    // open lands on the selected transcript match (span start + aligned frame)
    // instead of the segment start. Null for the broker-URL path.
    spanStartMs?: number | null;
    alignedFrameId?: number | null;
  };
  type AppIconResolution = {
    bundleId: string;
    iconPath: string | null;
  };
  type TimelineAppGroup = {
    key: string;
    boundaryFrameId: number | null;
    bundleId: string | null;
    appName: string | null;
    label: string;
    frameCount: number;
    rightPx: number;
    widthPx: number;
    iconLeftPx: number;
    iconSrc: string | null;
    showIcon: boolean;
    fallback: string;
    variant: "single" | "range";
  };
  type TimelineAppRun = Omit<TimelineAppGroup, "iconLeftPx" | "iconSrc" | "showIcon"> & {
    startIndex: number;
    endIndex: number;
  };
  type PendingTimelineScrollSample = {
    clientWidth: number;
    handlerStartedAt: number;
    scrollLeft: number;
    scrollWidth: number;
  };
  type TimelinePreviewDisplayMode = "parked" | "scrubbing";
  type TimelinePreviewDisplay = {
    filePath: string;
    frameId: number;
    source: "exact" | "scrub";
    sourceKind: FramePreviewDto["sourceKind"] | "none";
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
  let timelineRenderAnchorIndex = $state(0);
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
  // Currently selected audio segment for the inline player. Search-result
  // selections are pinned from the result payload so an aligned-frame jump can
  // move the visible lane without immediately closing the drawer.
  let selectedAudioSegmentId = $state<number | null>(null);
  let selectedAudioSegmentPinned = $state<AudioSegmentRecord | null>(null);
  let pendingAudioSeekMs = $state<number | null>(null);
  // Monotonic token used to discard stale `list_frames` responses. A reset
  // bumps this so any in-flight page request resolves into a no-op rather
  // than appending mismatched frames.
  let timelineGeneration = 0;

  let windowPlatform = $state<KeyboardPlatform>(detectKeyboardPlatform());

  // Preview file paths keyed by frame id. Reactive so the rail re-renders as
  // previews stream in without any extra plumbing.
  let previewCache = $state<Map<number, string>>(new Map());
  let previewMimeTypeCache = $state<Map<number, string>>(new Map());
  let previewSourceKindCache = $state<Map<number, FramePreviewDto["sourceKind"]>>(new Map());
  let previewRedactionCountCache = $state<Map<number, number>>(new Map());
  let previewLoadMsCache = $state<Map<number, number>>(new Map());
  let previewFailedAt = $state<Map<number, number>>(new Map());
  let scrubPreviewCache = $state<Map<number, string>>(new Map());
  let scrubPreviewMimeTypeCache = $state<Map<number, string>>(new Map());
  let scrubPreviewSourceKindCache = $state<Map<number, FramePreviewDto["sourceKind"]>>(new Map());
  let scrubPreviewLoadMsCache = $state<Map<number, number>>(new Map());
  let scrubPreviewFailedAt = $state<Map<number, number>>(new Map());
  let scrubPreviewIntervalCache = $state<Map<string, ScrubPreviewAvailabilityIntervalDto>>(new Map());
  // Tracks the in-flight requests so concurrent scrolls don't fan out a
  // request per slot per scroll tick for the same id.
  const previewInFlight = new Set<number>();
  const scrubPreviewInFlight = new Set<number>();
  type FrameActionStatus = {
    message: string;
    detail: string | null;
    tone: "neutral" | "error";
  };

  let frameActionStatus = $state<FrameActionStatus | null>(null);
  let frameActionStatusTimer: ReturnType<typeof setTimeout> | null = null;
  let frameActionStatusHovered = $state(false);
  // In-flight latch for the stage's copy/download frame-image actions so the
  // triggering menu item can show a "Copying…/Saving…" status and disable while
  // the async clipboard/file write runs. Cleared in each action's finally.
  let frameImageActionBusy = $state<null | "copy" | "download">(null);
  // In-flight latch for the OCR "copy all recognized text" action.
  let ocrCopyAllBusy = $state(false);
  let stageActionsMenuOpen = $state(false);
  // In-flight latch for the stage's "open captured URL" action: only one open
  // runs at a time on the stage, so a single boolean keeps a double-click on the
  // menu item from stacking opens. Reset in openCurrentFrameUrl's finally.
  let openingCurrentFrameUrl = $state(false);
  let stageActionsOpenedByKeyboard = false;
  let stageActionsTriggerEl = $state<HTMLButtonElement | null>(null);
  let stageActionsMenuEl = $state<HTMLDivElement | null>(null);
  let activePreviewFetchGeneration = 0;
  let activePreviewFetchTimer: ReturnType<typeof setTimeout> | null = null;
  let activeExactPreviewInFlight = false;
  let activeExactPreviewPendingFrameId: number | null = null;
  let scrubPreviewFetchGeneration = 0;
  let scrubPreviewFetchTimer: ReturnType<typeof setTimeout> | null = null;
  let scheduledScrubPreviewActiveIndex: number | null = null;
  let scheduledScrubPreviewGeneration = 0;
  let scrubPreviewWarmTimer: ReturnType<typeof setTimeout> | null = null;
  let scrubPreviewBatchInFlight = false;
  let scrubPreviewWarmInFlight = false;
  let scrubPreviewPendingActiveIndex: number | null = null;
  let previousActivePreviewIndex = 0;
  let previousActivePreviewFrameId: number | null = null;
  let lastTimelineScrollSample = { left: 0, at: 0 };
  let latestTimelineScrollLeft = 0;
  let latestTimelineScrubVelocityPxPerMs = 0;
  let pendingTimelineScrollSample: PendingTimelineScrollSample | null = null;
  let timelineScrollAnimationFrame: number | null = null;
  let previewScrubVelocityPxPerMs = $state(0);
  let previewCacheReuseCount = $state(0);
  let previewCacheHitCount = $state(0);
  let previewCacheMissCount = $state(0);
  let previewFailureCacheHitCount = $state(0);
  let previewInFlightJoinCount = $state(0);
  let previewDirectPathCount = $state(0);
  let previewGeneratedPathCount = $state(0);
  let previewStaleRetryCount = $state(0);
  let scrubPreviewHitCount = $state(0);
  let scrubPreviewMissCount = $state(0);
  let scrubPreviewBatchCount = $state(0);
  let scrubPreviewGeneratedCount = $state(0);
  let scrubPreviewMissingCount = $state(0);
  let scrubPreviewWarmCount = $state(0);
  let scrubPreviewQueuedCount = $state(0);
  let activePreviewLoadErrorFrameId = $state<number | null>(null);
  let timelineAppIconPathsByBundleId = $state<Record<string, string>>({});
  const requestedTimelineAppIconBundleIds = new Set<string>();
  // A refetched preview whose bytes still fail to decode in the <img> sentinel
  // would otherwise retry forever (fetch succeeds → paint fails → onerror →
  // refetch …). Cap decode retries per frameId; on exhaustion we surface a
  // terminal error in the always-visible stage status and stop refetching.
  const MAX_ACTIVE_PREVIEW_DECODE_RETRIES = 2;
  const activePreviewDecodeRetries = new Map<number, number>();

  function handleActivePreviewLoadError(frameId: number): void {
    const activeIndex = timelineFrames.findIndex((frame) => frame.id === frameId);
    const activeFrame = activeIndex >= 0 ? timelineFrames[activeIndex] : null;
    const intervalPreview = scrubPreviewIntervalForFrame(activeFrame);
    if (!previewCache.has(frameId) && (scrubPreviewCache.has(frameId) || intervalPreview)) {
      if (intervalPreview) {
        dropScrubPreviewIntervalCacheEntry(intervalPreview);
      }
      if (scrubPreviewCache.has(frameId)) {
        dropScrubPreviewCacheEntry(frameId);
      }
      rememberScrubPreviewFailure(frameId);
      if (activeIndex >= 0) {
        scrubPreviewFetchGeneration += 1;
        const gen = scrubPreviewFetchGeneration;
        void ensureLatestScrubPreviews(activeIndex, gen);
      }
      return;
    }
    if (activePreviewLoadErrorFrameId === frameId) return;
    const retries = activePreviewDecodeRetries.get(frameId) ?? 0;
    if (retries >= MAX_ACTIVE_PREVIEW_DECODE_RETRIES) {
      // Refetching keeps returning bytes the browser can't decode — stop the
      // loop and leave a terminal, visible error instead of churning silently.
      if (isTimelineActiveFrame(frameId)) {
        setFrameActionStatus("Preview couldn't be displayed", {
          detail: "This frame's image failed to decode after several attempts.",
          tone: "error",
        });
      }
      return;
    }
    activePreviewDecodeRetries.set(frameId, retries + 1);
    activePreviewLoadErrorFrameId = frameId;
    previewStaleRetryCount += 1;
    dropPreviewCacheEntry(frameId);
    clearPreviewFailure(frameId);
    void ensurePreview(frameId);
  }

  function handleActivePreviewLoad(frameId: number): void {
    // A successful repaint means the current bytes decoded fine, so clear this
    // frame's decode-retry budget. Otherwise a frame that transiently failed
    // (then recovered) keeps a poisoned counter, and a later genuine decode
    // failure on the same frame would trip the terminal error immediately.
    activePreviewDecodeRetries.delete(frameId);
  }

  const timelineActive = $derived(timelineFrames[timelineActiveIndex] ?? null);
  // The current frame's host (the guarded host+path up to the first "/"); empty
  // when the frame has no guarded URL. The raw URL never reaches the UI.
  const currentFrameHost = $derived(
    timelineActive?.url ? timelineActive.url.split("/")[0] : "",
  );
  const timelineFrameById = $derived.by<Map<number, FrameDto>>(() => {
    const framesById = new Map<number, FrameDto>();
    for (const frame of timelineFrames) framesById.set(frame.id, frame);
    return framesById;
  });
  const activePreviewPath = $derived(
    timelineActive ? previewCache.get(timelineActive.id) ?? null : null,
  );
  const activeExactPreviewLoadMs = $derived(
    timelineActive ? previewLoadMsCache.get(timelineActive.id) ?? null : null,
  );
  // Scrub previews display via the interval cache (`scrubPreviewIntervalForFrame`),
  // which records no per-frame JS load-ms — so report readiness, not a timing that
  // can never populate. ponytail: the per-frame scrubPreview*Cache maps are dead.
  const activeScrubPreviewReady = $derived(
    timelineActive ? scrubPreviewIntervalForFrame(timelineActive) != null : false,
  );
  const timelineHasMore = $derived(timelineHasNewer || !timelineExhausted);
  let lastPreviewReuseFrameId = $state<number | null>(null);
  let timelineActiveDuplicateOf = $state<FrameDto | null>(null);
  let timelineActiveDuplicateLookupGeneration = 0;
  let timelineActiveDuplicateLookupTimer: ReturnType<typeof setTimeout> | null = null;
  let timelinePreviewDisplay = $state<TimelinePreviewDisplay | null>(null);
  let timelinePreviewDisplayMode = $state<TimelinePreviewDisplayMode>("parked");
  let timelinePreviewDisplaySettleTimer: ReturnType<typeof setTimeout> | null = null;
  let timelinePreviewDisplayLastScrubAt = 0;

  function previewDisplayCandidateForFrame(
    frame: FrameDto | null,
    options: { allowExact: boolean },
  ): TimelinePreviewDisplay | null {
    if (!frame) return null;
    if (options.allowExact) {
      const exactPath = previewCache.get(frame.id);
      if (exactPath) {
        return {
          filePath: exactPath,
          frameId: frame.id,
          source: "exact",
          sourceKind: previewSourceKindCache.get(frame.id) ?? "none",
        };
      }
    }

    const interval = scrubPreviewIntervalForFrame(frame);
    if (interval?.preview) {
      return {
        filePath: interval.preview.filePath,
        frameId: frame.id,
        source: "scrub",
        sourceKind: interval.preview.sourceKind,
      };
    }

    const scrubPath = scrubPreviewCache.get(frame.id);
    if (scrubPath) {
      return {
        filePath: scrubPath,
        frameId: frame.id,
        source: "scrub",
        sourceKind: scrubPreviewSourceKindCache.get(frame.id) ?? "scrub_preview",
      };
    }

    return null;
  }

  function applyTimelinePreviewDisplay(
    candidate: TimelinePreviewDisplay | null,
    mode: TimelinePreviewDisplayMode,
    options: { clearOnMissing: boolean },
  ): void {
    if (timelinePreviewDisplayMode !== mode) {
      timelinePreviewDisplayMode = mode;
    }
    if (!candidate) {
      if (options.clearOnMissing && timelinePreviewDisplay !== null) {
        timelinePreviewDisplay = null;
      }
      return;
    }
    const current = timelinePreviewDisplay;
    if (
      current &&
      current.frameId === candidate.frameId &&
      current.filePath === candidate.filePath &&
      current.source === candidate.source &&
      current.sourceKind === candidate.sourceKind
    ) {
      return;
    }
    timelinePreviewDisplay = candidate;
  }

  function updateTimelinePreviewDisplayForFrame(
    frame: FrameDto | null,
    mode: TimelinePreviewDisplayMode,
    options: { allowExact: boolean; clearOnMissing: boolean },
  ): void {
    applyTimelinePreviewDisplay(
      previewDisplayCandidateForFrame(frame, { allowExact: options.allowExact }),
      mode,
      { clearOnMissing: options.clearOnMissing },
    );
  }

  function refreshScrubbingTimelinePreviewDisplay(): void {
    if (timelinePreviewDisplayMode !== "scrubbing") return;
    updateTimelinePreviewDisplayForFrame(timelineActive, "scrubbing", {
      allowExact: true,
      clearOnMissing: false,
    });
  }

  function clearTimelinePreviewDisplaySettleTimer(): void {
    if (timelinePreviewDisplaySettleTimer == null) return;
    clearTimeout(timelinePreviewDisplaySettleTimer);
    timelinePreviewDisplaySettleTimer = null;
  }

  function resetTimelinePreviewDisplay(): void {
    clearTimelinePreviewDisplaySettleTimer();
    timelinePreviewDisplay = null;
    timelinePreviewDisplayMode = "parked";
    timelinePreviewDisplayLastScrubAt = 0;
  }

  $effect(() => {
    if (timelinePreviewDisplayMode === "scrubbing") return;
    updateTimelinePreviewDisplayForFrame(timelineActive, "parked", {
      allowExact: true,
      clearOnMissing: true,
    });
  });

  $effect(() => {
    return () => {
      clearTimelinePreviewDisplaySettleTimer();
    };
  });

  $effect(() => {
    return () => {
      clearScrubPreviewFetchTimer();
    };
  });

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
    if (timelineActiveDuplicateLookupTimer != null) {
      clearTimeout(timelineActiveDuplicateLookupTimer);
      timelineActiveDuplicateLookupTimer = null;
    }
    timelineActiveDuplicateOf = null;

    if (!shouldLookup || !active) {
      return () => {};
    }

    timelineActiveDuplicateLookupTimer = setTimeout(() => {
      timelineActiveDuplicateLookupTimer = null;
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
    }, 200);

    return () => {
      if (timelineActiveDuplicateLookupTimer != null) {
        clearTimeout(timelineActiveDuplicateLookupTimer);
        timelineActiveDuplicateLookupTimer = null;
      }
    };
  });

  // Selected audio segment for the inline player. Resolved from the current
  // `audioSegments` list each render so the selection auto-clears when a
  // refresh drops the row (see `$effect` below). Audio media bytes are fetched
  // by id from Tauri so playback does not depend on asset protocol scope or on
  // accepting arbitrary frontend-provided file paths.
  const selectedAudioSegment = $derived(
    selectedAudioSegmentId == null
      ? null
      : audioSegments.find((s) => s.id === selectedAudioSegmentId) ??
        (selectedAudioSegmentPinned?.id === selectedAudioSegmentId
          ? selectedAudioSegmentPinned
          : null),
  );
  let selectedAudioSrc = $state<string | null>(null);
  let selectedAudioMediaLoading = $state(false);
  let selectedAudioMediaError = $state<string | null>(null);
  let selectedAudioMediaGeneration = 0;
  let selectedAudioTranscriptStatus = $state<AudioTranscriptStatus>("idle");
  let selectedAudioTranscriptText = $state<string | null>(null);
  let selectedAudioTranscriptSegments = $state<TranscriptionSegment[]>([]);
  let selectedAudioSpeakerTurns = $state<SpeakerTurnDto[]>([]);
  let selectedAudioSpeakerTurnsError = $state<string | null>(null);
  let selectedAudioSpeakerTurnsNotice = $state<string | null>(null);
  let selectedAudioSpeakerClusters = $state<SpeakerClusterDto[]>([]);
  let selectedAudioSpeakerAnalysisRunning = $state(false);
  let selectedAudioSpeakerAnalysisFailedJobId = $state<number | null>(null);
  let selectedAudioSpeakerAnalysisRetryLoading = $state(false);
  let personProfiles = $state<PersonProfileDto[]>([]);
  let speakerCorrectionError = $state<string | null>(null);
  let speakerCorrectionBusyClusterId = $state<number | null>(null);
  let speakerInlineBusyAction = $state<{
    clusterId: number;
    action: SpeakerInlineAction;
  } | null>(null);
  let speakerNameDrafts = $state<Record<number, string>>({});
  let speakerActionsOpenIndex = $state<number | null>(null);
  let speakerActionsPopoverEl = $state<HTMLDivElement | null>(null);
  let speakerActionsReturnFocusEl: HTMLElement | null = null;
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
    if (selectedAudioSegmentId == null) {
      pendingAudioSeekMs = null;
      selectedAudioSegmentPinned = null;
    }
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
      selectedAudioMediaError = humanizeError(err);
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
    selectedAudioSpeakerTurns = [];
    selectedAudioSpeakerClusters = [];
    selectedAudioSpeakerAnalysisRunning = false;
    selectedAudioSpeakerAnalysisFailedJobId = null;
    selectedAudioSpeakerAnalysisRetryLoading = false;
    speakerInlineBusyAction = null;
    speakerNameDrafts = {};
    speakerActionsOpenIndex = null;
    speakerCorrectionError = null;
    selectedAudioSpeakerTurnsError = null;
    selectedAudioSpeakerTurnsNotice = null;
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

  function overlapDurationMs(
    startA: number,
    endA: number,
    startB: number,
    endB: number,
  ): number {
    return Math.max(0, Math.min(endA, endB) - Math.max(startA, startB));
  }

  function segmentMidpointDistanceMs(
    turn: Pick<SpeakerTurnDto, "startMs" | "endMs">,
    segment: Pick<TranscriptionSegment, "startMs" | "endMs">,
  ): number {
    const turnMidpoint = (turn.startMs + turn.endMs) / 2;
    const segmentMidpoint = (segment.startMs + segment.endMs) / 2;
    return Math.abs(turnMidpoint - segmentMidpoint);
  }

  function transcriptTextsBySpeakerTurn(turns: SpeakerTurnDto[]): Map<number, string> {
    const textByTurnId = new Map<number, string>();

    for (const turn of turns) {
      const direct = turn.transcriptText?.trim();
      if (direct) {
        textByTurnId.set(turn.id, direct);
      }
    }

    return textByTurnId;
  }

  type SpeakerTranscriptGroup = {
    clusterId: number;
    speakerLabel: string;
    personId: number | null;
    suggestedPersonId: number | null;
    recognitionConfidence: SpeakerTurnDto["recognitionConfidence"];
    recognitionScore: number | null;
    suggestedMergeTargetClusterId: number | null;
    suggestedMergeScore: number | null;
    startMs: number;
    endMs: number;
    text: string;
    overlaps: boolean;
    turnIds: number[];
  };
  type SpeakerInlineAction = "confirm" | "reject" | "merge";
  type SpeakerTurnsLoadOptions = {
    refreshPersonProfiles?: boolean;
  };

  const selectedAudioSpeakerGroups = $derived.by<SpeakerTranscriptGroup[]>(() => {
    const transcriptByTurnId = transcriptTextsBySpeakerTurn(selectedAudioSpeakerTurns);
    const groups: SpeakerTranscriptGroup[] = [];
    for (const turn of selectedAudioSpeakerTurns) {
      const text = transcriptByTurnId.get(turn.id) ?? "";
      if (!text) continue;
      const previous = groups.at(-1);
      if (previous && previous.clusterId === turn.clusterId) {
        previous.endMs = Math.max(previous.endMs, turn.endMs);
        previous.text = `${previous.text} ${text}`.trim();
        previous.overlaps = previous.overlaps || turn.overlaps;
        previous.turnIds.push(turn.id);
        continue;
      }
      groups.push({
        clusterId: turn.clusterId,
        speakerLabel: turn.speakerLabel,
        personId: turn.personId,
        suggestedPersonId: turn.suggestedPersonId,
        recognitionConfidence: turn.recognitionConfidence,
        recognitionScore: turn.recognitionScore,
        suggestedMergeTargetClusterId:
          selectedAudioSpeakerClusters.find((cluster) => cluster.id === turn.clusterId)
            ?.suggestedMergeTargetClusterId ?? null,
        suggestedMergeScore:
          selectedAudioSpeakerClusters.find((cluster) => cluster.id === turn.clusterId)
            ?.suggestedMergeScore ?? null,
        startMs: turn.startMs,
        endMs: turn.endMs,
        text,
        overlaps: turn.overlaps,
        turnIds: [turn.id],
      });
    }
    return groups;
  });

  function speakerDisplayLabel(group: SpeakerTranscriptGroup): string {
    if (group.personId != null) return speakerProfileName(group.personId) ?? group.speakerLabel;
    if (group.suggestedPersonId != null) {
      return `Maybe ${speakerProfileName(group.suggestedPersonId) ?? speakerCleanLabel(group.speakerLabel)}`;
    }
    return group.speakerLabel;
  }

  function speakerCleanLabel(label: string): string {
    return label.replace(/^Maybe\s+/i, "").trim();
  }

  function isDefaultSpeakerLabel(label: string): boolean {
    return /^unknown speaker\s+\d+$/i.test(speakerCleanLabel(label));
  }

  function speakerProfileName(personId: number | null): string | null {
    if (personId == null) return null;
    return personProfiles.find((profile) => profile.id === personId)?.displayName ?? null;
  }

  function speakerPersistedName(group: SpeakerTranscriptGroup): string {
    return speakerCleanLabel(speakerDisplayLabel(group));
  }

  function speakerNameDraft(group: SpeakerTranscriptGroup): string {
    return speakerNameDrafts[group.clusterId] ?? speakerPersistedName(group);
  }

  function canRememberSpeakerProfile(group: SpeakerTranscriptGroup): boolean {
    const name = speakerNameDraft(group).trim();
    return name.length > 0 && !isDefaultSpeakerLabel(name);
  }

  function selectablePersonProfiles(group: SpeakerTranscriptGroup): PersonProfileDto[] {
    return personProfiles.filter((profile) =>
      profile.id !== group.personId && !isDefaultSpeakerLabel(profile.displayName)
    );
  }

  function speakerClusterOptionLabel(cluster: SpeakerClusterDto): string {
    if (cluster.personId != null) return speakerProfileName(cluster.personId) ?? cluster.speakerLabel;
    if (cluster.suggestedPersonId != null) {
      return speakerProfileName(cluster.suggestedPersonId) ?? `Maybe ${speakerCleanLabel(cluster.speakerLabel)}`;
    }
    return speakerCleanLabel(cluster.speakerLabel);
  }

  function speakerConfidenceLabel(group: SpeakerTranscriptGroup): string | null {
    if (group.recognitionScore == null && group.recognitionConfidence == null) return null;
    const score = group.recognitionScore == null
      ? null
      : group.recognitionScore <= 1
        ? `${Math.round(group.recognitionScore * 100)}%`
        : group.recognitionScore.toFixed(2);
    if (group.recognitionConfidence && score) return `${group.recognitionConfidence} · ${score}`;
    return group.recognitionConfidence ?? (score ? `score ${score}` : null);
  }

  function suggestedMergeTargetLabel(group: SpeakerTranscriptGroup): string | null {
    const targetId = group.suggestedMergeTargetClusterId;
    if (targetId == null) return null;
    const target = selectedAudioSpeakerClusters.find((cluster) => cluster.id === targetId);
    return target ? speakerClusterOptionLabel(target) : null;
  }

  function isFirstVisibleSpeakerClusterOccurrence(
    group: SpeakerTranscriptGroup,
    index: number,
  ): boolean {
    return selectedAudioSpeakerGroups.findIndex(
      (candidate) => candidate.clusterId === group.clusterId,
    ) === index;
  }

  function speakerSuggestedPersonName(group: SpeakerTranscriptGroup): string {
    return speakerProfileName(group.suggestedPersonId) ?? speakerCleanLabel(group.speakerLabel);
  }

  function shouldShowPersonSuggestionRow(
    group: SpeakerTranscriptGroup,
    index: number,
  ): boolean {
    return group.suggestedPersonId != null &&
      group.personId == null &&
      isFirstVisibleSpeakerClusterOccurrence(group, index);
  }

  function shouldShowMergeSuggestionRow(
    group: SpeakerTranscriptGroup,
    index: number,
  ): boolean {
    return group.suggestedMergeTargetClusterId != null &&
      suggestedMergeTargetLabel(group) != null &&
      isFirstVisibleSpeakerClusterOccurrence(group, index);
  }

  function formatSpeakerActionScore(score: number | null): string | null {
    if (score == null || !Number.isFinite(score)) return null;
    const percentage = score <= 1 ? score * 100 : score;
    return `${Math.round(Math.max(0, Math.min(100, percentage)))}%`;
  }

  function speakerActionConfidenceLabel(group: SpeakerTranscriptGroup): string | null {
    const confidence = group.recognitionConfidence?.toLowerCase() ?? null;
    const confidenceLabel = confidence === "high"
      ? "High confidence"
      : confidence === "medium"
        ? "Medium confidence"
        : null;
    const scoreLabel = developerOptions.value
      ? formatSpeakerActionScore(group.recognitionScore)
      : null;
    if (confidenceLabel && scoreLabel) return `${confidenceLabel} · ${scoreLabel}`;
    return confidenceLabel ?? scoreLabel;
  }

  function speakerActionScoreLabel(group: SpeakerTranscriptGroup): string | null {
    if (!developerOptions.value) return null;
    const scoreLabel = formatSpeakerActionScore(group.suggestedMergeScore);
    return scoreLabel ? `merge score ${scoreLabel}` : null;
  }

  function speakerActionBusyLabel(action: SpeakerInlineAction): string {
    switch (action) {
      case "confirm":
        return "Confirming...";
      case "reject":
        return "Rejecting...";
      case "merge":
        return "Merging...";
    }
  }

  function speakerInlineActionIsBusy(
    group: SpeakerTranscriptGroup,
    action: SpeakerInlineAction,
  ): boolean {
    return speakerInlineBusyAction?.clusterId === group.clusterId &&
      speakerInlineBusyAction.action === action;
  }

  function speakerInlineActionDisabled(group: SpeakerTranscriptGroup): boolean {
    return speakerCorrectionBusyClusterId === group.clusterId;
  }

  async function runSpeakerInlineAction(
    clusterId: number,
    action: SpeakerInlineAction,
    task: () => Promise<void>,
  ): Promise<void> {
    speakerInlineBusyAction = { clusterId, action };
    try {
      await task();
    } finally {
      if (
        speakerInlineBusyAction?.clusterId === clusterId &&
        speakerInlineBusyAction.action === action
      ) {
        speakerInlineBusyAction = null;
      }
    }
  }

  async function confirmInlineSpeakerSuggestion(group: SpeakerTranscriptGroup): Promise<void> {
    await runSpeakerInlineAction(
      group.clusterId,
      "confirm",
      () => confirmSpeakerSuggestion(group.clusterId),
    );
  }

  async function rejectInlineSpeakerSuggestion(group: SpeakerTranscriptGroup): Promise<void> {
    await runSpeakerInlineAction(
      group.clusterId,
      "reject",
      () => rejectSpeakerSuggestion(group.clusterId),
    );
  }

  async function mergeInlineSpeakerSuggestion(group: SpeakerTranscriptGroup): Promise<void> {
    await runSpeakerInlineAction(
      group.clusterId,
      "merge",
      () => mergeSpeakerClusterById(group.clusterId, group.suggestedMergeTargetClusterId),
    );
  }

  function updateSpeakerNameDraft(clusterId: number, event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    speakerNameDrafts = { ...speakerNameDrafts, [clusterId]: input.value };
  }

  function resetSpeakerNameDraft(group: SpeakerTranscriptGroup): void {
    speakerNameDrafts = { ...speakerNameDrafts, [group.clusterId]: speakerPersistedName(group) };
  }

  async function loadSelectedAudioSpeakerTurns(
    id: number,
    gen: number,
    speakerJobId: number | null = null,
    options: SpeakerTurnsLoadOptions = {},
  ): Promise<void> {
    try {
      const refreshPersonProfiles = options.refreshPersonProfiles ?? true;
      const [turns, profiles] = await Promise.all([
        invoke<SpeakerTurnDto[]>("list_speaker_turns", {
          request: { audioSegmentId: id },
        }),
        refreshPersonProfiles
          ? invoke<PersonProfileDto[]>("list_person_profiles")
          : Promise.resolve(personProfiles),
      ]);
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioSpeakerTurns = turns;
      if (refreshPersonProfiles) personProfiles = profiles;
      selectedAudioSpeakerTurnsError = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      const sessionId = turns[0]?.sessionId;
      selectedAudioSpeakerClusters = sessionId
        ? await invoke<SpeakerClusterDto[]>("list_speaker_clusters", {
            request: { sessionId },
          })
        : [];
      if (
        turns.length === 0 &&
        speakerJobId != null &&
        selectedAudioTranscriptStatus === "success"
      ) {
        selectedAudioSpeakerTurnsNotice = await loadSpeakerAnalysisEmptyNotice(speakerJobId);
      }
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerClusters = [];
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsError = humanizeError(err);
    }
  }

  async function refreshCurrentSpeakerTurns(
    options: SpeakerTurnsLoadOptions = {},
  ): Promise<void> {
    const id = selectedAudioSegmentId;
    if (id == null) return;
    // The transcript body (not the drawer) now owns the scroll, so preserve
    // its position across a turns refresh rather than the drawer's (which no
    // longer scrolls).
    const scrollEl = selectedAudioTranscriptContainerEl;
    const scrollTop = scrollEl?.scrollTop ?? null;
    await loadSelectedAudioSpeakerTurns(id, selectedAudioTranscriptGeneration, null, options);
    if (scrollEl && scrollTop != null && selectedAudioTranscriptContainerEl === scrollEl) {
      await tick();
      scrollEl.scrollTop = scrollTop;
    }
  }

  async function loadSpeakerAnalysisEmptyNotice(jobId: number): Promise<string> {
    const result = await invoke<ProcessingResultDto | null>("get_processing_result", {
      request: { jobId } satisfies GetProcessingResultRequest,
    });
    const skipReason = parseSpeakerAnalysisSkipReason(result?.structuredPayloadJson ?? null);
    if (skipReason === "too_short") {
      return "Speaker analysis skipped: audio segment is too short.";
    }
    if (skipReason === "silent") {
      return "Speaker analysis skipped: no speech-level audio detected.";
    }
    return "No speaker turns found.";
  }

  function parseSpeakerAnalysisSkipReason(
    structuredPayloadJson: string | null,
  ): SpeakerAnalysisSkipReason | null {
    if (!structuredPayloadJson) return null;
    try {
      const parsed = JSON.parse(structuredPayloadJson) as SpeakerAnalysisStructuredPayload;
      const skipReason = parsed.metadata?.provenance?.skipReason;
      return skipReason === "too_short" || skipReason === "silent" ? skipReason : null;
    } catch {
      return null;
    }
  }

  interface SystemAudioSpeechActivityPayload {
    speechDetected?: boolean;
  }

  function parseSystemAudioSpeechActivityPayload(
    structuredPayloadJson: string | null,
  ): SystemAudioSpeechActivityPayload | null {
    if (!structuredPayloadJson) return null;
    try {
      const parsed = JSON.parse(structuredPayloadJson) as SystemAudioSpeechActivityPayload;
      return typeof parsed.speechDetected === "boolean" ? parsed : null;
    } catch {
      return null;
    }
  }

  function latestProcessingJobForProcessor(
    jobs: ProcessingJobDto[],
    processor: string,
  ): ProcessingJobDto | null {
    return jobs
      .filter((job) => job.processor === processor)
      .sort((a, b) => b.id - a.id)[0] ?? null;
  }

  function processingJobIsPending(job: ProcessingJobDto | null): boolean {
    return job?.status === "queued" || job?.status === "running";
  }

  async function saveSpeakerClusterName(clusterId: number, label: string): Promise<void> {
    const trimmedLabel = label.trim();
    if (trimmedLabel.length === 0) return;
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      await invoke("name_speaker_cluster", { request: { clusterId, label: trimmedLabel } });
      const { [clusterId]: _removed, ...remainingDrafts } = speakerNameDrafts;
      speakerNameDrafts = remainingDrafts;
      await refreshCurrentSpeakerTurns({ refreshPersonProfiles: false });
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function saveSpeakerNameIfChanged(group: SpeakerTranscriptGroup): Promise<void> {
    const label = speakerNameDraft(group).trim();
    if (label.length === 0) {
      resetSpeakerNameDraft(group);
      return;
    }
    if (label === speakerPersistedName(group)) return;
    await saveSpeakerClusterName(group.clusterId, label);
  }

  function handleSpeakerNameKeydown(event: KeyboardEvent, group: SpeakerTranscriptGroup): void {
    if (event.key === "Enter") {
      event.preventDefault();
      void saveSpeakerNameIfChanged(group);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      resetSpeakerNameDraft(group);
      (event.currentTarget as HTMLInputElement).blur();
    }
  }

  // Viewport-anchored placement for the speaker-actions popover. The popover
  // renders in the top layer (`popover="manual"`), so neither the transcript's
  // scroll container nor the drawer's `overflow: hidden` can clip it; anchor
  // it just above the clicked chip and clamp it on-screen.
  let speakerActionsPopoverPos = $state<{ left: number; bottom: number } | null>(
    null,
  );

  function toggleSpeakerActions(index: number, event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    const chip = event.currentTarget as HTMLElement;
    speakerActionsReturnFocusEl = chip;
    if (speakerActionsOpenIndex === index) {
      speakerActionsOpenIndex = null;
      return;
    }
    const rect = chip.getBoundingClientRect();
    // Mirror the CSS width `min(42rem, 100vw - 64px)` so the left clamp keeps
    // the popover fully on-screen.
    const rem =
      Number.parseFloat(getComputedStyle(document.documentElement).fontSize) ||
      16;
    const width = Math.min(42 * rem, window.innerWidth - 64);
    speakerActionsPopoverPos = {
      left: Math.max(12, Math.min(rect.left, window.innerWidth - width - 12)),
      bottom: window.innerHeight - rect.top + 6,
    };
    speakerActionsOpenIndex = index;
  }

  function closeSpeakerActions(): void {
    speakerActionsOpenIndex = null;
  }

  $effect(() => {
    if (speakerActionsOpenIndex == null) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || speakerActionsOpenIndex == null) return;
      // Promote the freshly-rendered popover into the top layer before
      // focusing it (a `[popover]` element is display:none until shown).
      speakerActionsPopoverEl?.showPopover?.();
      const first = getFocusableElements(speakerActionsPopoverEl)[0] ?? speakerActionsPopoverEl;
      first?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (
        active &&
        speakerActionsPopoverEl?.contains(active)
      ) {
        speakerActionsReturnFocusEl?.focus({ preventScroll: true });
      }
    };
  });

  function onSpeakerLineClick(group: SpeakerTranscriptGroup): void {
    closeSpeakerActions();
    seekAudioToTimeMs(group.startMs);
  }

  async function createAndLinkSpeakerProfile(clusterId: number, displayName: string): Promise<void> {
    const trimmedDisplayName = displayName.trim();
    if (trimmedDisplayName.length === 0) return;
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      const profile = await invoke<PersonProfileDto>("create_person_profile", {
        request: { displayName: trimmedDisplayName, notes: null },
      });
      await invoke("link_speaker_cluster_to_person", {
        request: { clusterId, personId: profile.id, addEmbedding: true },
      });
      await refreshCurrentSpeakerTurns();
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function linkSpeakerCluster(clusterId: number, personId: number): Promise<void> {
    if (!Number.isFinite(personId) || personId <= 0) return;
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      await invoke("link_speaker_cluster_to_person", {
        request: { clusterId, personId, addEmbedding: true },
      });
      await refreshCurrentSpeakerTurns();
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function confirmSpeakerSuggestion(clusterId: number): Promise<void> {
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      await invoke("confirm_speaker_recognition_suggestion", {
        request: { clusterId, addEmbedding: true },
      });
      await refreshCurrentSpeakerTurns();
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function rejectSpeakerSuggestion(clusterId: number): Promise<void> {
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      await invoke("reject_speaker_recognition_suggestion", { request: { clusterId } });
      await refreshCurrentSpeakerTurns({ refreshPersonProfiles: false });
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function unlinkSpeakerProfile(clusterId: number): Promise<void> {
    speakerCorrectionBusyClusterId = clusterId;
    speakerCorrectionError = null;
    try {
      await invoke("unlink_speaker_cluster_from_person", { request: { clusterId } });
      await refreshCurrentSpeakerTurns({ refreshPersonProfiles: false });
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function mergeSpeakerClusterById(
    sourceClusterId: number,
    targetClusterId: number | null,
  ): Promise<void> {
    if (targetClusterId == null || !Number.isFinite(targetClusterId) || targetClusterId <= 0) return;
    speakerCorrectionBusyClusterId = sourceClusterId;
    speakerCorrectionError = null;
    try {
      await invoke("merge_speaker_clusters", {
        request: { sourceClusterId, targetClusterId },
      });
      await refreshCurrentSpeakerTurns({ refreshPersonProfiles: false });
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
  }

  async function moveSpeakerBlockTurns(
    group: SpeakerTranscriptGroup,
    targetClusterId: number,
  ): Promise<void> {
    if (!Number.isFinite(targetClusterId) || targetClusterId <= 0) return;
    speakerCorrectionBusyClusterId = group.clusterId;
    speakerCorrectionError = null;
    try {
      for (const turnId of group.turnIds) {
        await invoke("move_speaker_turn_to_cluster", {
          request: { turnId, targetClusterId },
        });
      }
      await refreshCurrentSpeakerTurns({ refreshPersonProfiles: false });
    } catch (err) {
      speakerCorrectionError = humanizeError(err);
    } finally {
      speakerCorrectionBusyClusterId = null;
    }
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
        selectedAudioTranscriptError = "Audio processing job not found";
        return;
      }
      if (job.processor === AUDIO_TRANSCRIPTION_PROCESSOR) {
        await applySelectedAudioTranscriptJob(id, gen, job);
        return;
      }
      if (job.processor === SPEAKER_ANALYSIS_PROCESSOR) {
        await applySelectedAudioSpeakerAnalysisJob(id, gen, job);
        return;
      }
      if (job.processor === SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR) {
        const shouldContinue = await applySelectedSystemAudioSpeechActivityJob(id, gen, job);
        if (shouldContinue && selectedAudioTranscriptIsCurrent(id, gen)) {
          await loadSelectedAudioSegmentTranscript(id, gen);
        }
        return;
      }
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptError = `Unexpected audio processing job: ${job.processor}`;
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioTranscriptError = humanizeError(err);
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

  async function loadLatestSpeakerAnalysisJob(id: number): Promise<ProcessingJobDto | null> {
    const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
      request: { subjectType: AUDIO_SEGMENT_SUBJECT_TYPE, subjectId: id },
    });
    return latestProcessingJobForProcessor(jobs, SPEAKER_ANALYSIS_PROCESSOR);
  }

  async function applySelectedAudioSpeakerAnalysisJob(
    id: number,
    gen: number,
    job: ProcessingJobDto,
  ): Promise<void> {
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return;

    if (processingJobIsPending(job)) {
      selectedAudioSpeakerAnalysisRunning = true;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsError = null;
      selectedAudioSpeakerTurnsNotice = null;
      scheduleSelectedAudioTranscriptPoll(id, job.id, gen);
      return;
    }

    selectedAudioSpeakerAnalysisRunning = false;
    clearSelectedAudioTranscriptPoll();

    if (job.status === "failed") {
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerClusters = [];
      selectedAudioSpeakerAnalysisFailedJobId = job.id;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioSpeakerTurnsError = job.lastError ?? "Speaker analysis failed";
      return;
    }

    selectedAudioSpeakerAnalysisFailedJobId = null;
    await loadSelectedAudioSpeakerTurns(id, gen, job.id);
  }

  async function waitForSelectedAudioSpeakerAnalysisIfNeeded(
    id: number,
    gen: number,
  ): Promise<void> {
    try {
      const speakerJob = await loadLatestSpeakerAnalysisJob(id);
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      if (speakerJob) {
        await applySelectedAudioSpeakerAnalysisJob(id, gen, speakerJob);
        return;
      }
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      await loadSelectedAudioSpeakerTurns(id, gen);
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioSpeakerTurnsError = humanizeError(err);
    }
  }

  async function applySelectedSystemAudioSpeechActivityJob(
    id: number,
    gen: number,
    job: ProcessingJobDto,
  ): Promise<boolean> {
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return false;

    if (processingJobIsPending(job)) {
      selectedAudioTranscriptStatus = "running";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = null;
      scheduleSelectedAudioTranscriptPoll(id, job.id, gen);
      return false;
    }

    clearSelectedAudioTranscriptPoll();

    if (job.status === "failed") {
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = job.lastError ?? "Speech detection failed";
      return false;
    }

    const speechResult = await invoke<ProcessingResultDto | null>("get_processing_result", {
      request: { jobId: job.id } satisfies GetProcessingResultRequest,
    });
    if (!selectedAudioTranscriptIsCurrent(id, gen)) return false;

    const speechPayload = parseSystemAudioSpeechActivityPayload(
      speechResult?.structuredPayloadJson ?? null,
    );
    if (speechPayload?.speechDetected === false) {
      selectedAudioTranscriptStatus = "empty";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = null;
      return false;
    }

    return true;
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
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = null;
      scheduleSelectedAudioTranscriptPoll(id, job.id, gen);
      return;
    }

    clearSelectedAudioTranscriptPoll();

    if (job.status === "failed") {
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
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
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
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
    await waitForSelectedAudioSpeakerAnalysisIfNeeded(id, gen);
  }

  async function loadSelectedAudioSegmentTranscript(id: number, gen: number): Promise<void> {
    try {
      const jobs = await invoke<ProcessingJobDto[]>("list_processing_jobs", {
        request: { subjectType: AUDIO_SEGMENT_SUBJECT_TYPE, subjectId: id },
      });
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;

      if (selectedAudioSegment?.source === "systemAudio") {
        const speechJob = latestProcessingJobForProcessor(
          jobs,
          SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR,
        );
        if (!speechJob) {
          clearSelectedAudioTranscriptPoll();
          selectedAudioTranscriptStatus = "missing";
          selectedAudioTranscriptText = null;
          selectedAudioTranscriptSegments = [];
          selectedAudioTranscriptError = null;
          return;
        }
        const shouldContinue = await applySelectedSystemAudioSpeechActivityJob(
          id,
          gen,
          speechJob,
        );
        if (!shouldContinue) {
          return;
        }
      }

      const transcriptionJobs = jobs.filter(
        (job) => job.processor === AUDIO_TRANSCRIPTION_PROCESSOR,
      );
      if (transcriptionJobs.length === 0) {
        clearSelectedAudioTranscriptPoll();
        selectedAudioTranscriptStatus = "missing";
        selectedAudioTranscriptText = null;
        selectedAudioTranscriptSegments = [];
        selectedAudioTranscriptError = null;
        await waitForSelectedAudioSpeakerAnalysisIfNeeded(id, gen);
        return;
      }

      const completed = transcriptionJobs
        .filter((job) => job.status === "completed")
        .sort((a, b) => b.id - a.id);
      const job = completed[0] ?? latestProcessingJobForProcessor(jobs, AUDIO_TRANSCRIPTION_PROCESSOR);
      if (!job) return;
      await applySelectedAudioTranscriptJob(id, gen, job);
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      clearSelectedAudioTranscriptPoll();
      selectedAudioTranscriptStatus = "error";
      selectedAudioTranscriptText = null;
      selectedAudioTranscriptSegments = [];
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = humanizeError(err);
    }
  }

  // If a segment was selected before its transcription/diarization job existed,
  // the load lands on "missing"/"empty" with no poll armed and stays there
  // forever. When the job is finally enqueued/completed, audio_segments_changed
  // fires — re-run the load (which re-checks speaker analysis) for the
  // still-selected segment so it recovers. Reuse the current generation so the
  // selection $effect's cleanup stays authoritative over any poll this arms.
  function reloadSelectedAudioTranscriptIfPending(): void {
    const id = selectedAudioSegmentId;
    if (id == null) return;
    if (
      selectedAudioTranscriptStatus !== "missing" &&
      selectedAudioTranscriptStatus !== "empty"
    ) {
      return;
    }
    void loadSelectedAudioSegmentTranscript(id, selectedAudioTranscriptGeneration);
  }

  const selectedAudioSpeakerRetryVisible = $derived(
    selectedAudioTranscriptStatus === "success" &&
      selectedAudioSpeakerTurnsError != null &&
      selectedAudioSpeakerAnalysisFailedJobId != null &&
      selectedAudioSegment != null,
  );
  const selectedAudioSpeakerRetryDisabled = $derived(
    !selectedAudioSpeakerRetryVisible ||
      selectedAudioSpeakerAnalysisRetryLoading ||
      selectedAudioSpeakerAnalysisRunning,
  );

  const selectedAudioTranscriptActionLabel = $derived(
    selectedAudioTranscriptStatus === "missing" ? "Run" : "Rerun",
  );
  const selectedAudioTranscriptActionDisabled = $derived(
    !selectedAudioSegment ||
      selectedAudioTranscriptRerunLoading ||
      selectedAudioTranscriptStatus === "loading" ||
      selectedAudioTranscriptStatus === "running",
  );
  const selectedAudioTranscriptActionTitle = $derived(
    selectedAudioTranscriptStatus === "loading"
        ? "Transcript is still loading"
        : selectedAudioTranscriptStatus === "running"
          ? "Transcription is queued or still processing"
          : selectedAudioSegment?.source === "systemAudio"
            ? `${selectedAudioTranscriptActionLabel} speech detection and transcription with current settings`
            : `${selectedAudioTranscriptActionLabel} transcription with current settings`,
  );

  async function reprocessSelectedAudioSegmentTranscript(): Promise<void> {
    const segment = selectedAudioSegment;
    if (!segment || selectedAudioTranscriptActionDisabled) return;
    const id = segment.id;

    selectedAudioTranscriptRerunLoading = true;
    selectedAudioTranscriptRerunError = null;
    try {
      const result = segment.source === "systemAudio"
        ? await invoke<SystemAudioSpeechActivityReprocessingResultDto>(
            "reprocess_system_audio_speech_activity",
            {
              request: {
                audioSegmentId: id,
              } satisfies ReprocessAudioSegmentTranscriptionRequest,
            },
          )
        : await invoke<AudioSegmentTranscriptionReprocessingResultDto>(
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
      selectedAudioSpeakerTurns = [];
      selectedAudioSpeakerAnalysisRunning = false;
      selectedAudioSpeakerAnalysisFailedJobId = null;
      selectedAudioSpeakerTurnsNotice = null;
      selectedAudioTranscriptError = null;
      selectedAudioTranscriptRerunError = null;
      if (result.job.processor === SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR) {
        const shouldContinue = await applySelectedSystemAudioSpeechActivityJob(
          id,
          gen,
          result.job,
        );
        if (shouldContinue && selectedAudioTranscriptIsCurrent(id, gen)) {
          await loadSelectedAudioSegmentTranscript(id, gen);
        }
        return;
      }
      await applySelectedAudioTranscriptJob(id, gen, result.job);
    } catch (err) {
      if (selectedAudioSegmentId !== id) return;
      selectedAudioTranscriptRerunError = humanizeError(err);
    } finally {
      if (selectedAudioSegmentId === id) {
        selectedAudioTranscriptRerunLoading = false;
      }
    }
  }

  async function reprocessSelectedAudioSegmentSpeakerAnalysis(): Promise<void> {
    const segment = selectedAudioSegment;
    if (!segment || selectedAudioSpeakerRetryDisabled) return;
    const id = segment.id;
    const gen = selectedAudioTranscriptGeneration;

    selectedAudioSpeakerAnalysisRetryLoading = true;
    speakerCorrectionError = null;
    try {
      const result = await invoke<{
        outcome: string;
        job: ProcessingJobDto;
      }>("reprocess_audio_segment_speaker_analysis", {
        request: {
          audioSegmentId: id,
        } satisfies ReprocessAudioSegmentSpeakerAnalysisRequest,
      });
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      await applySelectedAudioSpeakerAnalysisJob(id, gen, result.job);
    } catch (err) {
      if (!selectedAudioTranscriptIsCurrent(id, gen)) return;
      selectedAudioSpeakerTurnsError = humanizeError(err);
    } finally {
      if (selectedAudioSegmentId === id) {
        selectedAudioSpeakerAnalysisRetryLoading = false;
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
  // Whether the user has explicitly seeked (clicked a transcript segment, used
  // the scrubber, etc.) within the current segment. The active-segment
  // highlight is normally suppressed at the paused-at-zero start so a fresh
  // segment doesn't auto-highlight its first line; but once the user seeks —
  // even to the very first segment at 0ms while paused — that highlight should
  // resolve from the seek target. Reset whenever the selected segment changes.
  let audioHasSeeked = $state(false);

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
    selectedAudioTranscriptSegments.length === 0 ||
      (!audioIsPlaying && audioCurrentTime <= 0 && !audioHasSeeked)
      ? null
      : findActiveTranscriptSegmentIndex(selectedAudioTranscriptSegments, audioCurrentTime),
  );

  function findActiveSpeakerGroupIndex(
    groups: SpeakerTranscriptGroup[],
    currentTimeSeconds: number,
  ): number | null {
    if (!Number.isFinite(currentTimeSeconds) || currentTimeSeconds < 0) return null;
    const currentMs = Math.round(currentTimeSeconds * 1000);
    for (let index = groups.length - 1; index >= 0; index -= 1) {
      if (currentMs >= groups[index].startMs) return index;
    }
    return null;
  }

  const selectedAudioSpeakerActiveGroupIndex = $derived(
    selectedAudioSpeakerGroups.length === 0 ||
      (!audioIsPlaying && audioCurrentTime <= 0 && !audioHasSeeked)
      ? null
      : findActiveSpeakerGroupIndex(selectedAudioSpeakerGroups, audioCurrentTime),
  );

  $effect(() => {
    const activeIndex = selectedAudioSpeakerGroups.length > 0
      ? selectedAudioSpeakerActiveGroupIndex
      : selectedAudioTranscriptActiveSegmentIndex;
    const container = selectedAudioTranscriptContainerEl;
    if (activeIndex == null || !container) return;
    void tick().then(() => {
      const activeSegmentSelector = selectedAudioSpeakerGroups.length > 0
        ? `[data-speaker-group-index="${activeIndex}"]`
        : `[data-transcript-segment-index="${activeIndex}"]`;
      const activeSegment = container.querySelector<HTMLElement>(
        activeSegmentSelector,
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
    audioHasSeeked = false;
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
    // Mark an explicit seek so the active-segment highlight resolves even when
    // the target is the first segment at 0ms while paused.
    audioHasSeeked = true;
  }

  $effect(() => {
    const seekMs = pendingAudioSeekMs;
    const el = audioEl;
    if (seekMs == null || !el || selectedAudioSrc == null) return;
    const applySeek = () => {
      seekAudioToTimeMs(seekMs);
      pendingAudioSeekMs = null;
    };
    if (el.readyState >= 1) {
      applySeek();
      return;
    }
    el.addEventListener("loadedmetadata", applySeek, { once: true });
    return () => el.removeEventListener("loadedmetadata", applySeek);
  });

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

  // ─── Outside-click & wheel dismissal ─────────────────────────────────────
  // While the drawer is open, a pointerdown or wheel anywhere outside it
  // dismisses it (an open speaker-actions popover collapses first, so
  // dismissal is layered). The one exception: clicking an audio bar is a
  // SWITCH — the bar's own click handler reselects and the drawer stays open.
  // We listen for `wheel` rather than `scroll` because the rail is also
  // scrolled programmatically (scrub conversion, jump-to-frame, resize
  // re-anchoring, search-result lane moves) and those must not read as a
  // user's intent to dismiss.
  function onAudioDrawerOutsidePointerDown(event: PointerEvent) {
    if (selectedAudioSegmentId == null) return;
    const target = event.target;
    if (!(target instanceof Node)) return;
    if (audioDrawerEl?.contains(target)) return;
    const onAudioBar =
      target instanceof Element &&
      target.closest(".timeline-rail__audio-bar") != null;
    // A click on another segment's bar switches the drawer, never closes it —
    // collapse a transient speaker-actions popover first, then let the bar's
    // click handler reselect.
    if (onAudioBar) {
      if (speakerActionsOpenIndex != null) closeSpeakerActions();
      return;
    }
    if (speakerActionsOpenIndex != null) {
      closeSpeakerActions();
      return;
    }
    closeAudioDrawer();
  }

  function onAudioDrawerOutsideWheel(event: WheelEvent) {
    if (selectedAudioSegmentId == null) return;
    const target = event.target;
    if (!(target instanceof Node)) return;
    if (
      target instanceof Element &&
      target.closest(".audio-drawer__speaker-popover") != null
    ) {
      return;
    }
    // The popover is anchored to viewport coordinates, so any wheel outside
    // it (including scrolling the transcript underneath) collapses it before
    // it can drift away from its chip.
    if (speakerActionsOpenIndex != null) {
      closeSpeakerActions();
      return;
    }
    if (audioDrawerEl?.contains(target)) return;
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
    document.addEventListener("wheel", onAudioDrawerOutsideWheel, true);
    return () => {
      document.removeEventListener(
        "pointerdown",
        onAudioDrawerOutsidePointerDown,
        true,
      );
      document.removeEventListener("wheel", onAudioDrawerOutsideWheel, true);
    };
  });

  // Drop a stale selection if the segment no longer appears in the loaded
  // window. We compare ids rather than object identity because `audioSegments`
  // is rebuilt on every refresh.
  $effect(() => {
    if (selectedAudioSegmentId == null) return;
    if (!audioSegments.some((s) => s.id === selectedAudioSegmentId)) {
      if (selectedAudioSegmentPinned?.id === selectedAudioSegmentId) return;
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

  function closeAudioDrawer() {
    pendingAudioSeekMs = null;
    selectedAudioSegmentPinned = null;
    selectedAudioSegmentId = null;
  }

  function seekAudioBySeconds(deltaSeconds: number): void {
    const el = audioEl;
    if (!el) return;
    const duration = Number.isFinite(audioDuration) && audioDuration > 0
      ? audioDuration
      : Number.isFinite(el.duration) && el.duration > 0
        ? el.duration
        : Infinity;
    const nextTime = Math.max(0, Math.min(duration, el.currentTime + deltaSeconds));
    if (!Number.isFinite(nextTime)) return;
    el.currentTime = nextTime;
    audioCurrentTime = nextTime;
    audioHasSeeked = true;
  }

  function isAudioDrawerShortcutSuppressedTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(
      target.closest(
        'button, input, textarea, select, [contenteditable="true"], [role="button"], [role="slider"], [data-shortcuts-ignore]',
      ),
    );
  }

  function onAudioDrawerKeydown(e: KeyboardEvent) {
    if (selectedAudioSegmentId == null) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (speakerActionsOpenIndex != null) {
        closeSpeakerActions();
        return;
      }
      closeAudioDrawer();
      return;
    }

    if (
      matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.playPause), windowPlatform) &&
      !isAudioDrawerShortcutSuppressedTarget(e.target)
    ) {
      e.preventDefault();
      togglePlayPause();
      return;
    }

    if (
      !isAudioDrawerShortcutSuppressedTarget(e.target) &&
      (
        matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBack), windowPlatform) ||
        matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBackFast), windowPlatform) ||
        matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekForward), windowPlatform) ||
        matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekForwardFast), windowPlatform)
      )
    ) {
      e.preventDefault();
      if (matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBackFast), windowPlatform)) {
        seekAudioBySeconds(-30);
      } else if (matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekForwardFast), windowPlatform)) {
        seekAudioBySeconds(30);
      } else if (matchShortcut(e, effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBack), windowPlatform)) {
        seekAudioBySeconds(-5);
      } else {
        seekAudioBySeconds(5);
      }
      return;
    }

    trapTabKey(e, audioDrawerEl);
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

  // ─── "Play this moment" bridge ───────────────────────────────────────────
  // The audio segment (if any) whose [startUnixMs, endUnixMs] window contains
  // the active frame's capture time, so the user can jump straight from a
  // frame to hearing what was happening at that instant. Microphone wins when
  // both lanes cover the same moment; segments within a lane don't overlap.
  const activeFrameAudioMoment = $derived.by<{
    segment: AudioSegmentRecord;
    offsetMs: number;
  } | null>(() => {
    const frame = timelineActive;
    if (!frame || audioSegments.length === 0) return null;
    const capturedMs = parseCapturedAt(frame.capturedAt).getTime();
    if (!Number.isFinite(capturedMs)) return null;
    const overlapping = audioSegments.filter(
      (s) => capturedMs >= s.startUnixMs && capturedMs <= s.endUnixMs,
    );
    if (overlapping.length === 0) return null;
    const chosen =
      overlapping.find((s) => s.source === "microphone") ?? overlapping[0]!;
    return { segment: chosen, offsetMs: Math.max(0, capturedMs - chosen.startUnixMs) };
  });

  // Open the audio drawer on the segment that covers the active frame and seek
  // to the matching offset. Surfaces a status ack when no audio covers the
  // frame so the control never silently no-ops. Reuses the pinned-segment slot
  // so the selection survives a window that hasn't refreshed the segment list.
  function playActiveFrameMoment(): void {
    const moment = activeFrameAudioMoment;
    if (!moment) {
      setFrameActionStatus("No audio captured at this moment.", {
        detail: "This frame isn't covered by a microphone or system-audio segment.",
      });
      return;
    }
    selectedAudioSegmentPinned = moment.segment;
    pendingAudioSeekMs = moment.offsetMs;
    selectedAudioSegmentId = moment.segment.id;
  }

  // Custom tooltip state for the rail. `hoveredFrameId` tracks which slot the
  // pointer is currently over; `hoveredX` is the pointer x relative to the
  // rail-wrap so the tooltip can follow the cursor. When nothing is hovered we
  // fall back to showing the active frame's tooltip pinned at the center
  // cursor, so the user always has a readable timestamp readout for the frame
  // they're parked on.
  let timelineRailWrap: HTMLDivElement | null = $state(null);
  let timelineAudioLaneTrack: HTMLDivElement | null = $state(null);
  let activeTimelineSlotElement: HTMLElement | null = null;
  let timelineActiveTickSyncGeneration = 0;
  let hoveredFrameId = $state<number | null>(null);
  let hoveredX = $state<number | null>(null);

  $effect(() => {
    if (!timelineAudioLaneTrack) return;
    syncTimelineAudioLaneScroll(latestTimelineScrollLeft);
  });

  const tooltipFrame = $derived(
    hoveredFrameId != null
      ? (timelineFrameById.get(hoveredFrameId) ?? timelineActive)
      : timelineActive,
  );
  const tooltipIsHovered = $derived(
    hoveredFrameId != null && hoveredX != null,
  );

  // App identity of the frame shown in the centered readout. Keying the icon
  // and app-name on this makes them animate only when the *app* changes as the
  // user scrubs across an app boundary — not on every frame within one app
  // (the time/date update live inside the same DOM node).
  const tooltipAppKey = $derived(
    timelineAppIdentity(
      normalizedTimelineAppBundleId(tooltipFrame ?? ({} as FrameDto)),
      normalizedTimelineAppName(tooltipFrame ?? ({} as FrameDto)),
    ) ?? "__none__",
  );
  // Scrub direction (+1 = scrubbing toward older frames / "backward" in time,
  // -1 = toward newer). Drives which way the readout icon/name slide so the
  // swap reads as motion in the direction of travel rather than a hard cut.
  // Set synchronously in the scroll handler, so it's already current by the
  // time the resulting active-frame change re-keys the readout.
  let readoutScrubDirection = $state<1 | -1>(1);
  const READOUT_FLY_OFFSET_PX = 9;
  const READOUT_FLY_DURATION_MS = 190;
  // The readout transition is JS-driven (Svelte `fly`), so the CSS
  // `prefers-reduced-motion` rules elsewhere can't disable it — gate the
  // duration here instead so reduced-motion users get an instant swap.
  let readoutPrefersReducedMotion = $state(false);
  $effect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    readoutPrefersReducedMotion = mq.matches;
    const onChange = () => (readoutPrefersReducedMotion = mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });
  const readoutFlyDurationMs = $derived(
    readoutPrefersReducedMotion ? 0 : READOUT_FLY_DURATION_MS,
  );

  // Maximum scrollLeft for the rail. Track width = N * SLOT; rail has
  // symmetric viewport-sized margins on each side (`50cqi - 4px`) so the
  // first/last slot can sit under the centered cursor. That makes the total
  // scrollable width equal to `N*SLOT + (V - 8)`, hence `maxScroll = N*SLOT - 8`.
  // Clamped non-negative for the empty/short-list case.
  const timelineMaxScroll = $derived(
    Math.max(0, timelineFrames.length * TIMELINE_SLOT_WIDTH - 8),
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
      timelineRenderAnchorIndex -
        timelineHalfViewportSlots -
        TIMELINE_VIEWPORT_BUFFER,
    ),
  );
  const timelineWindowEnd = $derived(
    Math.min(
      timelineFrames.length,
      timelineRenderAnchorIndex +
        1 +
        timelineHalfViewportSlots +
        TIMELINE_VIEWPORT_BUFFER,
    ),
  );
  const timelineWindow = $derived(
    timelineFrames.slice(timelineWindowStart, timelineWindowEnd),
  );
  $effect(() => {
    timelineActiveTickSyncGeneration += 1;
    const gen = timelineActiveTickSyncGeneration;
    const activeIndex = timelineActiveIndex;
    if (
      timelineFrames.length === 0 ||
      activeIndex < timelineWindowStart ||
      activeIndex >= timelineWindowEnd
    ) {
      activeTimelineSlotElement?.classList.remove("timeline-rail__slot--active");
      activeTimelineSlotElement = null;
      return;
    }

    void tick().then(() => {
      if (gen !== timelineActiveTickSyncGeneration) return;
      syncTimelineActiveTick(activeIndex);
    });
  });
  $effect(() => {
    const frameCount = timelineFrames.length;
    if (frameCount === 0) {
      if (timelineRenderAnchorIndex !== 0) timelineRenderAnchorIndex = 0;
      return;
    }
    const clampedAnchor = Math.max(
      0,
      Math.min(frameCount - 1, timelineRenderAnchorIndex),
    );
    if (clampedAnchor !== timelineRenderAnchorIndex) {
      timelineRenderAnchorIndex = clampedAnchor;
      return;
    }
    const clampedActive = Math.max(
      0,
      Math.min(frameCount - 1, timelineActiveIndex),
    );
    if (
      Math.abs(clampedActive - timelineRenderAnchorIndex) >=
      TIMELINE_RENDER_REANCHOR_DISTANCE
    ) {
      timelineRenderAnchorIndex = clampedActive;
    }
  });
  const timelineAppRuns = $derived.by<TimelineAppRun[]>(() => {
    const startedAt = performance.now();
    const frames = timelineFrames;
    if (frames.length === 0) return [];

    const runs: TimelineAppRun[] = [];
    let runIdentity: string | null = null;
    let runBundleId: string | null = null;
    let runAppName: string | null = null;
    let runLastCapturedAtMs: number | null = null;
    let runStart = 0;

    function flushRun(endExclusive: number): void {
      if (!runIdentity) return;
      const runEnd = endExclusive - 1;
      const frameCount = runEnd - runStart + 1;
      const widthPx = frameCount * TIMELINE_SLOT_WIDTH;
      const label = runAppName ?? runBundleId ?? "Unknown app";
      const variant = frameCount === 1 ? "single" : "range";
      runs.push({
        key: timelineAppGroupKey(runIdentity, frames, runEnd),
        boundaryFrameId: endExclusive < frames.length ? frames[runEnd]?.id ?? null : null,
        bundleId: runBundleId,
        appName: runAppName,
        label,
        frameCount,
        startIndex: runStart,
        endIndex: runEnd,
        rightPx: runStart * TIMELINE_SLOT_WIDTH,
        widthPx,
        fallback: timelineAppIconFallback(runAppName, runBundleId),
        variant,
      });
    }

    for (let i = 0; i < frames.length; i++) {
      const frame = frames[i]!;
      const bundleId = normalizedTimelineAppBundleId(frame);
      const appName = normalizedTimelineAppName(frame);
      const identity = timelineAppIdentity(bundleId, appName);
      const capturedAtMs = parseCapturedAt(frame.capturedAt).getTime();
      const validCapturedAtMs = Number.isFinite(capturedAtMs) ? capturedAtMs : null;
      if (!identity) {
        flushRun(i);
        runIdentity = null;
        runBundleId = null;
        runAppName = null;
        runLastCapturedAtMs = null;
        runStart = i + 1;
        continue;
      }

      if (identity === runIdentity) {
        const shouldSplitForTimeGap = runLastCapturedAtMs != null &&
          validCapturedAtMs != null &&
          Math.abs(runLastCapturedAtMs - validCapturedAtMs) > TIMELINE_APP_GROUP_MAX_GAP_MS;
        if (shouldSplitForTimeGap) {
          flushRun(i);
          runIdentity = identity;
          runBundleId = bundleId;
          runAppName = appName;
          runLastCapturedAtMs = validCapturedAtMs;
          runStart = i;
          continue;
        }
        if (!runBundleId && bundleId) runBundleId = bundleId;
        if (!runAppName && appName) runAppName = appName;
        runLastCapturedAtMs = validCapturedAtMs;
        continue;
      }

      flushRun(i);
      runIdentity = identity;
      runBundleId = bundleId;
      runAppName = appName;
      runLastCapturedAtMs = validCapturedAtMs;
      runStart = i;
    }
    flushRun(frames.length);

    scrubPerfLogSlow("timeline_app_runs", performance.now() - startedAt, {
      frames: frames.length,
      runs: runs.length,
    });
    return runs;
  });

  function firstTimelineAppRunEndingAtOrAfter(
    runs: TimelineAppRun[],
    frameIndex: number,
  ): number {
    let low = 0;
    let high = runs.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if ((runs[mid]?.endIndex ?? -1) < frameIndex) {
        low = mid + 1;
      } else {
        high = mid;
      }
    }
    return low;
  }

  const timelineAppGroups = $derived.by<TimelineAppGroup[]>(() => {
    const startedAt = performance.now();
    const runs = timelineAppRuns;
    const windowStart = timelineWindowStart;
    const windowEnd = timelineWindowEnd;
    if (runs.length === 0 || windowStart >= windowEnd) return [];

    const groups: TimelineAppGroup[] = [];
    const iconSizePx = 20;
    const firstRun = firstTimelineAppRunEndingAtOrAfter(runs, windowStart);
    for (let i = firstRun; i < runs.length; i++) {
      const run = runs[i]!;
      if (run.startIndex >= windowEnd) break;
      if (run.endIndex < windowStart) continue;

      const visibleStart = Math.max(run.startIndex, windowStart);
      const visibleEnd = Math.min(run.endIndex, windowEnd - 1);
      const visibleWidthPx = (visibleEnd - visibleStart + 1) * TIMELINE_SLOT_WIDTH;
      const iconCenterOffsetFromRight =
        (visibleStart - run.startIndex) * TIMELINE_SLOT_WIDTH + visibleWidthPx / 2;
      groups.push({
        ...run,
        iconLeftPx: Math.max(
          2,
          Math.min(
            Math.max(2, run.widthPx - iconSizePx - 2),
            run.widthPx - iconCenterOffsetFromRight - iconSizePx / 2,
          ),
        ),
        iconSrc: run.bundleId ? timelineAppIconSrc(run.bundleId) : null,
        showIcon: run.variant === "range" && visibleWidthPx >= iconSizePx + 24,
      });
    }

    scrubPerfLogSlow("timeline_app_groups", performance.now() - startedAt, {
      runs: runs.length,
      windowStart,
      windowEnd,
      groups: groups.length,
    });
    return groups;
  });
  const timelineAppGroupBoundaryFrameIds = $derived.by<Set<number>>(() => {
    return new Set(
      timelineAppGroups
        .map((group) => group.boundaryFrameId)
        .filter((id): id is number => id != null),
    );
  });

  $effect(() => {
    const bundleIds = timelineAppGroups.map((group) => group.bundleId);
    void resolveTimelineAppIcons(bundleIds);
  });

  function normalizedTimelineAppBundleId(frame: FrameDto): string | null {
    const bundleId = frame.appBundleId?.trim();
    return bundleId ? bundleId : null;
  }

  function normalizedTimelineAppName(frame: FrameDto): string | null {
    const appName = frame.appName?.trim();
    return appName ? appName : null;
  }

  function timelineAppIdentity(bundleId: string | null, appName: string | null): string | null {
    if (bundleId) return `bundle:${bundleId}`;
    return appName ? `name:${appName.toLocaleLowerCase()}` : null;
  }

  function timelineAppGroupKey(
    identity: string,
    frames: FrameDto[],
    runEnd: number,
  ): string {
    // New frames prepend at the rail head, shifting every absolute index.
    // Anchor the keyed DOM node to the oldest frame in the run so existing
    // app icons are updated in place instead of remounting their <img>.
    const oldestFrameId = frames[runEnd]?.id;
    return `${identity}:oldest:${oldestFrameId ?? runEnd}`;
  }

  function timelineAppIconFallback(
    appName: string | null | undefined,
    bundleId: string | null | undefined,
  ): string {
    return ((appName ?? "").trim() || (bundleId ?? "").trim() || "?").slice(0, 1).toUpperCase();
  }

  function timelineAppIconSrc(bundleId: string): string | null {
    const iconPath = timelineAppIconPathsByBundleId[bundleId];
    return iconPath ? convertFileSrc(iconPath) : null;
  }

  function timelineAppGroupTitle(group: TimelineAppGroup): string {
    const countLabel = `${group.frameCount} frame${group.frameCount === 1 ? "" : "s"}`;
    return `${group.label} · ${countLabel}`;
  }

  function timelineFrameAppLabel(frame: FrameDto | null | undefined): string | null {
    if (!frame) return null;
    return normalizedTimelineAppName(frame) ?? normalizedTimelineAppBundleId(frame);
  }

  function timelineFrameAppIconSrc(frame: FrameDto | null | undefined): string | null {
    const bundleId = frame ? normalizedTimelineAppBundleId(frame) : null;
    return bundleId ? timelineAppIconSrc(bundleId) : null;
  }

  function timelineFrameAppFallback(frame: FrameDto | null | undefined): string {
    return timelineAppIconFallback(
      frame ? normalizedTimelineAppName(frame) : null,
      frame ? normalizedTimelineAppBundleId(frame) : null,
    );
  }

  function uniqueTimelineAppBundleIds(bundleIds: Array<string | null | undefined>): string[] {
    return [...new Set(bundleIds.map((bundleId) => bundleId?.trim() ?? "").filter(Boolean))];
  }

  async function resolveTimelineAppIcons(bundleIds: Array<string | null | undefined>): Promise<void> {
    const unresolvedBundleIds = uniqueTimelineAppBundleIds(bundleIds).filter((bundleId) => (
      !timelineAppIconPathsByBundleId[bundleId] && !requestedTimelineAppIconBundleIds.has(bundleId)
    ));
    if (unresolvedBundleIds.length === 0) return;
    for (const bundleId of unresolvedBundleIds) requestedTimelineAppIconBundleIds.add(bundleId);

    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: unresolvedBundleIds },
      });
      const nextIconPaths = { ...timelineAppIconPathsByBundleId };
      let changed = false;
      for (const icon of icons) {
        if (!icon.iconPath || nextIconPaths[icon.bundleId] === icon.iconPath) continue;
        nextIconPaths[icon.bundleId] = icon.iconPath;
        changed = true;
      }
      if (changed) {
        timelineAppIconPathsByBundleId = nextIconPaths;
      }
    } catch {
      for (const bundleId of unresolvedBundleIds) requestedTimelineAppIconBundleIds.delete(bundleId);
    }
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
    if (
      previewCache.size === 0 &&
      previewMimeTypeCache.size === 0 &&
      previewSourceKindCache.size === 0 &&
      previewRedactionCountCache.size === 0 &&
      previewLoadMsCache.size === 0 &&
      scrubPreviewCache.size === 0 &&
      scrubPreviewMimeTypeCache.size === 0 &&
      scrubPreviewSourceKindCache.size === 0 &&
      scrubPreviewLoadMsCache.size === 0 &&
      scrubPreviewFailedAt.size === 0
    ) {
      return;
    }
    const keep = new Set(frames.map((frame) => frame.id));
    const next = new Map<number, string>();
    for (const [frameId, url] of previewCache) {
      if (keep.has(frameId)) next.set(frameId, url);
    }
    if (next.size !== previewCache.size) {
      previewCache = next;
    }
    const nextPreviewMimeTypes = new Map<number, string>();
    for (const [frameId, mimeType] of previewMimeTypeCache) {
      if (keep.has(frameId)) nextPreviewMimeTypes.set(frameId, mimeType);
    }
    if (nextPreviewMimeTypes.size !== previewMimeTypeCache.size) {
      previewMimeTypeCache = nextPreviewMimeTypes;
    }
    const nextPreviewSourceKinds = new Map<number, FramePreviewDto["sourceKind"]>();
    for (const [frameId, sourceKind] of previewSourceKindCache) {
      if (keep.has(frameId)) nextPreviewSourceKinds.set(frameId, sourceKind);
    }
    if (nextPreviewSourceKinds.size !== previewSourceKindCache.size) {
      previewSourceKindCache = nextPreviewSourceKinds;
    }
    const nextPreviewRedactionCounts = new Map<number, number>();
    for (const [frameId, count] of previewRedactionCountCache) {
      if (keep.has(frameId)) nextPreviewRedactionCounts.set(frameId, count);
    }
    if (nextPreviewRedactionCounts.size !== previewRedactionCountCache.size) {
      previewRedactionCountCache = nextPreviewRedactionCounts;
    }
    const nextPreviewLoadMs = new Map<number, number>();
    for (const [frameId, loadMs] of previewLoadMsCache) {
      if (keep.has(frameId)) nextPreviewLoadMs.set(frameId, loadMs);
    }
    if (nextPreviewLoadMs.size !== previewLoadMsCache.size) {
      previewLoadMsCache = nextPreviewLoadMs;
    }
    const nextScrub = new Map<number, string>();
    for (const [frameId, url] of scrubPreviewCache) {
      if (keep.has(frameId)) nextScrub.set(frameId, url);
    }
    if (nextScrub.size !== scrubPreviewCache.size) {
      scrubPreviewCache = nextScrub;
    }
    const nextScrubMimeTypes = new Map<number, string>();
    for (const [frameId, mimeType] of scrubPreviewMimeTypeCache) {
      if (keep.has(frameId)) nextScrubMimeTypes.set(frameId, mimeType);
    }
    if (nextScrubMimeTypes.size !== scrubPreviewMimeTypeCache.size) {
      scrubPreviewMimeTypeCache = nextScrubMimeTypes;
    }
    const nextScrubSourceKinds = new Map<number, FramePreviewDto["sourceKind"]>();
    for (const [frameId, sourceKind] of scrubPreviewSourceKindCache) {
      if (keep.has(frameId)) nextScrubSourceKinds.set(frameId, sourceKind);
    }
    if (nextScrubSourceKinds.size !== scrubPreviewSourceKindCache.size) {
      scrubPreviewSourceKindCache = nextScrubSourceKinds;
    }
    const nextScrubLoadMs = new Map<number, number>();
    for (const [frameId, loadMs] of scrubPreviewLoadMsCache) {
      if (keep.has(frameId)) nextScrubLoadMs.set(frameId, loadMs);
    }
    if (nextScrubLoadMs.size !== scrubPreviewLoadMsCache.size) {
      scrubPreviewLoadMsCache = nextScrubLoadMs;
    }
    const nextScrubFailures = new Map<number, number>();
    for (const [frameId, failedAt] of scrubPreviewFailedAt) {
      if (keep.has(frameId)) nextScrubFailures.set(frameId, failedAt);
    }
    if (nextScrubFailures.size !== scrubPreviewFailedAt.size) {
      scrubPreviewFailedAt = nextScrubFailures;
    }
  }

  function syncTimelineAudioLaneScroll(scrollLeft: number): void {
    if (!timelineAudioLaneTrack) return;
    timelineAudioLaneTrack.style.transform = `translateX(${-scrollLeft}px)`;
  }

  function syncTimelineActiveTick(activeIndex: number): void {
    const next = timelineRail?.querySelector<HTMLElement>(
      `.timeline-rail__slot[data-timeline-slot-index="${activeIndex}"]`,
    ) ?? null;
    if (activeTimelineSlotElement === next) return;
    activeTimelineSlotElement?.classList.remove("timeline-rail__slot--active");
    next?.classList.add("timeline-rail__slot--active");
    activeTimelineSlotElement = next;
  }

  function commitTimelineScrollPosition(scrollLeft: number): void {
    latestTimelineScrollLeft = scrollLeft;
    syncTimelineAudioLaneScroll(scrollLeft);
    timelineScrollLeft = scrollLeft;
  }

  async function syncTimelineScrollToActiveFrame(
    opts: { animate?: boolean } = {},
  ): Promise<void> {
    await tick();
    if (!timelineRail) {
      commitTimelineScrollPosition(0);
      return;
    }
    const max = timelineRail.scrollWidth - timelineRail.clientWidth;
    const targetScrollLeft = Math.max(
      0,
      Math.min(max, max - timelineActiveIndex * TIMELINE_SLOT_WIDTH),
    );
    // Explicit user jumps animate the playhead to the new moment (matching the
    // arrow-key scrub's smooth scroll); every other caller — initial load,
    // refresh, resize recovery, head poll — hard-pins to avoid spurious motion.
    // Honor `prefers-reduced-motion` by falling back to the instant set.
    const animate =
      opts.animate === true &&
      typeof timelineRail.scrollTo === "function" &&
      !(
        typeof window !== "undefined" &&
        typeof window.matchMedia === "function" &&
        window.matchMedia("(prefers-reduced-motion: reduce)").matches
      );
    if (animate) {
      timelineRail.scrollTo({ left: targetScrollLeft, behavior: "smooth" });
    } else {
      timelineRail.scrollLeft = targetScrollLeft;
    }
    commitTimelineScrollPosition(targetScrollLeft);
  }

  function formatCapturedAt(ts: string): string {
    const d = parseCapturedAt(ts);
    if (isNaN(d.getTime())) return ts;
    return d.toLocaleString();
  }

  // Delegates to the shared compact formatter (identical behavior to the
  // answer-source / search-result cards). Kept as a thin local wrapper so the
  // many `formatCapturedAtCompact(...)` call sites in this file stay stable.
  function formatCapturedAtCompact(ts: string): string {
    return formatTimestampCompact(ts);
  }

  function formatUnixMs(ms: number): string {
    const d = new Date(ms);
    if (isNaN(d.getTime())) return "unknown";
    return d.toLocaleString();
  }

  /** Time-of-day for the rail readout, where the date is shown alongside it.
   *  Split out from {@link formatCapturedAt} so the readout can lead with the
   *  precise time and de-emphasize the calendar date. */
  function formatCapturedTimeOnly(ts: string): string {
    const d = parseCapturedAt(ts);
    if (isNaN(d.getTime())) return ts;
    return d.toLocaleTimeString();
  }

  function formatCapturedDateOnly(ts: string): string {
    const d = parseCapturedAt(ts);
    if (isNaN(d.getTime())) return "";
    return d.toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    });
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
    // Error-tone messages must persist until the user dismisses them (X / hover
    // is no longer the only escape) — only neutral success/progress acks
    // auto-clear. The banner carries an explicit close affordance for errors.
    if (frameActionStatus.tone === "error") return;
    frameActionStatusTimer = setTimeout(() => {
      frameActionStatus = null;
      frameActionStatusTimer = null;
    }, 2200);
  }

  function dismissFrameActionStatus() {
    clearFrameActionStatusTimer();
    frameActionStatus = null;
    frameActionStatusHovered = false;
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

  function scheduleLatestActivePreview(frameId: number, generation: number, delayMs: number): void {
    clearActivePreviewFetchTimer();
    activePreviewFetchTimer = setTimeout(() => {
      activePreviewFetchTimer = null;
      if (generation !== activePreviewFetchGeneration) return;
      if (!isTimelineActiveFrame(frameId)) return;
      void ensureLatestActivePreview(frameId, generation);
    }, delayMs);
  }

  async function cancelActivePreviewVideoRequests(): Promise<void> {
    try {
      const cancelled = await invoke<number>("cancel_active_frame_preview_video_requests");
      if (cancelled > 0) {
        scrubPerfLog("preview_video_generation_cancelled", { cancelled });
      }
    } catch (error) {
      scrubPerfLog("preview_video_generation_cancel_failed", {
        message: humanizeError(error),
      });
    }
  }

  function clearScrubPreviewFetchTimer(): void {
    if (scrubPreviewFetchTimer != null) {
      clearTimeout(scrubPreviewFetchTimer);
      scrubPreviewFetchTimer = null;
    }
    scheduledScrubPreviewActiveIndex = null;
    scheduledScrubPreviewGeneration = 0;
  }

  function clearScrubPreviewWarmTimer(): void {
    if (scrubPreviewWarmTimer != null) {
      clearTimeout(scrubPreviewWarmTimer);
      scrubPreviewWarmTimer = null;
    }
  }

  function scheduleLatestScrubPreviews(activeIndex: number, generation: number, delayMs: number): void {
    scheduledScrubPreviewActiveIndex = activeIndex;
    scheduledScrubPreviewGeneration = generation;
    if (scrubPreviewFetchTimer != null) return;
    scrubPreviewFetchTimer = setTimeout(() => {
      scrubPreviewFetchTimer = null;
      const scheduledActiveIndex = scheduledScrubPreviewActiveIndex;
      const scheduledGeneration = scheduledScrubPreviewGeneration;
      scheduledScrubPreviewActiveIndex = null;
      scheduledScrubPreviewGeneration = 0;
      if (scheduledActiveIndex == null) return;
      if (scheduledGeneration !== scrubPreviewFetchGeneration) return;
      void ensureLatestScrubPreviews(scheduledActiveIndex, scheduledGeneration);
    }, delayMs);
  }

  function scheduleScrubPreviewWarm(activeIndex: number, generation: number): void {
    clearScrubPreviewWarmTimer();
    scrubPreviewWarmTimer = setTimeout(() => {
      scrubPreviewWarmTimer = null;
      if (generation !== scrubPreviewFetchGeneration) return;
      void warmLatestScrubPreviews(activeIndex, generation);
    }, ACTIVE_PREVIEW_SCRUB_WARM_SETTLE_MS);
  }

  function trimPreviewCache(): void {
    if (previewCache.size <= PREVIEW_CACHE_MAX_ENTRIES) return;
    const next = new Map(previewCache);
    const evictedFrameIds: number[] = [];
    while (next.size > PREVIEW_CACHE_MAX_ENTRIES) {
      const oldestFrameId = next.keys().next().value;
      if (oldestFrameId == null) break;
      next.delete(oldestFrameId);
      evictedFrameIds.push(oldestFrameId);
    }
    previewCache = next;
    if (evictedFrameIds.length > 0) {
      const nextMimeTypes = new Map(previewMimeTypeCache);
      const nextSourceKinds = new Map(previewSourceKindCache);
      const nextRedactionCounts = new Map(previewRedactionCountCache);
      const nextLoadMs = new Map(previewLoadMsCache);
      const nextFailures = new Map(previewFailedAt);
      for (const frameId of evictedFrameIds) {
        nextMimeTypes.delete(frameId);
        nextSourceKinds.delete(frameId);
        nextRedactionCounts.delete(frameId);
        nextLoadMs.delete(frameId);
        nextFailures.delete(frameId);
      }
      previewMimeTypeCache = nextMimeTypes;
      previewSourceKindCache = nextSourceKinds;
      previewRedactionCountCache = nextRedactionCounts;
      previewLoadMsCache = nextLoadMs;
      previewFailedAt = nextFailures;
    }
  }

  function trimScrubPreviewCache(): void {
    if (scrubPreviewCache.size <= PREVIEW_CACHE_MAX_ENTRIES) return;
    const next = new Map(scrubPreviewCache);
    const evictedFrameIds: number[] = [];
    while (next.size > PREVIEW_CACHE_MAX_ENTRIES) {
      const oldestFrameId = next.keys().next().value;
      if (oldestFrameId == null) break;
      next.delete(oldestFrameId);
      evictedFrameIds.push(oldestFrameId);
    }
    scrubPreviewCache = next;
    if (evictedFrameIds.length > 0) {
      const nextMimeTypes = new Map(scrubPreviewMimeTypeCache);
      const nextSourceKinds = new Map(scrubPreviewSourceKindCache);
      const nextLoadMs = new Map(scrubPreviewLoadMsCache);
      const nextFailures = new Map(scrubPreviewFailedAt);
      for (const frameId of evictedFrameIds) {
        nextMimeTypes.delete(frameId);
        nextSourceKinds.delete(frameId);
        nextLoadMs.delete(frameId);
        nextFailures.delete(frameId);
      }
      scrubPreviewMimeTypeCache = nextMimeTypes;
      scrubPreviewSourceKindCache = nextSourceKinds;
      scrubPreviewLoadMsCache = nextLoadMs;
      scrubPreviewFailedAt = nextFailures;
    }
  }

  function touchPreviewCache(
    frameId: number,
    url: string,
    metadata?: {
      mimeType: string;
      sourceKind: FramePreviewDto["sourceKind"];
      secretRedactionCount?: number;
      loadMs: number;
    },
  ): void {
    const next = new Map(previewCache);
    next.delete(frameId);
    next.set(frameId, url);
    previewCache = next;
    if (metadata) {
      const nextMimeTypes = new Map(previewMimeTypeCache);
      nextMimeTypes.set(frameId, metadata.mimeType);
      previewMimeTypeCache = nextMimeTypes;
      const nextSourceKinds = new Map(previewSourceKindCache);
      nextSourceKinds.set(frameId, metadata.sourceKind);
      previewSourceKindCache = nextSourceKinds;
      const nextRedactionCounts = new Map(previewRedactionCountCache);
      nextRedactionCounts.set(frameId, metadata.secretRedactionCount ?? 0);
      previewRedactionCountCache = nextRedactionCounts;
      const nextLoadMs = new Map(previewLoadMsCache);
      nextLoadMs.set(frameId, metadata.loadMs);
      previewLoadMsCache = nextLoadMs;
    }
    trimPreviewCache();
    if (timelineActive?.id === frameId) {
      refreshScrubbingTimelinePreviewDisplay();
    }
  }

  function touchScrubPreviewCache(
    frameId: number,
    url: string,
    metadata?: {
      mimeType: string;
      sourceKind: FramePreviewDto["sourceKind"];
      loadMs: number;
    },
  ): void {
    const next = new Map(scrubPreviewCache);
    next.delete(frameId);
    next.set(frameId, url);
    scrubPreviewCache = next;
    if (metadata) {
      const nextMimeTypes = new Map(scrubPreviewMimeTypeCache);
      nextMimeTypes.set(frameId, metadata.mimeType);
      scrubPreviewMimeTypeCache = nextMimeTypes;
      const nextSourceKinds = new Map(scrubPreviewSourceKindCache);
      nextSourceKinds.set(frameId, metadata.sourceKind);
      scrubPreviewSourceKindCache = nextSourceKinds;
      const nextLoadMs = new Map(scrubPreviewLoadMsCache);
      nextLoadMs.set(frameId, metadata.loadMs);
      scrubPreviewLoadMsCache = nextLoadMs;
    }
    trimScrubPreviewCache();
    if (timelineActive?.id === frameId) {
      refreshScrubbingTimelinePreviewDisplay();
    }
  }

  function scrubPreviewIntervalKey(interval: Pick<ScrubPreviewAvailabilityIntervalDto, "segmentCacheKey" | "intervalStartVideoOffsetMs">): string {
    return `${interval.segmentCacheKey}:${interval.intervalStartVideoOffsetMs}`;
  }

  function scrubPreviewIntervalForFrame(frame: FrameDto | null): ScrubPreviewAvailabilityIntervalDto | null {
    if (!frame) return null;
    const capturedAtMs = Date.parse(frame.capturedAt);
    if (!Number.isFinite(capturedAtMs)) return null;
    let selected: ScrubPreviewAvailabilityIntervalDto | null = null;
    for (const interval of scrubPreviewIntervalCache.values()) {
      if (
        interval.preview &&
        capturedAtMs >= interval.intervalStartUnixMs &&
        capturedAtMs < interval.intervalEndUnixMs
      ) {
        if (!selected || interval.intervalStartUnixMs > selected.intervalStartUnixMs) {
          selected = interval;
        }
      }
    }
    return selected;
  }

  function touchScrubPreviewIntervalCache(interval: ScrubPreviewAvailabilityIntervalDto): void {
    const key = scrubPreviewIntervalKey(interval);
    const next = new Map(scrubPreviewIntervalCache);
    next.delete(key);
    next.set(key, interval);
    while (next.size > 1500) {
      const oldestKey = next.keys().next().value;
      if (oldestKey == null) break;
      next.delete(oldestKey);
    }
    scrubPreviewIntervalCache = next;
  }

  function dropScrubPreviewIntervalCacheEntry(interval: ScrubPreviewAvailabilityIntervalDto): void {
    const key = scrubPreviewIntervalKey(interval);
    if (!scrubPreviewIntervalCache.has(key)) return;
    const next = new Map(scrubPreviewIntervalCache);
    next.delete(key);
    scrubPreviewIntervalCache = next;
  }

  function scrubPreviewTimeWindowAround(activeIndex: number): { startUnixMs: number; endUnixMs: number } | null {
    const options = scrubPreviewWindowForVelocity();
    const start = Math.max(0, activeIndex - options.radius);
    const end = Math.min(timelineFrames.length - 1, activeIndex + options.radius);
    const times = timelineFrames
      .slice(start, end + 1)
      .map((frame) => Date.parse(frame.capturedAt))
      .filter(Number.isFinite);
    if (times.length === 0) return null;
    return {
      startUnixMs: Math.min(...times) - 1000,
      endUnixMs: Math.max(...times) + 1000,
    };
  }

  function rememberPreviewFailure(frameId: number): void {
    const next = new Map(previewFailedAt);
    next.set(frameId, Date.now());
    previewFailedAt = next;
  }

  function rememberScrubPreviewFailure(frameId: number): void {
    const next = new Map(scrubPreviewFailedAt);
    next.set(frameId, Date.now());
    scrubPreviewFailedAt = next;
  }

  function clearPreviewFailure(frameId: number): void {
    if (!previewFailedAt.has(frameId)) return;
    const next = new Map(previewFailedAt);
    next.delete(frameId);
    previewFailedAt = next;
  }

  function clearScrubPreviewFailure(frameId: number): void {
    if (!scrubPreviewFailedAt.has(frameId)) return;
    const next = new Map(scrubPreviewFailedAt);
    next.delete(frameId);
    scrubPreviewFailedAt = next;
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
    if (previewSourceKindCache.has(frameId)) {
      const nextSourceKinds = new Map(previewSourceKindCache);
      nextSourceKinds.delete(frameId);
      previewSourceKindCache = nextSourceKinds;
    }
    if (previewRedactionCountCache.has(frameId)) {
      const nextRedactionCounts = new Map(previewRedactionCountCache);
      nextRedactionCounts.delete(frameId);
      previewRedactionCountCache = nextRedactionCounts;
    }
    if (previewLoadMsCache.has(frameId)) {
      const nextLoadMs = new Map(previewLoadMsCache);
      nextLoadMs.delete(frameId);
      previewLoadMsCache = nextLoadMs;
    }
  }

  function dropScrubPreviewCacheEntry(frameId: number): void {
    if (scrubPreviewCache.has(frameId)) {
      const next = new Map(scrubPreviewCache);
      next.delete(frameId);
      scrubPreviewCache = next;
    }
    if (scrubPreviewMimeTypeCache.has(frameId)) {
      const nextMimeTypes = new Map(scrubPreviewMimeTypeCache);
      nextMimeTypes.delete(frameId);
      scrubPreviewMimeTypeCache = nextMimeTypes;
    }
    if (scrubPreviewSourceKindCache.has(frameId)) {
      const nextSourceKinds = new Map(scrubPreviewSourceKindCache);
      nextSourceKinds.delete(frameId);
      scrubPreviewSourceKindCache = nextSourceKinds;
    }
    if (scrubPreviewLoadMsCache.has(frameId)) {
      const nextLoadMs = new Map(scrubPreviewLoadMsCache);
      nextLoadMs.delete(frameId);
      scrubPreviewLoadMsCache = nextLoadMs;
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

  function recentlyFailedScrubPreview(frameId: number): boolean {
    const failedAt = scrubPreviewFailedAt.get(frameId);
    if (failedAt == null) return false;
    if (Date.now() - failedAt < PREVIEW_FAILURE_CACHE_TTL_MS) {
      return true;
    }
    clearScrubPreviewFailure(frameId);
    return false;
  }

  function scrubPreviewWindowForVelocity(): { radius: number; maxUncached: number } {
    if (previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_EXTREME_SCRUB_PX_PER_MS) {
      return {
        radius: ACTIVE_PREVIEW_SCRUB_VERY_FAST_RADIUS,
        maxUncached: ACTIVE_PREVIEW_SCRUB_VERY_FAST_MAX_UNCACHED,
      };
    }
    if (previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_VERY_FAST_SCRUB_PX_PER_MS) {
      return {
        radius: ACTIVE_PREVIEW_SCRUB_FAST_RADIUS,
        maxUncached: ACTIVE_PREVIEW_SCRUB_FAST_MAX_UNCACHED,
      };
    }
    if (previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS) {
      return {
        radius: ACTIVE_PREVIEW_SCRUB_MEDIUM_RADIUS,
        maxUncached: ACTIVE_PREVIEW_SCRUB_MEDIUM_MAX_UNCACHED,
      };
    }
    return {
      radius: ACTIVE_PREVIEW_SCRUB_RADIUS,
      maxUncached: ACTIVE_PREVIEW_SCRUB_MAX_UNCACHED,
    };
  }

  function scrubPreviewScheduleDelayMs(): number {
    return previewScrubVelocityPxPerMs >= ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS
      ? ACTIVE_PREVIEW_SCRUB_FAST_REQUEST_INTERVAL_MS
      : ACTIVE_PREVIEW_SCRUB_REQUEST_INTERVAL_MS;
  }

  function scrubPreviewFrameIdsAround(
    activeIndex: number,
    options: { radius: number; maxUncached: number } = scrubPreviewWindowForVelocity(),
  ): number[] {
    const ids: number[] = [];
    const start = Math.max(0, activeIndex - options.radius);
    const end = Math.min(timelineFrames.length - 1, activeIndex + options.radius);
    const indexes = [activeIndex];
    for (let distance = 1; activeIndex - distance >= start || activeIndex + distance <= end; distance += 1) {
      if (activeIndex - distance >= start) indexes.push(activeIndex - distance);
      if (activeIndex + distance <= end) indexes.push(activeIndex + distance);
    }
    for (const index of indexes) {
      const frame = timelineFrames[index];
      const id = frame?.id;
      if (
        id == null ||
        scrubPreviewCache.has(id) ||
        Boolean(scrubPreviewIntervalForFrame(frame)?.preview) ||
        scrubPreviewInFlight.has(id) ||
        recentlyFailedScrubPreview(id)
      ) {
        continue;
      }
      ids.push(id);
      if (ids.length >= options.maxUncached) break;
    }
    return ids;
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
      case "scrub_preview":
        return "scrub";
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

  async function confirmOriginalMediaAccessIfRedacted(frameId: number): Promise<boolean> {
    const redactionCount = previewRedactionCountCache.get(frameId) ?? 0;
    if (redactionCount <= 0) return true;
    return await ask(
      "Original capture may still contain redacted secrets. Continue?",
      {
        title: "Open Original Capture",
        kind: "warning",
        okLabel: "Continue",
        cancelLabel: "Cancel",
      },
    );
  }

  async function copyActiveFrameImage(): Promise<void> {
    if (frameImageActionBusy) return;
    const frame = timelineActive;
    const previewUrl = frame ? previewCache.get(frame.id) : null;
    if (!frame || !previewUrl) {
      setFrameActionStatus("Frame preview not ready yet");
      return;
    }

    frameImageActionBusy = "copy";
    try {
      if (!(await confirmOriginalMediaAccessIfRedacted(frame.id))) return;
      setFrameActionStatus("Copying…");
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
    } finally {
      frameImageActionBusy = null;
    }
  }

  async function downloadActiveFrameImage(): Promise<void> {
    if (frameImageActionBusy) return;
    const frame = timelineActive;
    const previewUrl = frame ? previewCache.get(frame.id) : null;
    if (!frame || !previewUrl) {
      setFrameActionStatus("Frame preview not ready yet");
      return;
    }

    frameImageActionBusy = "download";
    try {
      if (!(await confirmOriginalMediaAccessIfRedacted(frame.id))) return;
      setFrameActionStatus("Saving…");
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
    } finally {
      frameImageActionBusy = null;
    }
  }

  // Copy every recognized OCR text region for the active frame as one block.
  // The on-image overlay only reveals text on hover, so this gives a
  // discoverable, single-shot way to lift all recognized text — with visible
  // in-flight + success/failure feedback through the stage status banner.
  async function copyAllRecognizedText(): Promise<void> {
    if (ocrCopyAllBusy) return;
    const text = ocrObservations
      .map((obs) => obs.text)
      .join("\n")
      .trim();
    if (!text) {
      setFrameActionStatus("No recognized text to copy");
      return;
    }
    ocrCopyAllBusy = true;
    setFrameActionStatus("Copying text…");
    try {
      await writeText(text);
      const count = ocrObservations.length;
      setFrameActionStatus(`Copied ${count} text region${count === 1 ? "" : "s"}`);
      stageActionsMenuOpen = false;
    } catch (err) {
      setFrameActionStatus(
        `Copy failed: ${typeof err === "string" ? err : "clipboard write was rejected"}`,
      );
    } finally {
      ocrCopyAllBusy = false;
    }
  }

  // Open the current frame's captured http(s) page in the default browser via
  // the shared brokered helper (the raw URL stays in Rust; only the guarded
  // host+path ever reaches the UI). Pass `{ silent: true }` so the helper does
  // NOT pop its own dialog — the dashboard has its own inline frame-action
  // status line, so it branches on the returned status instead: `no-url` and
  // `error` surface that status (and leave the actions menu open), `opened`
  // closes the menu.
  async function openCurrentFrameUrl(): Promise<void> {
    const frame = timelineActive;
    if (!frame || openingCurrentFrameUrl) return;
    openingCurrentFrameUrl = true;
    try {
      const { status, error } = await openCapturedUrl(frame.id, { silent: true });
      if (status === "no-url") {
        setFrameActionStatus("No openable URL for this frame");
        return;
      }
      if (status === "error") {
        setFrameActionStatus(`Couldn't open URL: ${error}`);
        return;
      }
      stageActionsMenuOpen = false;
    } finally {
      openingCurrentFrameUrl = false;
    }
  }

  function openFrameActions(openedByKeyboard = false): void {
    if (!activePreviewPath) return;
    stageActionsOpenedByKeyboard = openedByKeyboard;
    stageActionsMenuOpen = true;
  }

  function closeFrameActions(): void {
    stageActionsMenuOpen = false;
  }

  function toggleFrameActions(openedByKeyboard = false): void {
    if (stageActionsMenuOpen) {
      closeFrameActions();
      return;
    }
    openFrameActions(openedByKeyboard);
  }

  function onFrameActionsTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" || event.key === " ") {
      stageActionsOpenedByKeyboard = true;
    }
  }

  function onFrameActionsPointerDownOutside(event: PointerEvent): void {
    if (!stageActionsMenuOpen) return;
    const target = event.target as Node | null;
    if (!target) return;
    if (stageActionsTriggerEl?.contains(target)) return;
    if (stageActionsMenuEl?.contains(target)) return;
    closeFrameActions();
  }

  $effect(() => {
    return () => {
      clearFrameActionStatusTimer();
    };
  });

  $effect(() => {
    if (!activePreviewPath) {
      stageActionsMenuOpen = false;
    }
  });

  $effect(() => {
    if (!stageActionsMenuOpen) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !stageActionsMenuOpen || !stageActionsOpenedByKeyboard) return;
      getFocusableElements(stageActionsMenuEl)[0]?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (
        (stageActionsOpenedByKeyboard && (!active || active === document.body)) ||
        (active && stageActionsMenuEl?.contains(active))
      ) {
        stageActionsTriggerEl?.focus({ preventScroll: true });
      }
      stageActionsOpenedByKeyboard = false;
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

  async function openBrokerCaptureResult(payload: BrokerOpenCaptureResultPayload): Promise<void> {
    if (payload.kind === "frame" && payload.frameId != null) {
      const frame = await invoke<FrameDto | null>("get_frame", {
        request: { frameId: payload.frameId },
      });
      if (!frame) {
        setFrameActionStatus("That capture is no longer available.", { tone: "error" });
        return;
      }
      await jumpToFrameWithBanner(frame);
      return;
    }
    if (payload.kind === "audio" && payload.audioSegmentId != null) {
      const request: GetAudioSegmentRequest = { audioSegmentId: payload.audioSegmentId };
      const audio = await invoke<AudioSegmentDto | null>("get_audio_segment", { request });
      const mapped = audio ? mapAudioSegmentDto(audio) : null;
      if (!mapped) {
        setFrameActionStatus("That capture is no longer available.", { tone: "error" });
        return;
      }
      if (!audioSegments.some((segment) => segment.id === mapped.id)) {
        audioSegments = [...audioSegments, mapped].sort((a, b) => a.startUnixMs - b.startUnixMs);
      }
      selectedAudioSegmentPinned = mapped;
      selectedAudioSegmentId = audio?.id ?? payload.audioSegmentId;
      // Seek to the selected match span (falling back to the segment start) and
      // jump the timeline to the aligned frame so mid-segment / out-of-window
      // matches land correctly.
      pendingAudioSeekMs = payload.spanStartMs ?? 0;
      if (payload.alignedFrameId != null) {
        const alignedFrame = await invoke<FrameDto | null>("get_frame", {
          request: { frameId: payload.alignedFrameId },
        });
        if (alignedFrame) await jumpToFrameWithBanner(alignedFrame);
      }
    }
  }

  // Returns whether any payload was queued (regardless of whether its jump
  // succeeded) so the cold-mount init can skip the latest-frames load that would
  // otherwise clobber the handed-off frame (see `initializeTimeline`).
  async function drainPendingBrokerOpenCaptureResults(): Promise<boolean> {
    let payloads: BrokerOpenCaptureResultPayload[];
    try {
      payloads = await invoke<BrokerOpenCaptureResultPayload[]>(
        "drain_pending_broker_open_capture_results",
      );
    } catch {
      setFrameActionStatus("Couldn't open that capture — please try again.", { tone: "error" });
      return false;
    }
    for (const payload of payloads) {
      try {
        await openBrokerCaptureResult(payload);
      } catch {
        // get_frame / get_audio_segment threw (capture gone or DB error) — the
        // broker handoff must never fail silently, so surface a visible note.
        setFrameActionStatus("That capture is no longer available.", { tone: "error" });
      }
    }
    return payloads.length > 0;
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
      audioSegmentsError = humanizeError(err);
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
  async function ensurePreview(
    frameId: number,
    options: { videoScope?: FramePreviewVideoScope } = {},
  ): Promise<void> {
    const startedAt = performance.now();
    if (previewCache.has(frameId)) {
      previewCacheHitCount += 1;
      const url = previewCache.get(frameId);
      if (url) touchPreviewCache(frameId, url);
      scrubPerfLogSlow("exact_preview_cache_hit", performance.now() - startedAt, { frameId }, 1);
      return;
    }
    if (recentlyFailedPreview(frameId)) {
      previewFailureCacheHitCount += 1;
      scrubPerfLogSlow("exact_preview_failure_cache_hit", performance.now() - startedAt, { frameId }, 1);
      return;
    }
    previewCacheMissCount += 1;
    if (previewInFlight.has(frameId)) {
      previewInFlightJoinCount += 1;
      scrubPerfLog("exact_preview_join", { frameId });
      return;
    }
    previewInFlight.add(frameId);
    const isActiveFrame = isTimelineActiveFrame(frameId);
    if (isActiveFrame) {
      setFrameActionStatus("Loading frame preview...");
    }
    try {
      const invokeStartedAt = performance.now();
      const dto = await invoke<FramePreviewDto | null>("get_frame_preview", {
        request: {
          frameId,
          videoScope: options.videoScope,
        } satisfies GetFramePreviewRequest,
      });
      const invokeDurationMs = performance.now() - invokeStartedAt;
      if (!dto) {
        throw new Error(`frame preview ${frameId} not found`);
      }
      const loadMs = performance.now() - startedAt;
      clearPreviewFailure(frameId);
      touchPreviewCache(frameId, dto.filePath, {
        mimeType: dto.mimeType,
        sourceKind: dto.sourceKind,
        secretRedactionCount: dto.secretRedactionCount,
        loadMs,
      });
      if (activePreviewLoadErrorFrameId === frameId) {
        activePreviewLoadErrorFrameId = null;
      }
      if (dto.sourceKind === "original_frame") {
        previewDirectPathCount += 1;
      } else {
        previewGeneratedPathCount += 1;
      }
      if (isTimelineActiveFrame(frameId)) {
        setFrameActionStatus(null);
      }
      scrubPerfLogSlow("exact_preview_loaded", loadMs, {
        frameId,
        invokeMs: invokeDurationMs,
        sourceKind: dto.sourceKind,
        active: isTimelineActiveFrame(frameId),
      }, SCRUB_PERF_SLOW_PREVIEW_MS);
    } catch (error) {
      const message = humanizeError(error);
      if (isPreviewGenerationCancelled(message)) {
        scrubPerfLog("exact_preview_cancelled", {
          frameId,
          durationMs: performance.now() - startedAt,
          active: isTimelineActiveFrame(frameId),
        });
        return;
      }

      rememberPreviewFailure(frameId);
      if (isTimelineActiveFrame(frameId)) {
        setFrameActionStatus(prettifyFramePreviewError(message), {
          detail: message,
          tone: "error",
        });
      }
      scrubPerfLog("exact_preview_error", {
        frameId,
        durationMs: performance.now() - startedAt,
        active: isTimelineActiveFrame(frameId),
        message,
      });
    } finally {
      previewInFlight.delete(frameId);
    }
  }

  async function ensureLatestActivePreview(frameId: number, generation: number): Promise<void> {
    if (generation !== activePreviewFetchGeneration || !isTimelineActiveFrame(frameId)) return;
    if (previewCache.has(frameId) || previewInFlight.has(frameId)) return;
    if (activeExactPreviewInFlight) {
      activeExactPreviewPendingFrameId = frameId;
      scrubPerfLog("exact_preview_deferred", { frameId });
      return;
    }

    activeExactPreviewInFlight = true;
    activeExactPreviewPendingFrameId = null;
    try {
      await ensurePreview(frameId, { videoScope: "active_frame" });
    } finally {
      activeExactPreviewInFlight = false;
      const pendingActive = timelineActive;
      const pendingFrameId = activeExactPreviewPendingFrameId;
      activeExactPreviewPendingFrameId = null;
      if (
        !pendingActive ||
        pendingFrameId !== pendingActive.id ||
        previewCache.has(pendingActive.id) ||
        previewInFlight.has(pendingActive.id)
      ) {
        return;
      }

      activePreviewFetchGeneration += 1;
      const nextGeneration = activePreviewFetchGeneration;
      scrubPerfLog("exact_preview_rescheduled_pending", { frameId: pendingActive.id });
      scheduleLatestActivePreview(
        pendingActive.id,
        nextGeneration,
        ACTIVE_PREVIEW_EXACT_SETTLE_MS,
      );
    }
  }

  async function ensureLatestScrubPreviews(activeIndex: number, generation: number): Promise<void> {
    if (generation !== scrubPreviewFetchGeneration) return;
    if (activeIndex !== timelineActiveIndex) return;
    if (scrubPreviewBatchInFlight) {
      const previousPendingIndex = scrubPreviewPendingActiveIndex;
      scrubPreviewPendingActiveIndex = activeIndex;
      if (
        previousPendingIndex == null ||
        Math.abs(previousPendingIndex - activeIndex) >= ACTIVE_PREVIEW_DEFERRED_LOG_MIN_DELTA
      ) {
        scrubPerfLog("scrub_preview_deferred", {
          activeIndex,
          velocity: previewScrubVelocityPxPerMs,
        });
      }
      return;
    }

    scrubPreviewBatchInFlight = true;
    scrubPreviewPendingActiveIndex = null;
    const window = scrubPreviewWindowForVelocity();
    const frameIds = scrubPreviewFrameIdsAround(activeIndex, window);
    try {
      await ensureScrubPreviews(frameIds, generation);
    } finally {
      scrubPreviewBatchInFlight = false;
      const pendingIndex = scrubPreviewPendingActiveIndex;
      scrubPreviewPendingActiveIndex = null;
      if (pendingIndex == null || pendingIndex !== timelineActiveIndex) return;

      scrubPreviewFetchGeneration += 1;
      const nextGeneration = scrubPreviewFetchGeneration;
      scrubPerfLog("scrub_preview_rescheduled_pending", {
        activeIndex: pendingIndex,
        velocity: previewScrubVelocityPxPerMs,
      });
      scheduleLatestScrubPreviews(
        pendingIndex,
        nextGeneration,
        scrubPreviewScheduleDelayMs(),
      );
    }
  }

  async function warmLatestScrubPreviews(activeIndex: number, generation: number): Promise<void> {
    if (generation !== scrubPreviewFetchGeneration) return;
    if (activeIndex !== timelineActiveIndex) return;
    if (scrubPreviewWarmInFlight) return;

    const timeWindow = scrubPreviewTimeWindowAround(activeIndex);
    if (!timeWindow) return;

    scrubPreviewWarmInFlight = true;
    scrubPreviewWarmCount += 1;
    try {
      const startedAt = performance.now();
      const dto = await invoke<ScrubPreviewAvailabilityDto>("get_scrub_preview_availability", {
        request: {
          startUnixMs: timeWindow.startUnixMs,
          endUnixMs: timeWindow.endUnixMs,
          enqueueMissing: false,
        } satisfies GetScrubPreviewAvailabilityRequest,
      });
      if (!scrubPreviewResponseShouldApply(generation, scrubPreviewFetchGeneration)) {
        scrubPerfLog("scrub_preview_warm_stale", {
          returned: dto.intervals.length,
          durationMs: performance.now() - startedAt,
        });
        return;
      }

      let ready = 0;
      let queued = 0;
      let missing = 0;
      for (const interval of dto.intervals) {
        if (interval.preview) {
          touchScrubPreviewIntervalCache(interval);
          ready += 1;
        } else if (interval.status === "queued") {
          queued += 1;
        } else {
          missing += 1;
        }
      }
      scrubPreviewQueuedCount += queued;
      scrubPreviewGeneratedCount += ready;
      scrubPerfLogSlow("scrub_preview_warm", performance.now() - startedAt, {
        returned: dto.intervals.length,
        ready,
        queued,
        missing,
      }, SCRUB_PERF_SLOW_PREVIEW_MS);
      if (ready > 0) {
        refreshScrubbingTimelinePreviewDisplay();
      }
    } finally {
      scrubPreviewWarmInFlight = false;
    }
  }

  async function ensureScrubPreviews(frameIds: number[], generation: number): Promise<void> {
    const startedAt = performance.now();
    const timeWindow = scrubPreviewTimeWindowAround(timelineActiveIndex);
    if (!timeWindow) return;
    if (frameIds.length === 0 && scrubPreviewIntervalForFrame(timelineActive)) {
      scrubPreviewHitCount += 1;
      scrubPerfLogSlow("scrub_preview_all_cached", performance.now() - startedAt, {
        requested: frameIds.length,
      }, 1);
      return;
    }
    scrubPreviewMissCount += frameIds.length;
    scrubPreviewBatchCount += 1;
    for (const frameId of frameIds) scrubPreviewInFlight.add(frameId);
    try {
      const invokeStartedAt = performance.now();
      const request: GetScrubPreviewAvailabilityRequest = {
        startUnixMs: timeWindow.startUnixMs,
        endUnixMs: timeWindow.endUnixMs,
        enqueueMissing: true,
      };
      const dto = await invoke<ScrubPreviewAvailabilityDto>("get_scrub_preview_availability", {
        request,
      });
      const invokeDurationMs = performance.now() - invokeStartedAt;
      const responseStale = generation !== scrubPreviewFetchGeneration;
      if (!scrubPreviewResponseShouldApply(generation, scrubPreviewFetchGeneration)) {
        scrubPerfLog("scrub_preview_stale", {
          requested: frameIds.length,
          uncached: frameIds.length,
          returned: dto.intervals.length,
          invokeMs: invokeDurationMs,
        });
        return;
      }
      const applyStartedAt = performance.now();
      let ready = 0;
      let queued = 0;
      let missing = 0;
      for (const interval of dto.intervals) {
        if (interval.preview) {
          touchScrubPreviewIntervalCache(interval);
          ready += 1;
          scrubPreviewGeneratedCount += 1;
        } else if (interval.status === "queued") {
          queued += 1;
          scrubPreviewQueuedCount += 1;
        } else {
          scrubPreviewMissingCount += 1;
          missing += 1;
        }
      }
      scrubPerfLogSlow("scrub_preview_batch", performance.now() - startedAt, {
        requested: frameIds.length,
        uncached: frameIds.length,
        returned: dto.intervals.length,
        generated: ready,
        queued,
        missing,
        stale: responseStale,
        invokeMs: invokeDurationMs,
        applyMs: performance.now() - applyStartedAt,
      }, SCRUB_PERF_SLOW_PREVIEW_MS);
      if (ready > 0) {
        refreshScrubbingTimelinePreviewDisplay();
      }
    } finally {
      for (const frameId of frameIds) scrubPreviewInFlight.delete(frameId);
    }
  }

  async function loadTimelinePage(
    reset = false,
    opts: { animate?: boolean } = {},
  ) {
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
        // scrollWidth, else we'd just set 0 → 0. For an explicit picker-Latest
        // commit (`animate`) this glides to the live head; every other reset
        // caller hard-cuts.
        await syncTimelineScrollToActiveFrame({ animate: opts.animate });
        // Drop cached previews from any prior generation — keeping them
        // would grow unboundedly across refreshes.
        previewCache = new Map();
        previewMimeTypeCache = new Map();
        previewSourceKindCache = new Map();
        previewRedactionCountCache = new Map();
        previewLoadMsCache = new Map();
        previewFailedAt = new Map();
        scrubPreviewCache = new Map();
        scrubPreviewMimeTypeCache = new Map();
        scrubPreviewSourceKindCache = new Map();
        scrubPreviewLoadMsCache = new Map();
        scrubPreviewFailedAt = new Map();
        scrubPreviewIntervalCache = new Map();
        resetTimelinePreviewDisplay();
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
      timelineError = humanizeError(err);
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
      timelineError = humanizeError(err);
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
          commitTimelineScrollPosition(max);
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

  function commitTimelineScrollSample(sample: PendingTimelineScrollSample): void {
    const maxScroll = sample.scrollWidth - sample.clientWidth;
    const advance = Math.max(0, maxScroll - sample.scrollLeft);
    const idx = Math.max(
      0,
      Math.min(
        timelineFrames.length - 1,
        Math.round(advance / TIMELINE_SLOT_WIDTH),
      ),
    );
    const activeChanged = idx !== timelineActiveIndex;
    if (activeChanged) {
      previewScrubVelocityPxPerMs = latestTimelineScrubVelocityPxPerMs;
      commitTimelineScrollPosition(sample.scrollLeft);
      markTimelinePreviewScrubbing(idx);
      timelineActiveIndex = idx;
    }
    scrubPerfRecordScroll(performance.now() - sample.handlerStartedAt, {
      ignored: false,
      scrollLeft: sample.scrollLeft,
      velocity: latestTimelineScrubVelocityPxPerMs,
      activeIndex: idx,
      activeChanged,
      frames: timelineFrames.length,
      windowStart: timelineWindowStart,
      windowEnd: timelineWindowEnd,
    });
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

  function commitPendingTimelineScrollSample(): void {
    timelineScrollAnimationFrame = null;
    const sample = pendingTimelineScrollSample;
    pendingTimelineScrollSample = null;
    if (!sample) return;
    commitTimelineScrollSample(sample);
  }

  function scheduleTimelineScrollCommit(sample: PendingTimelineScrollSample): void {
    pendingTimelineScrollSample = sample;
    if (timelineScrollAnimationFrame != null) return;
    timelineScrollAnimationFrame = requestAnimationFrame(commitPendingTimelineScrollSample);
  }

  function scheduleTimelinePreviewDisplaySettle(frameId: number): void {
    clearTimelinePreviewDisplaySettleTimer();
    timelinePreviewDisplaySettleTimer = setTimeout(() => {
      timelinePreviewDisplaySettleTimer = null;
      if ((timelineActive?.id ?? null) !== frameId) return;
      updateTimelinePreviewDisplayForFrame(timelineActive, "parked", {
        allowExact: true,
        clearOnMissing: true,
      });
    }, ACTIVE_PREVIEW_DISPLAY_SETTLE_MS);
  }

  function markTimelinePreviewScrubbing(activeIndex: number): void {
    const frame = timelineFrames[activeIndex] ?? null;
    if (!frame) return;

    const now = performance.now();
    const shouldSwapPreview =
      timelinePreviewDisplay == null ||
      now - timelinePreviewDisplayLastScrubAt >= ACTIVE_PREVIEW_DISPLAY_SCRUB_THROTTLE_MS;
    if (shouldSwapPreview) {
      timelinePreviewDisplayLastScrubAt = now;
      updateTimelinePreviewDisplayForFrame(frame, "scrubbing", {
        allowExact: true,
        clearOnMissing: true,
      });
    } else if (timelinePreviewDisplayMode !== "scrubbing") {
      timelinePreviewDisplayMode = "scrubbing";
    }
    scheduleTimelinePreviewDisplaySettle(frame.id);
  }

  function onTimelineScroll(event: Event) {
    const handlerStartedAt = performance.now();
    const el = event.currentTarget as HTMLDivElement;
    const now = handlerStartedAt;
    const scrollLeft = el.scrollLeft;
    latestTimelineScrollLeft = scrollLeft;
    const deltaMs = now - lastTimelineScrollSample.at;
    const scrollDelta = scrollLeft - lastTimelineScrollSample.left;
    if (deltaMs > 0) {
      latestTimelineScrubVelocityPxPerMs = Math.abs(scrollDelta) / deltaMs;
    }
    // Newest frame is anchored at the right, so a growing scrollLeft moves the
    // viewport toward older frames ("backward" in time → +1).
    if (scrollDelta > 0) readoutScrubDirection = 1;
    else if (scrollDelta < 0) readoutScrubDirection = -1;
    lastTimelineScrollSample = { left: scrollLeft, at: now };
    syncTimelineAudioLaneScroll(scrollLeft);
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
      scrubPerfRecordScroll(performance.now() - handlerStartedAt, {
        ignored: true,
        reason: "resize_client_width",
        scrollLeft,
        velocity: latestTimelineScrubVelocityPxPerMs,
        activeIndex: timelineActiveIndex,
      });
      return;
    }
    scheduleTimelineScrollCommit({
      clientWidth: el.clientWidth,
      handlerStartedAt,
      scrollLeft,
      scrollWidth: el.scrollWidth,
    });
  }

  $effect(() => {
    return () => {
      if (timelineScrollAnimationFrame != null) {
        cancelAnimationFrame(timelineScrollAnimationFrame);
        timelineScrollAnimationFrame = null;
      }
      pendingTimelineScrollSample = null;
    };
  });

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
    // Don't hijack wheel events that originate inside timeline-owned overlays
    // with their own scroll surfaces. These overlays are rendered inside the
    // same `<section>` that owns this listener, so without this guard their
    // native vertical scroll is cancelled and converted into rail scrubbing.
    const target = event.target;
    if (
      target instanceof Element &&
      target.closest(".timeline__picker")
    ) {
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
        timelineJump(event.shiftKey ? 10 : 1);
        break;
      case "ArrowRight":
        timelineJump(event.shiftKey ? -10 : -1);
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
    return isShortcutSuppressedTarget(target, [
      ".timeline__picker",
      ".timeline__stage-action-menu",
      ".audio-drawer",
      ".audio-drawer__speaker-popover",
    ]);
  }

  // Page-level timeline shortcuts: ArrowLeft/ArrowRight move the active frame
  // even when the thin rail itself does not have focus. Interactive surfaces
  // keep their own keyboard behavior (calendar navigation, buttons, audio
  // scrubbing, text selection, etc.).
  function closeDashboardTopSurface(): boolean {
    if (speakerActionsOpenIndex != null) {
      closeSpeakerActions();
      return true;
    }
    if (selectedAudioSegmentId != null) {
      closeAudioDrawer();
      return true;
    }
    if (pickerOpen) {
      pickerOpen = false;
      return true;
    }
    if (stageActionsMenuOpen) {
      closeFrameActions();
      return true;
    }
    return false;
  }

  function dashboardShortcutMatches(
    event: KeyboardEvent,
    definition: ShortcutDefinition,
  ): boolean {
    return matchShortcut(event, effectiveShortcut(definition), windowPlatform);
  }

  function isShortcutHelpKey(event: KeyboardEvent): boolean {
    return matchShortcut(
      event,
      effectiveShortcut({
        id: "toggleShortcutsHelp",
        label: "Show keyboard shortcuts",
        bindings: [{ key: "/" }],
        kind: "command",
        scope: "global",
      }),
      windowPlatform,
    );
  }

  function closeDashboardSurfacesForShortcutHelp(): void {
    if (speakerActionsOpenIndex != null) closeSpeakerActions();
    if (selectedAudioSegmentId != null) closeAudioDrawer();
    if (pickerOpen) pickerOpen = false;
    if (stageActionsMenuOpen) closeFrameActions();
  }

  function onDashboardWindowKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    if (isShortcutHelpKey(event) && !isShortcutSuppressedTarget(event.target)) {
      closeDashboardSurfacesForShortcutHelp();
      return;
    }

    if (event.key === "Escape") {
      if (closeDashboardTopSurface()) {
        event.preventDefault();
        event.stopPropagation();
      }
      return;
    }

    const timelineShortcutSuppressed = isTimelineShortcutSuppressedTarget(event.target);
    if (!timelineShortcutSuppressed) {
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.openJumpPicker)) {
        event.preventDefault();
        // Opening seeds from the active frame inside the jumper component
        // (rising-edge $effect). Guard so the shortcut never toggles closed.
        if (!pickerOpen) pickerOpen = true;
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.jumpLatest)) {
        if (!showJumpToLatestButton || timelineLoading || timelineLoadingMore || pickerJumping) return;
        event.preventDefault();
        void jumpToLatestFrame();
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.toggleOcr)) {
        if (!timelineActive) return;
        event.preventDefault();
        void toggleOcrForActiveFrame();
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.refreshTimeline)) {
        if (timelineLoading || timelineLoadingMore || audioSegmentsLoading) return;
        event.preventDefault();
        void refreshTimelineAndDashboard();
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.copyFrame)) {
        if (!activePreviewPath) return;
        event.preventDefault();
        void copyActiveFrameImage();
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.downloadFrame)) {
        if (!activePreviewPath) return;
        event.preventDefault();
        void downloadActiveFrameImage();
        return;
      }
      if (dashboardShortcutMatches(event, DASHBOARD_SHORTCUTS.playMoment)) {
        if (!timelineActive) return;
        event.preventDefault();
        playActiveFrameMoment();
        return;
      }
    }

    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (timelineShortcutSuppressed) return;
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      onTimelineKeyDown(event);
    }
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
    // Clicking the already-selected bar toggles the drawer closed; clicking a
    // different bar switches the drawer to that segment (the capture-phase
    // outside-pointerdown handler now lets bar clicks through for exactly this).
    selectedAudioSegmentPinned = null;
    pendingAudioSeekMs = null;
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
    void initializeTimeline();
  });

  // A capture-result handoff (e.g. an insights "show this frame in the
  // timeline") queues a payload in Rust *before* this cold window mounts. Drain
  // it first and let the jump load the window around the handed-off frame; only
  // fall back to the newest-first latest load when nothing was queued (or the
  // jump left the rail empty). Draining here — rather than in the broker
  // listener's on-mount `.then` — keeps the latest load from racing the jump and
  // clobbering it (which landed the user on the newest frame instead).
  async function initializeTimeline(): Promise<void> {
    // A receipt "Open in Timeline" queues a frame focus before this page mounts
    // (frontend-only handoff across the /insights→/ route switch). Consume it
    // first so the jump wins over the normal latest-first load.
    const focus = takePendingTimelineFocus();
    if (focus) {
      try {
        const frame = await invoke<FrameDto | null>("get_frame", {
          request: { frameId: focus.frameId },
        });
        if (frame) {
          await jumpToFrameWithBanner(frame);
          return;
        }
      } catch {
        /* fall through to normal init */
      }
    }
    const handled = await drainPendingBrokerOpenCaptureResults();
    if (!handled || timelineFrames.length === 0) {
      await loadTimelinePage(true);
    }
  }

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

  // Keep fast scrubbing on the low-cost scrub-preview tier. Exact active
  // previews can fall back to video extraction, so they are latest-only:
  // one active exact request may run at a time, and intermediate frames are
  // replaced by the current active frame after the settle window.
  $effect(() => {
    const active = timelineActive;
    const activeIndex = timelineActiveIndex;
    const activeFrameChanged = (active?.id ?? null) !== previousActivePreviewFrameId;
    const indexDelta = Math.abs(activeIndex - previousActivePreviewIndex);
    previousActivePreviewIndex = activeIndex;
    previousActivePreviewFrameId = active?.id ?? null;
    const shouldScheduleScrubPreview = timelineMovementShouldScheduleScrubPreview(
      activeFrameChanged ? indexDelta : 0,
      previewScrubVelocityPxPerMs,
      ACTIVE_PREVIEW_FAST_SCRUB_PX_PER_MS,
    );
    activePreviewFetchGeneration += 1;
    scrubPreviewFetchGeneration += 1;
    const gen = activePreviewFetchGeneration;
    const scrubGen = scrubPreviewFetchGeneration;
    const cleanupPreviewTimers = () => {
      clearActivePreviewFetchTimer();
      clearScrubPreviewWarmTimer();
    };
    clearActivePreviewFetchTimer();
    clearScrubPreviewWarmTimer();
    if (activeFrameChanged && activeExactPreviewInFlight) {
      void cancelActivePreviewVideoRequests();
    }
    if (!active) {
      return cleanupPreviewTimers;
    }

    if (previewCache.has(active.id) || previewInFlight.has(active.id)) {
      return cleanupPreviewTimers;
    }

    if (shouldScheduleScrubPreview) {
      if (scrubPreviewBatchInFlight) {
        const previousPendingIndex = scrubPreviewPendingActiveIndex;
        scrubPreviewPendingActiveIndex = activeIndex;
        if (
          previousPendingIndex == null ||
          Math.abs(previousPendingIndex - activeIndex) >= ACTIVE_PREVIEW_DEFERRED_LOG_MIN_DELTA
        ) {
          scrubPerfLog("scrub_preview_deferred", {
            activeIndex,
            velocity: previewScrubVelocityPxPerMs,
          });
        }
      } else {
        scheduleLatestScrubPreviews(
          activeIndex,
          scrubGen,
          scrubPreviewScheduleDelayMs(),
        );
      }
      scheduleScrubPreviewWarm(activeIndex, scrubGen);
    }

    if (activeExactPreviewInFlight) {
      activeExactPreviewPendingFrameId = active.id;
      scrubPerfLog("exact_preview_deferred", { frameId: active.id });
      return cleanupPreviewTimers;
    }
    scheduleLatestActivePreview(
      active.id,
      gen,
      activeExactPreviewDelayMs(
        shouldScheduleScrubPreview,
        ACTIVE_PREVIEW_EXACT_SETTLE_MS,
      ),
    );
    return cleanupPreviewTimers;
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
  const AUDIO_SEGMENT_SUBJECT_TYPE = "audio_segment";
  const OCR_SOURCE_IMAGE_PATH_OPTION = "mnemaSourceImagePath";
  const AUDIO_TRANSCRIPTION_PROCESSOR = "audio_transcription";
  const SPEAKER_ANALYSIS_PROCESSOR = "speaker_analysis";
  const SYSTEM_AUDIO_SPEECH_ACTIVITY_PROCESSOR = "system_audio_speech_activity";

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
      const ocrData = await loadOcrFromJob(job, invoke);
      applyLoadedOcrData(activeFrameId, sourceFrame, gen, ocrData);
    } catch (err) {
      if (!ocrIsCurrent(activeFrameId, gen)) return;
      applyLoadedOcrData(activeFrameId, sourceFrame, gen, {
        status: "error",
        observations: [],
        providerLabel: null,
        error: humanizeError(err),
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
      let ocrData = await loadOcrForFrame(sourceFrame, invoke);
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
          ocrData = await loadOcrForFrame(sourceFrame, invoke);
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
        error: humanizeError(err),
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

  const ocrEnabled = $derived(captureControls.recordingSettings?.ocr?.enabled ?? true);
  const ocrReadOnlyTooltip = "OCR is off. Saved OCR text can still be viewed.";
  const ocrRunDisabledTooltip = "OCR is off. Turn it on in Settings to run OCR again.";

  const ocrRerunDisabled = $derived(
    !timelineActive || !ocrEnabled || ocrRerunLoading || ocrStatus === "running",
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
      if (!previewCache.has(frameId)) {
        await ensurePreview(frameId);
      }
      // Only OCR pixels that belong to this frame: original_frame is the
      // frame's own artifact, video_fallback is its exact video position.
      // A segment_frame_fallback preview is a *different* frame's image —
      // OCRing it would persist wrong text under this frame's id; without a
      // usable source the backend falls back to frame.file_path (and fails
      // honestly if that artifact is gone).
      const sourceKind = previewSourceKindCache.get(frameId);
      const sourceImagePath =
        sourceKind === "original_frame" || sourceKind === "video_fallback"
          ? previewCache.get(frameId) ?? null
          : null;
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
      const ocrData = await loadOcrFromJob(result.job, invoke);
      applyLoadedOcrData(frameId, frame, gen, ocrData);
    } catch (err) {
      if (!ocrIsCurrent(frameId, gen)) return;
      applyLoadedOcrData(frameId, frame, gen, {
        status: "error",
        observations: [],
        providerLabel: null,
        error: humanizeError(err),
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

  // OCR succeeded with text, but the active frame carries no intrinsic
  // dimensions, so the boxes can't be anchored to an image rect (the overlay
  // renders nothing). Surface a hint pointing at the ⋯ → Copy text path so the
  // recognized text isn't silently inaccessible. Gated on a measured stage so
  // it never flashes during the initial mount before dimensions resolve.
  const ocrSuccessUnpositionable = $derived(
    ocrVisible &&
      ocrStatus === "success" &&
      timelineActive != null &&
      ocrFrameId === timelineActive.id &&
      ocrObservations.length > 0 &&
      stageWidth > 0 &&
      stageHeight > 0 &&
      (renderedImageRect.width <= 0 || renderedImageRect.height <= 0),
  );

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

  // Thin wrapper over the pure `ocrBoxStyle` (in $lib/frame-ocr): closes over
  // the measured image height so the template call site stays clean.
  const ocrBoxStyleLocal = (obs: OcrObservation): string =>
    ocrBoxStyle(obs, renderedImageRect.height);

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

  const ocrToggleTitle = $derived(
    `${!ocrEnabled && !ocrVisible ? ocrReadOnlyTooltip : ocrError ??
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
              : "Show OCR data for the active frame")} (O)`,
  );

  // ─── Date / time jump picker ──────────────────────────────────────────────
  // The jump picker UI (trigger + two-pane calendar/time popover, per-month
  // summary cache + stale-while-revalidate, focus trap, positioning) lives in
  // `TimelineJumper.svelte`. The dashboard keeps ownership of the actual
  // timeline-mutating jump (`jumpToFrame`, which loads a focused newest-first
  // window via `get_timeline_window_around_frame` and rebuilds preview caches)
  // and reaches into the component only for per-month cache invalidation.
  //
  // `pickerOpen` / `pickerJumping` stay here as bindable state so the
  // dashboard's keyboard shortcuts, shortcut-help table, wheel/Escape guards,
  // and tooltip gating keep working unchanged.
  let pickerOpen = $state(false);
  let pickerJumping = $state(false);
  let jumperRef = $state<ReturnType<typeof TimelineJumper> | null>(null);

  /**
   * Targeted invalidation of the jump picker's per-month summary cache. Given
   * newly-arrived frames, the component marks the LOCAL months they belong to
   * stale (not deleted) so the open picker keeps rendering its disabled-date
   * map until the background revalidation lands — a flicker-free
   * stale-while-revalidate. Thin delegate so existing head-poll / refresh call
   * sites stay unchanged.
   */
  function invalidatePickerMonthsForFrames(
    frames: { capturedAt: string }[],
  ): void {
    jumperRef?.invalidateMonthsForFrames(frames);
  }

  function invalidateLoadedPickerSummaryMonths(): void {
    jumperRef?.invalidateAllLoadedMonths();
  }

  // Performs the timeline jump to an already-resolved frame: loads a focused
  // newest-first window in one request, rebuilds the preview caches, and pins
  // the rail to the returned target index. Returns null on success or a
  // human-readable error string on failure (the caller decides where to
  // surface it — the picker's footer, or the stage status banner). `animate`
  // smooth-scrolls the playhead to the new moment for explicit user commits.
  async function jumpToFrame(
    target: FrameDto,
    opts: { animate?: boolean } = {},
  ): Promise<string | null> {
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
      if (gen !== timelineGeneration) return null;
      if (!window.frames[window.targetIndex] || window.frames[window.targetIndex]?.id !== target.id) {
        return "failed to focus selected frame";
      }
      timelineFrames = window.frames;
      timelineActiveIndex = window.targetIndex;
      timelineExhausted = !window.hasOlder;
      timelineHasNewer = window.hasNewer;
      timelineError = null;
      timelineShowingHistoricalWindow = window.hasNewer;
      previewCache = new Map();
      previewMimeTypeCache = new Map();
      previewSourceKindCache = new Map();
      previewRedactionCountCache = new Map();
      previewLoadMsCache = new Map();
      previewFailedAt = new Map();
      scrubPreviewCache = new Map();
      scrubPreviewMimeTypeCache = new Map();
      scrubPreviewSourceKindCache = new Map();
      scrubPreviewLoadMsCache = new Map();
      scrubPreviewFailedAt = new Map();
      scrubPreviewIntervalCache = new Map();
      activePreviewDecodeRetries.clear();
      resetTimelinePreviewDisplay();
      await syncTimelineScrollToActiveFrame({ animate: opts.animate });
      void refreshAudioSegments();
      return null;
    } catch (err) {
      if (gen !== timelineGeneration) return null;
      return humanizeError(err);
    } finally {
      if (gen === timelineGeneration) {
        timelineLoading = false;
        timelineLoadingMore = false;
      }
    }
  }

  // Surfaces a jump error from a NON-picker entry point (broker handoff,
  // duplicate-frame link) on the always-visible stage status banner — the
  // picker owns its own footer-strip error for picker-originated commits.
  async function jumpToFrameWithBanner(target: FrameDto): Promise<void> {
    const err = await jumpToFrame(target);
    if (err) setFrameActionStatus(err, { tone: "error" });
  }

  // Bridges the jumper component's commit to the dashboard's timeline jump.
  // Returns null on success (popover closes) or an error string (popover
  // stays open and shows it). `animate` smooth-scrolls the playhead.
  async function onJumperJump(target: FrameDto): Promise<string | null> {
    return jumpToFrame(target, { animate: true });
  }

  const latestFrameOffset = $derived(
    timelineFrames.length === 0 ? 0 : timelineActiveIndex + (timelineHasNewer ? 1 : 0),
  );
  const showJumpToLatestButton = $derived(latestFrameOffset > 50);

  // Snap the timeline to the live head ("Latest" / "snap to now"). Closes the
  // jump picker if open, then reloads the newest page. This is an explicit
  // user "go to now" commit, so the playhead glides to the live head rather
  // than hard-cutting (§12.6) — gated on `prefers-reduced-motion` inside
  // `syncTimelineScrollToActiveFrame`. Refresh / initial-load / head-poll
  // resets keep the hard-cut (they pass no `animate`).
  async function jumpToLatestFrame(): Promise<void> {
    pickerOpen = false;
    await loadTimelinePage(true, { animate: true });
  }

  $effect(() => {
    const timelineRows: KeyboardHelpGroup["rows"] = [];
    if (timelineFrames.length > 0) {
      timelineRows.push(
        DASHBOARD_SHORTCUTS.olderFrame,
        DASHBOARD_SHORTCUTS.newerFrame,
        DASHBOARD_SHORTCUTS.olderFrameFast,
        DASHBOARD_SHORTCUTS.newerFrameFast,
      );
    }
    timelineRows.push(
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.openJumpPicker),
        enabled: !pickerOpen,
      },
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.jumpLatest),
        enabled: showJumpToLatestButton && !timelineLoading && !timelineLoadingMore && !pickerJumping,
      },
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.refreshTimeline),
        enabled: !timelineLoading && !timelineLoadingMore && !audioSegmentsLoading,
      },
      DASHBOARD_SHORTCUTS.closeSurface,
    );

    const frameRows: KeyboardHelpGroup["rows"] = [
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.toggleOcr),
        label: ocrVisible ? "Hide OCR panel" : "Show OCR panel",
        enabled: timelineActive != null,
      },
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.copyFrame),
        enabled: activePreviewPath != null,
      },
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.downloadFrame),
        enabled: activePreviewPath != null,
      },
      {
        ...effectiveShortcut(DASHBOARD_SHORTCUTS.playMoment),
        enabled: activeFrameAudioMoment != null,
      },
    ];

    const groups: KeyboardHelpGroup[] = [
      {
        id: "dashboard.timeline",
        title: "Timeline",
        rows: timelineRows,
      },
      {
        id: "dashboard.frame",
        title: "Frame",
        rows: frameRows,
      },
    ];

    if (selectedAudioSegment != null) {
      groups.push({
        id: "dashboard.audio",
        title: "Audio Drawer",
        rows: [
          {
            ...effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.playPause),
            enabled: selectedAudioSrc != null,
          },
          {
            ...effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBack),
            enabled: selectedAudioSrc != null,
          },
          {
            ...effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekForward),
            enabled: selectedAudioSrc != null,
          },
          {
            ...effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekBackFast),
            enabled: selectedAudioSrc != null,
          },
          {
            ...effectiveShortcut(AUDIO_DRAWER_SHORTCUTS.seekForwardFast),
            enabled: selectedAudioSrc != null,
          },
          AUDIO_DRAWER_SHORTCUTS.close,
          AUDIO_DRAWER_SHORTCUTS.trapFocus,
        ],
      });
    }

    return setKeyboardHelpGroups("dashboard", groups);
  });

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
    let unlistenTimelineDataChanged: (() => void) | undefined;
    let unlistenScrubPreviewCacheChanged: (() => void) | undefined;
    let unlistenBrokerOpenCaptureResult: (() => void) | undefined;
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
      reloadSelectedAudioTranscriptIfPending();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenAudioSegmentsChanged = fn;
    });

    listen<TimelineDataChangedPayload>("timeline_data_changed", (event) => {
      if (event.payload.reason !== "retention" && event.payload.reason !== "delete_recent_capture") return;
      invalidateLoadedPickerSummaryMonths();
      const deletedFrameIds = new Set(event.payload.deletedFrameIds ?? []);
      const deletedAudioSegmentIds = new Set(event.payload.deletedAudioSegmentIds ?? []);
      const activeFrameId = timelineActive?.id ?? null;
      const previousActiveIndex = timelineActiveIndex;

      if (deletedFrameIds.size > 0) {
        const nextFrames = timelineFrames.filter((frame) => !deletedFrameIds.has(frame.id));
        if (nextFrames.length !== timelineFrames.length) {
          const activeWasDeleted = activeFrameId !== null && deletedFrameIds.has(activeFrameId);
          timelineFrames = nextFrames;
          if (timelineFrames.length === 0) {
            timelineActiveIndex = 0;
          } else if (activeFrameId !== null && !deletedFrameIds.has(activeFrameId)) {
            const nextActiveIndex = timelineFrames.findIndex((frame) => frame.id === activeFrameId);
            timelineActiveIndex = nextActiveIndex >= 0
              ? nextActiveIndex
              : Math.min(previousActiveIndex, timelineFrames.length - 1);
          } else {
            timelineActiveIndex = Math.min(previousActiveIndex, timelineFrames.length - 1);
          }
          prunePreviewCache(timelineFrames);
          void syncTimelineScrollToActiveFrame();
          // The active frame just vanished from under the user. The stage
          // status banner only renders when a frame is shown, so surface the
          // acknowledgment in the nearest-frame case (the empty case already
          // collapses to the "No frames yet" empty state).
          if (activeWasDeleted && timelineFrames.length > 0) {
            setFrameActionStatus(
              "This capture was deleted — showing the nearest frame.",
              { tone: "error" },
            );
          }
        }
      }

      if (deletedAudioSegmentIds.size > 0) {
        audioSegments = audioSegments.filter((segment) => !deletedAudioSegmentIds.has(segment.id));
        if (selectedAudioSegmentId !== null && deletedAudioSegmentIds.has(selectedAudioSegmentId)) {
          selectedAudioSegmentId = null;
        }
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenTimelineDataChanged = fn;
    });

    listen("scrub_preview_cache_changed", () => {
      const activeIndex = timelineActiveIndex;
      scrubPreviewFetchGeneration += 1;
      scheduleLatestScrubPreviews(
        activeIndex,
        scrubPreviewFetchGeneration,
        scrubPreviewScheduleDelayMs(),
      );
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenScrubPreviewCacheChanged = fn;
    });

    // Live broker events (a warm timeline receiving a handoff) drain here. The
    // cold-mount drain is owned by `initializeTimeline` so it can gate the
    // initial latest load on it; draining again in this `.then` would re-race
    // that load, so we only register the live listener.
    listen<BrokerOpenCaptureResultPayload>("broker_open_capture_result", () => {
      void drainPendingBrokerOpenCaptureResults();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenBrokerOpenCaptureResult = fn;
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
      unlistenTimelineDataChanged?.();
      unlistenScrubPreviewCacheChanged?.();
      unlistenBrokerOpenCaptureResult?.();
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
<svelte:window
  onpointerdown={(event) => {
    onFrameActionsPointerDownOutside(event);
    const target = event.target;
    if (
      !(target instanceof Element) ||
      (!target.closest(".audio-drawer__speaker-popover") &&
        !target.closest(".audio-drawer__speaker-chip"))
    ) {
      closeSpeakerActions();
    }
  }}
  onkeydown={onDashboardWindowKeyDown}
/>
<section class="timeline" onwheel={onTimelineWheel}>
  <header class="timeline__bar">
    <div class="timeline__bar-group timeline__bar-group--primary">
      <!-- Recording status indicator and start/stop controls now live in
           the app-wide title bar (see `routes/+layout.svelte`) so the
           recording affordance is visible regardless of which route is
           active. The timeline header retains only timeline-specific
           controls below (jump, OCR toggle, refresh). -->
      <TimelineJumper
        bind:this={jumperRef}
        bind:open={pickerOpen}
        bind:jumping={pickerJumping}
        activeFrame={timelineActive}
        timelineBusy={timelineLoading || timelineLoadingMore}
        showLatest={showJumpToLatestButton}
        onJump={onJumperJump}
        onJumpToLatest={jumpToLatestFrame}
      />
    </div>

    <div class="timeline__bar-group timeline__bar-group--secondary">
      {#if ocrVisible && timelineActive && ocrFrameId === timelineActive.id}
        {#if ocrProviderLabel}
          <span class="timeline__ocr-provider-chip" use:tip={ocrProviderLabel}>{ocrProviderLabel}</span>
        {/if}
        <button
          type="button"
          class="btn btn--ghost btn--sm timeline__ocr-rerun-btn"
          onclick={reprocessOcrForActiveFrame}
          disabled={ocrRerunDisabled}
          use:tip={!ocrEnabled
            ? ocrRunDisabledTooltip
            : ocrStatus === "running"
            ? "OCR is queued or still processing"
            : ocrStatus === "missing"
              ? "Run OCR for the active frame with current settings"
              : "Rerun OCR for the active frame with current settings"}
        >{ocrRerunButtonLabel}</button>
      {/if}
      <button
        class="btn btn--ghost btn--sm timeline__ocr-btn"
        class:timeline__ocr-btn--running={ocrStatus === "running"}
        class:timeline__ocr-btn--error={ocrStatus === "error"}
        class:timeline__ocr-btn--success={ocrStatus === "success"}
        onclick={toggleOcrForActiveFrame}
        disabled={!timelineActive}
        use:tip={ocrToggleTitle}
        aria-label={ocrVisible
          ? "Hide OCR data for active frame"
          : "Show OCR data for active frame"}
        aria-pressed={ocrVisible}
      >
        <span class="timeline__ocr-glyph" aria-hidden="true"><IconScanText /></span>
        <span>{ocrButtonLabel}</span>
        {#if ocrStatus === "success" && ocrObservations.length > 0}
          <span class="timeline__ocr-count">{ocrObservations.length}</span>
        {/if}
      </button>
      <button
        class="btn btn--ghost btn--sm"
        onclick={refreshTimelineAndDashboard}
        disabled={timelineLoading || timelineLoadingMore || audioSegmentsLoading}
        use:tip={"Refresh dashboard timeline (R)"}
      >refresh</button>
    </div>
  </header>

  {#if timelineError}
    <div class="timeline__error" role="alert">
      <div class="timeline__error-body">
        <span class="timeline__error-label">load error</span>
        <span class="timeline__error-msg">{timelineError}</span>
      </div>
      <button
        type="button"
        class="btn btn--ghost btn--sm timeline__error-retry"
        onclick={refreshTimelineAndDashboard}
        disabled={timelineLoading || timelineLoadingMore}
      >{timelineLoading ? "retrying…" : "retry"}</button>
    </div>
  {/if}

  <!-- Audio segment player drawer. Rendered as a non-modal bottom sheet
       that slides in only when an audio segment is selected. The timeline
       rail stays interactive while the drawer is open so the user can pick
       a different segment without dismissing first; selecting null (or
       pressing Escape / clicking close) hides the drawer entirely. The
       audio lane bars themselves remain visible above the rail so audio
       presence/discovery is unaffected. -->

  <div
    class="timeline__stage"
    class:timeline__stage--stale={timelineError && timelineFrames.length > 0}
    bind:this={stageEl}
  >
    <!-- Stage status banner is hoisted to a direct child of the stage (it is
         absolutely positioned bottom-right, so DOM order doesn't move it) so
         broker open-capture failures and deletion acks surface even when there
         is no active frame (e.g. the "No frames yet" empty state). -->
    {#if frameActionStatus}
      <div
        class="timeline__stage-status"
        class:timeline__stage-status--error={frameActionStatus.tone === "error"}
        role={frameActionStatus.tone === "error" ? "alert" : "status"}
        aria-live={frameActionStatus.tone === "error" ? "assertive" : "polite"}
        onpointerenter={onFrameActionStatusPointerEnter}
        onpointerleave={onFrameActionStatusPointerLeave}
      >
        <div class="timeline__stage-status-body">
          <div class="timeline__stage-status-summary">{frameActionStatus.message}</div>
          {#if frameActionStatus.detail}
            <div class="timeline__stage-status-detail">{frameActionStatus.detail}</div>
          {/if}
        </div>
        {#if frameActionStatus.tone === "error"}
          <button
            type="button"
            class="timeline__stage-status-close"
            aria-label="Dismiss message"
            use:tip={"Dismiss"}
            onclick={dismissFrameActionStatus}
          >
            <svg width="11" height="11" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true">
              <path d="M3.5 3.5l7 7M10.5 3.5l-7 7" />
            </svg>
          </button>
        {/if}
      </div>
    {/if}
    {#if timelineLoading && timelineFrames.length === 0}
      <div class="timeline__preview-pending">
        <span class="timeline__preview-pending-spinner" aria-hidden="true"></span>
        <span>loading frames…</span>
      </div>
    {:else if timelineFrames.length === 0}
      {#if captureControls.isCapturing}
        <!-- Capture is live but no frames have landed yet: drop the misleading
             "Press Record" cue and reassure that the first frames are imminent. -->
        <div class="timeline__empty timeline__empty--capturing">
          <span class="timeline__empty-glyph" aria-hidden="true"><IconClapperboard /></span>
          <h2 class="timeline__empty-title">
            <span class="timeline__empty-rec-dot" aria-hidden="true"></span>Recording started
          </h2>
          <p class="timeline__empty-hint">
            Your first frames will appear here in a moment…
          </p>
        </div>
      {:else}
        <div class="timeline__empty">
          <span class="timeline__empty-glyph" aria-hidden="true"><IconClapperboard /></span>
          <h2 class="timeline__empty-title">No frames yet</h2>
          <p class="timeline__empty-hint">
            Your timeline fills up as Mnema captures your screen.
          </p>
          <p class="timeline__empty-cue">
            Press <span class="timeline__empty-cue-key">Record</span> in the title bar above to start a capture session.
          </p>
        </div>
      {/if}
    {:else if timelineActive}
      {@const previewDisplay = timelinePreviewDisplay}
      {@const previewPath = previewDisplay?.filePath ?? null}
      {@const previewUrl = previewPath ? framePreviewAssetUrl(previewPath) : null}
      {#if previewUrl}
        {@const activeExactPreviewReady = previewCache.has(timelineActive.id)}
        {@const displayedActiveExactPreview = previewDisplay?.frameId === timelineActive.id && previewDisplay.source === "exact"}
        <div
          class="timeline__stage-actions"
          class:timeline__stage-actions--open={stageActionsMenuOpen}
        >
          {#if activeFrameAudioMoment}
            <!-- Play-this-moment bridge: opens the audio segment that overlaps
                 this frame's capture time and seeks to the matching offset.
                 Mirrors the "P" dashboard shortcut. -->
            <button
              type="button"
              class="btn btn--ghost btn--sm timeline__stage-action-trigger timeline__stage-play-moment"
              aria-label={`Play audio at this moment (${audioSourceLabel(activeFrameAudioMoment.segment.source)})`}
              use:tip={"Play audio at this moment (P)"}
              onclick={playActiveFrameMoment}
            ><span class="timeline__stage-action-glyph" aria-hidden="true"><IconHeadphones /></span></button>
          {/if}
          <button
            type="button"
            class="btn btn--ghost btn--sm timeline__stage-action-trigger"
            aria-label="Frame actions"
            aria-haspopup="dialog"
            aria-expanded={stageActionsMenuOpen}
            aria-controls="timeline-stage-action-menu"
            use:tip={"Frame actions (C copy, D download)"}
            bind:this={stageActionsTriggerEl}
            onkeydown={onFrameActionsTriggerKeydown}
            onpointerdown={() => { stageActionsOpenedByKeyboard = false; }}
            onclick={() => toggleFrameActions(stageActionsOpenedByKeyboard)}
          ><span class="timeline__stage-action-glyph" aria-hidden="true"><IconMoreHorizontal /></span></button>
          {#if stageActionsMenuOpen}
            <div
              id="timeline-stage-action-menu"
              class="timeline__stage-action-menu"
              role="group"
              aria-label="Frame actions"
              bind:this={stageActionsMenuEl}
            >
              <button
                type="button"
                class="timeline__stage-action-menu-item"
                onclick={copyActiveFrameImage}
                disabled={!activeExactPreviewReady || frameImageActionBusy !== null}
                aria-label="Copy active frame image"
                aria-busy={frameImageActionBusy === "copy"}
                use:tip={"Copy image (C)"}
              >{frameImageActionBusy === "copy" ? "copying…" : "copy"}</button>
              <button
                type="button"
                class="timeline__stage-action-menu-item"
                onclick={downloadActiveFrameImage}
                disabled={!activeExactPreviewReady || frameImageActionBusy !== null}
                aria-label="Download active frame image"
                aria-busy={frameImageActionBusy === "download"}
                use:tip={"Download image (D)"}
              >{frameImageActionBusy === "download" ? "saving…" : "download"}</button>
              {#if ocrVisible && ocrStatus === "success" && ocrFrameId === timelineActive.id && ocrObservations.length > 0}
                <button
                  type="button"
                  class="timeline__stage-action-menu-item"
                  onclick={copyAllRecognizedText}
                  disabled={ocrCopyAllBusy}
                  aria-busy={ocrCopyAllBusy}
                  aria-label="Copy all recognized text"
                  use:tip={"Copy all recognized on-screen text"}
                >{ocrCopyAllBusy ? "copying text…" : "copy text"}</button>
              {/if}
              {#if currentFrameHost}
                <button
                  type="button"
                  class="timeline__stage-action-menu-item timeline__stage-action-menu-item--open"
                  onclick={openCurrentFrameUrl}
                  disabled={openingCurrentFrameUrl}
                  use:tip={`Open ${currentFrameHost} in browser`}
                  aria-label={`Open ${currentFrameHost} in browser`}
                >
                  <svg class="timeline__stage-action-open-glyph" width="11" height="11" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M5.5 2.5H2.5v9h9v-3" />
                    <path d="M8 2.5h3.5V6" />
                    <path d="M7 7l4.5-4.5" />
                  </svg>
                  <span class="timeline__stage-action-open-host">{currentFrameHost}</span>
                </button>
              {/if}
            </div>
          {/if}
        </div>
        <div
          class="timeline__preview"
          role="img"
          aria-label={`frame ${previewDisplay?.frameId ?? timelineActive.id}`}
          style={`background-image: url("${previewUrl}");`}
        ></div>
        <img
          class="timeline__preview-load-sentinel"
          src={previewUrl}
          alt=""
          aria-hidden="true"
          onload={() => handleActivePreviewLoad(previewDisplay?.frameId ?? timelineActive.id)}
          onerror={() => handleActivePreviewLoadError(previewDisplay?.frameId ?? timelineActive.id)}
        />
        <!-- OCR overlay: anchored to the painted background-image rect
             (background-size: contain, centered) inside the stage. The
             rect is derived from stage size + the active frame's intrinsic
             width/height since there's no <img> element to measure.
             Pointer-events stay disabled so the overlay never blocks
             scrub/click on the stage. Boxes and labels only render once
             the exact active-frame preview is painted and an OCR run has
             produced observations for the currently active frame. -->
        {#if displayedActiveExactPreview && ocrVisible && ocrStatus === "success" && ocrFrameId === timelineActive.id && ocrObservations.length > 0 && renderedImageRect.width > 0 && renderedImageRect.height > 0}
          <div
            class="timeline__ocr-overlay"
            role="list"
            aria-label="Recognized on-screen text"
            style={`left: ${renderedImageRect.left}px; top: ${renderedImageRect.top}px; width: ${renderedImageRect.width}px; height: ${renderedImageRect.height}px;`}
          >
            {#each ocrObservations as obs, i (i)}
              <!-- Boxes are a pixel-positioned visual overlay; the recognized
                   text is exposed to assistive tech via the list/listitem role
                   + aria-label (no per-box tabindex — that would add dozens of
                   tab stops and trip a11y_no_noninteractive_tabindex). The
                   chip still reveals on hover for pointer users. -->
              <div
                class="timeline__ocr-box"
                role="listitem"
                style={ocrBoxStyleLocal(obs)}
                aria-label={`${obs.text} (${(obs.confidence * 100).toFixed(0)}% confidence)`}
                use:tip={`${obs.text} · ${(obs.confidence * 100).toFixed(0)}%`}
              >
                <span class="timeline__ocr-text">{obs.text}</span>
              </div>
            {/each}
            <span class="timeline__ocr-overlay-hint" aria-hidden="true">
              hover to read · copy all in ⋯
            </span>
          </div>
        {/if}
      {:else}
        <div class="timeline__preview-pending">
          {#if frameActionStatus?.tone !== "error"}
            <span class="timeline__preview-pending-spinner" aria-hidden="true"></span>
          {/if}
          <span>{frameActionStatus?.tone === "error" ? "preview unavailable" : "decoding preview…"}</span>
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

    {#if ocrSuccessUnpositionable}
      <div class="timeline__ocr-status timeline__ocr-status--empty" role="status" aria-live="polite">
        <span class="timeline__ocr-status-glyph" aria-hidden="true">⌶</span>
        <span>Text detected but can't be positioned — use ⋯ → Copy text.</span>
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
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">app</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.appName ?? "—"}{timelineActive.appBundleId ? ` (${timelineActive.appBundleId})` : ""}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">window</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.windowTitle ?? "—"}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">url</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.url ?? "—"}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">file</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">{timelineActive.filePath}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">ocr</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">
            {ocrStatus}{ocrSourceFrame?.ocrText ? ` · ${ocrSourceFrame.ocrText.length} chars` : ""}{ocrUsingEarlierFrame && ocrSourceFrame ? ` (from #${ocrSourceFrame.id})` : ""}
          </span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">image</span>
          <span class="timeline__overlay-val">{timelinePreviewDisplay?.source ?? "none"}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">source</span>
          <span class="timeline__overlay-val">{timelinePreviewDisplay?.sourceKind ?? "none"}</span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">fetch</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">
            exact {activeExactPreviewLoadMs == null ? "—" : `${activeExactPreviewLoadMs.toFixed(1)}ms`} scrub {activeScrubPreviewReady ? "ready" : "none"}
          </span>
        </div>
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
          <span class="timeline__overlay-key">scrub</span>
          <span class="timeline__overlay-val timeline__overlay-truncate">
            hit {scrubPreviewHitCount} req {scrubPreviewMissCount} batch {scrubPreviewBatchCount} ready {scrubPreviewGeneratedCount} queued {scrubPreviewQueuedCount} warm {scrubPreviewWarmCount} unavailable {scrubPreviewMissingCount}
          </span>
        </div>
        <div class="timeline__overlay-row">
          <span class="timeline__overlay-key">velocity</span>
          <span class="timeline__overlay-val">
            {previewScrubVelocityPxPerMs.toFixed(2)}px/ms
          </span>
        </div>
        {#if timelineActiveDuplicateOf}
          {@const duplicateOf = timelineActiveDuplicateOf}
          <div class="timeline__overlay-row">
            <span class="timeline__overlay-key">duplicateOf</span>
            <button
              type="button"
              class="timeline__overlay-link"
              onclick={() => void jumpToFrameWithBanner(duplicateOf)}
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
  <div
    class="timeline__rail-wrap"
    bind:this={timelineRailWrap}
  >
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
          {#each timelineAppGroups as group (group.key)}
            <div
              class="timeline-rail__app-group"
              class:timeline-rail__app-group--single={group.variant === "single"}
              class:timeline-rail__app-group--range={group.variant === "range"}
              style="right: {group.rightPx}px; width: {group.widthPx}px; --timeline-app-icon-left: {group.iconLeftPx}px"
              use:tip={timelineAppGroupTitle(group)}
              aria-hidden="true"
            >
              {#if group.showIcon}
                <span
                  class="timeline-rail__app-group-icon"
                  class:timeline-rail__app-group-icon--image={!!group.iconSrc}
                >
                  {#if group.iconSrc}
                    <img src={group.iconSrc} alt="" loading="lazy" />
                  {:else}
                    <span>{group.fallback}</span>
                  {/if}
                </span>
              {/if}
            </div>
          {/each}
          {#each timelineWindow as frame, j (frame.id)}
            {@const i = timelineWindowStart + j}
            {@const isAppGroupBoundary = timelineAppGroupBoundaryFrameIds.has(frame.id)}
            {@const isMajor = isAppGroupBoundary}
            <!-- Ticks are intentionally presentational (no role, not
                 focusable) so the parent's role="slider" is valid. The slider
                 itself owns position semantics via aria-valuenow/text, and
                 click-to-seek is handled by the rail's onclick. Slot 0
                 (newest) is anchored to the right of the track via `right:`. -->
            <div
              class="timeline-rail__slot"
              class:timeline-rail__slot--major={isMajor}
              class:timeline-rail__slot--app-boundary={isAppGroupBoundary}
              data-timeline-slot-index={i}
              style="right: {i * TIMELINE_SLOT_WIDTH}px"
              onpointerenter={(e) => onSlotPointerEnter(e, frame.id)}
              aria-hidden="true"
            >
              <span class="timeline-rail__tick"></span>
            </div>
          {/each}
        </div>
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
        <div class="timeline-rail__audio-lane-labels">
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--microphone">mic</span>
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--systemAudio">sys</span>
        </div>
        <div class="timeline-rail__audio-lane-viewport">
          {#if positionedAudioSegments.length > 0}
            <div
              class="timeline-rail__audio-lane-track"
              bind:this={timelineAudioLaneTrack}
              style="width: {timelineFrames.length *
                TIMELINE_SLOT_WIDTH}px"
            >
              <div class="timeline-rail__audio-row timeline-rail__audio-row--microphone" role="presentation">
                {#each positionedAudioSegments as seg (seg.id)}
                  {#if seg.visible && seg.source === "microphone"}
                    <button
                      type="button"
                      class="timeline-rail__audio-bar timeline-rail__audio-bar--microphone"
                      class:timeline-rail__audio-bar--selected={seg.id === selectedAudioSegmentId}
                      style="right: {seg.rightPx}px; width: {seg.widthPx}px"
                      use:tip={`${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} · ${seg.fileName} · ${formatUnixMs(seg.startUnixMs)} – ${formatUnixMs(seg.endUnixMs)}`}
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
                      use:tip={`${audioSourceLabel(seg.source)} segment ${seg.segmentIndex} · ${seg.fileName} · ${formatUnixMs(seg.startUnixMs)} – ${formatUnixMs(seg.endUnixMs)}`}
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
          {:else if audioSegmentsError}
            <div class="timeline-rail__audio-lane-error" role="alert">
              <span class="timeline-rail__audio-lane-error-label" use:tip={audioSegmentsError}>audio unavailable</span>
              <button
                type="button"
                class="btn btn--ghost btn--sm timeline-rail__audio-lane-retry"
                onclick={(e) => { e.stopPropagation(); void refreshAudioSegments(); }}
                onpointerdown={(e) => e.stopPropagation()}
                disabled={audioSegmentsLoading}
                use:tip={`Retry loading audio · ${audioSegmentsError}`}
              >{audioSegmentsLoading ? "retrying…" : "retry"}</button>
            </div>
          {:else}
            <span class="timeline-rail__audio-lane-empty">
              {#if audioSegmentsLoading}
                loading audio…
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
        <div class="timeline-rail__audio-lane-labels">
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--microphone">mic</span>
          <span class="timeline-rail__audio-lane-label timeline-rail__audio-lane-label--systemAudio">sys</span>
        </div>
        <div class="timeline-rail__audio-lane-viewport">
          {#if audioSegmentsError}
            <div class="timeline-rail__audio-lane-error" role="alert">
              <span class="timeline-rail__audio-lane-error-label" use:tip={audioSegmentsError}>audio unavailable</span>
              <button
                type="button"
                class="btn btn--ghost btn--sm timeline-rail__audio-lane-retry"
                onclick={() => void refreshAudioSegments()}
                disabled={audioSegmentsLoading}
                use:tip={`Retry loading audio · ${audioSegmentsError}`}
              >{audioSegmentsLoading ? "retrying…" : "retry"}</button>
            </div>
          {/if}
          <!-- The big stage empty state ("No frames yet") already carries the
               zero-frame messaging, so the lane stays silent here rather than
               stacking a redundant "no frames loaded" line beneath it. -->
        </div>
      </div>
    {/if}
    {#if timelineLoadingMore}
      <div class="timeline-rail__loading">loading…</div>
    {/if}
    {#if timelineFrames.length > 0 && tooltipFrame}
      {@const tooltipAppLabel = timelineFrameAppLabel(tooltipFrame)}
      {@const tooltipAppIconSrc = timelineFrameAppIconSrc(tooltipFrame)}
      <div
        id="timeline-rail-readout"
        class="timeline-rail__tooltip"
        class:timeline-rail__tooltip--pinned={!tooltipIsHovered}
        style={tooltipIsHovered && hoveredX != null
          ? `left: ${hoveredX}px; transform: translate(-50%, -100%);`
          : "left: 50%; transform: translate(-50%, -100%);"}
        role="tooltip"
      >
        {#if tooltipAppLabel}
          <span
            class="timeline-rail__tooltip-icon"
            class:timeline-rail__tooltip-icon--image={!!tooltipAppIconSrc}
            aria-hidden="true"
          >
            {#key tooltipAppKey}
              <span
                class="timeline-rail__tooltip-icon-inner"
                in:fly={{ x: -readoutScrubDirection * READOUT_FLY_OFFSET_PX, duration: readoutFlyDurationMs, opacity: 0 }}
                out:fly={{ x: readoutScrubDirection * READOUT_FLY_OFFSET_PX, duration: readoutFlyDurationMs, opacity: 0 }}
              >
                {#if tooltipAppIconSrc}
                  <img src={tooltipAppIconSrc} alt="" loading="lazy" />
                {:else}
                  <span>{timelineFrameAppFallback(tooltipFrame)}</span>
                {/if}
              </span>
            {/key}
          </span>
          <span class="timeline-rail__tooltip-copy">
            <span class="timeline-rail__tooltip-name-stack">
              {#key tooltipAppKey}
                <span
                  class="timeline-rail__tooltip-app-name"
                  in:fly={{ x: -readoutScrubDirection * READOUT_FLY_OFFSET_PX, duration: readoutFlyDurationMs, opacity: 0 }}
                  out:fly={{ x: readoutScrubDirection * READOUT_FLY_OFFSET_PX, duration: readoutFlyDurationMs, opacity: 0 }}
                >{tooltipAppLabel}</span>
              {/key}
            </span>
            <span class="timeline-rail__tooltip-meta">
              <span class="timeline-rail__tooltip-time">{formatCapturedTimeOnly(tooltipFrame.capturedAt)}</span>
              <span class="timeline-rail__tooltip-date">{formatCapturedDateOnly(tooltipFrame.capturedAt)}</span>
            </span>
          </span>
        {:else}
          <span class="timeline-rail__tooltip-copy timeline-rail__tooltip-copy--solo">
            <span class="timeline-rail__tooltip-time">{formatCapturedTimeOnly(tooltipFrame.capturedAt)}</span>
            <span class="timeline-rail__tooltip-date">{formatCapturedDateOnly(tooltipFrame.capturedAt)}</span>
          </span>
        {/if}
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
        use:tip={`${formatUnixMs(selectedAudioSegment.startUnixMs)} – ${formatUnixMs(selectedAudioSegment.endUnixMs)}`}
      >
        {formatTimeOfDay(selectedAudioSegment.startUnixMs)}
        <span class="audio-drawer__time-sep" aria-hidden="true">→</span>
        {formatTimeOfDay(selectedAudioSegment.endUnixMs)}
        <span class="audio-drawer__duration"
          >· {formatDurationSeconds(selectedAudioSegment.durationSeconds)}</span
        >
      </span>
      <span class="audio-drawer__file" use:tip={selectedAudioSegment.filePath}
        >{selectedAudioSegment.fileName}</span
      >
      <button
        type="button"
        class="audio-drawer__close"
        onclick={closeAudioDrawer}
        bind:this={audioDrawerCloseEl}
        aria-label="Close audio player"
      >
        <svg width="11" height="11" viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true">
          <path d="M3.5 3.5l7 7M10.5 3.5l-7 7" />
        </svg>
      </button>
    </div>
    {#if selectedAudioMediaLoading}
      <div class="audio-drawer__status" role="status" aria-live="polite" aria-busy="true">
        <span class="audio-drawer__spinner" aria-hidden="true"></span>
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
            <span class="audio-drawer__transcript-model" use:tip={selectedAudioTranscriptModelLabel}>
              · {selectedAudioTranscriptModelLabel}
            </span>
          {/if}
          {#if selectedAudioSpeakerGroups.length > 0}
            <span class="audio-drawer__transcript-hint">Click a line to jump · click a speaker to edit</span>
          {/if}
        </div>
        <div class="audio-drawer__transcript-actions">
          <button
            type="button"
            class="audio-drawer__transcript-action"
            onclick={reprocessSelectedAudioSegmentTranscript}
            disabled={selectedAudioTranscriptActionDisabled}
            use:tip={selectedAudioTranscriptActionTitle}
          >
            {selectedAudioTranscriptRerunLoading
              ? "Starting…"
              : selectedAudioTranscriptActionLabel}
          </button>
          <span
            class="audio-drawer__transcript-state audio-drawer__transcript-state--{selectedAudioTranscriptStatus}"
            role="status"
            aria-live="polite"
            aria-busy={selectedAudioSpeakerAnalysisRunning ||
              selectedAudioTranscriptStatus === "running" ||
              selectedAudioTranscriptStatus === "loading"}
          >
            {#if selectedAudioSpeakerAnalysisRunning || selectedAudioTranscriptStatus === "running" || selectedAudioTranscriptStatus === "loading"}
              <span class="audio-drawer__spinner audio-drawer__spinner--pill" aria-hidden="true"></span>
            {/if}
            {#if selectedAudioSpeakerAnalysisRunning}
              speakers
            {:else if selectedAudioSegment.source === "systemAudio" && selectedAudioTranscriptStatus === "running"}
              detecting speech
            {:else if selectedAudioSegment.source === "systemAudio" && selectedAudioTranscriptStatus === "empty"}
              no speech detected
            {:else if selectedAudioSegment.source === "systemAudio" && selectedAudioTranscriptStatus === "error"}
              speech detection failed
            {:else if selectedAudioTranscriptStatus === "loading"}
              loading
            {:else if selectedAudioTranscriptStatus === "running"}
              processing
            {:else if selectedAudioTranscriptStatus === "success"}
              completed
            {:else if selectedAudioTranscriptStatus === "empty"}
              no speech
            {:else if selectedAudioTranscriptStatus === "error"}
              error
            {:else if selectedAudioTranscriptStatus === "missing"}
              not run
            {:else}
              unavailable
            {/if}
          </span>
        </div>
      </div>
      {#if selectedAudioTranscriptRerunError}
        <p class="audio-drawer__transcript-error" role="alert">{selectedAudioTranscriptRerunError}</p>
      {/if}
      {#if selectedAudioTranscriptStatus === "success"}
        {#if selectedAudioSpeakerGroups.length > 0}
          <div
            class="audio-drawer__speaker-transcript"
            role="list"
            bind:this={selectedAudioTranscriptContainerEl}
          >
            {#each selectedAudioSpeakerGroups as group, index}
              {@const showPersonInlineAction = shouldShowPersonSuggestionRow(group, index)}
              {@const showMergeInlineAction = shouldShowMergeSuggestionRow(group, index)}
              <section
                class="audio-drawer__speaker-block"
                class:audio-drawer__speaker-block--actions-open={speakerActionsOpenIndex === index}
                class:audio-drawer__speaker-block--active={selectedAudioSpeakerActiveGroupIndex === index}
                data-speaker-group-index={index}
                role="listitem"
              >
                {#if speakerActionsOpenIndex === index}
                  <div
                    id={`speaker-actions-${group.clusterId}`}
                    class="audio-drawer__speaker-popover"
                    role="dialog"
                    popover="manual"
                    tabindex="-1"
                    aria-label={`Speaker actions for ${speakerDisplayLabel(group)}`}
                    bind:this={speakerActionsPopoverEl}
                    style:left={`${speakerActionsPopoverPos?.left ?? 12}px`}
                    style:bottom={`${speakerActionsPopoverPos?.bottom ?? 12}px`}
                    onpointerdown={(event) => event.stopPropagation()}
                  >
                    <div class="audio-drawer__speaker-popover-row audio-drawer__speaker-popover-row--name">
                      <label class="audio-drawer__speaker-label" for={`speaker-name-${group.clusterId}`}>
                        <span class="audio-drawer__speaker-toolset-label">Name</span>
                        <input
                          id={`speaker-name-${group.clusterId}`}
                          class="audio-drawer__speaker-label-input"
                          value={speakerNameDraft(group)}
                          aria-label={`Edit speaker name for ${formatTranscriptSegmentTitle(group)}`}
                          disabled={speakerCorrectionBusyClusterId === group.clusterId}
                          oninput={(event) => updateSpeakerNameDraft(group.clusterId, event)}
                          onblur={() => saveSpeakerNameIfChanged(group)}
                          onkeydown={(event) => handleSpeakerNameKeydown(event, group)}
                        />
                      </label>
                      <span class="audio-drawer__speaker-time">{formatTranscriptSegmentTitle(group)}</span>
                      {#if developerOptions.value && speakerConfidenceLabel(group)}
                        <span class="audio-drawer__speaker-confidence">
                          confidence {speakerConfidenceLabel(group)}
                        </span>
                      {/if}
                    </div>
                    {#if group.overlaps}
                      <div class="audio-drawer__speaker-overlap-note">Overlapping speech</div>
                    {/if}
                    <div class="audio-drawer__speaker-popover-row audio-drawer__speaker-popover-row--primary">
                      <button
                        type="button"
                        class="audio-drawer__speaker-tool audio-drawer__speaker-tool--primary"
                        disabled={speakerCorrectionBusyClusterId === group.clusterId || !canRememberSpeakerProfile(group)}
                        onclick={() => createAndLinkSpeakerProfile(group.clusterId, speakerNameDraft(group))}
                      >
                        Remember as profile
                      </button>
                      {#if group.personId != null}
                        <span class="audio-drawer__speaker-profile">
                          Linked to {speakerProfileName(group.personId) ?? speakerCleanLabel(group.speakerLabel)}
                        </span>
                      {/if}
                      {#if selectablePersonProfiles(group).length > 0}
                        <ActionSelect
                          compact
                          placeholder="Use saved person…"
                          ariaLabel="Use an existing saved person for this speaker"
                          disabled={speakerCorrectionBusyClusterId === group.clusterId}
                          options={selectablePersonProfiles(group).map((profile) => ({
                            value: String(profile.id),
                            label: profile.displayName,
                          }))}
                          onpick={(value) => linkSpeakerCluster(group.clusterId, Number(value))}
                        />
                      {/if}
                    </div>
                    <details class="audio-drawer__speaker-more">
                      <summary>More fixes</summary>
                      <div class="audio-drawer__speaker-popover-row audio-drawer__speaker-popover-row--secondary">
                        {#if group.personId != null}
                          {@const profileName = speakerProfileName(group.personId) ?? speakerCleanLabel(group.speakerLabel)}
                          <button
                            type="button"
                            class="audio-drawer__speaker-tool audio-drawer__speaker-tool--reject"
                            disabled={speakerCorrectionBusyClusterId === group.clusterId}
                            onclick={() => unlinkSpeakerProfile(group.clusterId)}
                          >
                            Not {profileName}
                          </button>
                        {/if}
                        {#if selectedAudioSpeakerClusters.length > 1}
                          {@const mergeTargets = selectedAudioSpeakerClusters
                            .filter((cluster) => cluster.id !== group.clusterId)
                            .map((cluster) => ({
                              value: String(cluster.id),
                              label: speakerClusterOptionLabel(cluster),
                            }))}
                          <ActionSelect
                            compact
                            placeholder="Same speaker as…"
                            ariaLabel="Merge this speaker cluster"
                            disabled={speakerCorrectionBusyClusterId === group.clusterId}
                            options={mergeTargets}
                            onpick={(value) => mergeSpeakerClusterById(group.clusterId, Number(value))}
                          />
                          <ActionSelect
                            compact
                            placeholder="Move this line to…"
                            ariaLabel="Move this visible speaker block"
                            disabled={speakerCorrectionBusyClusterId === group.clusterId}
                            options={mergeTargets}
                            onpick={(value) => moveSpeakerBlockTurns(group, Number(value))}
                          />
                        {/if}
                      </div>
                    </details>
                  </div>
                {/if}
                <div class="audio-drawer__speaker-label-stack">
                  <button
                    type="button"
                    class="audio-drawer__speaker-chip"
                    class:audio-drawer__speaker-chip--open={speakerActionsOpenIndex === index}
                    aria-haspopup="dialog"
                    aria-expanded={speakerActionsOpenIndex === index}
                    aria-controls={`speaker-actions-${group.clusterId}`}
                    use:tip={"Edit speaker"}
                    onclick={(event) => toggleSpeakerActions(index, event)}
                  >
                    {speakerPersistedName(group)}
                  </button>
                  {#if group.overlaps}
                    <span class="audio-drawer__speaker-overlap-note">Overlapping speech</span>
                  {/if}
                </div>
                <div class="audio-drawer__speaker-content">
                  {#if showPersonInlineAction || showMergeInlineAction}
                    <div
                      class="audio-drawer__speaker-actions"
                      role="group"
                      aria-label={`Speaker suggestions for ${speakerPersistedName(group)}`}
                    >
                      {#if showPersonInlineAction}
                        {@const suggestionName = speakerSuggestedPersonName(group)}
                        {@const confidenceLabel = speakerActionConfidenceLabel(group)}
                        <div class="audio-drawer__speaker-action-row">
                          <div class="audio-drawer__speaker-action-copy">
                            <span class="audio-drawer__speaker-action-title">Maybe {suggestionName}</span>
                            {#if confidenceLabel}
                              <span class="audio-drawer__speaker-action-meta">{confidenceLabel}</span>
                            {/if}
                          </div>
                          <div class="audio-drawer__speaker-action-buttons">
                            <button
                              type="button"
                              class="audio-drawer__speaker-action-button audio-drawer__speaker-action-button--confirm"
                              disabled={speakerInlineActionDisabled(group)}
                              aria-label={`Confirm ${suggestionName} for this speaker`}
                              onclick={() => confirmInlineSpeakerSuggestion(group)}
                            >
                              {speakerInlineActionIsBusy(group, "confirm")
                                ? speakerActionBusyLabel("confirm")
                                : "Confirm"}
                            </button>
                            <button
                              type="button"
                              class="audio-drawer__speaker-action-button audio-drawer__speaker-action-button--reject"
                              disabled={speakerInlineActionDisabled(group)}
                              aria-label={`Reject ${suggestionName} for this speaker`}
                              onclick={() => rejectInlineSpeakerSuggestion(group)}
                            >
                              {speakerInlineActionIsBusy(group, "reject")
                                ? speakerActionBusyLabel("reject")
                                : "Reject"}
                            </button>
                          </div>
                        </div>
                      {/if}
                      {#if showMergeInlineAction}
                        {@const mergeTargetLabel = suggestedMergeTargetLabel(group)}
                        {@const mergeScoreLabel = speakerActionScoreLabel(group)}
                        {#if mergeTargetLabel}
                          <div class="audio-drawer__speaker-action-row">
                            <div class="audio-drawer__speaker-action-copy">
                              <span class="audio-drawer__speaker-action-title">
                                Same speaker as {mergeTargetLabel}?
                              </span>
                              {#if mergeScoreLabel}
                                <span class="audio-drawer__speaker-action-meta">{mergeScoreLabel}</span>
                              {/if}
                            </div>
                            <div class="audio-drawer__speaker-action-buttons">
                              <button
                                type="button"
                                class="audio-drawer__speaker-action-button audio-drawer__speaker-action-button--confirm"
                                disabled={speakerInlineActionDisabled(group)}
                                aria-label={`Merge this speaker with ${mergeTargetLabel}`}
                                onclick={() => mergeInlineSpeakerSuggestion(group)}
                              >
                                {speakerInlineActionIsBusy(group, "merge")
                                  ? speakerActionBusyLabel("merge")
                                  : "Merge"}
                              </button>
                            </div>
                          </div>
                        {/if}
                      {/if}
                    </div>
                  {/if}
                  <button
                    type="button"
                    class="audio-drawer__speaker-text"
                    class:audio-drawer__speaker-text--active={selectedAudioSpeakerActiveGroupIndex === index}
                    use:tip={`Jump to ${formatTranscriptSegmentTitle(group)}`}
                    onclick={() => onSpeakerLineClick(group)}
                  >
                    {group.text}
                  </button>
                </div>
              </section>
            {/each}
          </div>
          {#if speakerCorrectionError}
            <p class="audio-drawer__transcript-error" role="alert">{speakerCorrectionError}</p>
          {/if}
        {:else if selectedAudioTranscriptSegments.length > 0}
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
                use:tip={`Jump to ${formatTranscriptSegmentTitle(segment)}`}
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
        {#if selectedAudioSpeakerTurnsError}
          <div class="audio-drawer__transcript-error-row" role="alert">
            <p class="audio-drawer__transcript-error">{selectedAudioSpeakerTurnsError}</p>
            {#if selectedAudioSpeakerRetryVisible}
              <button
                type="button"
                class="audio-drawer__transcript-action audio-drawer__transcript-action--retry"
                disabled={selectedAudioSpeakerRetryDisabled}
                onclick={reprocessSelectedAudioSegmentSpeakerAnalysis}
              >
                {selectedAudioSpeakerAnalysisRetryLoading
                  ? "Retrying..."
                  : "Retry speaker analysis"}
              </button>
            {/if}
          </div>
        {/if}
        {#if selectedAudioSpeakerTurnsNotice && !selectedAudioSpeakerTurnsError}
          <p class="audio-drawer__transcript-empty">{selectedAudioSpeakerTurnsNotice}</p>
        {/if}
      {:else if selectedAudioTranscriptStatus === "empty"}
        <p class="audio-drawer__transcript-empty">No speech detected in this segment.</p>
      {:else if selectedAudioTranscriptStatus === "loading"}
        <p class="audio-drawer__transcript-empty audio-drawer__transcript-empty--loading" role="status" aria-live="polite" aria-busy="true">
          <span class="audio-drawer__spinner" aria-hidden="true"></span>
          Loading transcript…
        </p>
      {:else if selectedAudioTranscriptStatus === "running"}
        <p class="audio-drawer__transcript-empty audio-drawer__transcript-empty--loading" role="status" aria-live="polite" aria-busy="true">
          <span class="audio-drawer__spinner" aria-hidden="true"></span>
          Transcription is queued or still processing.
        </p>
      {:else if selectedAudioTranscriptStatus === "error"}
        <p class="audio-drawer__transcript-error" role="alert">{selectedAudioTranscriptError}</p>
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
    position: relative;
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

  /* Align bar-2 control typography with the app titlebar (10px). */
  .timeline__bar .btn--sm {
    font-size: var(--text-xs);
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
    max-height: 50vh;
    padding: 8px 12px 10px;
    /* The drawer itself no longer scrolls — the meta row and transport stay
       pinned while only the transcript body scrolls (see
       `.audio-drawer__transcript`). Auto-scroll-to-active therefore moves the
       transcript, never the controls. */
    overflow: hidden;
    scrollbar-width: none;
    background: var(--app-surface-raised);
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

  .audio-drawer::-webkit-scrollbar {
    display: none;
  }

  .audio-drawer:focus-visible {
    border-color: var(--app-accent);
    box-shadow:
      0 18px 40px rgba(0, 0, 0, 0.55),
      var(--app-ring);
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

  /* Honor the OS reduced-motion preference for every CSS animation defined in
     this surface: the drawer's slide-in entrance, the OCR-running glyph pulse,
     and the OCR spinner. Spinners/pulses degrade to a static state; the drawer
     simply appears without the rising transform. */
  @media (prefers-reduced-motion: reduce) {
    .audio-drawer {
      animation: none;
    }

    .timeline__ocr-btn--running .timeline__ocr-glyph {
      animation: none;
    }

    .timeline__ocr-spinner,
    .timeline__preview-pending-spinner,
    .audio-drawer__spinner {
      animation: none;
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
      var(--app-source-mic),
      var(--app-source-mic-strong)
    );
  }

  .audio-drawer__source--systemAudio .audio-drawer__swatch {
    background: linear-gradient(
      90deg,
      var(--app-source-sysaudio),
      var(--app-source-sysaudio-strong)
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
    font-family: var(--app-font-mono);
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
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--app-text-muted);
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
    background: color-mix(in srgb, var(--app-danger-strong) 8%, transparent);
    outline: none;
  }

  /* A distinct ring on keyboard focus so the close affordance no longer reads
     identically to its hover state. */
  .audio-drawer__close:focus-visible {
    box-shadow: var(--app-ring);
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
    background: color-mix(in srgb, var(--app-record-glyph-start) 10%, transparent);
    border: 1px solid var(--app-status-running-border);
    border-radius: 50%;
    color: var(--app-status-running-fg);
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s,
      transform 0.08s;
  }

  .audio-drawer__play:hover {
    background: color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
    border-color: var(--app-record-glyph-start);
  }

  .audio-drawer__play:focus-visible {
    outline: none;
    border-color: var(--app-record-glyph-start);
    box-shadow: var(--app-ring-danger);
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
    color: var(--app-status-running-fg);
  }

  .audio-drawer__scrub:disabled {
    cursor: not-allowed;
    opacity: var(--app-disabled-opacity);
  }

  .audio-drawer__scrub::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--app-record-glyph-start) 0%,
      var(--app-record-glyph-start) var(--audio-progress, 0%),
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
    background: var(--app-record-glyph-start);
  }

  .audio-drawer__scrub::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-status-running-fg);
    border: 2px solid var(--app-surface-raised);
    margin-top: -3px;
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--app-record-glyph-start) 0%, transparent);
    transition:
      transform 0.12s,
      box-shadow 0.12s;
  }

  .audio-drawer__scrub::-moz-range-thumb {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--app-status-running-fg);
    border: 2px solid var(--app-surface-raised);
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--app-record-glyph-start) 0%, transparent);
    transition:
      transform 0.12s,
      box-shadow 0.12s;
  }

  .audio-drawer__scrub:hover::-webkit-slider-thumb,
  .audio-drawer__scrub:focus-visible::-webkit-slider-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
  }

  .audio-drawer__scrub:hover::-moz-range-thumb,
  .audio-drawer__scrub:focus-visible::-moz-range-thumb {
    transform: scale(1.15);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--app-record-glyph-start) 18%, transparent);
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

  /* Indeterminate motion for in-flight audio/transcript states so a long
     poll or media load reads as active rather than stuck. Reuses the shared
     `timeline-ocr-spin` keyframe; degrades to a static ring under
     reduced-motion (see the media query below). */
  .audio-drawer__spinner {
    flex: 0 0 auto;
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--app-text-muted) 30%, transparent);
    border-top-color: var(--app-text-muted);
    animation: timeline-ocr-spin 0.9s linear infinite;
  }

  .audio-drawer__spinner--pill {
    width: 9px;
    height: 9px;
    border-width: 1.25px;
    vertical-align: middle;
    margin-right: 4px;
  }

  .audio-drawer__transcript-empty--loading {
    display: flex;
    align-items: center;
    gap: 8px;
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
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-danger);
    padding-top: 1px;
  }

  .audio-drawer__error-msg {
    flex: 1 1 auto;
    font-family: var(--app-font-mono);
    word-break: break-word;
    line-height: 1.4;
  }

  .audio-drawer__transcript {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 2px;
    padding: 8px 10px;
    /* Take the drawer's remaining height and allow shrinking so the body
       below can own the scroll instead of the whole drawer. */
    flex: 1 1 auto;
    min-height: 0;
    background: color-mix(in srgb, var(--app-surface-hover) 58%, transparent);
    border: 1px solid var(--app-border);
    border-radius: 6px;
  }

  /* Scrollable transcript bodies: each becomes its own scroll region (so the
     transport stays pinned) with a thin, always-styled scrollbar as the scroll
     signifier and contained overscroll so a transcript scroll never chains out
     to dismiss the drawer or scroll the page. */
  .audio-drawer__speaker-transcript,
  .audio-drawer__transcript-text--segmented,
  .audio-drawer__transcript-text {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
  }

  .audio-drawer__speaker-transcript::-webkit-scrollbar,
  .audio-drawer__transcript-text--segmented::-webkit-scrollbar,
  .audio-drawer__transcript-text::-webkit-scrollbar {
    width: 8px;
  }

  .audio-drawer__speaker-transcript::-webkit-scrollbar-thumb,
  .audio-drawer__transcript-text--segmented::-webkit-scrollbar-thumb,
  .audio-drawer__transcript-text::-webkit-scrollbar-thumb {
    background: var(--app-border-strong);
    border-radius: 4px;
    border: 2px solid transparent;
    background-clip: padding-box;
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
    font-size: var(--text-xs);
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

  .audio-drawer__transcript-hint {
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
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
    font-size: var(--text-sm);
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

  .audio-drawer__transcript-action:focus-visible:not(:disabled) {
    box-shadow: var(--app-ring);
  }

  .audio-drawer__transcript-action:disabled {
    opacity: var(--app-disabled-opacity);
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
    color: var(--app-text);
    white-space: pre-wrap;
  }

  .audio-drawer__transcript-text--segmented {
    white-space: normal;
  }

  .audio-drawer__speaker-transcript {
    display: grid;
    gap: 2px;
  }

  .audio-drawer__speaker-block {
    position: relative;
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 8px;
    align-items: baseline;
    padding: 3px 0 3px 8px;
    border-left: 2px solid transparent;
    background: transparent;
  }

  .audio-drawer__speaker-block--active {
    border-left-color: var(--app-accent);
    background: color-mix(in srgb, var(--app-accent) 6%, transparent);
  }

  .audio-drawer__speaker-block--actions-open {
    border-left-color: var(--app-accent);
  }

  .audio-drawer__speaker-label-stack {
    display: grid;
    gap: 1px;
    align-self: start;
  }

  .audio-drawer__speaker-content {
    display: grid;
    gap: 3px;
    min-width: 0;
    align-self: start;
  }

  .audio-drawer__speaker-popover {
    /* Top-layer popover (`popover="manual"`): escapes the transcript's
       scroll clip and the drawer's `overflow: hidden`. The inline
       `left`/`bottom` (viewport coords, computed from the clicked chip)
       win over `inset: auto`; the margin/overflow resets undo the UA
       `[popover]` defaults so nested dropdowns aren't clipped. */
    position: fixed;
    inset: auto;
    /* The UA sheet also sets `height: fit-content`, which WebKit resolves to
       the full space between the viewport top and the inline `bottom` —
       stretching the popover into a giant panel. Reset it to content height. */
    height: auto;
    margin: 0;
    overflow: visible;
    display: grid;
    gap: 7px;
    width: min(42rem, calc(100vw - 64px));
    padding: 9px 10px;
    border: 1px solid var(--app-border-strong);
    border-radius: 8px;
    background: var(--app-surface-raised);
    /* The UA `[popover]` style sets `color: CanvasText`; restore the app
       palette the popover used to inherit from the drawer. */
    color: var(--app-text);
    box-shadow: var(--app-shadow-popover);
  }

  .audio-drawer__speaker-chip {
    display: inline-flex;
    gap: 5px;
    align-items: baseline;
    max-width: 10rem;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--app-accent);
    font: inherit;
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-align: left;
    text-transform: uppercase;
    cursor: pointer;
  }

  .audio-drawer__speaker-chip::after {
    content: "⋯";
    color: var(--app-text-faint);
    opacity: 0;
    transition: opacity 0.12s;
  }

  .audio-drawer__speaker-chip:hover,
  .audio-drawer__speaker-chip:focus-visible,
  .audio-drawer__speaker-chip--open {
    color: var(--app-text);
    outline: none;
  }

  /* Keyboard focus gets a ring so it's distinguishable from the hover tint. */
  .audio-drawer__speaker-chip:focus-visible {
    border-radius: 3px;
    box-shadow: var(--app-ring);
  }

  .audio-drawer__speaker-chip:hover::after,
  .audio-drawer__speaker-chip:focus-visible::after,
  .audio-drawer__speaker-chip--open::after {
    opacity: 1;
  }

  .audio-drawer__speaker-popover-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }

  .audio-drawer__speaker-popover-row--name {
    gap: 10px;
  }

  .audio-drawer__speaker-popover-row--primary {
    padding-top: 2px;
  }

  .audio-drawer__speaker-popover-row--secondary {
    padding-top: 7px;
  }

  .audio-drawer__speaker-more {
    margin-top: 1px;
    border-top: 1px solid color-mix(in srgb, var(--app-border) 72%, transparent);
    color: var(--app-text);
  }

  .audio-drawer__speaker-more > summary {
    width: fit-content;
    padding-top: 7px;
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .audio-drawer__speaker-more > summary:hover,
  .audio-drawer__speaker-more > summary:focus-visible {
    color: var(--app-text);
    outline: none;
  }

  .audio-drawer__speaker-label {
    display: inline-flex;
    gap: 7px;
    align-items: center;
    min-width: 0;
    max-width: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--app-text);
    font: inherit;
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: text;
  }

  .audio-drawer__speaker-label-input {
    width: min(100%, 10rem);
    min-width: 4.5rem;
    padding: 3px 6px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text);
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }

  .audio-drawer__speaker-label-input:focus {
    border-color: var(--app-accent);
    outline: none;
  }

  .audio-drawer__speaker-label-input:disabled {
    cursor: not-allowed;
    opacity: var(--app-disabled-opacity);
  }

  .audio-drawer__speaker-time {
    justify-self: start;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--app-text);
    font: inherit;
    /* Bumped from 9px: the uppercase secondary metadata was borderline-legible. */
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .audio-drawer__speaker-time:hover,
  .audio-drawer__speaker-time:focus-visible {
    color: var(--app-text);
    text-decoration: underline;
    text-underline-offset: 3px;
    outline: none;
  }

  .audio-drawer__speaker-toolset-label {
    color: var(--app-text-muted);
    font-size: var(--text-xs);
    font-weight: 900;
    letter-spacing: 0.13em;
    line-height: 1;
    text-transform: uppercase;
  }

  .audio-drawer__speaker-tool {
    min-height: 24px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    color: var(--app-text);
    font: inherit;
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .audio-drawer__speaker-tool {
    padding: 3px 8px;
    cursor: pointer;
  }

  .audio-drawer__speaker-tool--primary {
    border-color: color-mix(in srgb, var(--app-accent) 58%, var(--app-border));
    background: color-mix(in srgb, var(--app-accent) 14%, var(--app-surface));
    color: var(--app-accent);
  }

  .audio-drawer__speaker-tool--confirm {
    color: color-mix(in srgb, var(--app-accent) 78%, var(--app-text));
  }

  .audio-drawer__speaker-tool--reject {
    color: var(--app-warn);
  }

  .audio-drawer__speaker-profile {
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    line-height: 1.2;
    text-transform: uppercase;
  }

  .audio-drawer__speaker-tool:hover:not(:disabled),
  .audio-drawer__speaker-tool:focus-visible:not(:disabled) {
    border-color: var(--app-accent);
    color: var(--app-text);
    outline: none;
  }

  /* Reject is a benign-decline, not an affirmative — keep its hover warn-tinted
     rather than adopting the accent-green confirm treatment. */
  .audio-drawer__speaker-tool--reject:hover:not(:disabled),
  .audio-drawer__speaker-tool--reject:focus-visible:not(:disabled) {
    border-color: var(--app-warn-border);
    color: var(--app-warn);
  }

  .audio-drawer__speaker-tool:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .audio-drawer__speaker-label:hover,
  .audio-drawer__speaker-label:focus-within {
    color: var(--app-text);
    outline: none;
  }

  .audio-drawer__speaker-overlap-note {
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 700;
    line-height: 1.2;
  }

  .audio-drawer__speaker-confidence {
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .audio-drawer__speaker-actions {
    display: grid;
    gap: 3px;
    min-width: 0;
    margin: 1px 0 2px;
  }

  .audio-drawer__speaker-action-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 5px 10px;
    min-width: 0;
    padding: 4px 6px;
    border-left: 1px solid color-mix(in srgb, var(--app-accent) 42%, var(--app-border));
    background: color-mix(in srgb, var(--app-accent) 5%, transparent);
  }

  .audio-drawer__speaker-action-copy {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 5px 8px;
    min-width: 0;
    line-height: 1.25;
  }

  .audio-drawer__speaker-action-title {
    color: var(--app-text);
    font-size: 10px;
    font-weight: 800;
  }

  .audio-drawer__speaker-action-meta {
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 700;
  }

  .audio-drawer__speaker-action-buttons {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 5px;
    flex: 0 0 auto;
  }

  .audio-drawer__speaker-action-button {
    min-height: 22px;
    padding: 2px 8px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: color-mix(in srgb, var(--app-surface-raised) 72%, transparent);
    color: var(--app-text);
    font: inherit;
    font-size: 9px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      border-color 0.12s,
      color 0.12s,
      background 0.12s,
      opacity 0.12s;
  }

  .audio-drawer__speaker-action-button--confirm {
    color: color-mix(in srgb, var(--app-accent) 82%, var(--app-text));
  }

  .audio-drawer__speaker-action-button--reject {
    color: var(--app-warn);
  }

  .audio-drawer__speaker-action-button:hover:not(:disabled),
  .audio-drawer__speaker-action-button:focus-visible:not(:disabled) {
    border-color: var(--app-accent);
    background: color-mix(in srgb, var(--app-accent) 12%, var(--app-surface-raised));
    color: var(--app-text);
    outline: none;
  }

  /* Reject is a benign-decline, not an affirmative — keep its hover warn-tinted
     rather than adopting the confirm action's accent-green. */
  .audio-drawer__speaker-action-button--reject:hover:not(:disabled),
  .audio-drawer__speaker-action-button--reject:focus-visible:not(:disabled) {
    border-color: var(--app-warn-border);
    background: color-mix(in srgb, var(--app-warn) 12%, var(--app-surface-raised));
    color: var(--app-warn);
  }

  .audio-drawer__speaker-action-button:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .audio-drawer__speaker-text {
    width: 100%;
    min-width: 0;
    margin: 0;
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text);
    font: inherit;
    font-size: 13px;
    line-height: 1.55;
    text-align: left;
    cursor: pointer;
  }

  .audio-drawer__speaker-text:hover,
  .audio-drawer__speaker-text:focus-visible,
  .audio-drawer__speaker-text--active {
    background: color-mix(in srgb, var(--app-accent) 8%, transparent);
    outline: none;
  }

  /* Distinct keyboard-focus ring so a focused line isn't mistaken for a hover. */
  .audio-drawer__speaker-text:focus-visible {
    box-shadow: var(--app-ring);
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

  .audio-drawer__transcript-segment:focus-visible {
    box-shadow: var(--app-ring);
  }

  .audio-drawer__transcript-segment--active {
    background: color-mix(in srgb, var(--app-accent) 18%, transparent);
    color: var(--app-text);
  }

  .audio-drawer__transcript-empty {
    color: var(--app-text-muted);
    font-style: italic;
  }

  .audio-drawer__transcript-error-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px 10px;
  }

  .audio-drawer__transcript-action--retry {
    flex: 0 0 auto;
  }

  .audio-drawer__transcript-error {
    color: var(--app-danger-text);
    font-family: var(--app-font-mono);
    word-break: break-word;
  }

  @media (max-width: 640px) {
    .audio-drawer__speaker-block {
      grid-template-columns: minmax(0, 1fr);
      gap: 3px;
    }

    .audio-drawer__speaker-action-row {
      align-items: flex-start;
    }

    .audio-drawer__speaker-action-buttons {
      flex: 1 1 100%;
    }
  }

  /* ── Buttons (subset used by the timeline) ─────────────────── */
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: var(--text-sm);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .btn:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .btn:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }

  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
    font-size: var(--text-sm);
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: var(--text-sm);
  }

  /* The previous dashboard-local settings/menu anchor moved into the shared
     title bar as reusable surface actions, so those local rules were removed. */

  /* ── Error / empty ─────────────────────────────────────────── */
  .timeline__error {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px 12px;
    flex-wrap: wrap;
    padding: 10px 12px;
    background: var(--app-danger-bg-soft);
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    font-size: var(--text-sm);
    color: var(--app-danger-text);
  }

  .timeline__error-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .timeline__error-retry {
    flex: 0 0 auto;
  }

  .timeline__error-label {
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-danger);
  }

  .timeline__error-msg {
    font-family: var(--app-font-mono);
    word-break: break-word;
  }

  .timeline__empty {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
    justify-content: center;
    text-align: center;
    max-width: 360px;
    padding: 24px;
  }

  .timeline__empty-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 4px;
    color: var(--app-text-muted);
  }

  .timeline__empty-glyph :global(svg) {
    width: 40px;
    height: 40px;
    stroke-width: 1.5;
  }

  .timeline__empty-title {
    margin: 0;
    font-family: inherit;
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--app-text);
  }

  /* Live-capture variant of the empty state: a pulsing record dot inline with
     the title so the surface reads as "recording, waiting for first frames"
     rather than the idle "Press Record" prompt. */
  .timeline__empty--capturing .timeline__empty-title {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .timeline__empty-rec-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-danger-strong);
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--app-danger-strong) 60%, transparent);
    animation: timeline-empty-rec-pulse 1.6s ease-out infinite;
  }

  @keyframes timeline-empty-rec-pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--app-danger-strong) 55%, transparent);
    }
    70% {
      box-shadow: 0 0 0 6px color-mix(in srgb, var(--app-danger-strong) 0%, transparent);
    }
    100% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--app-danger-strong) 0%, transparent);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .timeline__empty-rec-dot {
      animation: none;
    }
  }

  .timeline__empty-hint {
    margin: 0;
    font-size: 13px;
    line-height: 1.45;
    color: var(--app-text-muted);
  }

  .timeline__empty-cue {
    margin: 4px 0 0;
    font-size: 13px;
    line-height: 1.45;
    color: var(--app-text-muted);
  }

  /* "Record" is a title-bar button, not a keystroke — so it reads as an
     emphasized inline label with a small record-dot glyph that matches the
     title-bar control, rather than a misleading kbd chip. */
  .timeline__empty-cue-key {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--app-text-strong);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .timeline__empty-cue-key::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-danger-strong);
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

  /* When the timeline load fails but stale frames remain decoded, dim and
     desaturate the stage so the last preview never reads as live data. The
     inline alert above carries the recovery action. */
  .timeline__stage--stale {
    opacity: var(--app-disabled-opacity);
    filter: grayscale(0.6);
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
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* The play-this-moment trigger tints toward the recording accent so it reads
     as a distinct "listen" affordance next to the neutral frame-actions menu. */
  .timeline__stage-play-moment {
    color: color-mix(in srgb, var(--app-accent) 70%, var(--app-text-muted));
  }

  .timeline__stage-play-moment:hover {
    color: var(--app-accent-strong, var(--app-accent));
  }

  .timeline__stage-actions--open {
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

  .timeline__stage-action-glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .timeline__stage-action-glyph :global(svg) {
    width: 18px;
    height: 18px;
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

  .timeline__stage-actions--open > .timeline__stage-action-trigger {
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

  /* Disabled menu item (preview-not-ready, or an open already in flight): dim it
     and drop the pointer cursor so it reads as inert without shifting layout. */
  .timeline__stage-action-menu-item:disabled {
    cursor: default;
    opacity: var(--app-disabled-opacity);
  }

  .timeline__stage-action-menu-item:disabled:hover {
    background: transparent;
    border-color: transparent;
    color: var(--app-text-muted);
  }

  /* The "open in browser" peer reuses the menu-item shell so it reads as a
     sibling of copy/download. The host is a real domain (mixed case), so it
     opts out of the items' uppercase label transform and lets a long host
     ellipsize within the menu's bounded width. */
  .timeline__stage-action-menu-item--open {
    gap: 7px;
    text-transform: none;
    letter-spacing: 0.02em;
  }

  .timeline__stage-action-open-glyph {
    flex: 0 0 auto;
  }

  .timeline__stage-action-open-host {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .timeline__stage-status {
    position: absolute;
    right: 10px;
    bottom: 10px;
    z-index: 2;
    max-width: min(60%, 360px);
    padding: 7px 9px;
    display: flex;
    align-items: flex-start;
    gap: 8px;
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

  .timeline__stage-status-body {
    display: grid;
    gap: 4px;
    min-width: 0;
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
    font-family: var(--app-font-mono);
    font-size: 9px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* Errors persist until dismissed, so they read at a more legible size than
     the transient neutral acks and carry a close affordance. */
  .timeline__stage-status--error {
    color: color-mix(in srgb, var(--app-danger) 72%, var(--app-text) 28%);
    border-color: color-mix(in srgb, var(--app-danger) 40%, var(--app-overlay-border));
    font-size: 11px;
    line-height: 1.4;
  }

  .timeline__stage-status--error .timeline__stage-status-detail {
    font-size: 10px;
  }

  .timeline__stage-status-close {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    margin: -1px -2px 0 0;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: inherit;
    opacity: 0.75;
    cursor: pointer;
    transition: opacity 0.12s ease, background 0.12s ease;
  }

  .timeline__stage-status-close:hover {
    opacity: 1;
    background: color-mix(in srgb, currentColor 14%, transparent);
  }

  .timeline__stage-status-close:focus-visible {
    outline: none;
    opacity: 1;
    box-shadow: var(--app-ring);
  }

  .timeline__preview-pending {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  .timeline__preview-pending-spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--app-text-muted) 30%, transparent);
    border-top-color: var(--app-text-muted);
    animation: timeline-ocr-spin 0.9s linear infinite;
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
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    align-self: center;
  }

  .timeline__overlay-val {
    font-family: var(--app-font-mono);
    font-size: 10px;
    color: var(--app-text);
    min-width: 0;
  }

  .timeline__overlay-link {
    font-family: var(--app-font-mono);
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

  /* When OCR is toggled ON the button carries a resting accent tint so its
     stateful (pressed) nature reads at a glance, distinct from the plain
     refresh ghost button. The :not() guards keep the run-state modifiers
     (running/error/success) authoritative over this resting colour. */
  .timeline__ocr-btn[aria-pressed="true"]:not(.timeline__ocr-btn--running):not(.timeline__ocr-btn--error):not(.timeline__ocr-btn--success) {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  .timeline__ocr-btn[aria-pressed="true"]:not(.timeline__ocr-btn--running):not(.timeline__ocr-btn--error):not(.timeline__ocr-btn--success) .timeline__ocr-glyph {
    color: var(--app-accent);
  }

  .timeline__ocr-glyph {
    display: inline-flex;
    align-items: center;
    line-height: 1;
    color: var(--app-text-muted);
  }

  .timeline__ocr-glyph :global(svg) {
    width: 1.2em;
    height: 1.2em;
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
    background: color-mix(in srgb, var(--app-warn) 6%, transparent);
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

  /* Persistent signifier that the overlaid boxes hold copyable recognized
     text — the per-box chip is otherwise only discoverable by hovering each
     box. Non-interactive (pointer-events stay off so scrub/click pass through)
     and tucked into the corner so it never competes with the frame. */
  .timeline__ocr-overlay-hint {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 2px 6px;
    border-radius: 3px;
    background: var(--app-ocr-chip-bg);
    color: var(--app-ocr-chip-text);
    border: 1px solid var(--app-ocr-chip-border);
    font-family: var(--app-font-mono);
    font-size: 10px;
    letter-spacing: 0.02em;
    line-height: 1.2;
    white-space: nowrap;
    opacity: 0.72;
    pointer-events: none;
  }

  .timeline__ocr-box {
    position: absolute;
    border: 1px solid var(--app-ocr-box);
    /* A faint at-rest fill (plus the text cursor) signals these boxes are
       live, copyable text rather than inert decoration — without tinting the
       preview enough to fight the underlying pixels. Hover deepens it. */
    background: color-mix(in srgb, var(--app-ocr-box-fill) 18%, transparent);
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

  .timeline__ocr-box:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
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
    font-family: var(--app-font-mono);
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
    font-size: var(--text-sm);
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
    font-family: var(--app-font-mono);
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

  /* Persistent center playhead: a pair of neutral carets framing the rail's
     midpoint so the relationship "center = currently shown frame" is
     self-evident without reading the floating tooltip. Carets sit over the
     rail's top/bottom edges (rail height = 36px), never covering the active
     tick body, and stay non-interactive. */
  .timeline__rail-wrap::before,
  .timeline__rail-wrap::after {
    content: "";
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    width: 0;
    height: 0;
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    pointer-events: none;
    z-index: 4;
  }

  .timeline__rail-wrap::before {
    top: 0;
    border-top: 5px solid var(--app-text-subtle);
  }

  .timeline__rail-wrap::after {
    top: 31px;
    border-bottom: 5px solid var(--app-text-subtle);
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
    /* Track is 34px + 1px top/bottom border = 36px. Locking the rail's
       height (rather than letting it derive from content) ensures that
       transient in-flow children (e.g. previous sticky loader, future
       overlays) cannot grow the rail and ripple height into the stage. */
    height: 36px;
    flex: 0 0 36px;
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
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .timeline-rail::-webkit-scrollbar {
    display: none;
  }

  .timeline-rail__track {
    position: relative;
    height: 34px;
    /* Symmetric viewport-relative spacers so the first/last frames can sit
       under the centered cursor caret. Using `cqi` (rail's inline size)
       rather than `%` (which resolves against the track's own width and
       drifts wildly with frame count) makes both centering and click
       positioning reliable. Margin — not padding — is required because slot
       ticks are absolutely positioned and would ignore padding offsets. */
    margin-left: calc(50cqi - 4px);
    margin-right: calc(50cqi - 4px);
  }

  .timeline-rail__app-group {
    position: absolute;
    top: 8px;
    z-index: 1;
    height: 20px;
    overflow: visible;
    pointer-events: none;
  }

  .timeline-rail__app-group-icon {
    position: absolute;
    top: 0;
    left: var(--timeline-app-icon-left);
    width: 20px;
    height: 20px;
    min-width: 20px;
    min-height: 20px;
    box-sizing: border-box;
    display: grid;
    place-items: center;
    border-radius: 5px;
    overflow: hidden;
    color: var(--app-text-strong);
    font-size: 10px;
    font-weight: 800;
    line-height: 1;
    background: color-mix(in srgb, var(--app-surface-raised) 96%, var(--app-bg));
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--app-border-strong) 70%, transparent),
      0 1px 3px rgba(0, 0, 0, 0.22);
  }

  .timeline-rail__app-group-icon--image {
    padding: 2px;
    background: color-mix(in srgb, var(--app-surface-raised) 88%, var(--app-bg));
  }

  .timeline-rail__app-group-icon img {
    width: 100%;
    height: 100%;
    display: block;
    border-radius: 3px;
    object-fit: contain;
  }

  .timeline-rail__slot {
    position: absolute;
    top: 8px;
    width: 8px;
    height: 18px;
    margin: 0;
    padding: 0;
    background: transparent;
    border: 0;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
    z-index: 2;
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
    padding: 0 4px 0 0;
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    user-select: none;
  }

  /* Each label owns half the lane height so its baseline lands on the
     vertical center of its row in the viewport (mic row center ~6.5px,
     sys row center ~19.5px within the 26px lane). */
  .timeline-rail__audio-lane-label {
    flex: 1 1 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    line-height: 1;
  }

  .timeline-rail__audio-lane-label--microphone {
    color: var(--app-source-mic);
  }

  .timeline-rail__audio-lane-label--systemAudio {
    color: var(--app-source-sysaudio);
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
    transform: translateX(0px);
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
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    pointer-events: none;
  }

  .timeline-rail__audio-lane-error {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    pointer-events: none;
  }

  .timeline-rail__audio-lane-error-label {
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-danger-text);
  }

  /* The retry now reuses the shared `btn btn--ghost btn--sm` style so the
     audio-lane retry matches the timeline retry (the danger-bordered variant
     was the lone inconsistent retry affordance). This class only restores
     pointer-events inside the pointer-events:none lane error row. */
  .timeline-rail__audio-lane-retry {
    pointer-events: auto;
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
      0 0 0 1px var(--app-border-hover);
  }

  .timeline-rail__audio-bar:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.6),
      var(--app-ring);
    z-index: 2;
  }

  .timeline-rail__audio-bar--selected {
    box-shadow:
      0 0 0 0.5px rgba(0, 0, 0, 0.6),
      0 0 0 1.5px var(--app-record-glyph-start),
      0 0 8px color-mix(in srgb, var(--app-record-glyph-start) 45%, transparent);
    z-index: 1;
  }

  .timeline-rail__audio-bar--microphone {
    background: linear-gradient(
      180deg,
      var(--app-source-mic),
      var(--app-source-mic-strong)
    );
  }

  .timeline-rail__audio-bar--systemAudio {
    background: linear-gradient(
      180deg,
      var(--app-source-sysaudio),
      var(--app-source-sysaudio-strong)
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
    background: var(--app-accent);
    box-shadow: var(--app-ring);
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

  .timeline-rail__slot--app-boundary {
    z-index: 3;
  }

  :global(.timeline-rail__slot--active) .timeline-rail__tick,
  :global(.timeline-rail__slot--active.timeline-rail__slot--major) .timeline-rail__tick {
    width: 2px;
    height: 22px;
    background: var(--app-record-glyph-start);
    box-shadow: 0 0 6px color-mix(in srgb, var(--app-record-glyph-start) 70%, transparent);
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
    font-size: var(--text-xs);
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
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr);
    align-items: center;
    column-gap: 9px;
    min-width: 204px;
    max-width: min(340px, calc(100vw - 24px));
    min-height: 40px;
    padding: 7px 10px 7px 7px;
    box-sizing: border-box;
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0;
    color: var(--app-text-strong);
    background: var(--app-status-bg);
    border: 1px solid var(--app-status-border);
    border-radius: 4px;
    box-shadow:
      0 8px 20px color-mix(in srgb, var(--app-bg) 58%, transparent),
      inset 0 1px 0 color-mix(in srgb, var(--app-text-strong) 6%, transparent);
    pointer-events: none;
    /* Subtle pointer hint below the bubble. */
  }

  .timeline-rail__tooltip-icon {
    width: 24px;
    height: 24px;
    box-sizing: border-box;
    display: grid;
    place-items: center;
    overflow: hidden;
    align-self: center;
    border-radius: 4px;
    color: var(--app-text);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    font-size: 10px;
    font-weight: 800;
    line-height: 1;
  }

  .timeline-rail__tooltip-icon--image {
    padding: 3px;
  }

  .timeline-rail__tooltip-icon img {
    width: 100%;
    height: 100%;
    display: block;
    border-radius: 3px;
    object-fit: contain;
  }

  /* Both the outgoing and incoming icon (during an app-change transition) are
     pinned to the same grid cell so they cross-slide in place instead of
     stacking into two rows; the icon container's `overflow: hidden` clips the
     slide. */
  .timeline-rail__tooltip-icon-inner {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
  }

  .timeline-rail__tooltip-copy {
    min-width: 0;
    display: grid;
    gap: 4px;
  }

  /* When no app label is known there's no icon column, so the copy spans the
     full bubble width instead of being squeezed into the 24px icon track. */
  .timeline-rail__tooltip-copy--solo {
    grid-column: 1 / -1;
  }

  .timeline-rail__tooltip-app-name,
  .timeline-rail__tooltip-date {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Overlap container for the keyed app name so the cross-slide copies share
     one grid cell — the cell auto-sizes to the wider name (no width collapse)
     and clips the horizontal slide. */
  .timeline-rail__tooltip-name-stack {
    min-width: 0;
    display: grid;
    overflow: hidden;
  }

  .timeline-rail__tooltip-app-name {
    grid-area: 1 / 1;
    color: var(--app-text-strong);
    font-size: 11px;
    font-weight: 760;
    line-height: 1.05;
  }

  /* Time leads, date trails on the same baseline so the readout answers
     "when" at a glance without a second wrapped line. */
  .timeline-rail__tooltip-meta {
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .timeline-rail__tooltip-time {
    flex: 0 0 auto;
    color: var(--app-text-strong);
    font-size: 10px;
    font-weight: 720;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    white-space: nowrap;
  }

  .timeline-rail__tooltip-date {
    color: var(--app-text-muted);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    font-weight: 680;
    line-height: 1;
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
    border-top: 4px solid var(--app-status-bg);
  }

  .timeline-rail__tooltip--pinned {
    box-shadow:
      0 8px 20px color-mix(in srgb, var(--app-bg) 58%, transparent),
      inset 0 1px 0 color-mix(in srgb, var(--app-text-strong) 6%, transparent);
  }

  .timeline-rail__tooltip--pinned::after {
    border-top-color: var(--app-status-bg);
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
    background: var(--app-surface);
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
  :global([data-theme="light"]) .timeline__stage-actions--open > .timeline__stage-action-trigger {
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
    color: var(--app-text-muted);
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
  :global([data-theme="light"]) .timeline__ocr-btn[aria-pressed="true"]:not(.timeline__ocr-btn--running):not(.timeline__ocr-btn--error):not(.timeline__ocr-btn--success) {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  :global([data-theme="light"]) .timeline__ocr-btn[aria-pressed="true"]:not(.timeline__ocr-btn--running):not(.timeline__ocr-btn--error):not(.timeline__ocr-btn--success) .timeline__ocr-glyph {
    color: var(--app-accent);
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
    background: var(--app-surface-raised);
  }
  :global([data-theme="light"]) .timeline-rail__track {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail__slot {
    background: transparent;
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
    color: var(--app-text-subtle);
  }
  :global([data-theme="light"]) .timeline-rail--placeholder {
    background: var(--app-surface);
    border-color: var(--app-border);
  }
  :global([data-theme="light"]) .timeline-rail__loading {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip {
    background: var(--app-status-bg);
    border-color: var(--app-status-border);
    color: var(--app-text-strong);
    box-shadow:
      0 8px 18px rgba(20, 28, 40, 0.12),
      inset 0 1px 0 rgba(255, 255, 255, 0.72);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip-icon {
    background: var(--app-surface-raised);
    border-color: var(--app-border);
    color: var(--app-text);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip-app-name {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip-time {
    color: var(--app-text-strong);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip-date {
    color: var(--app-text-muted);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip::after {
    border-top-color: var(--app-status-bg);
  }
  :global([data-theme="light"]) .timeline-rail__tooltip--pinned {
    box-shadow:
      0 8px 18px rgba(20, 28, 40, 0.12),
      inset 0 1px 0 rgba(255, 255, 255, 0.72);
  }
</style>
