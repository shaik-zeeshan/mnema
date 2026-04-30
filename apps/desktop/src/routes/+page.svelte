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
    FrameDto,
    FramePreviewDto,
    FrameRangeRequest,
    FrameSummaryDto,
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
  const TIMELINE_PREFETCH_AHEAD = 25;
  // Safety cap on the pages we'll auto-load while chasing a jump target so a
  // mis-typed range can't hang the UI on a runaway pagination loop.
  const TIMELINE_JUMP_PAGE_BUDGET = 500;
  // Render only the slots near the viewport plus a buffer on each side, so
  // large recordings don't tax the DOM. The track itself keeps its full width
  // so scroll-position-based active-index math is unaffected. The rail is
  // dense (8px ticks) so a generous buffer is cheap and avoids visible churn.
  const TIMELINE_VIEWPORT_BUFFER = 80;

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

  const timelineWindowStart = $derived(
    Math.max(
      0,
      Math.floor(timelineAdvance / TIMELINE_SLOT_WIDTH) - TIMELINE_VIEWPORT_BUFFER,
    ),
  );
  const timelineWindowEnd = $derived(
    Math.min(
      timelineFrames.length,
      Math.ceil((timelineAdvance + timelineViewportWidth) / TIMELINE_SLOT_WIDTH) +
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
  }

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

  // One-shot initial load. No session bootstrap — the timeline browses all
  // frames across every session, newest first.
  let timelineInitialized = false;
  $effect(() => {
    if (timelineInitialized) return;
    timelineInitialized = true;
    loadTimelinePage(true);
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
        onclick={() => loadTimelinePage(true)}
        disabled={timelineLoading || timelineLoadingMore}
      >refresh</button>
    </div>
  </header>

  {#if timelineError}
    <div class="timeline__error">
      <span class="timeline__error-label">load error</span>
      <span class="timeline__error-msg">{timelineError}</span>
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
  <div class="timeline__rail-wrap">
    {#if timelineFrames.length > 0}
      <div
        class="timeline-rail"
        bind:this={timelineRail}
        onscroll={onTimelineScroll}
        onkeydown={onTimelineKeyDown}
        onclick={onTimelineRailClick}
        role="slider"
        tabindex="0"
        aria-label="Timeline scrubber"
        aria-valuemin={1}
        aria-valuemax={Math.max(1, timelineFrames.length)}
        aria-valuenow={timelineActiveIndex + 1}
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
              aria-hidden="true"
            >
              <span class="timeline-rail__tick"></span>
            </div>
          {/each}
        </div>
        <span class="timeline-rail__cursor" aria-hidden="true"></span>
      </div>
    {:else}
      <!-- Empty placeholder reserves the rail's height so removing/adding
           the rail does not resize the stage. -->
      <div class="timeline-rail timeline-rail--placeholder" aria-hidden="true"></div>
    {/if}
    {#if timelineLoadingMore}
      <div class="timeline-rail__loading">loading…</div>
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
  .timeline__error,
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .timeline-rail {
    position: relative;
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
       pagination cannot push the rail's height. Pinned to the LEFT to stay
       clear of the newest-frame anchor on the right. */
    position: absolute;
    left: 8px;
    bottom: 4px;
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
</style>
