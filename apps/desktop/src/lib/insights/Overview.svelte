<script lang="ts">
  // Overview — the default Insights sub-surface (issues #104/#105/#108).
  //
  // Two tiers, tagged inline via .tier-badge (mirrors overview.html):
  //   FREE   = counting-only, grayscale, ALWAYS-ON. Aggregated from captured
  //            Search Context via `get_usage_charts` (time-per-app + activity
  //            heatmap). Renders for everyone, even with no engine and zero
  //            conclusions.
  //   ENGINE = the "color": categorized + focus charts from Activities, plus
  //            conclusion DELTAS for the range (what changed — the full dossier
  //            lives on Subjects) + the Activity story feed with corrections.
  //            Gated on Reasoning Engine availability.
  //
  // Engine-off NEVER shows an empty Overview: the FREE grayscale tiles stay and
  // the engine tiles + dossier are replaced with an "enable the engine" invite.
  //
  // Props:
  //   onOpenSubject?: (subject: string) => void — drill into a Subject.
  //   onOpenTab?: (tab) => void                 — jump Insights sub-surfaces.

  import { untrack } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettingsWindow } from "$lib/surface-windows";
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
    UserContextDigest,
    UserContextStatus,
    AiRuntimeStatus,
  } from "$lib/types/recording";
  import {
    CATEGORY_COLOR,
    CATEGORY_ORDER,
    UNCATEGORIZED_COLOR,
    CATEGORY_OPTIONS,
    categoryLabel,
    humanizeMs,
    humanizeHours,
    startOfDay,
    buildActivityThreads,
    type ActivityThread,
  } from "$lib/insights/activity-helpers";
  import CategoryDetailModal from "$lib/insights/CategoryDetailModal.svelte";
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

  // ── Date range ─────────────────────────────────────────────────────────
  type RangeMode = "day" | "week" | "month";
  let rangeMode = $state<RangeMode>("week");
  // `anchor` is a millis timestamp inside the currently-selected window; the
  // stepper moves it by one unit, the mode picks the window size. Bounds are
  // local-calendar.
  let anchor = $state<number>(Date.now());

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
  // Narrative lede for the active range. `null` is the normal absent case
  // (engine off, sparse range) — the lede silently omits, never errors.
  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);

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
  // Inline-correction in flight, keyed by activity id.
  let correctingActivity = $state<Set<number>>(new Set());

  // ── Humanisers ─────────────────────────────────────────────────────────
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
  const TOP_APP_COUNT = 5;
  const topAppUsage = $derived.by(() =>
    (usage?.timePerApp ?? []).slice(0, TOP_APP_COUNT),
  );
  // Rows carry the resolved icon (or letter fallback) beside each label;
  // re-derives as `iconPathsByBundleId` fills in.
  const topApps = $derived.by(() =>
    topAppUsage.map((a) => ({
      label: a.app,
      value: a.activeMs,
      sublabel: humanizeMs(a.activeMs),
      iconSrc: appIconSrc(a.appBundleId),
      fallback: appIconFallback(a.app, a.appBundleId),
    })),
  );

  // Icon resolutions are bundle-id-keyed facts, not range-scoped data: a late
  // response from a previous range still maps the right id to the right icon,
  // so the merge map doubles as a cross-range cache — no staleness token.
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

  // Ask for icons whenever the Top apps tile's apps change (usage payload /
  // range).
  $effect(() => {
    const ids = topAppUsage.map((a) => a.appBundleId);
    void untrack(() => resolveAppIcons(ids));
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
    // `value` is the RAW duration (ms) so the stacked-bar widths stay
    // proportional even for sub-hour categories; `display` carries the
    // human-readable legend readout. Rounding to whole hours here would
    // collapse every category under ~30min to a 0-width sliver — a single
    // hour-scale category (e.g. creating) would then claim the whole bar.
    const segments: {
      label: string;
      value: number;
      colorVar: string;
      display: string;
    }[] = [];
    for (const c of CATEGORY_ORDER) {
      const v = totals.get(c);
      if (v && v > 0) {
        segments.push({
          label: categoryLabel(c),
          value: v,
          colorVar: CATEGORY_COLOR[c],
          display: humanizeMs(v),
        });
      }
    }
    const uncat = totals.get("__uncat__");
    if (uncat && uncat > 0) {
      segments.push({
        label: "Uncategorized",
        value: uncat,
        colorVar: UNCATEGORIZED_COLOR,
        display: humanizeMs(uncat),
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

  // ── Conclusion deltas (engine tier) ───────────────────────────────────
  // The story shows only what CHANGED in the active range — formed,
  // strengthened, or started fading. Standing beliefs with no delta live on
  // the Subjects tab, not here; otherwise the feed reads identically every
  // week. Each live conclusion lands in at most one bucket.
  type ConclusionDeltaKind = "formed" | "strengthened" | "fading";
  interface ConclusionDelta {
    c: Conclusion;
    kind: ConclusionDeltaKind;
  }
  function isPinned(c: Conclusion): boolean {
    const o = pinnedOverride.get(c.id);
    return o === undefined ? c.pinned : o;
  }
  const conclusionDeltas = $derived.by<ConclusionDelta[]>(() => {
    const { startMs, endMs } = range;
    const inRange = (ms: number) => ms >= startMs && ms < endMs;
    const live = conclusions.filter(
      (c) => c.status !== "dismissed" && !dismissedIds.has(c.id),
    );
    const formed: ConclusionDelta[] = [];
    const strengthened: ConclusionDelta[] = [];
    const fading: ConclusionDelta[] = [];
    for (const c of live) {
      if (inRange(c.formedAtMs)) {
        formed.push({ c, kind: "formed" });
      } else if (c.status === "visible" && inRange(c.lastSupportedAtMs)) {
        strengthened.push({ c, kind: "strengthened" });
      } else if (c.status === "faded" && inRange(c.updatedAtMs)) {
        // `updatedAtMs` is the best available proxy for when the fade
        // transition happened.
        fading.push({ c, kind: "fading" });
      }
      // No delta in this range → excluded; the dossier lives on Subjects.
    }
    const byPinThenConfidence = (a: ConclusionDelta, b: ConclusionDelta) =>
      Number(isPinned(b.c)) - Number(isPinned(a.c)) ||
      b.c.confidence - a.c.confidence;
    formed.sort(byPinThenConfidence);
    strengthened.sort(byPinThenConfidence);
    fading.sort(byPinThenConfidence);
    return [...formed, ...strengthened, ...fading];
  });

  function deltaLabel(kind: ConclusionDeltaKind): string {
    if (kind === "formed") return `Formed this ${rangeMode}`;
    if (kind === "strengthened") return "Strengthened";
    return "Started fading";
  }

  function conclusionTrend(c: Conclusion): "up" | "steady" | "down" | "faded" {
    if (c.status === "faded") return "faded";
    // Heuristic from recency of last support vs. formation; we don't fetch
    // per-subject history here to keep the feed light.
    if (c.lastSupportedAtMs > c.formedAtMs + 3 * 86400000) return "up";
    if (Date.now() - c.lastSupportedAtMs > 14 * 86400000) return "down";
    return "steady";
  }

  // Deltas grouped by kind for the single "What changed" card — groups render
  // formed → strengthened → fading; within-group order (pinned first, then
  // confidence desc) is already settled by `conclusionDeltas`.
  const DELTA_KIND_ORDER: ConclusionDeltaKind[] = [
    "formed",
    "strengthened",
    "fading",
  ];
  const deltaGroups = $derived.by(() =>
    DELTA_KIND_ORDER.map((kind) => ({
      kind,
      deltas: conclusionDeltas.filter((d) => d.kind === kind),
    })).filter((g) => g.deltas.length > 0),
  );

  // Per-group default cap — the compression that keeps a busy week one screen.
  // Applies to formed/fading only; strengthened is the quiet group and shows
  // ZERO rows by default — its header doubles as the expander, and one click
  // reveals everything (no inner cap: the user asked for it).
  const DELTA_GROUP_CAP = 2;
  // Groups showing every row, keyed by kind.
  let expandedGroups = $state<Set<ConclusionDeltaKind>>(new Set());
  function toggleGroup(kind: ConclusionDeltaKind): void {
    const set = new Set(expandedGroups);
    if (set.has(kind)) set.delete(kind);
    else set.add(kind);
    expandedGroups = set;
  }

  // Expanded delta rows (confidence + actions + evidence), by conclusion id.
  let expandedDeltaRows = $state<Set<number>>(new Set());
  function toggleDeltaRow(id: number): void {
    const set = new Set(expandedDeltaRows);
    if (set.has(id)) set.delete(id);
    else set.add(id);
    expandedDeltaRows = set;
  }

  // ── Activity threads (#108 corrections) ───────────────────────────────
  // The range's activities grouped by category. No longer rendered inline in
  // the feed — they feed the CategoryDetailModal opened from the "Categories"
  // glance tile, where each thread expands to its raw activities + corrections.
  // Covers ALL of the range, not a newest-12 log slice.
  const activityThreads = $derived.by<ActivityThread[]>(() =>
    buildActivityThreads(rangeActivities),
  );

  // The "Categories" glance tile opens the per-category breakdown in a modal
  // (the threads no longer live inline in the feed — Phase 2 redesign). The
  // per-row "adjust" popover lives in the modal too.
  let categoryModalOpen = $state(false);

  // ── Needs attention (#? — tag what the engine missed) ──────────────────
  // Uncategorized in-range activities, newest first — the one place in the feed
  // where a quick correction earns its keep.
  const needsAttention = $derived(
    rangeActivities
      .filter((a) => a.category == null)
      .sort((a, b) => b.startedAtMs - a.startedAtMs),
  );
  // Cap the list to keep a busy range one screen; one toggle reveals the rest.
  const NEEDS_ATTENTION_CAP = 5;
  let needsAttentionExpanded = $state(false);

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

  // Narrative lede fetch. Backend returns null (not an error) when the engine
  // is off or the range is too sparse; real errors collapse into null too —
  // the lede is omitted, never an error surface. An unchanged range is a cheap
  // cache hit; a fresh range can take seconds (one model call).
  let digestRequestToken = 0;
  async function loadDigest(): Promise<void> {
    if (!statusLoaded || !engineOn) {
      digest = null;
      digestLoading = false;
      return;
    }
    const token = ++digestRequestToken;
    digestLoading = true;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<UserContextDigest | null>(
        "get_user_context_digest",
        { rangeKind: rangeMode, startMs, endMs },
      );
      if (token !== digestRequestToken) return; // range moved on — stale
      digest = next;
    } catch {
      if (token === digestRequestToken) digest = null;
    } finally {
      if (token === digestRequestToken) digestLoading = false;
    }
  }

  // Range steps debounce the digest fetch — a changed range can cost a paid
  // model call and the user may flick through weeks. Mount/refresh paths call
  // `loadDigest` directly instead.
  const DIGEST_DEBOUNCE_MS = 500;
  let digestDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleDigestLoad(): void {
    // Drop the old range's prose at once and invalidate in-flight responses —
    // last week's lede never sits over this week's cards.
    digestRequestToken += 1;
    digest = null;
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

  async function reloadAll(): Promise<void> {
    await loadStatus();
    await Promise.all([loadFree(), loadEngine(), loadDigest()]);
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
      // A new range is new content: stale expansion state would leave groups
      // open over different (possibly empty) rows.
      expandedGroups = new Set();
      expandedDeltaRows = new Set();
      void loadFree();
      void loadEngine();
      scheduleDigestLoad();
    });
    return () => {
      // A pending debounce dies with the range (or the component).
      if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
      digestDebounceTimer = null;
    };
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
      // Same range → cache hit; keeps the current lede up until fresh prose.
      void loadDigest();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });

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
          <button
            type="button"
            class="cat-bar-trigger"
            aria-haspopup="dialog"
            aria-label="View category breakdown"
            onclick={() => (categoryModalOpen = true)}
          >
            <StackedBar segments={categorySegments} showLegend={true} />
          </button>
          <span class="cat-bar-hint">view breakdown →</span>
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
      <!-- Mirrors the real feed: lede prose, then "What changed" rows +
           "Needs attention". -->
      <article class="entry entry--skeleton">
        <div class="sk-eyebrow">
          <Skeleton variant="text" width="64px" height="10px" />
          <Skeleton variant="text" width="48px" height="10px" />
        </div>
        <div class="sk-row">
          <Skeleton variant="text" width="92%" height="12px" />
        </div>
        <div class="sk-row">
          <Skeleton variant="text" width="64%" height="12px" />
        </div>
      </article>
      {#each Array.from({ length: 2 }) as _, card (card)}
        <article class="entry entry--skeleton">
          <div class="sk-eyebrow">
            <Skeleton variant="text" width="140px" height="10px" />
            <Skeleton variant="text" width="64px" height="10px" />
          </div>
          {#each Array.from({ length: 3 }) as _, r (r)}
            <div class="sk-row">
              <Skeleton variant="text" width={`${74 - r * 12}%`} height="12px" />
              <Skeleton variant="text" width="56px" height="10px" />
            </div>
          {/each}
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
        <!-- Narrative lede — the engine's read of the range, the feed's hero.
             A lede, not a control surface: no actions, and silently absent when
             the range is too sparse or the digest call fails. Layout top→bottom:
             eyebrow → headline (when present) → prose (or shimmer) → a 3-stat
             highlight row. -->
        {#if digest || digestLoading}
          <article class="entry entry--lede" aria-busy={!digest && digestLoading}>
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              In short
              <span class="rule"></span>
              {#if digest}{relativeTime(digest.generatedAtMs)}{/if}
            </p>
            {#if digest}
              <!-- Keyed on generation time: fresh prose replays the reveal,
                   a same-range cache hit does not. -->
              {#key digest.generatedAtMs}
                <div class="lede-body">
                  {#if digest.headline}
                    <h2 class="lede-headline">{digest.headline}</h2>
                  {/if}
                  <p class="lede-text">{digest.narrative}</p>
                </div>
              {/key}
            {:else}
              <div class="sk-row">
                <Skeleton variant="text" width="92%" height="12px" />
              </div>
              <div class="sk-row">
                <Skeleton variant="text" width="64%" height="12px" />
              </div>
            {/if}
            <!-- 3-stat highlight row — the range's headline numbers, sitting
                 where the app strip used to. Tracked is always present; Deep
                 and Top category render only when they have a value. -->
            <div class="lede-stats" aria-label="Highlights this {rangeMode}">
              <div class="lede-stat">
                <span class="lede-stat-n">{summary.totalLabel}</span>
                <span class="lede-stat-cap">tracked</span>
              </div>
              {#if summary.deepPct !== null}
                <div class="lede-stat">
                  <span class="lede-stat-n">{summary.deepPct}%</span>
                  <span class="lede-stat-cap">deep focus</span>
                </div>
              {/if}
              {#if categorySegments.length > 0}
                <div class="lede-stat">
                  <span class="lede-stat-n lede-stat-n--cat">
                    <span
                      class="lede-stat-swatch"
                      style="background:var({categorySegments[0].colorVar});"
                      aria-hidden="true"
                    ></span>
                    {categorySegments[0].label}
                  </span>
                  <span class="lede-stat-cap">top category</span>
                </div>
              {/if}
            </div>
          </article>
        {/if}

        <!-- What changed — one card; conclusion deltas as grouped rows -->
        {#if conclusionDeltas.length > 0}
          <article class="entry">
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              What changed
              <span class="rule"></span>
              expand a row for detail
            </p>
            <div class="delta-groups">
              {#each deltaGroups as g (g.kind)}
                {@const groupOpen = expandedGroups.has(g.kind)}
                <div class="delta-group">
                  {#if g.kind === "strengthened"}
                    <!-- Strengthened collapses to its header: the head is the
                         expander, identical typography, micro show/hide hint. -->
                    <button
                      type="button"
                      class="delta-group-head delta-group-head--toggle"
                      aria-expanded={groupOpen}
                      onclick={() => toggleGroup(g.kind)}
                    >
                      {deltaLabel(g.kind)} · {g.deltas.length}
                      <span class="delta-head-hint">{groupOpen ? "hide" : "show"}</span>
                    </button>
                  {:else}
                    <p class="delta-group-head">{deltaLabel(g.kind)} · {g.deltas.length}</p>
                  {/if}
                  {#each groupOpen
                    ? g.deltas
                    : g.kind === "strengthened"
                      ? []
                      : g.deltas.slice(0, DELTA_GROUP_CAP) as d (d.c.id)}
                    {@const c = d.c}
                    {@const open = expandedDeltaRows.has(c.id)}
                    <div class="delta-row" class:delta-row--faded={d.kind === "fading"}>
                      <div class="delta-line">
                        <span class="delta-statement">{c.statement}</span>
                        <!-- Subject + time sit on their own meta line so a long
                             subject name can't squeeze and wrap against the
                             statement. -->
                        <div class="delta-meta">
                          <button
                            type="button"
                            class="subject-chip"
                            onclick={() => onOpenSubject?.(c.subject)}
                          >
                            <span class="subject-chip-text">{c.subject}</span>
                          </button>
                          <span class="delta-when">{relativeTime(c.lastSupportedAtMs)}</span>
                          <button
                            type="button"
                            class="delta-toggle"
                            class:open
                            aria-expanded={open}
                            aria-label={open ? "Hide detail" : "Show detail"}
                            onclick={() => toggleDeltaRow(c.id)}>›</button
                          >
                        </div>
                      </div>
                      {#if open}
                        <div class="delta-detail">
                          <div class="delta-detail-line">
                            <span class="conf-wrap">
                              <ConfidenceBar
                                confidence={c.confidence}
                                trend={conclusionTrend(c)}
                              />
                            </span>
                            {#if d.kind === "fading"}
                              <span class="fade-note">Slipping below the line — kept for your history.</span>
                            {:else}
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
                          </div>
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
                        </div>
                      {/if}
                    </div>
                  {/each}
                  {#if g.kind !== "strengthened" && g.deltas.length > DELTA_GROUP_CAP}
                    <button
                      type="button"
                      class="evidence-link delta-more"
                      onclick={() => toggleGroup(g.kind)}
                    >
                      {groupOpen ? "show fewer" : `+${g.deltas.length - DELTA_GROUP_CAP} more`}
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
          </article>
        {/if}

        <!-- Needs attention — uncategorized in-range activities, with a compact
             inline category picker so the user can tag what the engine missed. -->
        {#if needsAttention.length > 0}
          {@const shown = needsAttentionExpanded
            ? needsAttention
            : needsAttention.slice(0, NEEDS_ATTENTION_CAP)}
          <article class="entry entry--attention">
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              Needs attention
              <span class="rule"></span>
              tag what the engine missed
            </p>
            <div class="attn-list">
              {#each shown as a (a.id)}
                <div class="attn-row" class:attn-row--busy={correctingActivity.has(a.id)}>
                  <div class="attn-main">
                    <span class="attn-title">{a.title}</span>
                    <span class="attn-time"
                      >{clockTime(a.startedAtMs)} · {humanizeMs(
                        Math.max(0, a.endedAtMs - a.startedAtMs),
                      )}</span
                    >
                  </div>
                  <label class="attn-pick">
                    <span class="attn-pick-label">Category</span>
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
                </div>
              {/each}
            </div>
            {#if needsAttention.length > NEEDS_ATTENTION_CAP}
              <button
                type="button"
                class="evidence-link attn-more"
                onclick={() => (needsAttentionExpanded = !needsAttentionExpanded)}
              >
                {needsAttentionExpanded
                  ? "show fewer"
                  : `+${needsAttention.length - NEEDS_ATTENTION_CAP} more`}
              </button>
            {/if}
          </article>
        {/if}

        {#if conclusionDeltas.length === 0 && needsAttention.length === 0}
          <div class="state state--empty">
            <p class="state-title">Nothing changed this {rangeMode}.</p>
            <p class="state-detail">
              No conclusions formed, strengthened, or faded in this range, and
              nothing left to tag. Step the date range, or keep working — your
              standing dossier lives on the Subjects tab.
            </p>
          </div>
        {:else}
          <div class="feed-end">— you're all caught up —</div>
        {/if}
      </div>
    {/if}
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

  /* Categories bar is a transparent trigger that opens the breakdown modal —
     the StackedBar still reads as the bar, with a subtle hover affordance. */
  .cat-bar-trigger {
    display: block;
    width: 100%;
    padding: 0;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    border-radius: 6px;
    transition: opacity 0.12s ease;
  }
  .cat-bar-trigger:hover {
    opacity: 0.85;
  }
  .cat-bar-trigger:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .cat-bar-hint {
    display: inline-block;
    margin-top: 8px;
    font-size: 11px;
    color: var(--app-text-muted);
    border-bottom: 1px dotted var(--app-border-strong);
    line-height: 1.3;
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
  }
  .sk-eyebrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 9px;
    margin-bottom: 8px;
  }
  .sk-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 9px 0;
  }
  .sk-row + .sk-row {
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

  /* Narrative lede — 2-4 sentences of prose, read not clicked. The feed's
     hero: a 2px accent edge + a wash that fades into the surface keep it
     distinct without leaving the terminal/green language, and the headline
     scale reads as a title above the rest of the feed. */
  .entry--lede {
    padding: 24px 26px 22px;
    border-left: 2px solid var(--app-accent);
    background: linear-gradient(
      to right,
      var(--app-accent-bg),
      var(--app-surface) 42%
    );
  }
  /* Fresh prose lands with a short reveal — the deliberate exception to the
     0.12s house transition. */
  .lede-body {
    animation: lede-reveal 0.25s ease;
  }
  @keyframes lede-reveal {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lede-body {
      animation: none;
    }
  }
  .lede-headline {
    margin: 0 0 10px;
    font-size: 24px;
    line-height: 1.22;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--app-text-strong);
  }
  .lede-text {
    margin: 0;
    font-size: 13px;
    line-height: 1.7;
    color: var(--app-text);
  }
  /* 3-stat highlight row — the range's headline numbers, a lighter echo of the
     "This {range}" tile's .week-stat / .week-sub .cell conventions. */
  .lede-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 14px 28px;
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px dashed var(--app-border);
  }
  .lede-stat {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .lede-stat-n {
    font-size: 17px;
    line-height: 1.1;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  /* The top-category "number" is a label, not a figure — it gets a swatch dot
     and ellipsises rather than wrapping. */
  .lede-stat-n--cat {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lede-stat-swatch {
    flex: 0 0 auto;
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .lede-stat-cap {
    font-size: 9px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }

  /* "What changed" — grouped conclusion-delta rows */
  .delta-groups {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .delta-group {
    display: flex;
    flex-direction: column;
  }
  .delta-group-head {
    margin: 0 0 2px;
    font-size: 9.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  /* Strengthened's head doubles as its expander — button reset keeps the
     typography identical to the plain heads; the hint is the only tell. */
  .delta-group-head--toggle {
    align-self: flex-start;
    display: inline-flex;
    align-items: baseline;
    gap: 7px;
    font: inherit;
    font-size: 9.5px;
    letter-spacing: 0.08em;
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .delta-group-head--toggle:hover {
    color: var(--app-text-muted);
  }
  .delta-head-hint {
    font-size: 9.5px;
    letter-spacing: 0.02em;
    text-transform: lowercase;
    color: var(--app-text-muted);
    border-bottom: 1px dotted var(--app-border-strong);
    padding-bottom: 1px;
    transition:
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .delta-group-head--toggle:hover .delta-head-hint {
    color: var(--app-text-strong);
    border-bottom-color: var(--app-border-hover);
  }
  .delta-row {
    display: flex;
    flex-direction: column;
    padding: 8px 0;
  }
  .delta-row + .delta-row {
    border-top: 1px dashed var(--app-border);
  }
  .delta-row--faded {
    opacity: 0.7;
  }
  /* Statement stacks above a meta line (subject + time + toggle) — the
     subject no longer competes with the statement for horizontal room. */
  .delta-line {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .delta-statement {
    min-width: 0;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--app-text-strong);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .delta-meta {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .delta-when {
    /* Pushes time + toggle to the right; the chip keeps its content width on
       the left and only ellipsises if the subject is very long. */
    margin-left: auto;
    flex: 0 0 auto;
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  /* Dedicated expand affordance — the chip stays its own button, so the row
     itself can't be one (a button inside a button is invalid HTML). */
  .delta-toggle {
    flex: 0 0 auto;
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font: inherit;
    font-size: 13px;
    line-height: 1;
    border: none;
    background: transparent;
    color: var(--app-text-faint);
    cursor: pointer;
    transition:
      transform 0.12s ease,
      color 0.12s ease;
  }
  .delta-toggle:hover {
    color: var(--app-text-strong);
  }
  .delta-toggle.open {
    transform: rotate(90deg);
  }
  .delta-detail {
    display: flex;
    flex-direction: column;
    padding: 10px 0 2px;
  }
  .delta-detail-line {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .delta-more {
    align-self: flex-start;
    margin-top: 7px;
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
    min-width: 0;
    max-width: 100%;
    transition: border-color 0.12s ease;
  }
  .subject-chip:hover {
    border-color: var(--app-accent);
  }
  /* Long subjects ellipsise rather than wrap to several lines. */
  .subject-chip-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    font-size: 10.5px;
    color: var(--app-text-faint);
    font-style: italic;
  }

  /* Needs attention — uncategorized in-range activities + an inline picker. */
  .entry--attention {
    background: var(--app-surface-subtle);
  }
  .attn-list {
    display: flex;
    flex-direction: column;
  }
  .attn-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 0;
    transition: opacity 0.12s ease;
  }
  .attn-row + .attn-row {
    border-top: 1px dashed var(--app-border);
  }
  .attn-row--busy {
    opacity: 0.5;
  }
  .attn-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .attn-title {
    font-size: 12.5px;
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attn-time {
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .attn-pick {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }
  .attn-pick-label {
    font-size: 9px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .attn-more {
    align-self: flex-start;
    margin-top: 9px;
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
