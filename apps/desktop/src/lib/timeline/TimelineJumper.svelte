<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // ── Timeline Jumper ───────────────────────────────────────────────────────
  // Two-pane "jump to date & time" popover (calendar | hourly time list)
  // extracted out of the dashboard `+page.svelte`. Orchestrator only — the
  // calendar pane (`JumperCalendar`), time-list pane (`JumperTimeList`) and the
  // per-month summary cache (`createJumperCache`) live in sibling files.
  //
  // Behavior (spec §12 grill resolutions, binding):
  //   - Preview-on-select: clicking a calendar day previews that day's hours
  //     WITHOUT moving the timeline. The timeline moves only on an explicit
  //     commit (hour row / "Latest of day" / global "Latest"), and every commit
  //     closes the popover.
  //   - Two-signifier split: committed "you are here" = accent LEFT BAR (echoes
  //     the playhead) on the real day cell + hour row; previewed day = accent
  //     FILL. A cell/row can carry both; the bar persists while previewing a
  //     different day.
  //   - Commit motion: flash the bar onto the chosen hour, close ~110ms later,
  //     and let the dashboard animate the playhead. `prefers-reduced-motion`
  //     collapses the flash/delay + playhead animation to instant.
  //
  // The "latest at or before X" resolution + focused-window load stay
  // backend-owned: this component resolves a moment to a range, then hands the
  // commit to the parent via `onJump` (which wraps `get_latest_frame_in_range`
  // → `get_timeline_window_around_frame`).
  import { tick } from "svelte";
  import {
    CalendarDate,
    getLocalTimeZone,
    today,
    type DateValue,
  } from "@internationalized/date";
  import { invoke } from "@tauri-apps/api/core";
  import { parseCapturedAt } from "$lib/format-time";
  import { humanizeError } from "$lib/format-error";
  import IconCalendar from "~icons/lucide/calendar";
  import IconChevronDown from "~icons/lucide/chevron-down";
  import type {
    DayCoverage,
    FrameDto,
    FrameRangeRequest,
  } from "$lib/types/app-infra";
  import { createJumperCache } from "./jumper-cache.svelte";
  import {
    type HourBucket,
    buildHourBuckets,
    dayRange,
    hourRange,
  } from "./jumper-time";
  import {
    type CoverageIndex,
    type DayRow,
    buildDayRows,
    calendarFieldsOfDate,
    dayHasCoverage,
    dayKeyOfDate,
    hasCoverageInHours,
    indexCoverage,
  } from "./jumper-coverage";
  import JumperCalendar from "./JumperCalendar.svelte";
  import JumperDayRows from "./JumperDayRows.svelte";
  import JumperTimeList from "./JumperTimeList.svelte";

  interface Props {
    /** The timeline's current active frame ("you are here"). */
    activeFrame: FrameDto | null;
    /** Popover open state — bindable so the parent can drive J-to-open. */
    open?: boolean;
    /** True while a commit (resolve + focused-window load) is in flight. */
    jumping?: boolean;
    /** Parent timeline busy (loading / loading-more) — disables commits. */
    timelineBusy?: boolean;
    /** Whether the standalone "latest" affordance should render. */
    showLatest?: boolean;
    /**
     * Perform the timeline jump to a resolved frame. Returns null on success,
     * or a human-readable error string on failure (the popover stays open and
     * surfaces it in the footer strip).
     */
    onJump: (frame: FrameDto) => Promise<string | null>;
    /** Snap the timeline to the live head ("Latest" / "snap to now"). */
    onJumpToLatest: () => void | Promise<void>;
  }

  let {
    activeFrame,
    open = $bindable(false),
    jumping = $bindable(false),
    timelineBusy = false,
    showLatest = false,
    onJump,
    onJumpToLatest,
  }: Props = $props();

  const cache = createJumperCache();

  // ── Per-day coverage (G6) ───────────────────────────────────────────────────
  // ONE cached GROUP BY over `capture_segments`, reloaded each time the menu
  // opens. It is the single authority for "does this day hold recording": the
  // day rows AND the month grid disable a day the same way, so you can never
  // land on an empty day. `coverageAt` gets read inside $derived, so it is
  // plain $state (the Map is replaced wholesale, never mutated in place).
  let coverage = $state<CoverageIndex>(new Map());
  let coverageLoading = $state(false);
  let coverageToday = $state(new Date());

  async function loadCoverage(): Promise<void> {
    coverageLoading = coverage.size === 0;
    try {
      const days = await invoke<DayCoverage[]>("list_day_coverage");
      coverage = indexCoverage(days);
      coverageToday = new Date();
    } catch (err) {
      // Coverage is navigation sugar: a failure disables the day rows rather
      // than blocking the calendar/hour panes, so surface it in the footer and
      // keep going.
      commitError = humanizeError(err);
    } finally {
      coverageLoading = false;
    }
  }

  const dayRows = $derived<DayRow[]>(buildDayRows(coverage, coverageToday));
  const todayFields = $derived(calendarFieldsOfDate(coverageToday));
  const yesterdayFields = $derived.by(() => {
    const d = new Date(coverageToday);
    d.setDate(d.getDate() - 1);
    return calendarFieldsOfDate(d);
  });
  const morningEnabled = $derived(
    hasCoverageInHours(coverage, todayFields, 0, 11),
  );
  const yesterdayEnabled = $derived(dayHasCoverage(coverage, yesterdayFields));

  // The previewed day (preview-on-select; does NOT move the timeline).
  let pickerSelectedDate = $state<DateValue | undefined>(undefined);
  let pickerPlaceholder = $state<DateValue>(today(getLocalTimeZone()));
  // Commit/jump error (distinct from the cache's month-load error). Either may
  // populate the footer strip.
  let commitError = $state<string | null>(null);
  let pickerStyle = $state("");
  // Transient flash of the you-are-here bar onto the just-committed hour row so
  // the selection registers visually before the popover dismisses (§12.6).
  let commitFlashHour = $state<number | null>(null);
  // Pending close-after-confirm-beat timer; cleared on any close/destroy so an
  // orphaned timer can never dismiss a freshly-reopened popover.
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  const displayError = $derived(commitError ?? cache.error);

  function prefersReducedMotion(): boolean {
    return (
      typeof window !== "undefined" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  }

  // ── Trigger readout + committed marker ──────────────────────────────────────
  // `Mon, Aug 3 · 14:32` — the pill states its position in the same shape the
  // readout above the rail does, since they describe the same moment.
  function formatTriggerLabel(ts: string): string {
    const d = parseCapturedAt(ts);
    if (isNaN(d.getTime())) return ts;
    const day = d.toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    });
    const time = d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
    return `${day} · ${time}`;
  }
  const triggerLabel = $derived(
    activeFrame ? formatTriggerLabel(activeFrame.capturedAt) : "no active frame",
  );

  const committedMoment = $derived.by<Date | null>(() => {
    if (!activeFrame) return null;
    const d = parseCapturedAt(activeFrame.capturedAt);
    return isNaN(d.getTime()) ? null : d;
  });

  /** "YYYY-MM-DD" of the committed moment — the day rows' "you are here". */
  const hereKey = $derived(
    committedMoment ? dayKeyOfDate(committedMoment) : null,
  );

  function sameLocalDay(
    d: { year: number; month: number; day: number },
    m: Date,
  ): boolean {
    return (
      d.year === m.getFullYear() &&
      d.month === m.getMonth() + 1 &&
      d.day === m.getDate()
    );
  }

  function isCommittedDate(date: { year: number; month: number; day: number }): boolean {
    return committedMoment ? sameLocalDay(date, committedMoment) : false;
  }

  const previewingCommittedDay = $derived.by<boolean>(() => {
    const m = committedMoment;
    return !!(m && pickerSelectedDate && sameLocalDay(pickerSelectedDate, m));
  });

  function isHereHour(hour: number): boolean {
    if (commitFlashHour === hour) return true;
    return previewingCommittedDay && committedMoment?.getHours() === hour;
  }

  // ── Hourly buckets for the previewed date ───────────────────────────────────
  // Today caps at the current hour; other dates render through 11 PM. Empty
  // hours render disabled. Counts feed the muted readout + neutral density fill.
  const timeBuckets = $derived.by<HourBucket[]>(() => {
    const d = pickerSelectedDate;
    if (!d) return [];
    const monthLoaded = cache.monthLoaded(d);
    return buildHourBuckets(
      d,
      new Date(),
      monthLoaded,
      monthLoaded ? cache.daySummaries(d) : undefined,
    );
  });
  const maxBucketCount = $derived(
    Math.max(1, ...timeBuckets.map((b) => b.count)),
  );

  const previewDayLabel = $derived.by<string>(() => {
    const d = pickerSelectedDate;
    if (!d) return "";
    const dt = new Date(d.year, d.month - 1, d.day);
    return dt.toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    });
  });

  // Earliest known recording date across the loaded months, for the footer
  // span. Approximated from loaded summaries (no dedicated backend call).
  const earliestKnownLabel = $derived.by<string>(() => {
    const min = cache.earliestKey();
    if (!min) return "";
    const [y, mo, da] = min.split("-").map((n) => parseInt(n, 10));
    const dt = new Date(y, mo - 1, da);
    return dt.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  });

  // Eagerly fetch the visible month whenever the placeholder lands on a new
  // month while the picker is open.
  $effect(() => {
    if (!open) return;
    void cache.load(pickerPlaceholder);
  });

  // ── Commit paths ────────────────────────────────────────────────────────────
  async function resolveAndCommit(rangeStart: Date, rangeEnd: Date): Promise<void> {
    jumping = true;
    commitError = null;
    try {
      const req: FrameRangeRequest = {
        capturedAtStart: rangeStart.toISOString(),
        capturedAtEnd: rangeEnd.toISOString(),
      };
      const frame = await invoke<FrameDto | null>("get_latest_frame_in_range", {
        request: req,
      });
      if (!frame) {
        commitError = "no frame in that range";
        commitFlashHour = null;
        return;
      }
      // Flash the you-are-here bar onto the hour we actually LANDED on (the
      // backend's "latest at or before" may resolve to an earlier hour). This
      // is what gives "Latest of day" its confirm-beat too — it has no clicked
      // row, so the resolved frame's hour is the only honest target (§12.6).
      // The resolved day matches the previewed day for both commit ranges, so
      // the row is present in the list.
      const landed = parseCapturedAt(frame.capturedAt);
      if (!isNaN(landed.getTime())) commitFlashHour = landed.getHours();
      const err = await onJump(frame);
      if (err) {
        commitError = err;
        commitFlashHour = null;
        return;
      }
      // Success: confirm beat, then dismiss. Reduced motion → instant close.
      if (prefersReducedMotion()) {
        close();
      } else {
        if (closeTimer != null) clearTimeout(closeTimer);
        closeTimer = setTimeout(() => {
          closeTimer = null;
          close();
        }, 110);
      }
    } catch (err) {
      commitError = humanizeError(err);
      commitFlashHour = null;
    } finally {
      jumping = false;
    }
  }

  async function commitDateLatest(): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    const { start, end } = dayRange(d);
    await resolveAndCommit(start, end);
  }

  async function commitHour(hour: number): Promise<void> {
    const d = pickerSelectedDate;
    if (!d) return;
    commitFlashHour = hour;
    const { start, end } = hourRange(d, hour);
    await resolveAndCommit(start, end);
  }

  /** A day row / quick target: preview the day, then land on its latest frame. */
  async function commitDay(
    date: { year: number; month: number; day: number },
    endHour?: number,
  ): Promise<void> {
    pickerSelectedDate = new CalendarDate(date.year, date.month, date.day);
    pickerPlaceholder = pickerSelectedDate;
    const { start, end } =
      endHour === undefined ? dayRange(date) : hourRange(date, endHour);
    await resolveAndCommit(start, end);
  }

  async function commitGlobalLatest(): Promise<void> {
    commitError = null;
    close();
    await onJumpToLatest();
  }

  // ── Open / close + seeding ───────────────────────────────────────────────────
  function close(): void {
    if (closeTimer != null) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    open = false;
  }
  function toggle(): void {
    open = !open;
  }

  // Belt-and-braces: clear any pending close-beat timer on unmount.
  $effect(() => () => {
    if (closeTimer != null) clearTimeout(closeTimer);
  });

  // Seed the picker from the active frame each time it opens so it reflects
  // "you are here" rather than whatever was last previewed.
  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      wasOpen = true;
      commitFlashHour = null;
      commitError = null;
      cache.clearError();
      void loadCoverage();
      if (committedMoment) {
        const cd = new CalendarDate(
          committedMoment.getFullYear(),
          committedMoment.getMonth() + 1,
          committedMoment.getDate(),
        );
        pickerPlaceholder = cd;
        pickerSelectedDate = cd;
      } else {
        pickerSelectedDate = undefined;
      }
    } else if (!open && wasOpen) {
      wasOpen = false;
      commitFlashHour = null;
    }
  });

  // ── Dialog a11y: focus trap, restore, Escape, click-outside, positioning ────
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
    const availableAbove = Math.max(
      160,
      triggerRect.top - triggerGap - viewportMargin,
    );
    // 500, not 420: the pill now sits on the playhead near the bottom of the
    // window, so the menu opens upward — and G6's contents (quick targets,
    // seven day rows, the month grid) only all fit above the fold at 500.
    const maxHeight = Math.min(500, Math.max(availableBelow, availableAbove));
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

    pickerStyle = `left: ${left}px; top: ${top}px; height: ${maxHeight}px;`;
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
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
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

  function onWindowPointerDown(e: PointerEvent) {
    if (!open) return;
    const target = e.target as Node | null;
    if (!target) return;
    if (pickerEl?.contains(target)) return;
    if (pickerTriggerEl?.contains(target)) return; // trigger handles its own toggle
    close();
  }

  // When the picker opens, move focus inside it; when it closes, restore focus
  // to the trigger so keyboard users don't get stranded.
  $effect(() => {
    if (!open) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || !open) return;
      updatePickerPosition();
      const focusable = getPickerFocusable();
      (focusable[0] ?? pickerEl)?.focus();
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (!active || active === document.body || pickerEl?.contains(active)) {
        pickerTriggerEl?.focus();
      }
    };
  });

  $effect(() => {
    if (!open) {
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

  // ── Exposed for the dashboard's head-poll / refresh cache invalidation ──────
  export function invalidateMonthsForFrames(
    frames: { capturedAt: string }[],
  ): void {
    cache.invalidateMonthsForFrames(frames);
  }
  export function invalidateAllLoadedMonths(): void {
    cache.invalidateAllLoadedMonths();
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div class="timeline__jump">
  <!-- Position pill (G6): the label IS your position, the chevron is the jump.
       One control owns both jobs — there is no second date input and no from/to
       pair anywhere on the timeline. -->
  <button
    class="ss-nowpill timeline__jump-trigger"
    class:timeline__jump-trigger--open={open}
    onclick={toggle}
    bind:this={pickerTriggerEl}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-controls="timeline-jump-picker"
    use:tip={"Jump to date and time (J)"}
  >
    <span class="timeline__jump-icon" aria-hidden="true"><IconCalendar /></span>
    <span class="ss-num timeline__jump-label">{triggerLabel}</span>
    <span class="timeline__jump-chevron" aria-hidden="true"><IconChevronDown /></span>
  </button>

  {#if showLatest}
    <button
      class="btn btn--ghost btn--sm timeline__jump-latest"
      onclick={() => void onJumpToLatest()}
      disabled={timelineBusy || jumping}
      use:tip={"Jump to latest frame (L)"}
    >latest</button>
  {/if}

  {#if open}
    <div
      class="timeline__picker"
      id="timeline-jump-picker"
      style={pickerStyle}
      role="dialog"
      aria-modal="false"
      aria-label="Jump to date and time"
      tabindex="-1"
      bind:this={pickerEl}
      onkeydown={onPickerKeydown}
    >
      <div class="timeline__picker-head">
        <span class="timeline__picker-title">Jump to date &amp; time</span>
        <button
          class="btn btn--accent btn--sm timeline__picker-global-latest"
          onclick={() => void commitGlobalLatest()}
          disabled={timelineBusy || jumping}
          use:tip={"Jump to latest frame"}
        >
          <span class="timeline__picker-glyph" aria-hidden="true">⟿</span>
          latest
        </button>
      </div>

      <div class="timeline__picker-panes">
        <div class="timeline__picker-days">
          <JumperDayRows
            rows={dayRows}
            loading={coverageLoading}
            busy={jumping || timelineBusy}
            {morningEnabled}
            {yesterdayEnabled}
            {hereKey}
            onJumpNow={() => void commitGlobalLatest()}
            onJumpMorning={() => void commitDay(todayFields, 11)}
            onJumpYesterday={() => void commitDay(yesterdayFields)}
            onJumpDay={(row) => void commitDay(row.date)}
          />
          <JumperCalendar
            bind:value={pickerSelectedDate}
            bind:placeholder={pickerPlaceholder}
            isDateDisabled={(d) => !dayHasCoverage(coverage, d)}
            {isCommittedDate}
          />
        </div>
        <JumperTimeList
          hasSelection={!!pickerSelectedDate}
          loading={cache.loading}
          dayLabel={previewDayLabel}
          buckets={timeBuckets}
          maxCount={maxBucketCount}
          busy={jumping || timelineBusy}
          {isHereHour}
          onCommitHour={(h) => void commitHour(h)}
          onCommitDayLatest={() => void commitDateLatest()}
        />
      </div>

      <div
        class="timeline__picker-foot"
        class:timeline__picker-foot--error={!!displayError}
      >
        {#if displayError}
          <span class="timeline__picker-foot-msg">{displayError}</span>
        {:else}
          <span class="timeline__picker-foot-span">
            {#if earliestKnownLabel}{earliestKnownLabel} → now · {/if}hour granularity
          </span>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  /* `.btn` + `--ghost` / `--sm` / `--accent`: shared primitive
     (system.css §6, routes/+layout.svelte). `.ss-nowpill` / `.ss-menu*`:
     direction 02's kit (lib/studio/studio-shell.css). */

  /* ── Trigger group ──────────────────────────────────────────────────────── */
  .timeline__jump {
    display: flex;
    align-items: center;
    gap: 6px;
    position: relative;
  }
  /* The position pill. `.ss-nowpill` carries the shape; only the button reset,
     the truncation and the open ring are local. */
  .timeline__jump-trigger {
    cursor: pointer;
    max-width: 260px;
  }
  .timeline__jump-trigger:hover {
    background: var(--app-surface-hover);
  }
  .timeline__jump-trigger--open {
    box-shadow:
      0 1px 3px rgb(0 0 0 / 20%),
      0 0 0 var(--hairline) var(--app-accent-border),
      var(--app-ring);
  }
  .timeline__jump-chevron {
    display: inline-flex;
    align-items: center;
    color: var(--app-text-muted);
  }
  .timeline__jump-chevron :global(svg) {
    width: 11px;
    height: 11px;
  }
  .timeline__jump-latest {
    flex: 0 0 auto;
  }
  .timeline__jump-icon {
    display: inline-flex;
    align-items: center;
    color: var(--app-accent);
  }
  .timeline__jump-icon :global(svg) {
    width: 12px;
    height: 12px;
  }
  .timeline__jump-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Popover shell ──────────────────────────────────────────────────────── */
  /* NSMenu anatomy: a raised sheet on a hairline ring, no card border. It is
     wider than the kit's 264px menu because it keeps the hour pane the app has
     always had beside the day rows + month grid. */
  .timeline__picker {
    position: fixed;
    z-index: 20;
    display: flex;
    flex-direction: column;
    width: min(560px, calc(100vw - 24px));
    box-sizing: border-box;
    overflow: hidden;
    background: var(--app-surface-raised);
    border: 0;
    border-radius: var(--r-lg);
    box-shadow:
      var(--shadow-popover, var(--app-shadow-popover)),
      0 0 0 var(--hairline) var(--app-border-strong);
    color: var(--app-text);
  }
  .timeline__picker-head {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 26px;
    flex: 0 0 auto;
    padding: 0 8px 0 10px;
    border-bottom: var(--hairline) solid var(--app-border);
  }
  .timeline__picker-title {
    flex: 1;
    font: var(--w-medium, 510) var(--t-label) / 1.4 var(--app-font-mono);
    letter-spacing: var(--ls-label, 0.02em);
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .timeline__picker-global-latest {
    flex: 0 0 auto;
    gap: 6px;
  }
  .timeline__picker-glyph {
    font-size: var(--t-ui);
    line-height: 1;
  }
  /* Left column: quick targets + seven day rows + the month grid, scrolled as
     one; right column: the hour list for the previewed day. */
  .timeline__picker-days {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    border-right: var(--hairline) solid var(--app-border);
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
  }
  .timeline__picker-panes {
    display: grid;
    grid-template-columns: 1fr 190px;
    /* Bound the panes to the popover's fixed height (height set inline by
       updatePickerPosition) so the time list scrolls instead of the popover
       resizing with content. minmax(0,1fr) + min-height:0 break the default min-content
       floor; the WebKit flex/grid overflow trap (memory: webkit-height-100). */
    grid-template-rows: minmax(0, 1fr);
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }

  /* ── Footer state strip ─────────────────────────────────────────────────── */
  .timeline__picker-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-top: var(--hairline) solid var(--app-border);
    font-size: var(--t-label);
    min-height: 24px;
  }
  .timeline__picker-foot-span {
    color: var(--app-text-subtle);
    letter-spacing: 0.03em;
    font-variant-numeric: tabular-nums;
  }
  .timeline__picker-foot-msg {
    color: var(--app-danger-text);
    word-break: break-word;
  }
  .timeline__picker-foot--error {
    color: var(--app-danger-text);
  }

  @media (max-width: 640px) {
    .timeline__picker {
      width: min(320px, calc(100vw - 24px));
    }
    .timeline__picker-days {
      border-right: none;
      border-bottom: 1px solid var(--app-border);
      overflow-y: visible;
    }
    .timeline__picker-panes {
      grid-template-columns: minmax(0, 1fr);
      /* Single column: calendar + time list stack and the whole pane scrolls
         within the fixed popover height instead of clipping. */
      grid-template-rows: auto auto;
      overflow-y: auto;
    }
  }
</style>
