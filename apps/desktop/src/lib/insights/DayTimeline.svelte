<script lang="ts">
  // DayTimeline — the Journal Insights sub-surface (Dayflow Slice 3). One local
  // day rendered as a river of AI-written activity cards on a time spine: a
  // digest lede, category edge-bar cards, focus chips, away-gaps, and a pending
  // slot at the live edge. Rendering makes ZERO LLM calls — it arranges four
  // already-cheap reads (activities + frames + status + digest) through the pure
  // `buildJournalDay` model (journal-day.ts) and `journal-view.ts` presentation
  // helpers. The river itself renders in <JournalRiver/> (kept split so both
  // files stay under the 800-line ceiling). Visual spec:
  // docs/mockups/dayflow/01-day-journal.html.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type {
    Activity,
    AiRuntimeStatus,
    UserContextDigest,
    UserContextStatus,
  } from "$lib/types/recording";
  import type { FrameSummaryDto } from "$lib/types/app-infra";
  import {
    humanizeHours,
    startOfDay,
    windowFor,
  } from "$lib/insights/activity-helpers";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { buildRiver, bandRiver } from "$lib/insights/journal-view";
  import { captureControls } from "$lib/capture-controls.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import JournalDateStepper from "$lib/insights/JournalDateStepper.svelte";
  import JournalRiver from "$lib/insights/JournalRiver.svelte";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";

  // ── Day range (always mode "day"; local midnight bounds) ────────────────
  let anchorMs = $state<number>(Date.now());
  const range = $derived(windowFor(anchorMs, "day"));
  // Disable stepping past the current day (mirrors Overview's `atLatest`).
  const atLatest = $derived(Date.now() < range.endMs);

  const dayLabel = $derived(
    new Date(range.startMs).toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    }),
  );

  // No Cards⇄Blocks toggle yet: a one-option Segmented is noise. The toggle
  // ships alongside the Blocks view (mockup 02).

  // ── Engine status ───────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let statusLoaded = $state(false);
  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) ||
      Boolean(ctxStatus?.engineAvailable),
  );

  // ── Loaded data ─────────────────────────────────────────────────────────
  let activities = $state<Activity[]>([]);
  let frames = $state<FrameSummaryDto[]>([]);
  let riverLoadedOnce = $state(false);
  let riverLoading = $state(false);

  let usage = $state<{ timePerApp: { activeMs: number }[] } | null>(null);
  let usageLoaded = $state(false);

  // Digest lede — same state machine as Overview.
  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);

  // Receipt drill-in (Slice 4 owns its own "Open in Timeline" navigation).
  let selectedActivity = $state<Activity | null>(null);

  // ── Derived model ───────────────────────────────────────────────────────
  // Activities scoped to the day exactly as Overview scopes its range (overlap,
  // start-inclusive) so the lede stats derive from the same set.
  const rangeActivities = $derived(
    activities.filter(
      (a) => a.startedAtMs < range.endMs && a.endedAtMs >= range.startMs,
    ),
  );

  const model = $derived(
    buildJournalDay({
      activities,
      frames,
      coveredUntilMs: ctxStatus?.coveredUntilMs ?? null,
      recording: captureControls.isRunning,
      engineAvailable: Boolean(ctxStatus?.engineAvailable),
      engineReason: ctxStatus?.reason ?? null,
      dayStartMs: range.startMs,
      dayEndMs: range.endMs,
    }),
  );

  const bands = $derived(bandRiver(buildRiver(model.slots, model.gaps)));
  const hasCards = $derived(model.slots.length > 0);

  const ledeStats = $derived(
    computeLedeStats({
      timePerApp: usage?.timePerApp ?? [],
      rangeActivities,
      rangeStartMs: range.startMs,
      rangeEndMs: range.endMs,
      engineOn,
    }),
  );
  const trackedLabel = $derived(humanizeHours(ledeStats.trackedMs));

  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    const diff = Date.now() - ms;
    if (diff < 60000) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 60) return `${min} min ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    return `${Math.floor(hr / 24)}d ago`;
  }

  // ── Empty-state gating (loading vs. genuinely empty) ────────────────────
  const showSkeleton = $derived(!riverLoadedOnce);
  const showNothingCaptured = $derived(
    riverLoadedOnce && !hasCards && !model.hasAnyCapture,
  );
  const showBeingWritten = $derived(
    riverLoadedOnce && !hasCards && model.hasAnyCapture,
  );

  // ── Loaders (gen-token guarded, mirrors Overview) ───────────────────────
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
    riverLoading = true;
    try {
      const { startMs, endMs } = range;
      const [nextActivities, nextFrames] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
          request: {
            capturedAtStart: new Date(startMs).toISOString(),
            capturedAtEnd: new Date(endMs).toISOString(),
          },
        }),
      ]);
      if (token !== rangeToken) return; // range moved on — stale
      activities = nextActivities;
      frames = nextFrames;
    } catch {
      // Best-effort: a failed read leaves the previous river; the pending slot /
      // empty panel still communicates state. (Activities/frames are read-only.)
    } finally {
      if (token === rangeToken) {
        riverLoading = false;
        riverLoadedOnce = true;
      }
    }
  }

  let usageToken = 0;
  async function loadUsage(): Promise<void> {
    const token = ++usageToken;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<{ timePerApp: { activeMs: number }[] }>(
        "get_usage_charts",
        { startMs, endMs },
      );
      if (token !== usageToken) return;
      usage = next;
    } catch {
      if (token === usageToken) usage = null;
    } finally {
      if (token === usageToken) usageLoaded = true;
    }
  }

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
      const { startMs, endMs } = range;
      const next = await invoke<UserContextDigest | null>(
        "get_user_context_digest",
        { rangeKind: "day", startMs, endMs },
      );
      if (token !== digestToken) return;
      digest = next;
    } catch {
      if (token === digestToken) digest = null;
    } finally {
      if (token === digestToken) digestLoading = false;
    }
  }

  async function regenerateDigest(): Promise<void> {
    if (!engineOn || digestRegenerating) return;
    const token = ++digestToken;
    digestRegenerating = true;
    digestLoading = false;
    digestError = null;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<UserContextDigest | null>(
        "regenerate_user_context_digest",
        { rangeKind: "day", startMs, endMs },
      );
      if (token !== digestToken) return;
      digest = next;
      if (!next) digestError = "Not enough activity in this day to write a read.";
    } catch (error) {
      if (token === digestToken)
        digestError =
          error instanceof Error ? error.message : "Couldn't write a read.";
    } finally {
      if (token === digestToken) digestRegenerating = false;
    }
  }

  // A day step can cost a paid model call (a fresh range misses the digest
  // cache), and a user may flick through days — so debounce the digest fetch on
  // range change. Mount / event refresh call `loadDigest()` directly.
  const DIGEST_DEBOUNCE_MS = 500;
  let digestDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleDigestLoad(): void {
    digestToken += 1; // invalidate any in-flight/queued load for the old day
    digest = null;
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
    await Promise.all([loadRange(), loadUsage(), loadDigest()]);
  }

  // Re-query on a day change. Skip the mount run (the mount effect owns the
  // first load) so the loaders don't double-fire.
  let rangePrimed = false;
  $effect(() => {
    range.startMs;
    range.endMs;
    void untrack(() => {
      if (!rangePrimed) {
        rangePrimed = true;
        return;
      }
      riverLoadedOnce = false;
      usageLoaded = false;
      void loadRange();
      void loadUsage();
      scheduleDigestLoad();
    });
    return () => {
      // A pending digest debounce dies with the day change (or the component).
      if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
      digestDebounceTimer = null;
    };
  });

  // ── Mount: first load + live refresh on new cards ───────────────────────
  $effect(() => {
    void untrack(() => reloadAll());
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    // A new card landing (or the watermark advancing) refreshes the river in
    // place — no `riverLoadedOnce` reset, so it updates without blanking to a
    // skeleton (that would flicker the whole day on every worker beat).
    void listen("user_context_changed", () => {
      void loadStatus();
      void loadRange();
      void loadUsage();
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
</script>

<section class="journal" aria-label="Journal">
  <!-- ── Header ── -->
  <div class="ov-header">
    <div class="titles">
      <h1>Journal</h1>
      <p class="subtitle">Your day, written down while you worked.</p>
    </div>
    <div class="ov-controls">
      <JournalDateStepper
        bind:anchorMs
        rangeStartMs={range.startMs}
        {atLatest}
        {dayLabel}
      />
    </div>
  </div>

  <!-- ── Digest lede (reused from Overview: headline + narrative + re-read) ── -->
  <article
    class="entry entry--lede"
    aria-busy={(!digest && digestLoading) || digestRegenerating}
  >
    <p class="eyebrow">
      <span class="diamond" aria-hidden="true">◆</span>
      <span class="tick" aria-hidden="true"></span>
      The read · {dayLabel}
      <span class="rule"></span>
      {#if digest}<span class="eyebrow-when">{relativeTime(digest.generatedAtMs)}</span>{/if}
      {#if engineOn}
        <button
          type="button"
          class="re-read"
          class:is-busy={digestRegenerating}
          onclick={regenerateDigest}
          disabled={digestRegenerating || (!digest && digestLoading)}
        >
          <span class="re-read-ico" aria-hidden="true">↻</span>
          {digestRegenerating ? "reading…" : "re-read"}
        </button>
      {/if}
    </p>
    {#if digest}
      {#key digest.generatedAtMs}
        <div class="lede-body">
          {#if digest.headline}
            <h2 class="lede-headline">{digest.headline}</h2>
          {/if}
          <p class="lede-text">{digest.narrative}</p>
        </div>
      {/key}
    {:else if digestLoading || digestRegenerating}
      <div class="sk-row"><Skeleton variant="text" width="92%" height="12px" /></div>
      <div class="sk-row"><Skeleton variant="text" width="64%" height="12px" /></div>
    {:else if digestError}
      <p class="lede-error">{digestError}</p>
    {/if}
    <!-- Four stats — tracked / deep focus % / top category / activities. The
         usage-derived tracked stat gates on `usageLoaded`, the engine-derived
         deep %/top category on the range load so a day switch never shows the
         previous day's numbers. -->
    <div class="lede-stats" aria-label="Day highlights">
      {#if usageLoaded}
        <div class="lede-stat">
          <span class="lede-stat-n">{trackedLabel}</span>
          <span class="lede-stat-cap">tracked</span>
        </div>
      {/if}
      {#if riverLoadedOnce && ledeStats.deepPct !== null}
        <div class="lede-stat">
          <span class="lede-stat-n">{ledeStats.deepPct}%</span>
          <span class="lede-stat-cap">deep focus</span>
        </div>
      {/if}
      {#if riverLoadedOnce && ledeStats.topCategory}
        <div class="lede-stat">
          <span class="lede-stat-n lede-stat-n--cat">
            <span
              class="lede-stat-swatch"
              style="background:var({ledeStats.topCategory.colorVar});"
              aria-hidden="true"
            ></span>
            {ledeStats.topCategory.label}
          </span>
          <span class="lede-stat-cap">top category</span>
        </div>
      {/if}
      {#if riverLoadedOnce}
        <div class="lede-stat">
          <span class="lede-stat-n">{model.slots.length}</span>
          <span class="lede-stat-cap">activities</span>
        </div>
      {/if}
    </div>
  </article>

  <!-- ── The river (skeleton / cards+pending / empty panels) ── -->
  <JournalRiver
    {bands}
    pending={model.pending}
    {showSkeleton}
    {hasCards}
    {showNothingCaptured}
    {showBeingWritten}
    {dayLabel}
    isToday={atLatest}
    onOpenActivity={(a) => (selectedActivity = a)}
  />
</section>

{#if selectedActivity}
  <ActivityReceipt activity={selectedActivity} onClose={() => (selectedActivity = null)} />
{/if}

<style>
  /* Journal surface. Mirrors Overview's reading column + lede tokens; the river
     styles live in <JournalRiver/>. All colours are app tokens (`--app-*`); the
     mockup's raw hex is only its self-contained copy of the same tokens. */
  .journal {
    display: flex;
    flex-direction: column;
    gap: 20px;
    width: 100%;
    max-width: 720px;
    margin: 0 auto;
  }
  @media (min-width: 1024px) {
    .journal {
      max-width: 860px;
    }
  }

  /* ---- Header (shared shape with Overview .ov-header) ---- */
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
    font-size: var(--text-xl);
    line-height: 1.2;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .ov-header .subtitle {
    margin: 3px 0 0;
    font-size: var(--text-base);
    color: var(--app-text-muted);
  }
  .ov-controls {
    display: inline-flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
  }

  /* ---- Digest lede (copied token-for-token from Overview) ---- */
  .entry {
    position: relative;
    padding: 20px 22px 18px;
    border: 1px solid var(--app-border);
    border-radius: 12px;
    background: var(--app-surface);
  }
  .entry--lede {
    padding: 24px 26px 22px;
    border-left: 2px solid var(--app-accent);
    background: linear-gradient(
      to right,
      var(--app-accent-bg),
      var(--app-surface) 42%
    );
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
  }
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
  .lede-error {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-danger, var(--app-text-subtle));
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
</style>
