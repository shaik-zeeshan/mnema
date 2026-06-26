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
  import { message } from "@tauri-apps/plugin-dialog";
  import { openSettings } from "$lib/surface-windows";
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
    startOfHour,
    buildActivityThreads,
    type ActivityThread,
  } from "$lib/insights/activity-helpers";
  import CategoryDetailModal from "$lib/insights/CategoryDetailModal.svelte";
  import AppDetailModal from "$lib/insights/AppDetailModal.svelte";
  import FocusDetailModal from "$lib/insights/FocusDetailModal.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import StackedBar from "$lib/insights/charts/StackedBar.svelte";
  import Heatmap from "$lib/insights/charts/Heatmap.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import Select from "$lib/components/Select.svelte";
  import { humanizeError } from "$lib/format-error";

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

  // Jump back to the current period (today / this week / this month) — the
  // stepper otherwise has no one-click way home once you've paged into the past.
  function resetRange(): void {
    anchor = Date.now();
  }

  // Date-range Segmented options (shared primitive replaces the hand-rolled
  // button group).
  const RANGE_OPTIONS = [
    { value: "day", label: "Day" },
    { value: "week", label: "Week" },
    { value: "month", label: "Month" },
  ];

  // The shared Select primitive can't round-trip an empty-string value (it
  // reads "" as "no selection" and shows the placeholder), so map the
  // Uncategorized option onto a sentinel and translate back on change.
  const CATEGORY_NONE = "__uncategorized__";
  const CATEGORY_SELECT_OPTIONS = CATEGORY_OPTIONS.map((o) => ({
    value: o.value === "" ? CATEGORY_NONE : o.value,
    label: o.label,
  }));

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
  // The manual re-read (re-digest button): a forced regeneration that bypasses
  // the backend cache/freshness floor. Unlike the silent auto-load, its failure
  // is shown — `digestError` carries the one-sentence reason ("…rejected your
  // API key", "…couldn't complete this request") so a read that never appears
  // (e.g. an OpenAI-compatible provider that fluffed the structured call) is no
  // longer a mystery.
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);

  let loadingFree = $state(true);
  let loadingEngine = $state(false);
  let freeError = $state<string | null>(null);
  // Engine-data (activities + conclusions) load failure for the active range.
  // Mirrors `freeError`: kept DISTINCT from the "still learning" empty state so a
  // real fetch failure surfaces a recoverable error (with Retry) instead of
  // misrepresenting itself as "the engine hasn't formed anything yet".
  let engineError = $state<string | null>(null);
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

  // The full ranked app list (NOT sliced) for the AppDetailModal — same row
  // shape the modal contract expects (app/activeMs/frameCount/iconSrc/fallback).
  // Backend already sorts `timePerApp` descending by active time; keep it.
  const allApps = $derived.by(() =>
    (usage?.timePerApp ?? []).map((a) => ({
      app: a.app,
      activeMs: a.activeMs,
      frameCount: a.frameCount,
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

  // Ask for icons whenever the usage payload / range changes. Resolve the FULL
  // app list (not just the top 5) so every row in the AppDetailModal gets an
  // icon, not only the ones shown on the small "Time" card.
  $effect(() => {
    const ids = (usage?.timePerApp ?? []).map((a) => a.appBundleId);
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
    const { startMs, endMs } = range;
    const totals = new Map<string, number>();
    for (const a of rangeActivities) {
      // Count only the portion of the activity that falls inside the active
      // range. An activity straddling a range boundary would otherwise add its
      // full duration here (and again in the wider Week window), double-counting
      // the out-of-range span. Clip to `[startMs, endMs)` so each bucket sums
      // only the time actually spent in this range.
      const clippedStart = Math.max(a.startedAtMs, startMs);
      const clippedEnd = Math.min(a.endedAtMs, endMs);
      const dur = Math.max(0, clippedEnd - clippedStart);
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

  // `categorySegments` is ordered by CATEGORY_ORDER for a stable bar layout,
  // so its first element is NOT the busiest category. Rank by actual time
  // (`value` is raw ms) to surface the real top category in the lede.
  const topCategory = $derived(
    [...categorySegments].sort((a, b) => b.value - a.value)[0],
  );

  // ── ENGINE TILE 3: focus heatmap (day rows × time-of-day slots) ───────
  // Twelve 2h slots covering the full local day (12a-12a); cell value = avg
  // focus weight. Covering all 24h means early/late work (before 8a, after 6p)
  // lands in its own real slot instead of being folded into an edge slot and
  // mislabeled as morning/evening work.
  const FOCUS_WEIGHT: Record<string, number> = {
    deep: 1.0,
    mixed: 0.55,
    distracted: 0.2,
  };
  const SLOT_SPAN_HOURS = 2;
  const SLOT_START_HOUR = 0;
  const SLOT_COUNT = 24 / SLOT_SPAN_HOURS; // 12 slots: 12a,2a,…,10p

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
      // Full-day band: every 0–23 hour maps to its own slot, so off-band
      // (early/late) hours aren't folded into an edge slot. The clamp is now
      // only a defensive guard against an out-of-range hour, never a fold.
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
    // Spark granularity follows the selected period: a Day window buckets the
    // heatmap by HOUR (its native granularity — `get_usage_charts` floors each
    // bucket to the hour), Week/Month by calendar DAY. Coarser-than-day stays
    // day; finer-than-hour isn't needed since the source is already hourly.
    const sparkByHour = rangeMode === "day";
    const sparkLabel = sparkByHour ? "per hour" : "per day";
    const perBucket = new Map<number, number>();
    for (const b of buckets) {
      if (b.intensityCount <= 0) continue;
      const key = sparkByHour
        ? startOfHour(b.bucketStartMs)
        : startOfDay(b.bucketStartMs);
      perBucket.set(key, (perBucket.get(key) ?? 0) + b.intensityCount);
    }
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
    // Period-aware spark for the mini bar strip (ordered by bucket start).
    const spark = [...perBucket.entries()]
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
      sparkLabel,
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

  // The "What changed" card splits in two: a Pinned section (pinned conclusions
  // pulled out so they stay put) and the changes proper, grouped formed →
  // strengthened → fading. Within-group order (confidence desc) is already
  // settled by `conclusionDeltas`.
  const DELTA_KIND_ORDER: ConclusionDeltaKind[] = [
    "formed",
    "strengthened",
    "fading",
  ];

  // Pinned deltas live in their own section and never appear in the change
  // groups; the rest flow through the capped "What changed" list.
  const pinnedDeltas = $derived(conclusionDeltas.filter((d) => isPinned(d.c)));
  const unpinnedDeltas = $derived(
    conclusionDeltas.filter((d) => !isPinned(d.c)),
  );

  // A single hard cap across the whole "What changed" list (all groups
  // combined) — the main page stays a glance; the rest live on Subjects, with
  // no inline expander to dump everything here.
  const WHAT_CHANGED_CAP = 10;
  const visibleDeltas = $derived(unpinnedDeltas.slice(0, WHAT_CHANGED_CAP));
  // Re-group the capped slice for rendering. Each head shows the full group size
  // so the totals stay honest even when the global cap hides some rows.
  const visibleDeltaGroups = $derived.by(() =>
    DELTA_KIND_ORDER.map((kind) => ({
      kind,
      deltas: visibleDeltas.filter((d) => d.kind === kind),
      total: unpinnedDeltas.filter((d) => d.kind === kind).length,
    })).filter((g) => g.deltas.length > 0),
  );

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
  let appsModalOpen = $state(false);
  let focusModalOpen = $state(false);

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

  // Monotonic gen tokens guard the range-scoped fetches against stale responses:
  // rapid ‹/› stepping fires overlapping loads, so gate every state assignment
  // on the token still being current — a slow earlier-range response that lands
  // after a newer fetch is dropped instead of overwriting the current tiles.
  // (Same shape as loadDigest's `digestRequestToken`.)
  let freeRequestToken = 0;
  async function loadFree(): Promise<void> {
    const token = ++freeRequestToken;
    loadingFree = true;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<UsageCharts>("get_usage_charts", {
        startMs,
        endMs,
      });
      if (token !== freeRequestToken) return; // range moved on — stale
      usage = next;
      freeError = null;
    } catch (error) {
      if (token === freeRequestToken)
        freeError = humanizeError(error);
    } finally {
      if (token === freeRequestToken) loadingFree = false;
    }
  }

  // Fetch the activities for the active range. Passing `startMs`/`endMs` makes
  // the backend return EVERY activity overlapping the selected period in one
  // call (overlap-bounded, not recency-capped) — so navigating to a past
  // week/month or a busy month (>400 activities) no longer truncates or misses
  // the window the way the old newest-400 global scan did.
  let engineRequestToken = 0;
  async function loadEngine(): Promise<void> {
    const token = ++engineRequestToken;
    if (!engineOn) {
      activities = [];
      conclusions = [];
      engineError = null;
      engineLoadedOnce = true;
      return;
    }
    loadingEngine = true;
    try {
      const { startMs, endMs } = range;
      const nextActivities = await invoke<Activity[]>(
        "list_user_context_activities",
        { startMs, endMs },
      );
      if (token !== engineRequestToken) return; // range moved on — stale
      const nextConclusions = await invoke<Conclusion[]>(
        "list_user_context_conclusions",
        { includeFaded: true },
      );
      if (token !== engineRequestToken) return; // range moved on — stale
      activities = nextActivities;
      conclusions = nextConclusions;
      engineError = null;
      // Clear stale optimistic overrides now that we have fresh truth.
      pinnedOverride = new Map();
      dismissedIds = new Set();
    } catch (error) {
      // A real fetch failure is NOT the "still learning" empty state — record it
      // so the feed renders a recoverable error with Retry.
      if (token === engineRequestToken)
        engineError = humanizeError(error);
    } finally {
      if (token === engineRequestToken) {
        loadingEngine = false;
        engineLoadedOnce = true;
      }
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
    digestError = null;
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

  // Re-read: force a fresh digest for the current range, ignoring the backend
  // cache + freshness floor. The deliberate counterpart to the silent auto-load
  // — this is a user action, so a failure is surfaced in `digestError` rather
  // than collapsed to an empty lede.
  async function regenerateDigest(): Promise<void> {
    if (!engineOn || digestRegenerating) return;
    const token = ++digestRequestToken;
    digestRegenerating = true;
    digestLoading = false;
    digestError = null;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<UserContextDigest | null>(
        "regenerate_user_context_digest",
        { rangeKind: rangeMode, startMs, endMs },
      );
      if (token !== digestRequestToken) return; // range moved on — stale
      digest = next;
      if (!next) {
        // Not an error: the range simply has too little activity to read.
        digestError = "Not enough activity in this range to write a read.";
      }
    } catch (error) {
      if (token === digestRequestToken) digestError = humanizeError(error);
    } finally {
      if (token === digestRequestToken) digestRegenerating = false;
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
      // A new range is new content: stale expansion state would leave rows
      // open over different (possibly empty) content.
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
    } catch (error) {
      // Surface the failure — a silent no-op leaves the user thinking the
      // correction stuck. The event refresh will still reconcile state.
      const detail = humanizeError(error);
      await message(detail, {
        title: "Couldn't update category",
        kind: "error",
      });
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
    } catch (error) {
      // Surface the failure — a silent no-op leaves the user thinking the
      // correction stuck. The event refresh will still reconcile state.
      const detail = humanizeError(error);
      await message(detail, {
        title: "Couldn't update focus",
        kind: "error",
      });
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
    } catch (error) {
      // revert on failure — and explain why the pin snapped back.
      const revert = new Map(pinnedOverride);
      revert.set(c.id, !next);
      pinnedOverride = revert;
      const detail = humanizeError(error);
      await message(detail, {
        title: next ? "Couldn't pin conclusion" : "Couldn't unpin conclusion",
        kind: "error",
      });
    }
  }

  // Dismiss now has an UNDO window: rather than vanishing (and persisting) at
  // once, the row collapses into a "Dismissed · Undo" placeholder and the backend
  // dismiss is DEFERRED. If the user clicks Undo before the window elapses the
  // pending commit is cancelled and the row returns — no backend call was made,
  // so no un-dismiss command is needed. After the window the commit fires and the
  // row leaves the feed for good.
  const DISMISS_UNDO_MS = 5000;
  let pendingDismiss = $state<Map<number, ReturnType<typeof setTimeout>>>(
    new Map(),
  );

  function dismissConclusion(c: Conclusion): void {
    if (pendingDismiss.has(c.id)) return;
    const timer = setTimeout(() => void commitDismiss(c), DISMISS_UNDO_MS);
    const next = new Map(pendingDismiss);
    next.set(c.id, timer);
    pendingDismiss = next;
  }

  function undoDismiss(c: Conclusion): void {
    const timer = pendingDismiss.get(c.id);
    if (timer !== undefined) clearTimeout(timer);
    const next = new Map(pendingDismiss);
    next.delete(c.id);
    pendingDismiss = next;
  }

  async function commitDismiss(c: Conclusion): Promise<void> {
    // Clear the pending state, hide the row (optimistic), then persist.
    const next = new Map(pendingDismiss);
    next.delete(c.id);
    pendingDismiss = next;
    const set = new Set(dismissedIds);
    set.add(c.id);
    dismissedIds = set;
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
    } catch (error) {
      const revert = new Set(dismissedIds);
      revert.delete(c.id);
      dismissedIds = revert;
      const detail = humanizeError(error);
      await message(detail, {
        title: "Couldn't dismiss conclusion",
        kind: "error",
      });
    }
  }

  function enableEngine(): void {
    void openSettings("intelligence");
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
  // FREE tiles show a skeleton whenever a usage fetch is in flight — both the
  // first load (`usage === null`) AND every re-fetch on a range change. Gating
  // only on `!usage` would let the PREVIOUS range's `usage` (and the sparkbar /
  // stats derived from it) linger on screen during a reload; the user wants
  // nothing stale shown mid-load, so we follow the in-flight `loadingFree` flag
  // directly. `loadingFree` starts `true` and is re-set at the top of every
  // `loadFree()` call, so it covers re-loads too.
  const freeLoading = $derived(loadingFree);
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

  // Each exhibit card is a clickable trigger for its detail modal ONLY when its
  // populated branch renders (not while loading or empty/locked) — these mirror
  // the template branch conditions exactly so "openable" ⇔ the chart shows.
  const timeOpenable = $derived(!freeLoading && topApps.length > 0);
  const categoriesOpenable = $derived(
    engineOn && !engineTilesLoading && categorySegments.length > 0,
  );
  const focusOpenable = $derived(
    engineOn && !engineTilesLoading && focusRows.length > 0,
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
      <Segmented
        options={RANGE_OPTIONS}
        value={rangeMode}
        onValueChange={(v) => setMode(v as RangeMode)}
        ariaLabel="Date range"
      />
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
        <!-- Jump home — only when paged into the past (the stepper otherwise has
             no one-click way back to the current period). -->
        {#if !atLatest}
          <button
            class="range-today"
            type="button"
            onclick={resetRange}
            title="Jump to the current period"
          >
            {rangeMode === "day"
              ? "Today"
              : rangeMode === "week"
                ? "This week"
                : "This month"}
          </button>
        {/if}
      </div>
    </div>
  </div>

  <!-- ── THE READ — full-width AI narrative hero ──
       The engine's read of the range, promoted to the top of the page and the
       single home for the range's headline numbers. On the engine path it
       ALWAYS renders, so the stats footer is always present: prose/headline
       and the skeleton only appear when there's a digest or one is loading;
       the eyebrow + stats footer stand alone otherwise. -->
  {#if engineOn}
    <article
      class="entry entry--lede"
      aria-busy={(!digest && digestLoading) || digestRegenerating}
    >
      <p class="eyebrow">
        <span class="diamond" aria-hidden="true">◆</span>
        <span class="tick" aria-hidden="true"></span>
        The read
        <span class="rule"></span>
        {#if digest}<span class="eyebrow-when">{relativeTime(digest.generatedAtMs)}</span>{/if}
        <!-- Re-read: force a fresh narrative for this range, bypassing the
             backend cache. Available whenever the engine is on, so a range
             whose read failed (empty lede) can still be retried. -->
        <button
          type="button"
          class="re-read"
          class:is-busy={digestRegenerating}
          onclick={regenerateDigest}
          disabled={digestRegenerating || (!digest && digestLoading)}
          title="Write a fresh read for this range"
        >
          <span class="re-read-ico" aria-hidden="true">↻</span>
          {digestRegenerating ? "reading…" : "re-read"}
        </button>
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
      {:else if digestLoading || digestRegenerating}
        <div class="sk-row">
          <Skeleton variant="text" width="92%" height="12px" />
        </div>
        <div class="sk-row">
          <Skeleton variant="text" width="64%" height="12px" />
        </div>
      {:else if digestError}
        <p class="lede-error">{digestError}</p>
      {/if}
      <!-- Stats footer — the single source of truth for the range's headline
           numbers. Tracked is always present; deep focus %, top category, the
           daily average, and the per-day sparkbar render only when they have a
           value. The usage-derived stats (tracked / daily avg / sparkbar) gate
           on `freeLoading` and the engine-derived stats (deep focus % / top
           category) on `engineTilesLoading` so a range switch never shows the
           PREVIOUS range's numbers or sparkbar while the new range refetches. -->
      <div class="lede-stats" aria-label="Highlights this {rangeMode}">
        {#if freeLoading}
          <div class="tile-skeleton tile-skeleton--stat" aria-busy="true">
            <div class="sk-stat-row">
              <Skeleton variant="text" width="64px" height="16px" />
              <Skeleton variant="text" width="64px" height="16px" />
            </div>
          </div>
        {:else}
          <div class="lede-stat">
            <span class="lede-stat-n">{summary.totalLabel}</span>
            <span class="lede-stat-cap">tracked</span>
          </div>
          <div class="lede-stat">
            <span class="lede-stat-n">{summary.avgLabel}</span>
            <span class="lede-stat-cap">daily avg</span>
          </div>
        {/if}
        {#if !engineTilesLoading && summary.deepPct !== null}
          <div class="lede-stat">
            <span class="lede-stat-n">{summary.deepPct}%</span>
            <span class="lede-stat-cap">deep focus</span>
          </div>
        {/if}
        {#if !engineTilesLoading && topCategory}
          <div class="lede-stat">
            <span class="lede-stat-n lede-stat-n--cat">
              <span
                class="lede-stat-swatch"
                style="background:var({topCategory.colorVar});"
                aria-hidden="true"
              ></span>
              {topCategory.label}
            </span>
            <span class="lede-stat-cap">top category</span>
          </div>
        {/if}
        {#if !freeLoading && summary.spark.length > 0}
          <div class="lede-stat lede-stat--spark">
            <div class="sparkbar" aria-hidden="true">
              {#each summary.spark as v, i (i)}
                <span
                  style="height:{summary.sparkMax > 0
                    ? Math.max(8, (v / summary.sparkMax) * 100)
                    : 0}%;"
                ></span>
              {/each}
            </div>
            <span class="lede-stat-cap">{summary.sparkLabel}</span>
          </div>
        {/if}
      </div>
    </article>
  {:else}
    <!-- ── Free-tier hero ──
         No AI narrative on free, so the hero slot becomes a deterministic
         factual read of the range + the free headline numbers + an
         enable-engine invite. Reuses the lede shell for visual continuity. -->
    <article class="entry entry--lede" aria-busy={freeLoading}>
      <p class="eyebrow">
        <span class="diamond" aria-hidden="true">◆</span>
        <span class="tick" aria-hidden="true"></span>
        This {rangeMode}
        <span class="rule"></span>
      </p>
      {#if freeLoading}
        <div class="sk-row">
          <Skeleton variant="text" width="86%" height="12px" />
        </div>
        <div class="sk-row">
          <Skeleton variant="text" width="52%" height="12px" />
        </div>
      {:else}
        <div class="lede-body">
          <p class="lede-text">
            {summary.totalLabel} tracked{#if topApps.length > 0} across {topApps.length}
              {topApps.length === 1 ? "app" : "apps"}{/if}.{#if topApps[0]}
              Most of it in {topApps[0].label}.{/if}
          </p>
        </div>
      {/if}
      <!-- Stats footer — the free headline numbers. Tracked + daily avg always;
           the per-day sparkbar when there's any day-level signal. Deep focus %
           and top category are engine-only and intentionally omitted here. -->
      <div class="lede-stats" aria-label="Highlights this {rangeMode}">
        {#if freeLoading}
          <div class="tile-skeleton tile-skeleton--stat" aria-busy="true">
            <div class="sk-stat-row">
              <Skeleton variant="text" width="64px" height="16px" />
              <Skeleton variant="text" width="64px" height="16px" />
            </div>
          </div>
        {:else}
          <div class="lede-stat">
            <span class="lede-stat-n">{summary.totalLabel}</span>
            <span class="lede-stat-cap">tracked</span>
          </div>
          <div class="lede-stat">
            <span class="lede-stat-n">{summary.avgLabel}</span>
            <span class="lede-stat-cap">daily avg</span>
          </div>
          {#if summary.spark.length > 0}
            <div class="lede-stat lede-stat--spark">
              <div class="sparkbar" aria-hidden="true">
                {#each summary.spark as v, i (i)}
                  <span
                    style="height:{summary.sparkMax > 0
                      ? Math.max(8, (v / summary.sparkMax) * 100)
                      : 0}%;"
                  ></span>
                {/each}
              </div>
              <span class="lede-stat-cap">{summary.sparkLabel}</span>
            </div>
          {/if}
        {/if}
      </div>
      <!-- Enable-engine invite — the single CTA for the free tier. -->
      <div class="lede-invite">
        <p class="lede-invite-text">
          Turn on the Engine for the story behind these hours — categories,
          focus, and what changed.
        </p>
        <button type="button" class="btn btn--accent" onclick={enableEngine}>
          Enable engine
        </button>
      </div>
    </article>
  {/if}

  <!-- ── Exhibits — demoted supporting evidence strip ──
       The metric charts (Time / Categories / Focus), demoted below THE READ
       hero into a quieter full-width strip: supporting evidence for the
       narrative, not a co-equal dashboard. The big-number "This {range}" tile
       was removed — its numbers now live in the hero's stats footer. Tiles
       render for both tiers; tier is communicated structurally now, so the
       per-tile Free/Engine badges are dropped for calm. -->
  <section class="exhibits" aria-label="Supporting charts">
    <p class="eyebrow">
      <span class="diamond" aria-hidden="true">◆</span>
      <span class="tick" aria-hidden="true"></span>
      Exhibits
      <span class="rule"></span>
    </p>
    <div class="exhibits-grid">
      <!-- Time — the whole card is a conditional button (role/tabindex are
           paired and only set when openable); the analyzer can't see the
           conditional role, so the tabindex pairing is safe to ignore. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="exhibit"
        class:exhibit--clickable={timeOpenable}
        role={timeOpenable ? "button" : undefined}
        tabindex={timeOpenable ? 0 : undefined}
        aria-haspopup={timeOpenable ? "dialog" : undefined}
        aria-label={timeOpenable ? "View app usage detail" : undefined}
        onclick={timeOpenable ? () => (appsModalOpen = true) : undefined}
        onkeydown={timeOpenable
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                appsModalOpen = true;
              }
            }
          : undefined}
      >
        <div class="exhibit-head">
          <span class="exhibit-title">Time</span>
        </div>
        <div class="exhibit-body">
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
            <span class="exhibit-hint">view all apps →</span>
          {/if}
        </div>
      </div>

      <!-- Categories — conditional button; see Time card note above. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="exhibit"
        class:exhibit--clickable={categoriesOpenable}
        role={categoriesOpenable ? "button" : undefined}
        tabindex={categoriesOpenable ? 0 : undefined}
        aria-haspopup={categoriesOpenable ? "dialog" : undefined}
        aria-label={categoriesOpenable ? "View category breakdown" : undefined}
        onclick={categoriesOpenable ? () => (categoryModalOpen = true) : undefined}
        onkeydown={categoriesOpenable
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                categoryModalOpen = true;
              }
            }
          : undefined}
      >
        <div class="exhibit-head">
          <span class="exhibit-title">Categories</span>
        </div>
        <div class="exhibit-body">
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
          {:else if engineError}
            <!-- A failed engine fetch must NOT masquerade as "no data" — say it
                 couldn't load and offer a compact retry. -->
            <p class="tile-note tile-note--error">
              Couldn't load.
              <button
                type="button"
                class="tile-retry"
                onclick={(e) => {
                  e.stopPropagation();
                  void loadEngine();
                }}
              >
                Retry
              </button>
            </p>
          {:else if categorySegments.length === 0}
            <p class="tile-note">No categorized activity yet.</p>
          {:else}
            <StackedBar segments={categorySegments} showLegend={true} fill={true} />
            <span class="exhibit-hint">view breakdown →</span>
          {/if}
        </div>
      </div>

      <!-- Focus — conditional button; see Time card note above. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="exhibit"
        class:exhibit--clickable={focusOpenable}
        role={focusOpenable ? "button" : undefined}
        tabindex={focusOpenable ? 0 : undefined}
        aria-haspopup={focusOpenable ? "dialog" : undefined}
        aria-label={focusOpenable ? "View focus detail" : undefined}
        onclick={focusOpenable ? () => (focusModalOpen = true) : undefined}
        onkeydown={focusOpenable
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                focusModalOpen = true;
              }
            }
          : undefined}
      >
        <div class="exhibit-head">
          <span class="exhibit-title">Focus</span>
        </div>
        <div class="exhibit-body">
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
          {:else if engineError}
            <!-- A failed engine fetch must NOT masquerade as "no data" — say it
                 couldn't load and offer a compact retry. -->
            <p class="tile-note tile-note--error">
              Couldn't load.
              <button
                type="button"
                class="tile-retry"
                onclick={(e) => {
                  e.stopPropagation();
                  void loadEngine();
                }}
              >
                Retry
              </button>
            </p>
          {:else if focusRows.length === 0}
            <p class="tile-note">No focus signal yet.</p>
          {:else}
            <Heatmap
              rows={focusRows}
              colorMode="focus"
              legend="deep · mixed · scattered"
            />
            <span class="exhibit-hint">view focus detail →</span>
          {/if}
        </div>
      </div>
    </div>
  </section>

  {#if freeError}
    <div class="state state--error">
      <p class="state-title">Couldn't load your usage charts.</p>
      <p class="state-detail">{freeError}</p>
      <button
        type="button"
        class="re-read state-retry"
        onclick={() => void loadFree()}
        disabled={loadingFree}
      >
        <span class="re-read-ico" aria-hidden="true">↻</span>
        Try again
      </button>
    </div>
  {/if}

  {#if feedLoading}
    <!-- ── Loading skeleton for the story/dossier feed ── -->
    <!-- Shown until status resolves (and, when the engine is on, until the
         range's engine data lands) so we never flash the "enable the engine"
         invite or the "still learning" empty state before we actually know. -->
    <div class="feed-column" aria-busy="true" aria-label="Loading your story">
      <!-- Mirrors the real feed: "What changed" rows + "Needs attention".
           The lede skeleton is handled by THE READ hero above. -->
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
    <!-- Free tier: the free hero + Exhibits above cover everything; the story
         feed (What changed / Needs attention) is engine-only, so nothing
         renders here. -->
  {:else}
    <!-- ── Story / dossier feed (ENGINE) ── -->
    {#if engineError}
      <div class="feed-column">
        <div class="state state--error">
          <p class="state-title">Couldn't load your engine data.</p>
          <p class="state-detail">{engineError}</p>
          <button
            type="button"
            class="re-read state-retry"
            onclick={() => void loadEngine()}
            disabled={loadingEngine}
          >
            <span class="re-read-ico" aria-hidden="true">↻</span>
            Try again
          </button>
        </div>
      </div>
    {:else if engineEmpty}
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
        <!-- One row of the "What changed"/"Pinned" lists; shared by both
             sections so a pinned delta renders identically to a changing one. -->
        {#snippet deltaRow(d: ConclusionDelta)}
          {@const c = d.c}
          {@const open = expandedDeltaRows.has(c.id)}
          {#if pendingDismiss.has(c.id)}
            <!-- Undo window: the dismiss hasn't committed yet — collapse to a
                 quiet "Dismissed · Undo" line so the action is reversible and
                 isn't a silent vanish. -->
            <div class="delta-row delta-row--dismissed" role="status">
              <span class="dismissed-note">Dismissed “{c.statement}”</span>
              <button
                type="button"
                class="dismissed-undo"
                onclick={() => undoDismiss(c)}
              >
                Undo
              </button>
            </div>
          {:else}
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
          {/if}
        {/snippet}

        <!-- Pinned — conclusions the user pinned, kept in view and out of the
             change groups so they never scroll off behind the cap. -->
        {#if pinnedDeltas.length > 0}
          <article class="entry">
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              Pinned
              <span class="rule"></span>
              kept in view
            </p>
            <div class="delta-groups">
              <div class="delta-group">
                {#each pinnedDeltas as d (d.c.id)}
                  {@render deltaRow(d)}
                {/each}
              </div>
            </div>
          </article>
        {/if}

        <!-- What changed — conclusion deltas as grouped rows, capped to the
             newest 10 across all groups with one expander for the rest. -->
        {#if visibleDeltaGroups.length > 0}
          <article class="entry">
            <p class="eyebrow">
              <span class="diamond" aria-hidden="true">◆</span>
              <span class="tick" aria-hidden="true"></span>
              What changed
              <span class="rule"></span>
              expand a row for detail
            </p>
            <!-- One-line explainer so the vocabulary ("conclusion", confidence)
                 isn't opaque to a first-time reader. -->
            <p class="section-explainer">
              Conclusions are what the engine has inferred about you — each carries
              a confidence the evidence keeps raising or letting fade.
            </p>
            <div class="delta-groups">
              {#each visibleDeltaGroups as g (g.kind)}
                <div class="delta-group">
                  <p class="delta-group-head">{deltaLabel(g.kind)} · {g.total}</p>
                  {#each g.deltas as d (d.c.id)}
                    {@render deltaRow(d)}
                  {/each}
                </div>
              {/each}
            </div>
            <!-- The group heads sum the FULL group sizes but the list is capped,
                 so when more changed than fits, say so honestly and point to the
                 standing dossier (Subjects) where they all live. -->
            {#if unpinnedDeltas.length > WHAT_CHANGED_CAP}
              <div class="delta-overflow">
                <span class="delta-overflow-count">
                  showing {visibleDeltas.length} of {unpinnedDeltas.length}
                </span>
                <button
                  type="button"
                  class="evidence-link"
                  onclick={() => onOpenTab?.("subjects")}
                >
                  View all in Subjects →
                </button>
              </div>
            {/if}
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
                  <div class="attn-pick">
                    <span class="attn-pick-label">Category</span>
                    <Select
                      options={CATEGORY_SELECT_OPTIONS}
                      value={a.category ?? CATEGORY_NONE}
                      disabled={correctingActivity.has(a.id)}
                      onValueChange={(v) =>
                        void correctCategory(
                          a,
                          (v === CATEGORY_NONE ? null : v) as ActivityCategory | null,
                        )}
                    />
                  </div>
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

  <!-- ── Ask entry bar — last child of the overview, both tiers ── -->
  <button class="ask-entry" type="button" onclick={() => onOpenTab?.("chat")}>
    <span class="glyph" aria-hidden="true">◇</span>
    <span class="label">Ask about your history</span>
    <span class="hint">Opens Chat →</span>
  </button>

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

  <AppDetailModal
    open={appsModalOpen}
    apps={allApps}
    {rangeLabel}
    onClose={() => (appsModalOpen = false)}
  />
  <FocusDetailModal
    open={focusModalOpen}
    activities={rangeActivities}
    {focusRows}
    {rangeMode}
    {rangeLabel}
    onClose={() => (focusModalOpen = false)}
  />
</section>

<style>
  .overview {
    display: flex;
    flex-direction: column;
    gap: 20px;
    width: 100%;
    /* Stepped reading column: fill the surface on narrow widths, then cap at
       discrete breakpoints so the content doesn't sprawl on large/ultrawide
       displays (replaces a fluid `max-width: 60%` that widened without bound). */
    max-width: 720px;
    margin: 0 auto;
  }
  @media (min-width: 1024px) {
    .overview {
      max-width: 860px;
    }
  }
  @media (min-width: 1280px) {
    .overview {
      max-width: 1024px;
    }
  }
  @media (min-width: 1600px) {
    .overview {
      max-width: 1200px;
    }
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
    font-size: var(--text-base);
    color: var(--app-text-muted);
    text-transform: capitalize;
  }
  .ov-controls {
    display: inline-flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
  }

  .date-stepper {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
  }
  .date-stepper .nav {
    width: 24px;
    height: 24px;
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
  .date-stepper .nav:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .date-stepper .nav:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: default;
  }
  .date-stepper .range-label {
    color: var(--app-text);
    letter-spacing: 0.02em;
    font-variant-numeric: tabular-nums;
  }
  /* "Today / This week / This month" reset — quiet pill, only shown in the past. */
  .date-stepper .range-today {
    margin-left: 2px;
    font: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
    padding: 3px 9px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .date-stepper .range-today:hover {
    border-color: var(--app-accent);
  }
  .date-stepper .range-today:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* ---- Bento glance band ---- */
  /* Exhibits — demoted supporting-evidence strip. Derived from the bento
     .glance-tile styles but lighter: subtler surface/border, less padding,
     lower min-height, thinner headers. The hero must clearly dominate this. */
  .exhibits-grid {
    display: grid;
    /* Stack the exhibits on narrow surfaces, pair them two-up once there's room
       — same breakpoint that widens the overview column, so wrapping and width
       step together rather than cards squeezing at small sizes. */
    grid-template-columns: 1fr;
    gap: 10px;
    /* Stretch both cards in a row to the tallest one so Time and Categories
       read as an equal-height pair. The shorter card's slack isn't dead space:
       its `view … →` hint is pushed to the bottom edge (margin-top: auto),
       turning the gap into breathing room. */
    align-items: stretch;
  }
  @media (min-width: 860px) {
    .exhibits-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }
  .exhibits-grid .exhibit:last-child {
    grid-column: 1 / -1;
  }
  /* Pin the top-row pair (Time / Categories) to the loaded Time card's height
     so the row never resizes between the loading skeleton and real content
     (which would make the fill bar visibly re-grow after load). The figure is
     the Time card's own layout: 22 (padding) + 23 (header) + 112 (5 app rows ×
     16 + 4 gaps × 8) + 26 (bottom hint) ≈ 183. */
  @media (min-width: 860px) {
    .exhibits-grid .exhibit:not(:last-child) {
      min-height: 190px;
    }
  }
  .exhibit {
    display: flex;
    flex-direction: column;
    min-height: 120px;
    padding: 11px 12px;
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    min-width: 0;
    overflow: hidden;
    transition: border-color 0.12s ease;
  }
  .exhibit:hover {
    border-color: var(--app-border-hover);
  }
  /* The whole card is the trigger when its detail modal is openable. */
  .exhibit--clickable {
    cursor: pointer;
  }
  .exhibit--clickable:hover {
    border-color: var(--app-border-hover);
  }
  /* Reinforce the "whole card opens detail" affordance: on card hover the hint
     lifts to accent (text + underline) so the cue and hover feedback agree. */
  .exhibit--clickable:hover .exhibit-hint {
    color: var(--app-accent);
    border-bottom-color: var(--app-accent-border);
  }
  .exhibit--clickable:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .exhibit--clickable:focus-visible .exhibit-hint {
    color: var(--app-accent);
    border-bottom-color: var(--app-accent-border);
  }
  .exhibit-head {
    display: flex;
    align-items: center;
    gap: 7px;
    margin-bottom: 9px;
  }
  .exhibit-title {
    font-size: var(--text-xs);
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
  .exhibit-body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    /* Top-align content directly under the header — no floating in the middle
       with dead space above and below. */
    justify-content: flex-start;
  }

  .tile-note {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  .tile-note--locked {
    color: var(--app-text-subtle);
    font-style: italic;
  }
  /* Engine-fetch failure on a glance tile — distinct from the muted "no data"
     note, with an inline retry. */
  .tile-note--error {
    color: var(--app-danger);
    display: inline-flex;
    align-items: baseline;
    gap: 7px;
    flex-wrap: wrap;
  }
  .tile-retry {
    font: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 1px 7px;
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    background: transparent;
    color: var(--app-danger-text);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .tile-retry:hover {
    border-color: var(--app-danger);
    background: var(--app-danger-bg);
  }
  .tile-retry:focus-visible {
    outline: none;
    box-shadow: var(--app-ring-danger);
  }

  /* Non-interactive visual cue at the bottom of each exhibit card hinting that
     the whole card opens a detail modal. */
  .exhibit-hint {
    display: inline-block;
    /* Sit at the bottom edge of the card; a stretched card's slack lands here
       as breathing room rather than a gap under the chart. padding-top keeps a
       minimum gap from the chart when the card is short and the auto margin
       collapses. */
    margin-top: auto;
    padding-top: 12px;
    align-self: flex-start;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    border-bottom: 1px dotted var(--app-border-strong);
    line-height: 1.3;
    transition:
      color 0.12s ease,
      border-bottom-color 0.12s ease;
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
    position: sticky;
    bottom: 0;
    z-index: 2;
    /* Mask the scroll container's 28px bottom padding so scrolling content
       doesn't peek through below the docked bar (the bar pins 28px above the
       scrollport's bottom edge). The downward solid shadow paints page-bg over
       that gap; it's clipped by the scroll container's overflow. */
    box-shadow: 0 28px 0 0 var(--app-bg);
  }
  .ask-entry:hover {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  /* Visible keyboard focus on the primary Overview→Chat handoff. The second
     shadow layer preserves the sticky bottom-padding mask (see box-shadow above). */
  .ask-entry:focus-visible {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow:
      var(--app-ring),
      0 28px 0 0 var(--app-bg);
  }
  .ask-entry .glyph {
    color: var(--app-accent-strong);
    font-size: var(--text-md);
  }
  .ask-entry .label {
    flex: 1 1 auto;
    font-size: var(--text-md);
  }
  .ask-entry .hint {
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .ask-entry:hover .hint,
  .ask-entry:focus-visible .hint {
    color: var(--app-accent-strong);
  }

  /* ---- Story feed ---- */
  .feed-column {
    width: 100%;
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
    font-size: var(--text-xs);
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
  .eyebrow-when {
    flex: 0 0 auto;
  }
  /* Re-read button — sits at the far right of the eyebrow, in the same muted
     terminal register; lifts to the accent on hover. */
  .re-read {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin: 0;
    padding: 2px 7px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: transparent;
    color: var(--app-text-subtle);
    font: inherit;
    font-size: var(--text-xs);
    letter-spacing: 0.18em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .re-read:hover:not(:disabled) {
    color: var(--app-accent);
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .re-read:disabled {
    cursor: default;
    opacity: 0.6;
  }
  /* Retry affordance on a page-level error card — the re-read pill, sized to its
     content and nudged off the detail line. */
  .state-retry {
    align-self: flex-start;
    margin-top: 4px;
  }
  .re-read:not(:disabled):active {
    transform: translateY(1px);
  }
  .re-read-ico {
    font-size: var(--text-base);
    line-height: 1;
    letter-spacing: 0;
  }
  .re-read.is-busy .re-read-ico {
    animation: re-read-spin 0.8s linear infinite;
  }
  @keyframes re-read-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .re-read.is-busy .re-read-ico {
      animation: none;
    }
    .re-read:not(:disabled):active {
      transform: none;
    }
  }
  /* Re-read failure reason — sits where the prose would, in the same scale,
     tinted toward the app's danger register without shouting. */
  .lede-error {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-danger, var(--app-text-subtle));
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
    font-size: var(--text-md);
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
    font-size: var(--text-lg);
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
    font-size: var(--text-xs);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  /* The per-day sparkbar rides the stats footer as a final cell, pushed to the
     right so the figures read first. Reuses the shared .sparkbar primitive. */
  .lede-stat--spark {
    margin-left: auto;
    min-width: 88px;
  }
  .lede-stat--spark .sparkbar {
    width: 100%;
  }
  /* Free-tier enable-engine invite — a quiet muted line + the accent CTA,
     sitting below the stats footer in the same hero shell. */
  .lede-invite {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    margin-top: 16px;
  }
  .lede-invite-text {
    flex: 1 1 240px;
    min-width: 0;
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.6;
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
    font-size: var(--text-xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  /* Quiet one-line explainer for first-time readers, under a section eyebrow. */
  .section-explainer {
    margin: -2px 0 12px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  /* Honest "showing N of M" footer + path to the rest when the capped list hides
     changes. */
  .delta-overflow {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 12px;
    padding-top: 10px;
    border-top: 1px dashed var(--app-border);
  }
  .delta-overflow-count {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
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
  /* Undo-window placeholder: a quiet single line with a dotted Undo link. */
  .delta-row--dismissed {
    flex-direction: row;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .dismissed-note {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    font-style: italic;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .dismissed-undo {
    flex: 0 0 auto;
    font: inherit;
    font-size: var(--text-sm);
    padding: 0 0 1px;
    border: 0;
    border-bottom: 1px dotted var(--app-accent-border);
    background: transparent;
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .dismissed-undo:hover {
    color: var(--app-accent);
  }
  .dismissed-undo:focus-visible {
    outline: none;
    color: var(--app-accent);
    box-shadow: var(--app-ring);
    border-radius: 3px;
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
    font-size: var(--text-base);
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
    font-size: var(--text-xs);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  /* Dedicated expand affordance — the chip stays its own button, so the row
     itself can't be one (a button inside a button is invalid HTML). */
  .delta-toggle {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font: inherit;
    font-size: var(--text-md);
    line-height: 1;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-subtle);
    cursor: pointer;
    transition:
      transform 0.12s ease,
      color 0.12s ease;
  }
  .delta-toggle:hover {
    color: var(--app-text-strong);
  }
  .delta-toggle:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
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
  .conf-wrap {
    flex: 0 0 auto;
  }

  .subject-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: var(--text-xs);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
    color: var(--app-text);
  }
  .ev-stance {
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .evidence-empty {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
  }

  .fade-note {
    font-size: var(--text-xs);
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
    font-size: var(--text-base);
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .attn-time {
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .attn-pick :global(.select-wrapper) {
    width: 160px;
  }
  .attn-more {
    align-self: flex-start;
    margin-top: 9px;
  }

  .feed-end {
    text-align: center;
    padding: 6px 0 0;
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }

  /* ---- Free-tier enable-engine CTA button ---- */
  .btn {
    font: inherit;
    font-size: var(--text-sm);
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
      color 0.12s ease,
      box-shadow 0.12s ease,
      transform 0.06s ease;
  }
  .btn:hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .btn:not(:disabled):active {
    transform: translateY(1px);
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
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.6;
  }
</style>
