<script lang="ts">
  // DayTimeline — the Journal view (insights.html #journal). One local day: a
  // digest lede with stats, the day spine (DaySpine), and "What you did"
  // newest-first with the side column (JournalRiver). Rendering makes ZERO LLM
  // calls beyond the cached digest — it arranges cheap reads (activities +
  // frames + audio + status + usage) through `buildJournalDay` (journal-day.ts)
  // and the journal-view.ts helpers. A day with capture but nothing written up
  // (engine not deriving) takes the capture-run shape: grey runs, no cards.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { goto } from "$app/navigation";
  import type { Activity, AiRuntimeStatus, UserContextDigest, UserContextStatus } from "$lib/types/recording";
  import type { AudioSegmentDto, FrameSummaryDto } from "$lib/types/app-infra";
  import type { Span } from "$lib/insights/journal-day";
  import type { JournalEmpty } from "$lib/insights/journal-view";
  import { retentionVerdict } from "$lib/insights/retention";
  import { captureSession } from "$lib/session.svelte";
  import { categoryLabel, humanizeMs, windowFor } from "$lib/insights/activity-helpers";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { buildRiver, capturedHoursByDay, categoryMix, hhmm, weekDays } from "$lib/insights/journal-view";
  import { densityFromHeatmap, toIso } from "$lib/components/date-input";
  import { captureControls, resumeCapture, startCapture } from "$lib/capture-controls.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { renderIdle } from "$lib/render-idle.svelte";
  import DateInput from "$lib/components/DateInput.svelte";
  import DaySpine from "$lib/insights/DaySpine.svelte";
  import JournalRiver from "$lib/insights/JournalRiver.svelte";
  import { inBar } from "$lib/insights/in-bar";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInbox from "~icons/lucide/inbox";
  import IconSparkle from "~icons/lucide/sparkle";

  interface UsageCharts {
    timePerApp: { app: string; activeMs: number }[];
    activityHeatmap: { bucketStartMs: number; intensityCount: number }[];
  }
  const DAY_MS = 86_400_000;

  // ── Day (local midnight bounds), stepped by the bar's DateInput ─────────
  let dayIso = $state(toIso(new Date()));
  const range = $derived(windowFor(new Date(`${dayIso}T12:00`).getTime(), "day"));
  const atLatest = $derived(Date.now() < range.endMs);
  const dayLabel = $derived(
    new Date(range.startMs).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" }),
  );
  function stepDay(k: number): void {
    const d = new Date(`${dayIso}T12:00`);
    d.setDate(d.getDate() + k);
    if (toIso(d) <= toIso(new Date())) dayIso = toIso(d);
  }

  // ── Engine status ───────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let statusLoaded = $state(false);
  const engineOn = $derived(Boolean(aiStatus?.enabled && aiStatus?.available) || Boolean(ctxStatus?.engineAvailable));
  /** Nothing new is derived (AI off, derivation off, not set up, unreachable). */
  const noDerive = $derived(statusLoaded && !ctxStatus?.engineAvailable);
  const pitch = $derived(statusLoaded && aiStatus?.hasProviders === false);

  // ── Loaded data ─────────────────────────────────────────────────────────
  let activities = $state<Activity[]>([]);
  let weekActivities = $state<Activity[]>([]);
  let frames = $state<FrameSummaryDto[]>([]);
  let audio = $state<Span[]>([]);
  let failedWindows = $state<Span[]>([]);
  let riverLoadedOnce = $state(false);
  let loadError = $state(false);
  let nowMs = $state(Date.now());
  let usage = $state<UsageCharts | null>(null);
  let usageLoaded = $state(false);
  let history = $state<UsageCharts | null>(null); // ~120 days: DateInput density + capture-only week bars
  let runApps = $state<Record<number, string[]>>({});
  let hot = $state<number | null>(null);

  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);

  // ── Derived model ───────────────────────────────────────────────────────
  const rangeActivities = $derived(
    activities.filter((a) => a.startedAtMs < range.endMs && a.endedAtMs >= range.startMs),
  );
  const settings = $derived(captureControls.recordingSettings);
  const model = $derived(
    buildJournalDay({
      activities,
      frames,
      audio,
      failedWindows,
      retentionPolicy: settings?.retentionPolicy ?? null,
      nowMs,
      coveredUntilMs: ctxStatus?.coveredUntilMs ?? null,
      recording: captureControls.isRunning,
      engineAvailable: Boolean(ctxStatus?.engineAvailable),
      engineReason: ctxStatus?.reason ?? null,
      dayStartMs: range.startMs,
      dayEndMs: range.endMs,
    }),
  );
  const rows = $derived(buildRiver(model.slots, model.gaps, model.failed));
  const hasCards = $derived(model.slots.length > 0);
  const dayActs = $derived(model.slots.map((s) => s.activity));
  const mix = $derived(categoryMix(dayActs));

  // ── Truthful day states (Direction A frames 6a–6c, 7a–7d) ───────────────
  const userPaused = $derived(captureControls.isRunning && captureControls.isUserPaused);
  const recordingLive = $derived(atLatest && captureControls.isRunning && !userPaused);
  const recordingSince = $derived.by<number | null>(() => {
    const sessions = captureSession.value?.sourceSessions;
    const starts = [sessions?.screen, sessions?.microphone, sessions?.systemAudio]
      .map((s) => s?.startedAtUnixMs)
      .filter((ms): ms is number => typeof ms === "number" && ms > 0);
    return starts.length ? Math.max(Math.min(...starts), range.startMs) : null;
  });
  // Recording today with nothing summarized yet: the first-stretch slot shows
  // even before the first frame lands (JR-04/05).
  const pending = $derived(
    model.pending.active || !recordingLive || hasCards
      ? model.pending
      : {
          active: true,
          sinceMs: recordingSince,
          audioOnly: false,
          reason: ctxStatus?.engineAvailable
            ? { kind: "summarizing" as const }
            : { kind: "engine_unavailable" as const, reason: ctxStatus?.reason ?? "" },
        },
  );
  // Capture-run shape: captured, nothing written up, and nothing being derived.
  const captureRun = $derived(!hasCards && model.hasAnyCapture && noDerive);
  const nothingWrittenWhy = $derived(
    !captureRun
      ? null
      : ctxStatus?.reason === "user_context_disabled"
        ? "Continuous derivation is off, so nothing was written up."
        : aiStatus && !aiStatus.enabled
          ? "AI features were off, so nothing was written up."
          : "AI wasn't available, so nothing was written up.",
  );
  const backfillDays = $derived(
    settings?.userContext?.backfillGoDeeper ? null : (settings?.userContext?.backfillWindowDays ?? null),
  );
  const olderThanBackfill = $derived(backfillDays != null && range.endMs <= Date.now() - backfillDays * DAY_MS);
  const retention = $derived(retentionVerdict(range.startMs, range.endMs, settings?.retentionPolicy));
  const empty = $derived.by<JournalEmpty | null>(() => {
    if (rows.length > 0 || pending.active || captureRun) return null;
    if (!model.hasAnyCapture) {
      if (atLatest && (!captureControls.isRunning || userPaused)) return { kind: "idle", paused: userPaused };
      if (retention?.kind === "removed") return { kind: "retention", days: retention.days };
      return { kind: "nothing" };
    }
    if (olderThanBackfill && backfillDays != null)
      return { kind: "older", days: backfillDays, footageKept: retention?.kind !== "removed" };
    if (ctxStatus?.backfilling) return { kind: "queued" };
    return { kind: "unsummarized" };
  });

  const ledeStats = $derived(
    computeLedeStats({
      timePerApp: usage?.timePerApp ?? [],
      rangeActivities,
      rangeStartMs: range.startMs,
      rangeEndMs: range.endMs,
      // Cards written before the engine went off keep their focus figure.
      engineOn: engineOn || model.slots.length > 0,
    }),
  );
  const tracked = $derived(humanizeMs(ledeStats.trackedMs));
  const focused = $derived(rangeActivities.filter((a) => a.focus));
  const topPct = $derived.by(() => {
    const total = mix.reduce((t, m) => t + m.ms, 0);
    const top = mix.find((m) => (m.category ? categoryLabel(m.category) : "Uncategorized") === ledeStats.topCategory?.label);
    return total && top ? Math.round((top.ms / total) * 100) : null;
  });
  const n = $derived(model.slots.length);
  const headline = $derived((digest?.headline ?? "").split(" — "));
  const week = $derived(
    weekDays(range.startMs, weekActivities, capturedHoursByDay(history?.activityHeatmap ?? []), Date.now()),
  );
  const density = $derived(history ? densityFromHeatmap(history.activityHeatmap) : undefined);
  const spinePending = $derived(
    pending.active && pending.sinceMs !== null && pending.reason
      ? { sinceMs: pending.sinceMs, paused: pending.reason.kind !== "summarizing" }
      : null,
  );

  // ── Loaders (gen-token guarded) ─────────────────────────────────────────
  async function loadStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    aiStatus = ai;
    ctxStatus = ctx;
    statusLoaded = true;
  }

  let rangeToken = 0;
  async function loadRange(): Promise<void> {
    const token = ++rangeToken;
    const { startMs, endMs } = range;
    const weekStartMs = weekDays(startMs, [], {}, 0)[0].startMs;
    try {
      const request = { capturedAtStart: new Date(startMs).toISOString(), capturedAtEnd: new Date(endMs).toISOString() };
      const [nextActivities, nextFrames, nextAudio, nextFailed, nextWeek] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", { request }),
        invoke<AudioSegmentDto[]>("list_audio_segments", { request }),
        // Best-effort extras: a hole marker or a week bar is never a reason to fail the day.
        invoke<[number, number][]>("list_failed_derivation_windows", { startMs, endMs }).catch(() => [] as [number, number][]),
        invoke<Activity[]>("list_user_context_activities", { startMs: weekStartMs, endMs: weekStartMs + 7 * DAY_MS }).catch(
          () => [] as Activity[],
        ),
      ]);
      if (token !== rangeToken) return; // day moved on — stale
      activities = nextActivities;
      frames = nextFrames;
      audio = nextAudio.map((a) => ({ startMs: Date.parse(a.startedAt), endMs: Date.parse(a.endedAt) }));
      failedWindows = nextFailed.map(([s, e]) => ({ startMs: s, endMs: e }));
      weekActivities = nextWeek;
      nowMs = Date.now();
      loadError = false;
    } catch {
      // Say so — never "Nothing captured" for a read that failed (IMPLEMENTATION §4 bugs).
      if (token === rangeToken) loadError = true;
    } finally {
      if (token === rangeToken) riverLoadedOnce = true;
    }
  }

  let usageToken = 0;
  async function loadUsage(): Promise<void> {
    const token = ++usageToken;
    try {
      const next = await invoke<UsageCharts>("get_usage_charts", { startMs: range.startMs, endMs: range.endMs });
      if (token === usageToken) usage = next;
    } catch {
      if (token === usageToken) usage = null;
    } finally {
      if (token === usageToken) usageLoaded = true;
    }
  }

  async function loadHistory(): Promise<void> {
    const end = Date.now();
    history = await invoke<UsageCharts>("get_usage_charts", { startMs: end - 120 * DAY_MS, endMs: end }).catch(() => null);
  }

  // Capture-run labels: each run's top apps. ponytail: one usage read per run;
  // batch into one command if a fragmented day ever makes this slow.
  let runsToken = 0;
  $effect(() => {
    const runs = captureRun ? model.runs : [];
    const token = ++runsToken;
    if (!runs.length) return;
    void Promise.all(
      runs.map((r) =>
        invoke<UsageCharts>("get_usage_charts", { startMs: r.startMs, endMs: Math.max(r.endMs, r.startMs + 1) })
          .then((u) => [r.startMs, [...u.timePerApp].sort((a, b) => b.activeMs - a.activeMs).slice(0, 3).map((x) => x.app)] as const)
          .catch(() => [r.startMs, [] as string[]] as const),
      ),
    ).then((pairs) => {
      if (token === runsToken) runApps = Object.fromEntries(pairs);
    });
  });

  let digestToken = 0;
  async function loadDigest(): Promise<void> {
    if (!statusLoaded || !engineOn) {
      digest = null;
      digestLoading = false;
      return;
    }
    const token = ++digestToken;
    digestLoading = true;
    digestError = null;
    try {
      const next = await invoke<{ digest?: UserContextDigest }>("get_user_context_digest", {
        rangeKind: "day",
        startMs: range.startMs,
        endMs: range.endMs,
      });
      if (token === digestToken) digest = next.digest ?? null;
    } catch (error) {
      if (token === digestToken) digestError = String(error);
    } finally {
      if (token === digestToken) digestLoading = false;
    }
  }

  // The busy flag gets its own sequence: `user_context_changed` fires
  // loadDigest on every worker beat, so a token-gated reset would leave the
  // button stuck on "Re-reading…". Result writes stay `digestToken`-gated.
  let regenSeq = 0;
  async function regenerateDigest(): Promise<void> {
    if (!engineOn || digestRegenerating) return;
    const token = ++digestToken;
    const regen = ++regenSeq;
    digestRegenerating = true;
    digestLoading = false;
    digestError = null;
    try {
      const next = await invoke<{ digest?: UserContextDigest }>("regenerate_user_context_digest", {
        rangeKind: "day",
        startMs: range.startMs,
        endMs: range.endMs,
      });
      if (token === digestToken) digest = next.digest ?? null;
    } catch (error) {
      if (token === digestToken) digestError = String(error);
    } finally {
      if (regen === regenSeq) digestRegenerating = false;
    }
  }

  // A day step can cost a paid model call (a fresh range misses the digest
  // cache), so debounce the digest fetch on day change.
  let digestDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleDigestLoad(): void {
    digestToken += 1;
    digest = null;
    digestRegenerating = false;
    digestError = null;
    if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
    digestDebounceTimer = null;
    if (!statusLoaded || !engineOn) {
      digestLoading = false;
      return;
    }
    digestLoading = true;
    digestDebounceTimer = setTimeout(() => {
      digestDebounceTimer = null;
      void loadDigest();
    }, 500);
  }

  function retry(): void {
    riverLoadedOnce = false;
    void loadStatus().then(() => Promise.all([loadRange(), loadUsage(), loadDigest()]));
  }

  // Re-query on a day change (the mount effect owns the first load).
  let rangePrimed = false;
  $effect(() => {
    range.startMs;
    void untrack(() => {
      if (!rangePrimed) return void (rangePrimed = true);
      riverLoadedOnce = false;
      usageLoaded = false;
      hot = null;
      void loadRange();
      void loadUsage();
      scheduleDigestLoad();
    });
    return () => {
      if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
      digestDebounceTimer = null;
    };
  });

  // Recording start/stop, and a 60 s beat while recording today: new capture
  // doesn't emit `user_context_changed` (JR-05).
  let recPrimed = false;
  $effect(() => {
    captureControls.isRunning;
    captureControls.isUserPaused;
    untrack(() => {
      if (recPrimed) void loadStatus().then(loadRange);
      recPrimed = true;
    });
  });
  $effect(() => {
    if (!recordingLive) return;
    const id = setInterval(() => {
      if (!renderIdle()) void loadStatus().then(() => Promise.all([loadRange(), loadUsage()]));
    }, 60_000);
    return () => clearInterval(id);
  });

  // `[` / `]` step the day (insights.html "This week · [ ] to step").
  $effect(() => {
    function onKey(e: KeyboardEvent): void {
      if ((e.target as HTMLElement | null)?.closest?.("input, textarea, select, [contenteditable]")) return;
      if (e.key === "[") stepDay(-1);
      else if (e.key === "]") stepDay(1);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  $effect(() => {
    void untrack(() => loadStatus().then(() => Promise.all([loadRange(), loadUsage(), loadDigest(), loadHistory()])));
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    // A new card (or the watermark advancing) refreshes in place — no skeleton.
    void listen("user_context_changed", () => {
      void loadStatus();
      void loadRange();
      void loadUsage();
      void loadDigest();
    }).then((fn) => (disposed ? fn() : (unlisten = fn)));
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function pickActivity(id: number): void {
    document.getElementById(`in-act-${id}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }
</script>

{#snippet stat(label: string, value: string, delta: string)}
  <div class="mx-stat">
    <span class="mx-stat__label">{label}</span>
    <span class="mx-stat__value">
      {#each value.match(/\d+|[^\d\s]+/g) ?? [] as part, i (i)}{#if /\d/.test(part) || part === "—"}{i ? " " : ""}{part}{:else}<small>{part}</small>{/if}{/each}
    </span>
    {#if delta}<span class="mx-stat__delta">{delta}</span>{/if}
  </div>
{/snippet}

{#snippet centerEmpty(title: string, text: string, tone: string | null)}
  <div class="mx-empty mx-empty--center in-center" data-tone={tone ?? undefined}>
    <span class="mx-empty__glyph">
      {#if tone === "danger"}<IconAlert width="18" height="18" />{:else if pitch && !hasCards}<IconSparkle width="18" height="18" />{:else}<IconInbox width="18" height="18" />{/if}
    </span>
    <h4 class="mx-empty__title">{title}</h4>
    <p class="mx-empty__text">{text}</p>
    <div class="mx-empty__acts">
      {#if tone === "danger"}
        <button type="button" class="mx-btn mx-btn--sm" onclick={retry}><IconRetry width="13" height="13" aria-hidden="true" />Try again</button>
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => void goto("/")}>Open in Timeline</button>
      {:else if pitch && !hasCards}
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => void openSettings("intelligence")}>Set up AI</button>
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => void goto("/")}>Open Timeline</button>
      {:else if empty?.kind === "idle"}
        {#if empty.paused}
          <button type="button" class="mx-btn mx-btn--sm" disabled={captureControls.loadingPause} onclick={() => void resumeCapture()}>Resume recording</button>
        {:else}
          <button type="button" class="mx-btn mx-btn--sm" disabled={captureControls.loadingStart} onclick={() => void startCapture()}>Start recording</button>
        {/if}
      {:else if !atLatest}
        <button type="button" class="mx-btn mx-btn--sm" onclick={() => (dayIso = toIso(new Date()))}>Go to today</button>
      {/if}
    </div>
  </div>
{/snippet}

<div class="in-jctl" use:inBar>
  <DateInput mode="day" value={dayIso} align="end" {density} onchange={(r) => (dayIso = r.start)} />
  <button type="button" class="mx-btn" disabled={atLatest} onclick={() => (dayIso = toIso(new Date()))}>Today</button>
</div>

<section class="in-jwrap" aria-label="Journal">
  {#if !riverLoadedOnce}
    <div aria-busy="true">
      <span class="mx-sr">Loading the day</span>
      <div class="in-read">
        <div class="in-skel-read"><div class="mx-skel" style="width:140px;height:9px"></div><div class="mx-skel mx-skel--title"></div><div class="mx-skel mx-skel--title"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div></div>
        <div class="in-skel-stats">{#each [0, 1, 2, 3] as i (i)}<div><div class="mx-skel"></div><div class="mx-skel"></div></div>{/each}</div>
      </div>
      <div class="mx-panel in-skel-spine"><div class="mx-skel" style="width:120px;height:9px"></div><div class="mx-skel" style="height:64px;margin-top:var(--s-5)"></div></div>
      <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
      <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
    </div>
  {:else if loadError}
    {@render centerEmpty(`Couldn't load ${dayLabel}`, "Reading this day's activities failed. Nothing is lost — the footage is still on the Timeline.", "danger")}
  {:else if pitch && !hasCards}
    {@render centerEmpty("Your journal is written by AI", "Once a provider is connected, Mnema writes each half hour up as an activity — with a digest of the day on top. Capture is already on the Timeline.", null)}
  {:else if empty}
    {#if empty.kind === "idle"}
      {@render centerEmpty(empty.paused ? "Recording is paused" : "Mnema isn't recording", `${empty.paused ? "Resume" : "Start recording"} and today's journal fills in as you work. The first card usually lands within about 10 minutes.`, null)}
    {:else if empty.kind === "retention"}
      {@render centerEmpty(`Nothing left from ${dayLabel}`, `Removed by your ${empty.days}-day retention.`, null)}
    {:else if empty.kind === "older"}
      {@render centerEmpty("This day wasn't summarized", `Older than the ${empty.days} days Mnema summarizes on its own.${empty.footageKept ? " The footage is still on the Timeline." : ""}`, null)}
    {:else if empty.kind === "queued"}
      {@render centerEmpty("Not summarized yet", "Mnema is catching up on older history, newest first — past days fill in as it goes. The footage is already on the Timeline.", null)}
    {:else if empty.kind === "unsummarized"}
      {@render centerEmpty(`No cards for ${dayLabel}`, "Nothing in this day's capture was summarized into a card. The footage is on the Timeline.", null)}
    {:else}
      {@render centerEmpty(`Nothing captured on ${dayLabel}`, "There's no capture on this day, so there's no journal to show. Days with any recording at all show whatever was captured.", null)}
    {/if}
  {:else}
    <div class="in-read">
      <div class="mx-reveal" style="--i:1">
        {#if captureRun}
          <span class="mx-kicker">{dayLabel}</span>
          <h1 class="mx-display">{tracked} tracked. <em>Nothing written up.</em></h1>
          <p class="mx-prose">{nothingWrittenWhy} Everything captured is on the Timeline.</p>
        {:else}
          <span class="mx-kicker">{noDerive && !digest ? dayLabel : `${atLatest ? "Digest so far" : "Digest"} · ${dayLabel}`}</span>
          {#if n >= 2 && engineOn && !digest && (digestLoading || digestRegenerating)}
            <div aria-busy="true">
              <span class="mx-sr">Writing the digest</span>
              <div class="in-skel-read"><div class="mx-skel mx-skel--title"></div><div class="mx-skel mx-skel--title"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div></div>
            </div>
            <div class="in-read__foot"><span class="mx-inline"><span class="mx-spin mx-spin--sm"></span>Writing the digest…</span></div>
          {:else if n < 2}
            <h1 class="mx-display">{n ? `Too little captured on ${dayLabel} to write a digest.` : "Your day is being written."}</h1>
            <p class="mx-prose">{n ? "A digest needs at least two activities; this day has one. It's below, as captured." : "Capture is landing. The digest appears once two activities are written up."}</p>
          {:else if digest}
            <h1 class="mx-display">{headline[0] || `${n} activities, ${tracked} tracked.`}{#if headline[1]}<em> — {headline.slice(1).join(" — ")}</em>{/if}</h1>
            <p class="mx-prose">{digest.narrative}</p>
            <div class="in-read__foot">
              <span class="mx-label num">written {hhmm(digest.generatedAtMs)}{atLatest && ctxStatus?.coveredUntilMs ? ` · covers through ${hhmm(ctxStatus.coveredUntilMs)}` : ""}</span>
              {#if engineOn}
                <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" aria-busy={digestRegenerating} disabled={digestRegenerating} onclick={regenerateDigest}>
                  {#if digestRegenerating}<span class="mx-spin mx-spin--sm"></span>Re-reading…{:else}<IconRetry width="13" height="13" aria-hidden="true" />Re-read{/if}
                </button>
              {/if}
            </div>
          {:else}
            <!-- ponytail: one copy for every "no digest" cause until IN-B1's reason lands. -->
            <h1 class="mx-display">{n} activities, {tracked} tracked.</h1>
            <p class="mx-prose">
              {noDerive
                ? `No digest while ${ctxStatus?.reason === "user_context_disabled" ? "continuous derivation" : "AI"} is off — the cards below were written before.`
                : digestError
                  ? "Couldn't write the digest this time. The cards below are unaffected."
                  : "No digest for this day yet."}
            </p>
            {#if digestError && engineOn}
              <div class="in-read__foot">
                <button type="button" class="mx-btn mx-btn--sm" onclick={regenerateDigest}><IconRetry width="13" height="13" aria-hidden="true" />Try again</button>
              </div>
            {/if}
          {/if}
        {/if}
      </div>
      <div class="in-lede-stats mx-reveal" style="--i:2">
        {#if model.audioOnly}
          {@render stat("Tracked · app time", "—", "app time · screen off")}
        {:else if usageLoaded}
          {@render stat("Tracked · app time", tracked, atLatest ? `through ${hhmm(nowMs)}` : "app active time")}
        {/if}
        {#if captureRun}
          {@render stat("Activities", "—", "none · needs AI")}
        {:else}
          {@render stat("Activities", String(n), n ? `${model.gaps.length} away-gap${model.gaps.length === 1 ? "" : "s"}` : "none written up yet")}
        {/if}
        {#if n && ledeStats.deepPct !== null}
          {@render stat("Deep focus · by activity", `${ledeStats.deepPct}%`, `${focused.filter((a) => a.focus === "deep").length} of ${focused.length} activities`)}
        {/if}
        {#if n && ledeStats.topCategory}
          <div class="mx-stat">
            <span class="mx-stat__label">Top category</span>
            <span class="mx-stat__value in-tval"><span class="in-sw" style="--c: var({ledeStats.topCategory.colorVar})"></span>{ledeStats.topCategory.label}</span>
            {#if topPct !== null}<span class="mx-stat__delta">{topPct}% of activity time</span>{/if}
          </div>
        {/if}
      </div>
    </div>

    <div class="mx-reveal" style="--i:3">
      <DaySpine
        slots={model.slots}
        gaps={model.gaps}
        runs={captureRun ? model.runs.map((span) => ({ span, apps: runApps[span.startMs] ?? [] })) : null}
        pending={captureRun ? null : spinePending}
        nowMs={atLatest ? nowMs : null}
        {hot}
        onHot={(id) => (hot = id)}
        onPick={pickActivity}
      />
    </div>

    <JournalRiver
      {rows}
      {pending}
      failure={engineOn ? (ctxStatus?.summarizingFailure ?? null) : null}
      {dayLabel}
      {nothingWrittenWhy}
      {week}
      dayStartMs={range.startMs}
      {mix}
      apps={[...(usage?.timePerApp ?? [])].sort((a, b) => b.activeMs - a.activeMs).slice(0, 5)}
      {hot}
      onHot={(id) => (hot = id)}
      onPickDay={(ms) => (dayIso = toIso(new Date(ms)))}
    />
  {/if}
</section>

<style>
  /* insights.html Journal lede (`.in-read`) + page glue; kit tokens only. AI-written
     titles can carry long unbreakable tokens (URLs, paths) — `anywhere` stops them
     inflating the grid's min-content. */
  .in-jwrap { overflow-wrap: anywhere; animation: in-fade var(--t-med) var(--ease-quart); }
  .in-jctl { display: flex; align-items: center; gap: var(--s-3); }
  .in-read { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s-7); align-items: end; padding: var(--s-4) 0 var(--s-6); margin-bottom: var(--s-6); border-bottom: 1px solid var(--mx-hairline); }
  .in-read .mx-display { margin: var(--s-4) 0; max-width: 24ch; }
  .in-read .mx-display em { font-style: normal; font-weight: 500; color: var(--app-text-subtle); }
  .in-read__foot { display: flex; align-items: center; gap: var(--s-3); margin-top: var(--s-5); }
  .in-lede-stats { display: grid; grid-template-columns: repeat(2, minmax(132px, auto)); gap: var(--s-5) var(--s-6); padding-bottom: 4px; }
  .in-tval { font: 600 22px/1 var(--font-sans); letter-spacing: -0.02em; display: flex; align-items: center; gap: 9px; }
  .in-sw { display: inline-block; width: 10px; height: 10px; border-radius: 2px; background: var(--c); flex: none; }
  .in-skel-read { display: grid; gap: 12px; padding-top: var(--s-4); }
  .in-skel-read .mx-skel--title { height: 34px; width: 78%; }
  .in-skel-read .mx-skel--title + .mx-skel--title { width: 52%; }
  .in-skel-stats { display: grid; grid-template-columns: repeat(2, 132px); gap: var(--s-5) var(--s-6); }
  .in-skel-stats div { display: grid; gap: 8px; }
  .in-skel-stats .mx-skel:first-child { height: 8px; width: 60%; }
  .in-skel-stats .mx-skel:last-child { height: 24px; width: 90%; }
  .in-skel-spine { padding: var(--s-5); margin-bottom: var(--s-7); }
  .in-center { margin-top: var(--s-6); }
  @keyframes in-fade { from { opacity: 0; } }
</style>
