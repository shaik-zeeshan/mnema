<script lang="ts">
  // DayTimeline — the Today front page (Warm Paper redesign, Slice 3): greeting
  // → digest paragraph (↻ re-read, mono lede stats) → suggestion chips +
  // Ask-Mnema composer → the journal river as ledger prose. Rendering makes
  // ZERO LLM calls — it arranges four already-cheap reads (activities + frames
  // + status + digest) through the pure `buildJournalDay` model (journal-day.ts)
  // and `journal-view.ts` presentation helpers; suggestion chips are mechanical
  // templates (chip-fill.ts). Composer submits route into Chat over the
  // conversation bus (`requestNewChat(prefill)` — the /insights owner switches
  // the view when the bus fires). The river renders in <JournalRiver/> (kept
  // split so both files stay under the 800-line ceiling). Visual spec:
  // docs/mockups/unified-shell/main-surface/story-first-v5.html frame 1.
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
  import { fillChips } from "$lib/insights/chip-fill";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import JournalDateStepper from "$lib/insights/JournalDateStepper.svelte";
  import JournalRiver from "$lib/insights/JournalRiver.svelte";
  import TodayComposer from "$lib/insights/TodayComposer.svelte";
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

  let usage = $state<{ timePerApp: { app: string; activeMs: number }[] } | null>(
    null,
  );
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

  // ── Front-page dressing (greeting, date line, chips, chat hand-off) ─────
  // Suggestion chips only dress the live front page — a past day is an archive
  // read, not a "now" surface.
  const chips = $derived(
    atLatest
      ? fillChips({
          activities: rangeActivities,
          timePerApp: usage?.timePerApp ?? [],
          nowMs: Date.now(),
        })
      : [],
  );

  const frontDate = $derived(
    new Date(range.startMs)
      .toLocaleDateString(undefined, {
        weekday: "long",
        month: "long",
        day: "numeric",
        year: "numeric",
      })
      .toUpperCase()
      .replaceAll(", ", " · "),
  );

  const greeting = $derived.by(() => {
    if (!atLatest) {
      // A past day heads with its name instead of a greeting.
      return `${new Date(range.startMs).toLocaleDateString(undefined, {
        weekday: "long",
        month: "long",
        day: "numeric",
      })}.`;
    }
    const hour = new Date().getHours();
    if (hour < 12) return "Good morning.";
    if (hour < 17) return "Good afternoon.";
    return "Good evening.";
  });

  // Composer submit → route into Chat. `requestNewChat(prefill)` bumps the
  // conversation bus; the /insights owner watches the bus and switches the
  // shell to the Chat surface, where the question lands in the composer
  // (focused, caret at end) ready to send — the bus never auto-sends.
  function askInChat(text: string): void {
    conversationStore.requestNewChat(text);
  }

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
      const next = await invoke<{
        timePerApp: { app: string; activeMs: number }[];
      }>("get_usage_charts", { startMs, endMs });
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

  // The busy flag gets its own sequence: `digestToken` is shared with
  // `loadDigest`, and the `user_context_changed` listener fires loadDigest on
  // every worker beat — routine during a multi-second re-read. A token-gated
  // reset would then never run, leaving the button stuck on "reading…"
  // (re-entry is blocked by the `digestRegenerating` guard). The result writes
  // stay `digestToken`-gated so a newer load still wins the data.
  let regenSeq = 0;
  async function regenerateDigest(): Promise<void> {
    if (!engineOn || digestRegenerating) return;
    const token = ++digestToken;
    const regen = ++regenSeq;
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
      if (regen === regenSeq) digestRegenerating = false;
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

<section class="journal" aria-label="Today">
  <!-- ── Front-page header: date line + stepper, greeting, digest, stats ── -->
  <header class="front">
    <div class="front-top">
      <div class="front-date">{frontDate}</div>
      <JournalDateStepper
        bind:anchorMs
        rangeStartMs={range.startMs}
        {atLatest}
        {dayLabel}
      />
    </div>
    <h1 class="front-greet">{greeting}</h1>

    <!-- Digest paragraph — the day narrated in 3–5 sentences, ↻ re-read inline
         at the end (mockup frame 1). Hidden while the engine is off: the
         river's pending slot carries the why. -->
    {#if engineOn}
      <div
        class="front-digest"
        aria-busy={(!digest && digestLoading) || digestRegenerating}
      >
        {#if digest}
          {#key digest.generatedAtMs}
            <p class="digest-text">
              {digest.narrative}<button
                type="button"
                class="reread"
                class:is-busy={digestRegenerating}
                title="Re-read this day's digest"
                aria-label="Re-read this day's digest"
                onclick={regenerateDigest}
                disabled={digestRegenerating}
              >↻</button><span class="digest-when">
                {digestRegenerating
                  ? "re-reading…"
                  : `read ${relativeTime(digest.generatedAtMs)}`}</span>
            </p>
          {/key}
        {:else if digestLoading || digestRegenerating}
          <div class="sk-row"><Skeleton variant="text" width="92%" height="12px" /></div>
          <div class="sk-row"><Skeleton variant="text" width="64%" height="12px" /></div>
        {:else}
          <p class="digest-text digest-text--empty">
            {digestError ?? "No read of this day yet."}<button
              type="button"
              class="reread"
              title="Read this day"
              aria-label="Read this day"
              onclick={regenerateDigest}
            >↻</button>
          </p>
        {/if}
      </div>
    {/if}

    <!-- Four lede stats in a quiet mono row — tracked / deep focus / top
         category / activities. The usage-derived tracked stat gates on
         `usageLoaded`, the engine-derived ones on the range load so a day
         switch never shows the previous day's numbers. -->
    <div class="front-stats" aria-label="Day highlights">
      {#if usageLoaded}
        <span><span class="g" aria-hidden="true">▣</span><b>{trackedLabel}</b> tracked</span>
      {/if}
      {#if riverLoadedOnce && ledeStats.deepPct !== null}
        <span><span class="g" aria-hidden="true">●</span><b>{ledeStats.deepPct}%</b> deep focus</span>
      {/if}
      {#if riverLoadedOnce && ledeStats.topCategory}
        <span>
          <span
            class="g g--swatch"
            style="background:var({ledeStats.topCategory.colorVar});"
            aria-hidden="true"
          ></span><b>{ledeStats.topCategory.label}</b> top category
        </span>
      {/if}
      {#if riverLoadedOnce}
        <span><span class="g" aria-hidden="true">✦</span><b>{model.slots.length}</b> activities</span>
      {/if}
    </div>

    <!-- Composer + suggestion chips — today only; a past day is an archive. -->
    {#if atLatest}
      <TodayComposer {chips} onSubmit={askInChat} />
    {/if}
  </header>

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
    /* AI-written titles/summaries can carry long unbreakable tokens (URLs,
       paths); without this they blow out the river's 1fr grid track and give
       the whole page an x-scroll. `anywhere` (not `break-word`) so the token
       also stops inflating min-content sizing. */
    overflow-wrap: anywhere;
  }
  @media (min-width: 1024px) {
    .journal {
      max-width: 860px;
    }
  }

  /* ---- Front-page header (mockup frame 1 `.front`, app tokens) ---- */
  .front {
    display: flex;
    flex-direction: column;
  }
  .front-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .front-date {
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    color: var(--app-text-faint);
  }
  .front-greet {
    margin: 12px 0 0;
    font-family: var(--app-font-narrative);
    font-size: 30px;
    line-height: 1.15;
    font-weight: 400;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }

  /* ---- Digest paragraph (↻ inline at the end) ---- */
  .front-digest {
    margin-top: 10px;
    max-width: 700px;
  }
  .digest-text {
    margin: 0;
    font-family: var(--app-font-narrative);
    font-size: 15px;
    line-height: 1.68;
    color: var(--app-text);
    animation: digest-reveal 0.25s ease;
  }
  .digest-text--empty {
    color: var(--app-text-muted);
  }
  @keyframes digest-reveal {
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
    .digest-text {
      animation: none;
    }
  }
  .reread {
    border: 0;
    background: none;
    cursor: pointer;
    padding: 0 2px;
    margin-left: 5px;
    font-size: var(--text-md);
    line-height: 1;
    color: var(--app-text-faint);
    vertical-align: 0;
  }
  .reread:hover:not(:disabled) {
    color: var(--app-accent);
  }
  .reread:focus-visible {
    outline: none;
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }
  .reread:disabled {
    cursor: default;
    opacity: var(--app-busy-opacity);
  }
  .reread.is-busy {
    display: inline-block;
    animation: reread-spin 0.8s linear infinite;
  }
  @keyframes reread-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .reread.is-busy {
      animation: none;
    }
  }
  .digest-when {
    margin-left: 7px;
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    color: var(--app-text-faint);
    white-space: nowrap;
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

  /* ---- Lede stats — one quiet mono row ---- */
  .front-stats {
    display: flex;
    align-items: center;
    gap: 8px 18px;
    flex-wrap: wrap;
    margin-top: 12px;
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    letter-spacing: 0.03em;
    color: var(--app-text-faint);
  }
  .front-stats b {
    color: var(--app-text-muted);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .front-stats .g {
    color: var(--app-accent);
    margin-right: 5px;
  }
  .front-stats .g--swatch {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    vertical-align: 0;
  }
</style>
