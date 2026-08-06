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
  import { humanizeHours } from "$lib/insights/activity-helpers";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { buildRiver, bandRiver } from "$lib/insights/journal-view";
  import { captureControls } from "$lib/capture-controls.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import { journalDate } from "$lib/insights/journal-date.svelte";
  import JournalRiver from "$lib/insights/JournalRiver.svelte";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";

  // ── Day range (always mode "day"; local midnight bounds) ────────────────
  // The date control itself lives in the title bar (page 08: the date control
  // is chrome, not content) — this surface only reads the shared store.
  const range = $derived(journalDate.range);
  const atLatest = $derived(journalDate.atLatest);
  const dayLabel = $derived(journalDate.dayLabel);

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

<section class="journal" aria-label="Journal">
  <!-- ── Header. The date control is NOT here: page 08 puts it in the chrome
       (<JournalTitlebarControls/>), so the content carries only the read. ── -->
  <div class="jhead">
    <h1 class="t-title">Journal</h1>
    <p class="t-meta">Your day, written down while you worked.</p>
  </div>

  <!-- ── The read (digest lede) — one opaque plate, the widest reading measure
       on the page. ── -->
  <article
    class="plate lede"
    aria-busy={(!digest && digestLoading) || digestRegenerating}
  >
    <div class="lede__eb">
      <span class="t-label lede__tag">◆ The read · {dayLabel}</span>
      {#if digest}
        <span class="t-meta is-num lede__when">{relativeTime(digest.generatedAtMs)}</span>
      {/if}
      {#if engineOn}
        <button
          type="button"
          class="btn btn--ghost btn--sm lede__re"
          class:is-busy={digestRegenerating}
          onclick={regenerateDigest}
          disabled={digestRegenerating || (!digest && digestLoading)}
        >
          <span class="lede__re-ico" aria-hidden="true">↻</span>
          {digestRegenerating ? "reading…" : "re-read"}
        </button>
      {/if}
    </div>
    {#if digest}
      {#key digest.generatedAtMs}
        <div class="lede__body">
          {#if digest.headline}
            <h2 class="lede__headline">{digest.headline}</h2>
          {/if}
          <p class="t-read lede__text">{digest.narrative}</p>
        </div>
      {/key}
    {:else if digestLoading || digestRegenerating}
      <div class="sk-row"><Skeleton variant="text" width="92%" height="12px" /></div>
      <div class="sk-row"><Skeleton variant="text" width="64%" height="12px" /></div>
    {:else if digestError}
      <p class="t-read lede__error">{digestError}</p>
    {/if}
  </article>

  <!-- Four stat plates — the only four the backend actually computes. The
       usage-derived tracked stat gates on `usageLoaded`, the engine-derived
       deep %/top category on the range load so a day switch never shows the
       previous day's numbers. -->
  <div class="stats" aria-label="Day highlights">
    {#if usageLoaded}
      <div class="plate stat">
        <span class="stat__v is-num">{trackedLabel}</span>
        <span class="t-label">tracked</span>
      </div>
    {/if}
    {#if riverLoadedOnce && ledeStats.deepPct !== null}
      <div class="plate stat">
        <span class="stat__v is-num">{ledeStats.deepPct}%</span>
        <span class="t-label">deep focus</span>
      </div>
    {/if}
    {#if riverLoadedOnce && ledeStats.topCategory}
      <div class="plate stat">
        <span class="stat__v stat__v--sm">
          <em style="background:var({ledeStats.topCategory.colorVar});" aria-hidden="true"></em>
          {ledeStats.topCategory.label}
        </span>
        <span class="t-label">top category</span>
      </div>
    {/if}
    {#if riverLoadedOnce}
      <div class="plate stat">
        <span class="stat__v is-num">{model.slots.length}</span>
        <span class="t-label">activities</span>
      </div>
    {/if}
  </div>

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
  /* Journal surface (page 08 — the day as a river of plates). Every readable
     thing lands on an opaque `.plate`; the only material on this surface is the
     title bar's, which the date capsule rides. Colours are app tokens
     (`--app-*`, `--cat-*`, `--focus-*`) — the mockup's raw hex is only its
     self-contained copy of the same tokens. */
  .journal {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 1100px;
    margin: 0 auto;
    /* AI-written titles/summaries can carry long unbreakable tokens (URLs,
       paths); without this they blow out the river's 1fr grid track and give
       the whole page an x-scroll. `anywhere` (not `break-word`) so the token
       also stops inflating min-content sizing. */
    overflow-wrap: anywhere;
  }

  /* ---- Header ---- */
  .jhead {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0 0 14px;
  }
  .jhead h1 {
    margin: 0;
  }
  .jhead p {
    margin: 0;
  }

  /* ---- The read ---- */
  .lede {
    padding: 14px 16px 16px;
    margin-bottom: 14px;
  }
  .lede__eb {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .lede__tag {
    color: var(--app-accent);
  }
  .lede__when {
    color: var(--app-text-subtle);
  }
  .lede__re {
    margin-left: auto;
  }
  .lede__re-ico {
    font-size: var(--t-ui);
    line-height: 1;
    display: inline-block;
  }
  .lede__re.is-busy .lede__re-ico {
    animation: re-read-spin 0.8s linear infinite;
  }
  @keyframes re-read-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lede__re.is-busy .lede__re-ico {
      animation: none;
    }
  }
  .lede__body {
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
    .lede__body {
      animation: none;
    }
  }
  .lede__headline {
    margin: 0 0 6px;
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
    max-width: 62ch;
  }
  .lede__text {
    margin: 0;
    color: var(--app-text-muted);
  }
  .lede__error {
    margin: 0;
    color: var(--app-danger, var(--app-text-subtle));
  }
  .sk-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 0;
  }

  /* ---- The four statistics — one plate each ---- */
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 12px;
    margin-bottom: 20px;
  }
  .stat {
    padding: 10px 12px 11px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .stat__v {
    display: flex;
    align-items: center;
    gap: 7px;
    font: var(--w-semi) var(--t-display) / 1.1 var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
    min-width: 0;
  }
  .stat__v--sm {
    font: var(--w-semi) var(--t-title) / 1.2 var(--app-font-sans);
    letter-spacing: var(--ls-title);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stat__v em {
    flex: 0 0 auto;
    width: 11px;
    height: 11px;
    border-radius: 3px;
    font-style: normal;
  }

  /* Narrow window: the four statistics fold to two rows rather than shrink
     into unreadable columns. */
  @media (max-width: 760px) {
    .stats {
      grid-template-columns: repeat(2, 1fr);
    }
  }
</style>
