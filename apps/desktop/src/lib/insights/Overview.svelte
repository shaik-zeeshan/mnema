<script lang="ts">
  // Overview — the default Insights sub-surface (issues #104/#105/#108).
  //
  // Two tiers, tagged inline via .tier-badge (mirrors overview.html):
  //   FREE   = counting-only, grayscale, ALWAYS-ON. Aggregated from captured
  //            Search Context via `get_usage_charts` (time-per-app + activity
  //            heatmap). Renders for everyone, even with no engine and zero
  //            conclusions.
  //   ENGINE = the "color": categorized + focus charts from Activities, plus the
  //            dossier (Conclusions) + the Activity story feed with corrections.
  //            Gated on Reasoning Engine availability.
  //
  // Engine-off NEVER shows an empty Overview: the FREE grayscale tiles stay and
  // the engine tiles + dossier are replaced with an "enable the engine" invite.
  //
  // Props:
  //   onOpenSubject?: (subject: string) => void — drill into a Subject.
  //   onOpenTab?: (tab) => void                 — jump Insights sub-surfaces.

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettingsWindow } from "$lib/surface-windows";
  import type {
    Activity,
    ActivityCategory,
    Conclusion,
    UserContextStatus,
    AiRuntimeStatus,
  } from "$lib/types/recording";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import StackedBar from "$lib/insights/charts/StackedBar.svelte";
  import Heatmap from "$lib/insights/charts/Heatmap.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  interface Props {
    onOpenSubject?: (subject: string) => void;
    onOpenTab?: (tab: "overview" | "subjects" | "context" | "chat") => void;
  }

  let { onOpenSubject, onOpenTab }: Props = $props();

  // ── Usage-chart DTO (FREE tier; mirrors capture-types/usage_charts.rs) ──
  interface AppUsage {
    app: string;
    appBundleId: string | null;
    activeMs: number;
    frameCount: number;
  }
  interface SiteUsage {
    domain: string;
    activeMs: number;
    frameCount: number;
  }
  interface AppTransition {
    fromApp: string;
    toApp: string;
    count: number;
  }
  interface HeatmapBucket {
    bucketStartMs: number;
    intensityCount: number;
  }
  interface UsageCharts {
    rangeStartMs: number;
    rangeEndMs: number;
    timePerApp: AppUsage[];
    timePerSite: SiteUsage[];
    appTransitions: AppTransition[];
    activityHeatmap: HeatmapBucket[];
  }

  // ── Category → colour token mapping (engine tier) ──────────────────────
  const CATEGORY_COLOR: Record<ActivityCategory, string> = {
    coding: "--cat-coding",
    research: "--cat-research",
    communication: "--cat-communication",
    design: "--cat-design",
    testing: "--cat-testing",
    personal: "--cat-personal",
    distractions: "--cat-distractions",
  };
  // Stable legend ordering.
  const CATEGORY_ORDER: ActivityCategory[] = [
    "coding",
    "research",
    "communication",
    "design",
    "testing",
    "personal",
    "distractions",
  ];
  const UNCATEGORIZED_COLOR = "--chart-grey-3";

  function categoryLabel(c: ActivityCategory): string {
    return c.charAt(0).toUpperCase() + c.slice(1);
  }

  // ── Date range ─────────────────────────────────────────────────────────
  type RangeMode = "day" | "week" | "month";
  let rangeMode = $state<RangeMode>("week");
  // `anchor` is a millis timestamp inside the currently-selected window; the
  // stepper moves it by one unit, the mode picks the window size. Bounds are
  // local-calendar.
  let anchor = $state<number>(Date.now());

  function startOfDay(ms: number): number {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }
  // Local-calendar bounds [startMs, endMs) for the active range.
  const range = $derived.by<{ startMs: number; endMs: number }>(() => {
    if (rangeMode === "day") {
      const start = startOfDay(anchor);
      const end = new Date(start);
      end.setDate(end.getDate() + 1);
      return { startMs: start, endMs: end.getTime() };
    }
    if (rangeMode === "week") {
      // Week starts Monday (local).
      const d = new Date(startOfDay(anchor));
      const dow = (d.getDay() + 6) % 7; // 0 = Monday
      d.setDate(d.getDate() - dow);
      const start = d.getTime();
      const end = new Date(start);
      end.setDate(end.getDate() + 7);
      return { startMs: start, endMs: end.getTime() };
    }
    // month
    const d = new Date(anchor);
    const start = new Date(d.getFullYear(), d.getMonth(), 1).getTime();
    const end = new Date(d.getFullYear(), d.getMonth() + 1, 1).getTime();
    return { startMs: start, endMs: end };
  });

  function stepRange(dir: -1 | 1): void {
    const d = new Date(anchor);
    if (rangeMode === "day") d.setDate(d.getDate() + dir);
    else if (rangeMode === "week") d.setDate(d.getDate() + dir * 7);
    else d.setMonth(d.getMonth() + dir);
    anchor = d.getTime();
  }

  function setMode(mode: RangeMode): void {
    rangeMode = mode;
  }

  const rangeLabel = $derived.by<string>(() => {
    const { startMs, endMs } = range;
    const start = new Date(startMs);
    const lastDay = new Date(endMs - 1);
    const monthFmt: Intl.DateTimeFormatOptions = {
      month: "short",
      day: "numeric",
    };
    if (rangeMode === "day") {
      return start.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
        year:
          start.getFullYear() === new Date().getFullYear()
            ? undefined
            : "numeric",
      });
    }
    if (rangeMode === "month") {
      return start.toLocaleDateString(undefined, {
        month: "long",
        year: "numeric",
      });
    }
    // week → "Jun 2 – 8" or "May 30 – Jun 5"
    const sameMonth = start.getMonth() === lastDay.getMonth();
    const startStr = start.toLocaleDateString(undefined, monthFmt);
    const endStr = sameMonth
      ? String(lastDay.getDate())
      : lastDay.toLocaleDateString(undefined, monthFmt);
    return `${startStr} – ${endStr}`;
  });

  // Is the active window the current one (disables "next" stepping past now)?
  const atLatest = $derived(Date.now() < range.endMs);

  // ── Engine gating ────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) ||
      Boolean(ctxStatus?.engineAvailable),
  );

  // ── Loaded data ────────────────────────────────────────────────────────
  let usage = $state<UsageCharts | null>(null);
  let activities = $state<Activity[]>([]);
  let conclusions = $state<Conclusion[]>([]);

  let loadingFree = $state(true);
  let loadingEngine = $state(false);
  let freeError = $state<string | null>(null);
  // Whether the engine-status calls have resolved at least once. Until then we
  // don't yet know engine on/off, so engine tiles + the story feed show a
  // skeleton rather than flashing the "enable the engine" invite.
  let statusLoaded = $state(false);
  // Whether the engine data (activities + conclusions) has loaded at least once
  // for the CURRENT range. Drives the engine-tile / feed skeleton vs. empty
  // distinction so "still learning" only shows after a real load with no data.
  let engineLoadedOnce = $state(false);

  // Per-conclusion local optimistic overrides (pin / dismiss reflect at once).
  let pinnedOverride = $state<Map<number, boolean>>(new Map());
  let dismissedIds = $state<Set<number>>(new Set());
  // Expanded "view evidence" conclusions.
  let expandedConclusions = $state<Set<number>>(new Set());
  // Inline-correction in flight, keyed by activity id.
  let correctingActivity = $state<Set<number>>(new Set());

  // ── Humanisers ─────────────────────────────────────────────────────────
  function humanizeMs(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "0m";
    const totalMin = Math.round(ms / 60000);
    const h = Math.floor(totalMin / 60);
    const m = totalMin % 60;
    if (h <= 0) return `${m}m`;
    if (m === 0) return `${h}h`;
    return `${h}h ${m}m`;
  }
  function humanizeHours(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "0h";
    const h = ms / 3600000;
    if (h < 10) return `${(Math.round(h * 10) / 10).toString()}h`;
    return `${Math.round(h)}h`;
  }
  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    if (day < 7) return `${day}d ago`;
    const wk = Math.floor(day / 7);
    if (wk < 5) return `${wk}w ago`;
    const mo = Math.floor(day / 30);
    if (mo < 12) return `${mo}mo ago`;
    return `${Math.floor(day / 365)}y ago`;
  }
  function clockTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  }

  // ── FREE TILE 1: top apps (MiniBars) ─────────────────────────────────
  const topApps = $derived.by(() => {
    const list = usage?.timePerApp ?? [];
    return list.slice(0, 5).map((a) => ({
      label: a.app,
      value: a.activeMs,
      sublabel: humanizeMs(a.activeMs),
    }));
  });

  // ── ENGINE TILE 2: categories aggregated by activity duration ─────────
  // Activities filtered to the active range by half-open `[startMs, endMs)`
  // span overlap. The start boundary is INCLUSIVE (`endedAtMs >= startMs`) so a
  // zero-duration activity sitting exactly on the local-midnight start (e.g. the
  // first activity of the day, where `startedAtMs === endedAtMs === startOfDay`)
  // is not dropped from the Day window — without this, such an item appears in
  // Week (whose earlier start keeps it) but vanishes from Day, even though all
  // its data is "today". The end boundary stays EXCLUSIVE to match the
  // `[startMs, endMs)` contract and avoid double-counting at midnight.
  const rangeActivities = $derived.by<Activity[]>(() => {
    const { startMs, endMs } = range;
    return activities.filter(
      (a) => a.startedAtMs < endMs && a.endedAtMs >= startMs,
    );
  });

  const categorySegments = $derived.by(() => {
    const totals = new Map<string, number>();
    for (const a of rangeActivities) {
      const dur = Math.max(0, a.endedAtMs - a.startedAtMs);
      if (dur <= 0) continue;
      const key = a.category ?? "__uncat__";
      totals.set(key, (totals.get(key) ?? 0) + dur);
    }
    const segments: { label: string; value: number; colorVar: string }[] = [];
    for (const c of CATEGORY_ORDER) {
      const v = totals.get(c);
      if (v && v > 0) {
        segments.push({
          label: categoryLabel(c),
          value: Math.round(v / 3600000), // hours for legend readout
          colorVar: CATEGORY_COLOR[c],
        });
      }
    }
    const uncat = totals.get("__uncat__");
    if (uncat && uncat > 0) {
      segments.push({
        label: "Uncategorized",
        value: Math.round(uncat / 3600000),
        colorVar: UNCATEGORIZED_COLOR,
      });
    }
    return segments;
  });

  // ── ENGINE TILE 3: focus heatmap (day rows × time-of-day slots) ───────
  // Six slots roughly 8a-8p in 2h bands; cell value = avg focus weight.
  const FOCUS_WEIGHT: Record<string, number> = {
    deep: 1.0,
    mixed: 0.55,
    distracted: 0.2,
  };
  const SLOT_COUNT = 6; // 8a,10a,12p,2p,4p,6p
  const SLOT_START_HOUR = 8;
  const SLOT_SPAN_HOURS = 2;

  const focusRows = $derived.by(() => {
    // Group range activities by local day → slot, averaging focus weight.
    const days = new Map<number, { sum: number[]; n: number[] }>();
    for (const a of rangeActivities) {
      const focus = a.focus;
      if (focus == null) continue;
      const w = FOCUS_WEIGHT[focus] ?? 0;
      const start = new Date(a.startedAtMs);
      const dayKey = startOfDay(a.startedAtMs);
      const hour = start.getHours();
      let slot = Math.floor((hour - SLOT_START_HOUR) / SLOT_SPAN_HOURS);
      slot = Math.max(0, Math.min(SLOT_COUNT - 1, slot));
      let day = days.get(dayKey);
      if (!day) {
        day = { sum: new Array(SLOT_COUNT).fill(0), n: new Array(SLOT_COUNT).fill(0) };
        days.set(dayKey, day);
      }
      day.sum[slot] += w;
      day.n[slot] += 1;
    }
    const out: { label: string; cells: number[] }[] = [];
    const sortedKeys = [...days.keys()].sort((x, y) => x - y);
    for (const key of sortedKeys) {
      const day = days.get(key)!;
      const cells = day.sum.map((s, i) => (day.n[i] > 0 ? s / day.n[i] : 0));
      out.push({
        label: new Date(key).toLocaleDateString(undefined, { weekday: "short" }),
        cells,
      });
    }
    return out;
  });

  // ── FREE TILE 4: this-range summary stats ─────────────────────────────
  const summary = $derived.by(() => {
    const buckets = usage?.activityHeatmap ?? [];
    // Total tracked time ≈ sum of app active time (the honest "time on app").
    const totalMs = (usage?.timePerApp ?? []).reduce(
      (acc, a) => acc + a.activeMs,
      0,
    );
    // Active days = distinct local-calendar days with any heatmap intensity.
    const activeDays = new Set<number>();
    const perDay = new Map<number, number>();
    for (const b of buckets) {
      if (b.intensityCount <= 0) continue;
      const dayKey = startOfDay(b.bucketStartMs);
      activeDays.add(dayKey);
      perDay.set(dayKey, (perDay.get(dayKey) ?? 0) + b.intensityCount);
    }
    const days = activeDays.size;
    const avgMs = days > 0 ? totalMs / days : 0;
    // Deep-focus % over range activities (engine tier only).
    let deepPct: number | null = null;
    if (engineOn) {
      let deep = 0;
      let counted = 0;
      for (const a of rangeActivities) {
        const focus = a.focus;
        if (focus == null) continue;
        counted += 1;
        if (focus === "deep") deep += 1;
      }
      deepPct = counted > 0 ? Math.round((deep / counted) * 100) : null;
    }
    // Per-day spark for the mini bar strip (ordered by day).
    const spark = [...perDay.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([, v]) => v);
    const sparkMax = spark.reduce((m, v) => Math.max(m, v), 0);
    return {
      totalLabel: humanizeHours(totalMs),
      days,
      avgLabel: humanizeHours(avgMs),
      deepPct,
      spark,
      sparkMax,
    };
  });

  // ── Dossier conclusions (engine tier) ─────────────────────────────────
  // visible first (by confidence), faded below. dismissed dropped.
  function isPinned(c: Conclusion): boolean {
    const o = pinnedOverride.get(c.id);
    return o === undefined ? c.pinned : o;
  }
  const dossier = $derived.by<Conclusion[]>(() => {
    const live = conclusions.filter(
      (c) => c.status !== "dismissed" && !dismissedIds.has(c.id),
    );
    const visible = live
      .filter((c) => c.status === "visible")
      .sort(
        (a, b) =>
          Number(isPinned(b)) - Number(isPinned(a)) ||
          b.confidence - a.confidence,
      );
    const faded = live
      .filter((c) => c.status === "faded")
      .sort((a, b) => b.confidence - a.confidence);
    return [...visible, ...faded];
  });

  function conclusionTrend(c: Conclusion): "up" | "steady" | "down" | "faded" {
    if (c.status === "faded") return "faded";
    // Heuristic from recency of last support vs. formation; we don't fetch
    // per-subject history here to keep the feed light.
    if (c.lastSupportedAtMs > c.formedAtMs + 3 * 86400000) return "up";
    if (Date.now() - c.lastSupportedAtMs > 14 * 86400000) return "down";
    return "steady";
  }

  // ── Story activities (#108 corrections) ───────────────────────────────
  const storyActivities = $derived.by<Activity[]>(() => {
    return [...rangeActivities]
      .sort((a, b) => b.startedAtMs - a.startedAtMs)
      .slice(0, 12);
  });

  function activityFocus(a: Activity): string | null {
    return a.focus ?? null;
  }

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

  async function loadFree(): Promise<void> {
    loadingFree = true;
    try {
      const { startMs, endMs } = range;
      usage = await invoke<UsageCharts>("get_usage_charts", { startMs, endMs });
      freeError = null;
    } catch (error) {
      freeError = error instanceof Error ? error.message : String(error);
    } finally {
      loadingFree = false;
    }
  }

  // Page through activities to cover the range. Newest-first; stop once we've
  // walked past the range start (or hit a sane cap to stay responsive).
  async function loadEngine(): Promise<void> {
    if (!engineOn) {
      activities = [];
      conclusions = [];
      engineLoadedOnce = true;
      return;
    }
    loadingEngine = true;
    try {
      const { startMs } = range;
      const PAGE = 100;
      const MAX = 400;
      const collected: Activity[] = [];
      let offset = 0;
      while (offset < MAX) {
        const page = await invoke<Activity[]>("list_user_context_activities", {
          limit: PAGE,
          offset,
        });
        if (page.length === 0) break;
        collected.push(...page);
        const oldest = page[page.length - 1];
        // Newest-first: once the oldest of this page predates the window start,
        // no further pages can contribute.
        if (oldest.startedAtMs < startMs) break;
        if (page.length < PAGE) break;
        offset += PAGE;
      }
      activities = collected;
      conclusions = await invoke<Conclusion[]>(
        "list_user_context_conclusions",
        { includeFaded: true },
      );
      // Clear stale optimistic overrides now that we have fresh truth.
      pinnedOverride = new Map();
      dismissedIds = new Set();
    } catch {
      // Best-effort; engine tiles degrade to empty / "still learning".
    } finally {
      loadingEngine = false;
      engineLoadedOnce = true;
    }
  }

  async function reloadAll(): Promise<void> {
    await loadStatus();
    await Promise.all([loadFree(), loadEngine()]);
  }

  // Re-query when the range changes (mode or step). Mark the range-scoped data
  // as "loading again" so the glance tiles + feed show skeletons during the
  // re-fetch instead of briefly showing the previous range's content or a
  // premature empty state.
  $effect(() => {
    // track range bounds
    range.startMs;
    range.endMs;
    void untrack(() => {
      engineLoadedOnce = false;
      void loadFree();
      void loadEngine();
    });
  });

  // ── Correction / pin / dismiss commands ───────────────────────────────
  async function correctCategory(
    a: Activity,
    category: ActivityCategory | null,
  ): Promise<void> {
    const next = new Set(correctingActivity);
    next.add(a.id);
    correctingActivity = next;
    try {
      await invoke("user_context_correct_activity_category", {
        id: a.id,
        category,
      });
      // Reflect immediately; refresh arrives via user_context_changed.
      activities = activities.map((x) =>
        x.id === a.id ? { ...x, category } : x,
      );
    } catch {
      // leave as-is; the event refresh will reconcile
    } finally {
      const done = new Set(correctingActivity);
      done.delete(a.id);
      correctingActivity = done;
    }
  }

  async function correctFocus(
    a: Activity,
    focus: "deep" | "mixed" | "distracted" | null,
  ): Promise<void> {
    const next = new Set(correctingActivity);
    next.add(a.id);
    correctingActivity = next;
    try {
      await invoke("user_context_correct_activity_focus", { id: a.id, focus });
      activities = activities.map((x) =>
        x.id === a.id ? ({ ...x, focus } as Activity) : x,
      );
    } catch {
      // ignore
    } finally {
      const done = new Set(correctingActivity);
      done.delete(a.id);
      correctingActivity = done;
    }
  }

  async function togglePin(c: Conclusion): Promise<void> {
    const next = !isPinned(c);
    const map = new Map(pinnedOverride);
    map.set(c.id, next);
    pinnedOverride = map;
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: next });
    } catch {
      // revert on failure
      const revert = new Map(pinnedOverride);
      revert.set(c.id, !next);
      pinnedOverride = revert;
    }
  }

  async function dismissConclusion(c: Conclusion): Promise<void> {
    const set = new Set(dismissedIds);
    set.add(c.id);
    dismissedIds = set;
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
    } catch {
      const revert = new Set(dismissedIds);
      revert.delete(c.id);
      dismissedIds = revert;
    }
  }

  function toggleEvidence(c: Conclusion): void {
    const set = new Set(expandedConclusions);
    if (set.has(c.id)) set.delete(c.id);
    else set.add(c.id);
    expandedConclusions = set;
  }

  function enableEngine(): void {
    void openSettingsWindow("intelligence");
  }

  // ── Mount ──────────────────────────────────────────────────────────────
  $effect(() => {
    void untrack(() => reloadAll());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadStatus();
      void loadEngine();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  const CATEGORY_OPTIONS: { value: ActivityCategory | ""; label: string }[] = [
    { value: "", label: "Uncategorized" },
    ...CATEGORY_ORDER.map((c) => ({
      value: c,
      label: categoryLabel(c),
    })),
  ];
  const FOCUS_OPTIONS: { value: string; label: string }[] = [
    { value: "", label: "—" },
    { value: "deep", label: "Deep" },
    { value: "mixed", label: "Mixed" },
    { value: "distracted", label: "Scattered" },
  ];

  const engineEmpty = $derived(
    engineOn &&
      !loadingEngine &&
      engineLoadedOnce &&
      activities.length === 0 &&
      conclusions.length === 0,
  );

  // ── Skeleton gating ────────────────────────────────────────────────────
  // FREE tiles show a skeleton until the first usage payload lands; they are
  // engine-independent (resolve on usage data even when the engine is off).
  const freeLoading = $derived(loadingFree && !usage);
  // Engine tiles (Categories/Focus) show a skeleton while we don't yet know the
  // engine state OR while the engine data is still loading for this range. When
  // the engine is known-off they fall through to the "enable the engine" note.
  const engineTilesLoading = $derived(
    !statusLoaded || (engineOn && (loadingEngine || !engineLoadedOnce)),
  );
  // The story/dossier feed is loading until status resolves and (when on) the
  // engine data has loaded once for the current range.
  const feedLoading = $derived(
    !statusLoaded || (engineOn && (loadingEngine || !engineLoadedOnce)),
  );
  const SKELETON_FEED_ROWS = 3;
</script>

<section class="overview" aria-label="Overview">
  <!-- ── Page header ── -->
  <div class="ov-header">
    <div class="titles">
      <h1>Overview</h1>
      <p class="subtitle">Scan your {rangeMode}, then read what it adds up to.</p>
    </div>
    <div class="ov-controls">
      <div class="date-range" role="group" aria-label="Date range">
        <button
          type="button"
          class:active={rangeMode === "day"}
          aria-pressed={rangeMode === "day"}
          onclick={() => setMode("day")}>Day</button
        >
        <button
          type="button"
          class:active={rangeMode === "week"}
          aria-pressed={rangeMode === "week"}
          onclick={() => setMode("week")}>Week</button
        >
        <button
          type="button"
          class:active={rangeMode === "month"}
          aria-pressed={rangeMode === "month"}
          onclick={() => setMode("month")}>Month</button
        >
      </div>
      <div class="date-stepper">
        <button class="nav" type="button" aria-label="Previous" onclick={() => stepRange(-1)}>‹</button>
        <span class="range-label">{rangeLabel}</span>
        <button
          class="nav"
          type="button"
          aria-label="Next"
          disabled={atLatest}
          onclick={() => stepRange(1)}>›</button
        >
      </div>
    </div>
  </div>

  <!-- ── Bento glance band ── -->
  <section class="glance-band" aria-label="At-a-glance summary">
    <!-- Time (FREE) -->
    <div class="glance-tile">
      <div class="glance-head">
        <span class="glance-title">Time</span>
        <span class="spacer"></span>
        <span class="tier-badge tier-badge--free">Free</span>
      </div>
      <div class="glance-body">
        {#if freeLoading}
          <div class="tile-skeleton tile-skeleton--bars" aria-busy="true">
            {#each Array.from({ length: 4 }) as _, i (i)}
              <div class="sk-bar-row">
                <Skeleton variant="text" width="34%" height="10px" />
                <Skeleton height="9px" radius="999px" />
              </div>
            {/each}
          </div>
        {:else if topApps.length === 0}
          <p class="tile-note">No tracked app time in this range.</p>
        {:else}
          <MiniBars items={topApps} />
        {/if}
      </div>
    </div>

    <!-- Categories (ENGINE) -->
    <div class="glance-tile">
      <div class="glance-head">
        <span class="glance-title">Categories</span>
        <span class="spacer"></span>
        <span class="tier-badge tier-badge--engine">Engine</span>
      </div>
      <div class="glance-body">
        {#if engineTilesLoading}
          <div class="tile-skeleton" aria-busy="true">
            <Skeleton height="14px" radius="6px" />
            <div class="sk-legend-rows">
              {#each Array.from({ length: 3 }) as _, i (i)}
                <Skeleton variant="text" width={`${70 - i * 12}%`} height="10px" />
              {/each}
            </div>
          </div>
        {:else if !engineOn}
          <p class="tile-note tile-note--locked">Enable the engine to light up categories.</p>
        {:else if categorySegments.length === 0}
          <p class="tile-note">No categorized activity yet.</p>
        {:else}
          <StackedBar segments={categorySegments} showLegend={true} />
        {/if}
      </div>
    </div>

    <!-- Focus (ENGINE) -->
    <div class="glance-tile">
      <div class="glance-head">
        <span class="glance-title">Focus</span>
        <span class="spacer"></span>
        <span class="tier-badge tier-badge--engine">Engine</span>
      </div>
      <div class="glance-body">
        {#if engineTilesLoading}
          <div class="tile-skeleton tile-skeleton--heat" aria-busy="true">
            {#each Array.from({ length: 3 }) as _, r (r)}
              <div class="sk-heat-row">
                <Skeleton variant="text" width="22px" height="9px" />
                <div class="sk-heat-cells">
                  {#each Array.from({ length: 6 }) as _, c (c)}
                    <Skeleton height="10px" radius="2px" />
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {:else if !engineOn}
          <p class="tile-note tile-note--locked">Enable the engine to see focus.</p>
        {:else if focusRows.length === 0}
          <p class="tile-note">No focus signal yet.</p>
        {:else}
          <Heatmap
            rows={focusRows}
            colorMode="focus"
            legend="deep · mixed · scattered"
          />
        {/if}
      </div>
    </div>

    <!-- This range (FREE) -->
    <div class="glance-tile">
      <div class="glance-head">
        <span class="glance-title">This {rangeMode}</span>
        <span class="spacer"></span>
        <span class="tier-badge tier-badge--free">Free</span>
      </div>
      <div class="glance-body">
        {#if freeLoading}
          <div class="tile-skeleton tile-skeleton--stat" aria-busy="true">
            <Skeleton variant="text" width="58%" height="27px" />
            <Skeleton variant="text" width="78%" height="10px" />
            <div class="sk-stat-row">
              <Skeleton variant="text" width="44px" height="13px" />
              <Skeleton variant="text" width="44px" height="13px" />
            </div>
            <Skeleton height="22px" radius="3px" />
          </div>
        {:else}
        <div class="week-stat">
          <div class="week-big">
            <span class="n">{summary.totalLabel}</span>
            <div class="u">tracked · {summary.days} active {summary.days === 1 ? "day" : "days"}</div>
          </div>
          <div class="week-sub">
            <div class="cell">
              <div class="n">{summary.avgLabel}</div>
              <div class="l">Daily avg</div>
            </div>
            {#if engineOn && summary.deepPct !== null}
              <div class="cell">
                <div class="n">{summary.deepPct}%</div>
                <div class="l">Deep focus</div>
              </div>
            {/if}
          </div>
          {#if summary.spark.length > 0}
            <div class="sparkbar" aria-hidden="true">
              {#each summary.spark as v (v)}
                <span
                  style="height:{summary.sparkMax > 0
                    ? Math.max(8, (v / summary.sparkMax) * 100)
                    : 0}%;"
                ></span>
              {/each}
            </div>
          {/if}
        </div>
        {/if}
      </div>
    </div>
  </section>

  <!-- ── Ask entry bar ── -->
  <button class="ask-entry" type="button" onclick={() => onOpenTab?.("chat")}>
    <span class="glyph" aria-hidden="true">◇</span>
    <span class="label">Ask about your history</span>
    <span class="hint">Opens Chat →</span>
  </button>

  {#if freeError}
    <div class="state state--error">
      <p class="state-title">Couldn't load your usage charts.</p>
      <p class="state-detail">{freeError}</p>
    </div>
  {/if}

  {#if feedLoading}
    <!-- ── Loading skeleton for the story/dossier feed ── -->
    <!-- Shown until status resolves (and, when the engine is on, until the
         range's engine data lands) so we never flash the "enable the engine"
         invite or the "still learning" empty state before we actually know. -->
    <div class="story-rule">
      <span class="line"></span>The story this {rangeMode}<span class="line"></span>
    </div>
    <div class="feed-column" aria-busy="true" aria-label="Loading your story">
      {#each Array.from({ length: SKELETON_FEED_ROWS }) as _, i (i)}
        <article class="entry entry--skeleton">
          <div class="sk-eyebrow">
            <Skeleton variant="text" width="160px" height="10px" />
            <Skeleton variant="text" width="64px" height="10px" />
          </div>
          <Skeleton variant="text" width="84%" height="18px" />
          <Skeleton variant="text" width="62%" height="18px" />
          <div class="sk-conclusion">
            <Skeleton variant="text" width="96px" height="18px" radius="4px" />
            <Skeleton width="120px" height="8px" radius="999px" />
          </div>
        </article>
      {/each}
    </div>
  {:else if !engineOn}
    <!-- ── No-engine invite (FREE tiles already shown above) ── -->
    <div class="card no-engine">
      <div class="ne-head">
        <span class="section-title">
          Without a reasoning engine
          <span class="tier-badge tier-badge--free">Free</span>
        </span>
      </div>
      <div class="ne-grid">
        <div class="ne-mini">
          <div class="cap">Glance band — free tiles only</div>
          {#if topApps.length > 0}
            <MiniBars items={topApps.slice(0, 3)} />
          {:else}
            <p class="tile-note">No tracked app time yet.</p>
          {/if}
        </div>
        <div class="ne-invite">
          <p>
            The grayscale <span class="strong">Time</span> and
            <span class="strong">This {rangeMode}</span> tiles stay. Categories, Focus,
            and your story feed go dark until a reasoning engine is on.
          </p>
          <p>
            Enable a reasoning engine to light up categories, focus, and your
            dossier.
          </p>
          <div>
            <button type="button" class="btn btn--accent" onclick={enableEngine}>
              Enable engine
            </button>
          </div>
        </div>
      </div>
    </div>
  {:else}
    <!-- ── Story / dossier feed (ENGINE) ── -->
    <div class="story-rule">
      <span class="line"></span>The story this {rangeMode}<span class="line"></span>
    </div>

    {#if engineEmpty}
      <div class="feed-column">
        <div class="state state--empty">
          <p class="state-title">Mnema is still learning…</p>
          <p class="state-detail">
            The engine is on, but it hasn't formed any Activities or Conclusions
            for this range yet.
            {#if ctxStatus?.backfilling}
              It's currently backfilling your history — check back shortly.
            {:else}
              As you work, categorized activity and your dossier will appear here.
            {/if}
          </p>
        </div>
      </div>
    {:else}
      <div class="feed-column">
        <!-- Conclusions dossier -->
        {#if dossier.length > 0}
          {#each dossier as c (c.id)}
            <article class="entry" class:entry--faded={c.status === "faded"}>
              <p class="eyebrow">
                <span class="diamond" aria-hidden="true">◆</span>
                <span class="tick" aria-hidden="true"></span>
                {c.status === "faded" ? "Quietly fading" : "Standing understanding"}
                <span class="rule"></span>
                {relativeTime(c.lastSupportedAtMs)}
              </p>
              <h2>{c.statement}</h2>

              <div class="conclusion">
                <span class="conclusion-statement">
                  <button
                    type="button"
                    class="subject-chip"
                    onclick={() => onOpenSubject?.(c.subject)}
                  >
                    {c.subject}
                  </button>
                </span>
                <span class="conf-wrap">
                  <ConfidenceBar
                    confidence={c.confidence}
                    trend={conclusionTrend(c)}
                  />
                </span>
                {#if c.status !== "faded"}
                  <span class="gentle-actions">
                    <button
                      type="button"
                      class="gentle-btn"
                      class:is-pinned={isPinned(c)}
                      onclick={() => void togglePin(c)}
                    >
                      {isPinned(c) ? "Pinned ◆" : "Pin"}
                    </button>
                    <button
                      type="button"
                      class="gentle-btn"
                      onclick={() => void dismissConclusion(c)}
                    >
                      Dismiss
                    </button>
                  </span>
                {/if}
                <button
                  type="button"
                  class="evidence-link"
                  onclick={() => toggleEvidence(c)}
                >
                  {expandedConclusions.has(c.id)
                    ? "hide evidence"
                    : "view evidence →"}
                </button>
              </div>

              {#if expandedConclusions.has(c.id)}
                <div class="evidence-list">
                  {#if c.evidence.length === 0}
                    <p class="evidence-empty">No grounding activities recorded.</p>
                  {:else}
                    {#each c.evidence as ev (ev.activityId + "-" + ev.stance)}
                      <div class="evidence-row">
                        <span
                          class="ev-stance"
                          class:ev-stance--contradict={ev.stance === "contradict"}
                          >{ev.stance === "contradict" ? "contradicts" : "supports"}</span
                        >
                        <span class="ev-title"
                          >{ev.activityTitle ?? `Activity #${ev.activityId}`}</span
                        >
                        {#if ev.activityStartedAtMs}
                          <span class="ev-time">{clockTime(ev.activityStartedAtMs)}</span>
                        {/if}
                      </div>
                    {/each}
                  {/if}
                </div>
              {/if}

              {#if c.status === "faded"}
                <p class="fade-note">Below the line — kept for your history.</p>
              {/if}
            </article>
          {/each}
        {/if}

        <!-- Activity story with #108 inline corrections -->
        {#if storyActivities.length > 0}
          <article class="entry entry--activities">
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              Recent activity
              <span class="rule"></span>
              correct as you go
            </p>
            <div class="act-list">
              {#each storyActivities as a (a.id)}
                <div class="act-row" class:act-row--busy={correctingActivity.has(a.id)}>
                  <div class="act-main">
                    <span class="act-title">{a.title}</span>
                    <span class="act-time">{clockTime(a.startedAtMs)}</span>
                  </div>
                  <div class="act-correct">
                    <label class="corr">
                      <span class="corr-label">Category</span>
                      <select
                        class="corr-select"
                        value={a.category ?? ""}
                        disabled={correctingActivity.has(a.id)}
                        onchange={(e) =>
                          void correctCategory(
                            a,
                            (e.currentTarget.value || null) as ActivityCategory | null,
                          )}
                      >
                        {#each CATEGORY_OPTIONS as opt (opt.value)}
                          <option value={opt.value}>{opt.label}</option>
                        {/each}
                      </select>
                    </label>
                    <label class="corr">
                      <span class="corr-label">Focus</span>
                      <select
                        class="corr-select"
                        value={activityFocus(a) ?? ""}
                        disabled={correctingActivity.has(a.id)}
                        onchange={(e) =>
                          void correctFocus(
                            a,
                            (e.currentTarget.value || null) as
                              | "deep"
                              | "mixed"
                              | "distracted"
                              | null,
                          )}
                      >
                        {#each FOCUS_OPTIONS as opt (opt.value)}
                          <option value={opt.value}>{opt.label}</option>
                        {/each}
                      </select>
                    </label>
                  </div>
                </div>
              {/each}
            </div>
          </article>
        {/if}

        {#if dossier.length === 0 && storyActivities.length === 0}
          <div class="state state--empty">
            <p class="state-title">Nothing for this {rangeMode} yet.</p>
            <p class="state-detail">
              Step the date range, or keep working — categorized activity and
              your dossier will fill in.
            </p>
          </div>
        {:else}
          <div class="feed-end">— you're all caught up —</div>
        {/if}
      </div>
    {/if}
  {/if}
</section>

<style>
  .overview {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  /* ---- Page header ---- */
  .ov-header {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    flex-wrap: wrap;
  }
  .ov-header .titles {
    flex: 1 1 auto;
    min-width: 0;
  }
  .ov-header h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .ov-header .subtitle {
    margin: 3px 0 0;
    font-size: 12px;
    color: var(--app-text-muted);
    text-transform: capitalize;
  }
  .ov-controls {
    display: inline-flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
  }

  /* Date-range segmented control (mirrors the canonical segmented look). */
  .date-range {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .date-range button {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    padding: 0 11px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .date-range button:hover {
    color: var(--app-text-strong);
  }
  .date-range button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  .date-stepper {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--app-text-muted);
  }
  .date-stepper .nav {
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-subtle);
    cursor: pointer;
    font: inherit;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .date-stepper .nav:hover:not(:disabled) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .date-stepper .nav:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .date-stepper .range-label {
    color: var(--app-text);
    letter-spacing: 0.02em;
    font-variant-numeric: tabular-nums;
  }

  /* ---- Bento glance band ---- */
  .glance-band {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
  }
  @media (max-width: 920px) {
    .glance-band {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  .glance-tile {
    display: flex;
    flex-direction: column;
    min-height: 156px;
    padding: 12px 13px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    min-width: 0;
    overflow: hidden;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .glance-tile:hover {
    border-color: var(--app-border-hover);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
  }
  .glance-head {
    display: flex;
    align-items: center;
    gap: 7px;
    margin-bottom: 11px;
  }
  .glance-title {
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .glance-head .spacer {
    flex: 1 1 auto;
  }
  .glance-body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }
  .tile-note {
    margin: 0;
    font-size: 11px;
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  .tile-note--locked {
    color: var(--app-text-faint);
    font-style: italic;
  }

  /* ---- Tile / feed loading skeletons ---- */
  .tile-skeleton {
    display: flex;
    flex-direction: column;
    gap: 9px;
    width: 100%;
  }
  .tile-skeleton--bars {
    gap: 11px;
  }
  .sk-bar-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sk-legend-rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tile-skeleton--heat {
    gap: 7px;
  }
  .sk-heat-row {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .sk-heat-cells {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 3px;
    flex: 1 1 auto;
  }
  .tile-skeleton--stat {
    gap: 9px;
  }
  .sk-stat-row {
    display: flex;
    gap: 14px;
  }

  .entry--skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .sk-eyebrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 9px;
    margin-bottom: 1px;
  }
  .sk-conclusion {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 13px;
    padding-top: 13px;
    border-top: 1px dashed var(--app-border);
  }

  /* tier-badge (mirrors app.css) */
  .tier-badge {
    font-size: 9.5px;
    letter-spacing: 0.08em;
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid var(--app-neutral-border);
    background: var(--app-neutral-bg);
    color: var(--app-neutral-text);
    text-transform: uppercase;
  }
  .tier-badge--free {
    border-color: var(--app-neutral-border);
    background: var(--app-neutral-bg);
    color: var(--app-neutral-text);
  }
  .tier-badge--engine {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }

  /* This-range stat tile */
  .week-stat {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .week-big .n {
    font-size: 27px;
    line-height: 1;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .week-big .u {
    font-size: 10px;
    color: var(--app-text-muted);
    letter-spacing: 0.03em;
    margin-top: 4px;
  }
  .week-sub {
    display: flex;
    gap: 14px;
  }
  .week-sub .cell .n {
    font-size: 13px;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .week-sub .cell .l {
    font-size: 9px;
    color: var(--app-text-muted);
    letter-spacing: 0.03em;
    text-transform: uppercase;
  }
  .sparkbar {
    display: flex;
    gap: 3px;
    align-items: flex-end;
    height: 22px;
  }
  .sparkbar span {
    flex: 1;
    border-radius: 2px;
    background: var(--app-accent);
    opacity: 0.82;
  }

  /* ---- Ask entry bar ---- */
  .ask-entry {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface-subtle);
    color: var(--app-text);
    cursor: pointer;
    font: inherit;
    text-align: left;
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      color 0.12s ease;
  }
  .ask-entry:hover {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .ask-entry .glyph {
    color: var(--app-accent-strong);
    font-size: 13px;
  }
  .ask-entry .label {
    flex: 1 1 auto;
    font-size: 12.5px;
  }
  .ask-entry .hint {
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .ask-entry:hover .hint {
    color: var(--app-accent-strong);
  }

  /* ---- Story feed ---- */
  .story-rule {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 720px;
    margin: 4px auto -2px;
    font-size: 10px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .story-rule .line {
    flex: 1 1 auto;
    height: 1px;
    background: var(--app-border);
  }

  .feed-column {
    width: 100%;
    max-width: 720px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .entry {
    position: relative;
    padding: 20px 22px 18px;
    border: 1px solid var(--app-border);
    border-radius: 12px;
    background: var(--app-surface);
  }
  .entry--faded {
    opacity: 0.7;
  }

  .eyebrow {
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    margin: 0 0 11px;
  }
  .eyebrow .tick {
    flex: 0 0 auto;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }
  .eyebrow .rule {
    flex: 1 1 auto;
    height: 1px;
    background: var(--app-border);
  }
  .eyebrow .diamond {
    color: var(--app-text-faint);
    letter-spacing: 0;
  }

  .entry h2 {
    margin: 0 0 10px;
    font-size: 18px;
    line-height: 1.4;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }

  /* conclusion row */
  .conclusion {
    margin-top: 15px;
    padding-top: 13px;
    border-top: 1px dashed var(--app-border);
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .conclusion-statement {
    flex: 1 1 200px;
    min-width: 0;
  }
  .conf-wrap {
    flex: 0 0 auto;
  }

  .subject-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: border-color 0.12s ease;
  }
  .subject-chip:hover {
    border-color: var(--app-accent);
  }

  .gentle-actions {
    display: inline-flex;
    gap: 4px;
  }
  .gentle-btn {
    font: inherit;
    font-size: 11px;
    padding: 3px 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .gentle-btn:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
    border-color: var(--app-border);
  }
  .gentle-btn.is-pinned {
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }

  .evidence-link {
    font: inherit;
    font-size: 11px;
    color: var(--app-text-muted);
    background: transparent;
    border: none;
    border-bottom: 1px dotted var(--app-border-strong);
    padding: 0 0 1px;
    cursor: pointer;
    white-space: nowrap;
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .evidence-link:hover {
    color: var(--app-text-strong);
    border-bottom-color: var(--app-border-hover);
  }

  .evidence-list {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px dashed var(--app-border);
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .evidence-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 11.5px;
    color: var(--app-text);
  }
  .ev-stance {
    font-size: 9.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    flex: 0 0 auto;
  }
  .ev-stance--contradict {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
    color: var(--app-danger-text);
  }
  .ev-title {
    color: var(--app-text-strong);
  }
  .ev-time {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .evidence-empty {
    margin: 0;
    font-size: 11px;
    color: var(--app-text-muted);
  }

  .fade-note {
    margin-top: 12px;
    font-size: 10.5px;
    color: var(--app-text-faint);
    font-style: italic;
  }

  /* activity story / corrections */
  .entry--activities {
    background: var(--app-surface-subtle);
  }
  .act-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .act-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 9px 0;
    transition: opacity 0.12s ease;
  }
  .act-row + .act-row {
    border-top: 1px dashed var(--app-border);
  }
  .act-row--busy {
    opacity: 0.5;
  }
  .act-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 200px;
  }
  .act-title {
    font-size: 12.5px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .act-time {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .act-correct {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    flex: 0 0 auto;
  }
  .corr {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .corr-label {
    font-size: 9px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .corr-select {
    font: inherit;
    font-size: 11px;
    padding: 3px 6px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text);
    cursor: pointer;
    transition: border-color 0.12s ease;
  }
  .corr-select:hover:not(:disabled) {
    border-color: var(--app-border-hover);
  }
  .corr-select:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .corr-select:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .feed-end {
    text-align: center;
    padding: 6px 0 0;
    font-size: 10.5px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }

  /* ---- No-engine card ---- */
  .card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 14px;
  }
  .section-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .no-engine {
    max-width: 720px;
    margin: 8px auto 0;
    background: var(--app-surface-subtle);
  }
  .no-engine .ne-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .no-engine .ne-grid {
    display: grid;
    grid-template-columns: 1.1fr 1fr;
    gap: 14px;
    align-items: start;
  }
  @media (max-width: 720px) {
    .no-engine .ne-grid {
      grid-template-columns: 1fr;
    }
  }
  .no-engine .ne-mini {
    border: 1px dashed var(--app-border);
    border-radius: 8px;
    padding: 10px 11px;
    background: var(--app-surface);
  }
  .no-engine .ne-mini .cap {
    font-size: 9.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    margin-bottom: 8px;
  }
  .no-engine .ne-invite {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .no-engine .ne-invite p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--app-text-muted);
  }
  .no-engine .ne-invite .strong {
    color: var(--app-text);
  }

  .btn {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0 12px;
    height: 28px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .btn:hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .btn--accent {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .btn--accent:hover {
    border-color: var(--app-accent);
    color: var(--app-accent);
  }

  /* ---- States ---- */
  .state {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .state--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .state--empty {
    border-style: dashed;
  }
  .state-title {
    margin: 0;
    font-size: 13px;
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
    line-height: 1.6;
  }
</style>
