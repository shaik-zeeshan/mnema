<script lang="ts">
  // ══ JOURNAL — an addressable DESTINATION inside Overview ═══════════════════
  //
  // `/overview/journal`, not a fourth top-level surface: the window still has
  // exactly Timeline and Overview, the switcher keeps Overview lit while you
  // are in here, and the titlebar grows the back control (`+layout.svelte`).
  // ⌘2 and Escape walk back one step.
  //
  // Direction 05 "Tactile Instruments". The river is prose and rows — Journal
  // adds NO instrument. The one instrument-grade face here is the 24-hour
  // coverage strip, Overview's readout (`.ti-cov` in a `.ti-well`) reused at day
  // scale and read-only: "which hours of this day hold frames" is a physical
  // fact the surface already loads every timestamp for.
  //
  // Round-4 **G8** binds every number on the page. The read's four counts, the
  // strip, the gap sentence and each card's frame count are all real reads with
  // real denominators; a fact that is null renders NO number and NO sentence.
  // None of the mockup's invented figures (4,568 frames, 6.7h, 62 %) survives.
  //
  // Everything below the header is REUSED from the Insights Journal, which
  // already solved this day: `buildJournalDay` buckets one local day of
  // activities + frames + the worker watermark, `journal-view.ts` merges and
  // bands the river, `lede-stats.ts` computes the counts the same way Overview
  // does, and `<JournalRiver/>` / `<ActivityReceipt/>` render the river and the
  // receipt (both reskinned in place for this direction). This file is the
  // destination's shell: loading, the read, and the coverage face.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import IconRefresh from "~icons/lucide/refresh-cw";
  import IconSparkles from "~icons/lucide/sparkles";
  import type { Activity, UserContextDigest, UserContextStatus } from "$lib/types/recording";
  import type { AiRuntimeStatus } from "$lib/types/recording";
  import type { DayCoverage, FrameSummaryDto } from "$lib/types/app-infra";
  import { humanizeHours, windowFor } from "$lib/insights/activity-helpers";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { bandRiver, buildRiver } from "$lib/insights/journal-view";
  import JournalDateStepper from "$lib/insights/JournalDateStepper.svelte";
  import JournalRiver from "$lib/insights/JournalRiver.svelte";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { clockLabel, dayKey, hourCells } from "$lib/overview/day-math";
  import { coverageReadout, relativeTime } from "$lib/overview/journal/day-read";

  // ── The day (always local midnight bounds; the stepper writes the anchor) ──
  let anchorMs = $state<number>(Date.now());
  const range = $derived(windowFor(anchorMs, "day"));
  const atLatest = $derived(Date.now() < range.endMs);
  const dayLabel = $derived(
    new Date(range.startMs).toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    }),
  );

  // ── Engine status ─────────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let statusLoaded = $state(false);
  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) || Boolean(ctxStatus?.engineAvailable),
  );

  // ── Loaded data ───────────────────────────────────────────────────────────
  let activities = $state<Activity[]>([]);
  let frames = $state<FrameSummaryDto[]>([]);
  let coverage = $state<DayCoverage[]>([]);
  let riverLoadedOnce = $state(false);
  let usage = $state<{ timePerApp: { activeMs: number }[] } | null>(null);
  let usageLoaded = $state(false);

  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);

  let selectedActivity = $state<Activity | null>(null);
  // Only read to stamp "2 min ago"; a wall clock would re-render the page every
  // second for a figure that rounds to the minute.
  let now = $state(Date.now());

  // ── Derived model ─────────────────────────────────────────────────────────
  const rangeActivities = $derived(
    activities.filter((a) => a.startedAtMs < range.endMs && a.endedAtMs >= range.startMs),
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

  // ── The coverage face (the day's own `list_day_coverage` row) ─────────────
  const dayCoverage = $derived(coverage.find((d) => d.day === dayKey(new Date(range.startMs))));
  const cells = $derived(hourCells(dayCoverage?.hours));
  const litHours = $derived(cells.reduce((n, lit) => n + (lit ? 1 : 0), 0));
  const covReadout = $derived(coverageReadout(litHours, model.gaps, clockLabel));

  const stamp = $derived(digest ? relativeTime(digest.generatedAtMs, now) : "");

  // ── Empty-state gating (loading vs. genuinely empty) ──────────────────────
  const showSkeleton = $derived(!riverLoadedOnce);
  const showNothingCaptured = $derived(riverLoadedOnce && !hasCards && !model.hasAnyCapture);
  const showBeingWritten = $derived(riverLoadedOnce && !hasCards && model.hasAnyCapture);

  // ── Loaders (gen-token guarded, lifted from the Insights Journal) ─────────
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
    try {
      const { startMs, endMs } = range;
      const [nextActivities, nextFrames, nextCoverage] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
          request: {
            capturedAtStart: new Date(startMs).toISOString(),
            capturedAtEnd: new Date(endMs).toISOString(),
          },
        }),
        invoke<DayCoverage[]>("list_day_coverage").catch(() => []),
      ]);
      if (token !== rangeToken) return; // the day moved on — stale
      activities = nextActivities;
      frames = nextFrames;
      coverage = nextCoverage ?? [];
    } catch {
      // Best-effort: a failed read leaves the previous river standing; every
      // read here is read-only, so there is nothing to roll back.
    } finally {
      if (token === rangeToken) riverLoadedOnce = true;
    }
  }

  let usageToken = 0;
  async function loadUsage(): Promise<void> {
    const token = ++usageToken;
    try {
      const { startMs, endMs } = range;
      const next = await invoke<{ timePerApp: { activeMs: number }[] }>("get_usage_charts", {
        startMs,
        endMs,
      });
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
      const next = await invoke<UserContextDigest | null>("get_user_context_digest", {
        rangeKind: "day",
        startMs,
        endMs,
      });
      if (token !== digestToken) return;
      digest = next;
      now = Date.now();
    } catch {
      if (token === digestToken) digest = null;
    } finally {
      if (token === digestToken) digestLoading = false;
    }
  }

  // The busy flag gets its own sequence: `digestToken` is shared with
  // `loadDigest`, which the `user_context_changed` listener fires on every
  // worker beat — a token-gated reset would then never run and the button would
  // stay stuck on "reading…". Result writes stay token-gated so a newer load
  // still wins the data.
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
      const next = await invoke<UserContextDigest | null>("regenerate_user_context_digest", {
        rangeKind: "day",
        startMs,
        endMs,
      });
      if (token !== digestToken) return;
      digest = next;
      now = Date.now();
      if (!next) digestError = "Not enough of this day to write a read.";
    } catch (error) {
      if (token === digestToken)
        digestError = error instanceof Error ? error.message : "Couldn't write a read.";
    } finally {
      if (regen === regenSeq) digestRegenerating = false;
    }
  }

  // A day step can cost a paid model call (a fresh range misses the digest
  // cache) and a user may flick through days, so the digest fetch is debounced
  // on a range change. Mount / event refresh call `loadDigest()` directly.
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
    digestLoading = true; // the placeholder spans the debounce window too
    digestDebounceTimer = setTimeout(() => {
      digestDebounceTimer = null;
      void loadDigest();
    }, DIGEST_DEBOUNCE_MS);
  }

  async function reloadAll(): Promise<void> {
    await loadStatus();
    await Promise.all([loadRange(), loadUsage(), loadDigest()]);
  }

  // Re-query on a day change; the mount effect owns the first load.
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
      if (digestDebounceTimer != null) clearTimeout(digestDebounceTimer);
      digestDebounceTimer = null;
    };
  });

  // Mount: first load + live refresh as new cards land. No `riverLoadedOnce`
  // reset on the event path — blanking the whole day to a skeleton on every
  // worker beat is a flicker, not a load.
  $effect(() => {
    void untrack(() => reloadAll());
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
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

<div class="dest">
  <header class="dest__bar">
    <span class="t-title">Journal</span>
    <span class="t-meta">your day, written down while you worked</span>
    <span class="dest__sp"></span>
    <JournalDateStepper bind:anchorMs rangeStartMs={range.startMs} {atLatest} {dayLabel} />
  </header>

  <div class="ti-pane dest__pane">
    <div class="jcol">
      <!-- ══ THE READ — the day's digest, and four counts that are all real ══
           G8: every stat renders only once its own read has landed AND has an
           answer. Nothing here has a zero face. -->
      <section
        class="read"
        aria-label="The read"
        aria-busy={(!digest && digestLoading) || digestRegenerating}
      >
        <div class="read__l">
          <div class="read__eye">
            <IconSparkles class="read__spark" />
            <span class="t-label">The read · {dayLabel}</span>
            <span class="read__rule"></span>
            {#if stamp}<span class="t-meta is-mono is-num">{stamp}</span>{/if}
            {#if engineOn}
              <button
                type="button"
                class="btn btn--sm read__reread"
                class:is-busy={digestRegenerating}
                onclick={regenerateDigest}
                disabled={digestRegenerating || (!digest && digestLoading)}
              >
                <IconRefresh />
                {digestRegenerating ? "reading…" : "re-read"}
              </button>
            {/if}
          </div>

          {#if digest}
            {#key digest.generatedAtMs}
              <div class="read__body">
                {#if digest.headline}<h2 class="read__hd">{digest.headline}</h2>{/if}
                <p class="t-read read__prose">{digest.narrative}</p>
              </div>
            {/key}
          {:else if digestLoading || digestRegenerating}
            <div class="read__sk"><i style="width:88%"></i><i style="width:96%"></i><i style="width:54%"></i></div>
          {:else if digestError}
            <p class="t-read read__err">{digestError}</p>
          {:else if !engineOn}
            <p class="t-read read__empty">
              No read of this day. Mnema writes one once the Reasoning Engine is
              connected — the river below is the day itself either way.
            </p>
          {:else}
            <p class="t-read read__empty">
              No read of this day yet. Mnema writes one once the Reasoning Engine
              has enough of a day to read.
            </p>
          {/if}
        </div>

        <div class="stats" aria-label="Day highlights">
          {#if usageLoaded && ledeStats.trackedMs > 0}
            <div class="stat">
              <div class="stat__v is-num">{humanizeHours(ledeStats.trackedMs)}</div>
              <div class="t-meta">tracked</div>
            </div>
          {/if}
          {#if riverLoadedOnce && ledeStats.deepPct !== null}
            <div class="stat">
              <div class="stat__v is-num">{ledeStats.deepPct}%</div>
              <div class="t-meta">deep focus</div>
            </div>
          {/if}
          {#if riverLoadedOnce && ledeStats.topCategory}
            <div class="stat">
              <div class="stat__v stat__v--cat">
                <i style="background:var({ledeStats.topCategory.colorVar});"></i>
                {ledeStats.topCategory.label}
              </div>
              <div class="t-meta">top category</div>
            </div>
          {/if}
          {#if riverLoadedOnce && model.slots.length > 0}
            <div class="stat">
              <div class="stat__v is-num">{model.slots.length}</div>
              <div class="t-meta">activities</div>
            </div>
          {/if}
        </div>
      </section>

      <!-- ══ THE COVERAGE FACE — Overview's instrument, at day scale ═════════
           Read-only, and NOT a new instrument: on Journal as on Overview an
           instrument reads; you turn it in Settings. -->
      <div class="dayband">
        <span class="t-label">Coverage</span>
        <div class="ti-well dayband__well">
          <div class="ti-cov" aria-label="Capture by hour of this day">
            {#each cells as lit, hour (hour)}
              <i class:h={lit}></i>
            {/each}
          </div>
          <div class="ti-cov__scale" aria-hidden="true">
            <span>00</span><span>08</span><span>16</span><span>24</span>
          </div>
        </div>
        <span class="t-meta dayband__out">
          {covReadout ?? (riverLoadedOnce ? "No frames captured on this day." : "")}
        </span>
      </div>

      <!-- ══ THE RIVER ══════════════════════════════════════════════════════ -->
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
    </div>
  </div>
</div>

{#if selectedActivity}
  <ActivityReceipt activity={selectedActivity} onClose={() => (selectedActivity = null)} />
{/if}

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* A surface step, never a container border: the hairline is an inset shadow
     on the bar itself, so the window keeps its one border. */
  .dest__bar {
    flex: 0 0 auto;
    height: 40px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--s-16);
    box-shadow: inset 0 -1px 0 var(--app-border);
  }
  .dest__sp {
    flex: 1 1 auto;
  }
  /* No top padding on the scrollport: the river's band rules are sticky at
     `top: 0`, which is the scrollport's PADDING box — a padding-top would leave
     a strip above the pinned rule for cards to scroll through. The inset moves
     onto the column instead. */
  .dest__pane {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--s-20) var(--s-32);
  }
  /* One reading column, centred. AI-written titles carry unbreakable tokens
     (paths, URLs); without this they blow out the river's 1fr track and give
     the whole destination an x-scroll. */
  .jcol {
    width: 100%;
    max-width: 760px;
    margin: 0 auto;
    overflow-wrap: anywhere;
  }

  /* ── the read ───────────────────────────────────────────────────────────
     Prose on the left, the counts on the right. No card, no border: the read
     is the page's opening paragraph, not a panel. */
  .read {
    display: flex;
    gap: var(--s-20);
    align-items: flex-start;
    padding-top: var(--s-16);
  }
  .read__l {
    flex: 1 1 auto;
    min-width: 0;
  }
  .read__eye {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-bottom: var(--s-6);
  }
  .read__eye :global(.read__spark) {
    width: 11px;
    height: 11px;
    flex: 0 0 auto;
    color: var(--app-accent);
  }
  .read__rule {
    flex: 1 1 auto;
    height: var(--hairline);
    background: var(--app-border);
  }
  .read__reread {
    height: var(--h-sm);
    flex: 0 0 auto;
    border: 0;
    background: transparent;
    color: var(--app-text-muted);
    font-size: var(--t-meta);
  }
  .read__reread:hover:not([disabled]) {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }
  .read__reread :global(svg) {
    width: 11px;
    height: 11px;
  }
  .read__reread.is-busy :global(svg) {
    animation: read-spin 0.9s linear infinite;
  }
  @keyframes read-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .read__hd {
    margin: 0 0 var(--s-4);
    font: var(--w-semi) var(--t-title) / var(--lh-title) var(--app-font-sans);
    letter-spacing: var(--ls-title);
    color: var(--app-text-strong);
  }
  .read__prose,
  .read__err,
  .read__empty {
    margin: 0;
    max-width: 62ch;
  }
  .read__err {
    color: var(--app-danger);
  }
  .read__empty {
    color: var(--app-text-subtle);
  }
  .read__body {
    animation: read-reveal var(--dur-regular) var(--ease-out);
  }
  @keyframes read-reveal {
    from {
      opacity: 0;
      transform: translateY(3px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .read__body,
    .read__reread.is-busy :global(svg) {
      animation: none;
    }
  }
  .read__sk {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding-top: var(--s-4);
  }
  .read__sk i {
    display: block;
    height: 10px;
    border-radius: var(--r-sm);
    background: var(--ti-empty);
  }

  /* The four counts. Tabular, quiet captions, no gauge: focus is a three-value
     label the model assigns, not a quantity you can spend — the page that
     wanted a seventh instrument was this one, and it stays plain numbers. */
  .stats {
    flex: 0 0 auto;
    display: grid;
    grid-template-columns: repeat(2, 98px);
    gap: var(--s-12);
  }
  .stat__v {
    display: flex;
    align-items: center;
    gap: 5px;
    font: var(--w-semi) var(--t-display) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
    margin-bottom: 3px;
  }
  .stat__v--cat {
    font: var(--w-medium) var(--t-ui) / 1.35 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    min-width: 0;
  }
  .stat__v--cat i {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  /* ── the coverage face ───────────────────────────────────────────────── */
  .dayband {
    display: flex;
    align-items: center;
    gap: var(--s-12);
    margin: var(--s-16) 0 var(--s-4);
  }
  .dayband__well {
    flex: 1 1 auto;
    padding: 5px 6px;
  }
  .dayband__out {
    flex: 0 0 auto;
    width: 176px;
  }

  /* ── the 800×600 floor: the counts drop under the prose, the readout wraps
        below the strip. Nothing is removed — Journal is a reading surface. ── */
  @media (max-width: 900px) {
    .read {
      flex-direction: column;
      gap: var(--s-12);
    }
    .stats {
      grid-template-columns: repeat(4, minmax(0, 1fr));
      width: 100%;
    }
    .dayband {
      flex-wrap: wrap;
    }
    .dayband__well {
      flex: 1 1 100%;
      order: 3;
    }
    .dayband__out {
      flex: 1 1 auto;
      width: auto;
      order: 2;
      text-align: right;
    }
  }
</style>
