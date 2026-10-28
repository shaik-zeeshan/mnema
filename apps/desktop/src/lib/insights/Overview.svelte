<script lang="ts">
  // Overview — the default Insights view (insights.html#overview): a period
  // picker on the bar, then the read (kicker · headline · prose) beside four
  // derived stats, "Where the <unit> went" as per-slot stacked columns with the
  // focus split/heat and app time beside it, and the dossier's changes.
  //
  // Capture figures come from `get_usage_charts` (screen frames only) and render
  // for everyone; the read, categories and focus need the engine. With AI off,
  // never set up, or derivation off the view is capture-only: grey columns and
  // "needs AI" where the engine would speak (IMPLEMENTATION.md §4, ruling 10).
  import { inBar } from "$lib/insights/in-bar";
  import { tip } from "$lib/components/tooltip";
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { message } from "@tauri-apps/plugin-dialog";
  import { renderIdle, renderResumeTick } from "$lib/render-idle.svelte";
  import {
    appIconFallback,
    canonicalBundleIdForComparison,
    iconPathForBundleId,
    mergeIconResolutions,
    unresolvedIconBundleIds,
    type AppIconResolution,
  } from "$lib/app-privacy-exclusion";
  import type {
    Activity,
    ActivityCategory,
    Conclusion,
    DigestAbsentReason,
    UserContextDigest,
    UserContextDigestResult,
    UserContextStatus,
    AiRuntimeStatus,
  } from "$lib/types/recording";
  import {
    startOfDay,
    historyWindow,
    pickerDensity,
    windowFor,
    shiftAnchor,
    buildActivityThreads,
    type ActivityThread,
    type RangeMode,
  } from "$lib/insights/activity-helpers";
  import {
    activityColumns,
    appTimeColumns,
    categoryRows,
    computeLedeStats,
    deepFocusDelta,
    focusCounts,
    focusHeat,
    listedAppRows,
  } from "$lib/insights/lede-stats";
  import { addDays, capsuleLabel, densityFromHeatmap, isoWeek, parseIso, toIso } from "$lib/components/date-input";
  import DateInput from "$lib/components/DateInput.svelte";
  import CategoryDetailModal from "$lib/insights/CategoryDetailModal.svelte";
  import AppDetailModal from "$lib/insights/AppDetailModal.svelte";
  import FocusDetailModal from "$lib/insights/FocusDetailModal.svelte";
  import OverviewLede, { type LedeCapture, type LedeEngine } from "$lib/insights/OverviewLede.svelte";
  import OverviewExhibits from "$lib/insights/OverviewExhibits.svelte";
  import OverviewFeed from "$lib/insights/OverviewFeed.svelte";
  import { humanizeError } from "$lib/format-error";
  import OverviewStates, {
    clockLabel,
    type OverviewState,
  } from "$lib/insights/OverviewStates.svelte";
  import { retentionVerdict } from "$lib/insights/retention";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";
  import IconSparkle from "~icons/lucide/sparkle";

  interface Props {
    onOpenSubject?: (subject: string) => void;
    onOpenTab?: (tab: "subjects") => void;
  }

  let { onOpenSubject, onOpenTab }: Props = $props();

  // ── Usage-chart DTO (mirrors capture-types/usage_charts.rs) ──
  interface AppUsage {
    app: string;
    appBundleId: string | null;
    activeMs: number;
    frameCount: number;
  }
  interface HeatmapBucket {
    bucketStartMs: number;
    intensityCount: number;
  }
  interface UsageCharts {
    rangeStartMs: number;
    rangeEndMs: number;
    timePerApp: AppUsage[];
    activityHeatmap: HeatmapBucket[];
  }

  // ── Period ─────────────────────────────────────────────────────────────
  // `anchor` is a millis timestamp inside the selected period; the DateInput
  // picks the unit and steps it (never past now). Bounds are local-calendar.
  let rangeMode = $state<RangeMode>("week");
  let anchor = $state<number>(Date.now());
  const range = $derived(windowFor(anchor, rangeMode));
  // The period before — the baseline for the tracked and deep-focus deltas.
  const prevWindow = $derived(windowFor(shiftAnchor(anchor, rangeMode, -1), rangeMode));
  const atLatest = $derived(Date.now() < range.endMs);

  function onPeriod(next: { start: string; unit: string }): void {
    rangeMode = next.unit as RangeMode;
    anchor = parseIso(next.start).getTime();
  }

  const firstDay = $derived(new Date(range.startMs));
  const lastDay = $derived(new Date(range.endMs - 1));
  // "Oct 5 – 11" · "October 2026" · "Oct 9" — the modals' and Ask's label.
  const rangeLabel = $derived(capsuleLabel("period", rangeMode, firstDay, lastDay, new Date()));
  const scope = $derived(atLatest ? (rangeMode === "day" ? "today" : `this ${rangeMode}`) : rangeLabel);
  const when = $derived(
    rangeMode === "week"
      ? `week ${isoWeek(firstDay)}`
      : rangeMode === "month"
        ? firstDay.toLocaleDateString("en-US", { month: "long" })
        : firstDay.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" }),
  );
  // "Ask about this week" → a pre-scoped chat (CH3: `?q=&from=&to=`, local days).
  const askHref = $derived(
    `/chat?q=${encodeURIComponent(`What did ${scope} add up to?`)}&from=${toIso(firstDay)}&to=${toIso(lastDay)}`,
  );

  // ── Engine gating ────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  // The picker starts at the first capture still on disk (nothing to show
  // before it); a year back only until the status read lands.
  const minDay = $derived(
    ctxStatus ? toIso(new Date(ctxStatus.earliestCaptureAtMs ?? Date.now())) : toIso(addDays(new Date(), -365)),
  );
  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) ||
      Boolean(ctxStatus?.engineAvailable),
  );
  // Capture-only: AI off, never set up, or continuous derivation off.
  const capOnly = $derived(
    (aiStatus != null && (!aiStatus.hasProviders || !aiStatus.enabled)) ||
      ctxStatus?.reason === "user_context_disabled",
  );

  // ── Loaded data ────────────────────────────────────────────────────────
  let usage = $state<UsageCharts | null>(null);
  // Picker density for the ~120 days before the period (same window as Journal),
  // from `get_capture_hours` — the heatmap's non-empty hours, nothing else.
  let historyDensity = $state<Record<string, number>>({});
  // Previous-window baselines (see `loadPrev`).
  let prevUsage = $state<UsageCharts | null>(null);
  let prevActivities = $state<Activity[]>([]);
  let activities = $state<Activity[]>([]);
  let conclusions = $state<Conclusion[]>([]);
  // The read for the period. `null` is the normal absent case (sparse range).
  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);
  // The manual re-read: a forced regeneration past the backend cache; its
  // failure is shown (`digestError` carries the one-sentence reason).
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);
  let digestReason = $state<DigestAbsentReason | null>(null);

  let loadingFree = $state(true);
  // Quiet refresh (OV-22): skeletons only until the CURRENT range has loaded
  // once; background refreshes swap numbers in place. Reset on range change.
  let freeLoadedOnce = $state(false);
  // Which capture families have anything in the range (OV-15/16).
  let presence = $state<{ screen: boolean; audio: boolean } | null>(null);
  let loadingEngine = $state(false);
  let freeError = $state<string | null>(null);
  // Engine-data (activities + conclusions) load failure for the active range —
  // a recoverable error, never the empty state.
  let engineError = $state<string | null>(null);
  let statusLoaded = $state(false);
  let engineLoadedOnce = $state(false);
  // Inline-correction in flight, keyed by activity id.
  let correctingActivity = $state<Set<number>>(new Set());

  // ── Apps (+ icons for the All-apps modal) ─────────────────────────────
  // The listed apps; the tracked total still sums every row.
  const listedApps = $derived(listedAppRows(usage?.timePerApp ?? []));
  const allApps = $derived.by(() =>
    listedApps.map((a) => ({
      app: a.app,
      activeMs: a.activeMs,
      frameCount: a.frameCount,
      iconSrc: appIconSrc(a.appBundleId),
      fallback: appIconFallback(a.app, a.appBundleId),
    })),
  );

  // Icons are bundle-id-keyed facts, so the merge map doubles as a cross-range cache.
  let iconPathsByBundleId = $state<Record<string, string>>({});
  const requestedCanonicalIconBundleIds = new Set<string>();

  async function resolveAppIcons(
    bundleIds: Array<string | null | undefined>,
  ): Promise<void> {
    const unresolved = unresolvedIconBundleIds(
      bundleIds,
      iconPathsByBundleId,
      requestedCanonicalIconBundleIds,
    );
    if (unresolved.length === 0) return;
    for (const id of unresolved) {
      requestedCanonicalIconBundleIds.add(canonicalBundleIdForComparison(id));
    }
    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: unresolved },
      });
      const result = mergeIconResolutions(iconPathsByBundleId, icons);
      if (result.changed) iconPathsByBundleId = result.iconPathsByBundleId;
    } catch {
      for (const id of unresolved) {
        requestedCanonicalIconBundleIds.delete(canonicalBundleIdForComparison(id));
      }
      // Icons are decorative; the letter fallback keeps working.
    }
  }

  function appIconSrc(bundleId: string | null): string | null {
    if (!bundleId) return null;
    const iconPath = iconPathForBundleId(bundleId, iconPathsByBundleId);
    return iconPath ? convertFileSrc(iconPath) : null;
  }

  // Resolve the full app list so every All-apps row gets its icon.
  $effect(() => {
    const ids = (usage?.timePerApp ?? []).map((a) => a.appBundleId);
    void untrack(() => resolveAppIcons(ids));
  });

  // ── Range activities + derived figures ─────────────────────────────────
  // Overlap with [startMs, endMs): the start is INCLUSIVE so a zero-length
  // activity on local midnight stays in its Day window.
  const rangeActivities = $derived.by<Activity[]>(() => {
    const { startMs, endMs } = range;
    return activities.filter((a) => a.startedAtMs < endMs && a.endedAtMs >= startMs);
  });
  // Shared with the Journal lede: same label ⇒ same computation.
  const ledeStats = $derived(
    computeLedeStats({
      timePerApp: usage?.timePerApp ?? [],
      rangeActivities,
      rangeStartMs: range.startMs,
      rangeEndMs: range.endMs,
      engineOn,
    }),
  );
  const focus = $derived(focusCounts(rangeActivities));
  const deepDelta = $derived(deepFocusDelta(rangeActivities, prevActivities));
  const categories = $derived(categoryRows(rangeActivities, range.startMs, range.endMs));
  const heat = $derived(focusHeat(rangeActivities, range.startMs, range.endMs, rangeMode));
  const columns = $derived(
    capOnly
      ? appTimeColumns(usage?.activityHeatmap ?? [], ledeStats.trackedMs, range.startMs, range.endMs, rangeMode, Date.now())
      : activityColumns(rangeActivities, range.startMs, range.endMs, rangeMode, Date.now()),
  );
  // The period's own usage overrides history, so the visible days stay live.
  const density = $derived(pickerDensity(historyDensity, usage?.activityHeatmap ?? []));
  // Captured days = distinct local days with any heatmap intensity.
  const capturedDays = $derived(
    new Set((usage?.activityHeatmap ?? []).filter((b) => b.intensityCount > 0).map((b) => startOfDay(b.bucketStartMs))).size,
  );
  // Tracked vs the matched slice of the previous period (see `loadPrev`);
  // hidden without a baseline.
  const trackedDeltaMs = $derived.by<number | null>(() => {
    if (!usage || !prevUsage) return null;
    const prev = prevUsage.timePerApp.reduce((t, a) => t + a.activeMs, 0);
    return prev > 0 ? ledeStats.trackedMs - prev : null;
  });
  const activityThreads = $derived.by<ActivityThread[]>(() => buildActivityThreads(rangeActivities));

  let categoryModalOpen = $state(false);
  let appsModalOpen = $state(false);
  let focusModalOpen = $state(false);

  // ── Loaders ────────────────────────────────────────────────────────────
  async function loadStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    aiStatus = ai;
    ctxStatus = ctx;
    statusLoaded = true;
  }

  // Monotonic gen tokens drop a slow earlier-range response that lands after a
  // newer fetch, so rapid stepping never shows another period's numbers.
  let freeRequestToken = 0;
  async function loadFree(): Promise<void> {
    const token = ++freeRequestToken;
    loadingFree = true;
    try {
      const { startMs, endMs } = range;
      const [next, nextPresence] = await Promise.all([
        invoke<UsageCharts>("get_usage_charts", { startMs, endMs }),
        // Presence only refines copy; a failure keeps the plain wording.
        invoke<{ screen: boolean; audio: boolean }>("capture_presence", { startMs, endMs }).catch(() => null),
      ]);
      if (token !== freeRequestToken) return;
      usage = next;
      presence = nextPresence;
      freeError = null;
      freeLoadedOnce = true;
    } catch (error) {
      if (token === freeRequestToken) {
        freeError = humanizeError(error);
        // Never show another range's numbers under this range's label (OV-13).
        if (!freeLoadedOnce) usage = null;
      }
    } finally {
      if (token === freeRequestToken) loadingFree = false;
    }
  }

  // Baselines for the deltas: usage charts (tracked) and activities (deep
  // focus) over the previous window, truncated to the active window's elapsed
  // fraction so an in-progress period compares pace with pace. On error the
  // deltas simply hide.
  let prevRequestToken = 0;
  async function loadPrev(): Promise<void> {
    const token = ++prevRequestToken;
    const pw = prevWindow;
    const now = Date.now();
    const endMs = now >= range.endMs ? pw.endMs : Math.min(pw.startMs + Math.max(0, now - range.startMs), pw.endMs);
    const [nextUsage, nextActs] = await Promise.all([
      invoke<UsageCharts>("get_usage_charts", { startMs: pw.startMs, endMs }).catch(() => null),
      capOnly
        ? Promise.resolve([])
        : invoke<Activity[]>("list_user_context_activities", { startMs: pw.startMs, endMs }).catch(() => []),
    ]);
    if (token !== prevRequestToken) return;
    prevUsage = nextUsage;
    prevActivities = nextActs;
  }

  // Every activity + conclusion overlapping the period in one call each. Read
  // from the DB, so a paused engine still shows what was written.
  let engineRequestToken = 0;
  async function loadEngine(): Promise<void> {
    const token = ++engineRequestToken;
    if (capOnly) {
      activities = [];
      conclusions = [];
      engineError = null;
      engineLoadedOnce = true;
      return;
    }
    loadingEngine = true;
    try {
      const { startMs, endMs } = range;
      const [nextActivities, nextConclusions] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: true, startMs, endMs }),
      ]);
      if (token !== engineRequestToken) return;
      activities = nextActivities;
      conclusions = nextConclusions;
      engineError = null;
    } catch (error) {
      if (token === engineRequestToken) engineError = humanizeError(error);
    } finally {
      if (token === engineRequestToken) {
        loadingEngine = false;
        engineLoadedOnce = true;
      }
    }
  }

  // The read: `get_user_context_digest` is a cheap cache hit for an unchanged
  // range (a fresh one costs one model call) and returns null when the range is
  // too sparse. `force` is the Re-read: past the cache + freshness floor, and
  // its failure is shown (a failed refresh keeps the read we had, OV-06/07).
  let digestRequestToken = 0;
  async function loadDigest(force = false): Promise<void> {
    if (!statusLoaded || !engineOn) {
      digest = digestReason = null;
      digestLoading = false;
      return;
    }
    if (force && digestRegenerating) return;
    const token = ++digestRequestToken;
    digestRegenerating = force;
    digestLoading = !force;
    digestError = null;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<UserContextDigestResult>(
        force ? "regenerate_user_context_digest" : "get_user_context_digest",
        { rangeKind: rangeMode, startMs, endMs },
      );
      if (token !== digestRequestToken) return; // range moved on — stale
      [digest, digestReason] = [next.digest ?? null, next.reason ?? null];
    } catch (error) {
      if (token === digestRequestToken) digestError = String(error);
    } finally {
      if (token === digestRequestToken) digestLoading = digestRegenerating = false;
    }
  }

  // Range steps debounce the digest fetch — a changed range can cost a paid
  // model call and the user may flick through weeks. Mount/refresh paths call
  // `loadDigest` directly instead.
  const DIGEST_DEBOUNCE_MS = 500;
  let digestDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  // Guards the range-watch effect against double-loading on mount — see the
  // effect below. Plain (non-reactive) flag: it only gates effect bookkeeping.
  let rangeWatchPrimed = false;
  function scheduleDigestLoad(): void {
    // Drop the old range's prose at once and invalidate in-flight responses —
    // last week's lede never sits over this week's cards.
    digestRequestToken += 1;
    digest = digestReason = null;
    // A new range drops any pending re-read and its error — last range's reason
    // never sits over this one (the token bump also frees a stuck spinner).
    digestRegenerating = false;
    digestError = null;
    if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
    digestDebounceTimer = null;
    if (!statusLoaded || !engineOn) {
      digestLoading = false;
      return;
    }
    digestLoading = true; // placeholder spans the debounce window too
    digestDebounceTimer = setTimeout(() => {
      digestDebounceTimer = null;
      void loadDigest();
    }, DIGEST_DEBOUNCE_MS);
  }

  async function loadHistory(): Promise<void> {
    // Just the hours with capture: the full usage read loads every frame row.
    const hours = await invoke<number[]>("get_capture_hours", historyWindow(minDay, Date.now())).catch(() => null);
    if (hours) historyDensity = densityFromHeatmap(hours.map((bucketStartMs) => ({ bucketStartMs, intensityCount: 1 })));
  }

  async function reloadAll(): Promise<void> {
    await loadStatus();
    await Promise.all([loadFree(), loadPrev(), loadEngine(), loadDigest(), loadHistory()]);
  }

  // Re-query when the period changes. The mount effect owns the first load.
  $effect(() => {
    range.startMs;
    range.endMs;
    void untrack(() => {
      if (!rangeWatchPrimed) {
        rangeWatchPrimed = true;
        return;
      }
      engineLoadedOnce = false;
      freeLoadedOnce = false;
      presence = null;
      void loadFree();
      void loadPrev();
      void loadEngine();
      scheduleDigestLoad();
    });
    return () => {
      if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
      digestDebounceTimer = null;
    };
  });

  // ── Corrections ───────────────────────────────────────────────────────
  // Reflected at once; the user_context_changed refresh reconciles. A failure
  // is surfaced — a silent no-op would leave the user thinking it stuck.
  async function correct(a: Activity, field: "category" | "focus", value: string | null): Promise<void> {
    correctingActivity = new Set(correctingActivity).add(a.id);
    try {
      await invoke(`user_context_correct_activity_${field}`, { id: a.id, [field]: value });
      activities = activities.map((x) => (x.id === a.id ? ({ ...x, [field]: value } as Activity) : x));
    } catch (error) {
      await message(humanizeError(error), { title: `Couldn't update ${field}`, kind: "error" });
    } finally {
      const done = new Set(correctingActivity);
      done.delete(a.id);
      correctingActivity = done;
    }
  }
  const correctCategory = (a: Activity, c: ActivityCategory | null) => correct(a, "category", c);
  const correctFocus = (a: Activity, f: "deep" | "mixed" | "distracted" | null) => correct(a, "focus", f);

  // ── Mount ──────────────────────────────────────────────────────────────
  function reloadForContextChange(): void {
    void loadStatus();
    void loadEngine();
    // The engine beat is the only live signal during recording, so refresh the
    // usage tiles (time-per-app / heatmap / transitions) and the vs-previous
    // baseline here too — otherwise they freeze while the engine tiles update.
    void loadFree();
    void loadPrev();
    // Same range → cache hit; keeps the current lede up until fresh prose.
    void loadDigest();
  }

  // A context change that lands while nothing can render (screens asleep /
  // window hidden) is deferred: the skeleton-swap re-render would strand
  // non-purgeable IOSurfaces (see $lib/render-idle.svelte). The resume-tick
  // effect below replays the newest deferred reload.
  let pendingContextReload = false;

  $effect(() => {
    void untrack(() => reloadAll());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      if (renderIdle()) {
        pendingContextReload = true;
        return;
      }
      reloadForContextChange();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  $effect(() => {
    if (renderResumeTick() === 0 || !pendingContextReload) return;
    // A focus flip can bump the tick while screens are still asleep — stay
    // deferred until rendering is actually possible.
    if (renderIdle()) return;
    pendingContextReload = false;
    untrack(reloadForContextChange);
  });

  // ── Loading gates ─────────────────────────────────────────────────────
  // Skeletons only until the period has loaded once (no stale numbers).
  const freeLoading = $derived(loadingFree && !freeLoadedOnce);
  const engineTilesLoading = $derived(!statusLoaded || (!capOnly && !engineLoadedOnce));

  // Tracked time stays live while recording (the worker beat alone can be
  // minutes apart). Quiet: `freeLoadedOnce` keeps the numbers up meanwhile.
  $effect(() => {
    if (!captureControls.isRunning || !atLatest) return;
    const timer = setInterval(() => {
      if (!renderIdle()) untrack(() => void Promise.all([loadFree(), loadPrev()]));
    }, 60_000);
    return () => clearInterval(timer);
  });

  // ── Truthful states (Direction A frames 2a–2b, 4a–4c, 5a–5c) ──────────
  const DAY_MS = 86_400_000;
  const settings = $derived(captureControls.recordingSettings);
  const nothingCaptured = $derived(presence != null && !presence.screen && !presence.audio);
  // The usage fetch failed with nothing loaded for this range (OV-13).
  const usageFailed = $derived(freeError != null && usage == null && !freeLoading);
  // Footage retention over this range (OV-14), and why there's no app time.
  const retention = $derived(presence ? retentionVerdict(range.startMs, range.endMs, settings?.retentionPolicy) : null);
  const noAppTime = $derived.by<{ cap: string; note: string } | null>(() => {
    if (!presence || presence.screen) return null;
    if (retention?.kind === "removed")
      return {
        cap: "tracked · removed by retention",
        note: `Removed by your ${retention.days}-day retention, so there's no app time. The summaries and the read are kept.`,
      };
    if (presence.audio)
      return {
        cap: "app time · audio only",
        note: "This range has audio only — app time comes from screen recording.",
      };
    return null;
  });
  // Older than the window the worker summarizes on its own (OV-20).
  const backfillDays = $derived(
    settings?.userContext?.backfillGoDeeper ? null : (settings?.userContext?.backfillWindowDays ?? null),
  );
  const olderThanBackfill = $derived(
    engineOn &&
      backfillDays != null &&
      range.endMs <= Date.now() - backfillDays * DAY_MS &&
      engineLoadedOnce &&
      rangeActivities.length === 0,
  );
  // 2a: current range, nothing captured, and not recording (or user-paused).
  const userPaused = $derived(captureControls.isRunning && captureControls.isUserPaused);
  const idleState = $derived<OverviewState | null>(
    atLatest && nothingCaptured && (!captureControls.isRunning || userPaused)
      ? { kind: "idle", paused: userPaused }
      : null,
  );
  // 2b: recording, but the worker hasn't written an activity for this range.
  const waitingFirst = $derived(
    engineOn &&
      atLatest &&
      captureControls.isRunning &&
      !userPaused &&
      engineLoadedOnce &&
      !engineError &&
      rangeActivities.length === 0,
  );
  // "Recording since": earliest source start, clamped to the start of today.
  const recordingSince = $derived.by<number | null>(() => {
    const sessions = captureSession.value?.sourceSessions;
    const starts = [sessions?.screen, sessions?.microphone, sessions?.systemAudio]
      .map((s) => s?.startedAtUnixMs)
      .filter((ms): ms is number => typeof ms === "number" && ms > 0);
    if (starts.length === 0) return null;
    return Math.max(Math.min(...starts), startOfDay(Date.now()));
  });
  // 4b: summarizing keeps failing.
  const failingState = $derived<OverviewState | null>(
    engineOn && ctxStatus?.summarizingFailure
      ? {
          kind: "failing",
          failure: ctxStatus.summarizingFailure,
          coveredUntilMs: ctxStatus.coveredUntilMs ?? null,
        }
      : null,
  );
  // 4a: "summarized through" watermark, only while it cuts into this range.
  const watermark = $derived.by<string | null>(() => {
    const covered = ctxStatus?.coveredUntilMs;
    // ponytail: no activities = nothing summarized; the cursor alone only marks empty time.
    if (!engineOn || covered == null || rangeActivities.length === 0) return null;
    if (covered < range.startMs || covered >= Math.min(range.endMs, Date.now())) return null;
    return `summarized through ${clockLabel(covered)}`;
  });
  // Category/Focus tile copy when the range has no summarized activity (OV-23).
  const engineTileEmpty = $derived(
    olderThanBackfill
      ? `Not summarized — this ${rangeMode} is older than ${backfillDays} days.`
      : waitingFirst
        ? "Fills in after the first summary."
        : atLatest
          ? null
          : "Nothing summarized for this range.",
  );

  // ── View model ─────────────────────────────────────────────────────────
  const capture = $derived<LedeCapture>({
    loading: !statusLoaded || freeLoading,
    failed: usageFailed,
    trackedMs: ledeStats.trackedMs,
    deltaMs: trackedDeltaMs,
    dailyAvgMs: capturedDays ? ledeStats.trackedMs / capturedDays : 0,
    capturedDays,
    appCount: listedApps.length,
    topApp: listedApps[0]?.app ?? null,
    noAppTime,
    partlyRemoved: retention?.kind === "partly",
    nothing: nothingCaptured,
    idle: idleState,
  });
  const engine = $derived<LedeEngine>({
    capOnly,
    paused: statusLoaded && !capOnly && !engineOn,
    loading: engineTilesLoading,
    failed: engineError != null,
    activityCount: rangeActivities.length,
    focus,
    deepDelta,
    top: ledeStats.topCategory,
    digest,
    digestReason,
    digestLoading,
    regenerating: digestRegenerating,
    digestError,
    waitingFirst,
    recordingSince: recordingSince ? clockLabel(recordingSince) : null,
    olderNote: olderThanBackfill
      ? `Older than the ${backfillDays} days Mnema summarizes on its own.${retention?.kind !== "removed" ? " The footage is still on the Timeline." : ""}`
      : null,
    watermark,
    catchingUp: Boolean(ctxStatus?.backfilling && engineOn),
  });

  // ⌘J asks about the shown period (capture phase: ahead of the shell's ⌘J,
  // which focuses Ask everywhere else).
  function onKeydown(event: KeyboardEvent): void {
    if (capOnly || event.key.toLowerCase() !== "j" || !(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
    if (categoryModalOpen || appsModalOpen || focusModalOpen) return;
    event.preventDefault();
    void goto(askHref);
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<section class="overview" aria-label="Overview">
  <div class="ov-ctl" use:inBar>
    <a
      class="mx-btn mx-btn--ghost"
      href={capOnly ? undefined : askHref}
      aria-disabled={capOnly || undefined}
      use:tip={capOnly ? "Ask needs AI — turn it on in Settings" : `Opens Chat with ${scope} in scope`}
      ><IconSparkle class="mx-btn__accent" width="14" height="14" aria-hidden="true" /><span>Ask about {scope}</span><kbd
        >⌘J</kbd
      ></a
    >
    <DateInput
      mode="period"
      unit={rangeMode}
      value={toIso(firstDay)}
      min={minDay}
      align="end"
      {density}
      onchange={onPeriod}
    />
  </div>

  {#if failingState}
    <!-- 4b: the cause, not "still learning…" forever. -->
    <OverviewStates state={failingState} />
  {/if}

  <OverviewLede
    period={{ unit: rangeMode, current: atLatest, scope, when }}
    {capture}
    {engine}
    onReread={() => void loadDigest(true)}
  />

  <OverviewExhibits
    unit={rangeMode}
    {scope}
    {capOnly}
    nothing={nothingCaptured && rangeActivities.length === 0}
    {columns}
    {categories}
    {focus}
    {heat}
    apps={listedApps}
    usage={{ loading: !statusLoaded || freeLoading, failed: usageFailed ? freeError : null, noAppNote: noAppTime?.note ?? null }}
    engine={{ loading: engineTilesLoading, failed: engineError, emptyNote: engineTileEmpty }}
    onRetryUsage={() => void loadFree()}
    onRetryEngine={() => void loadEngine()}
    onOpenCategories={() => (categoryModalOpen = true)}
    onOpenFocus={() => (focusModalOpen = true)}
    onOpenApps={() => (appsModalOpen = true)}
  />

  {#if !capOnly}
    <OverviewFeed
      unit={rangeMode}
      {scope}
      startMs={range.startMs}
      endMs={range.endMs}
      {conclusions}
      activities={rangeActivities}
      loading={engineTilesLoading}
      error={engineError}
      correcting={correctingActivity}
      onRetry={() => void loadEngine()}
      onCorrectCategory={(a, c) => void correctCategory(a, c)}
      onOpenSubject={(s) => onOpenSubject?.(s)}
      onOpenSubjects={() => onOpenTab?.("subjects")}
    />
  {/if}

  <CategoryDetailModal
    open={categoryModalOpen}
    threads={activityThreads}
    {rangeMode}
    {rangeLabel}
    {correctingActivity}
    onClose={() => (categoryModalOpen = false)}
    onCorrectCategory={correctCategory}
    onCorrectFocus={correctFocus}
  />
  <AppDetailModal open={appsModalOpen} apps={allApps} {rangeLabel} onClose={() => (appsModalOpen = false)} />
  <FocusDetailModal
    open={focusModalOpen}
    activities={rangeActivities}
    focusRows={heat}
    {rangeLabel}
    onClose={() => (focusModalOpen = false)}
  />
</section>

<style>
  .ov-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
</style>
