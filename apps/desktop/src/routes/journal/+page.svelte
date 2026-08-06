<script lang="ts">
  // Journal — the day as a river, with a receipt behind every card (mockup 08).
  //
  // Journal is a DESTINATION, not a third surface: the window still has exactly
  // two (Timeline · Overview). It opens from Overview's Today tile and the tool
  // strip's first control is the way back. Inside, the studio shape is unchanged
  // — the tool strip navigates the day, the river is the one scrolling region,
  // the inspector carries the selected activity's record, and the status strip
  // (owned by the root layout) stays welded to the bottom edge.
  //
  // The data machinery is the Insights Journal's, reused unchanged:
  // `buildJournalDay` (the day model), `journal-view` (banding, card states,
  // paused copy), `lede-stats` (the four figures), and `ActivityReceipt` (the
  // frames/scrub/filmstrip/transcript behind one card).
  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { Activity } from "$lib/types/recording";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { humanizeHours } from "$lib/insights/activity-helpers";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { bandRiver, buildRiver, type BandLabel } from "$lib/insights/journal-view";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import { partitionEvidence, type TurnView } from "$lib/insights/receipt-audio";
  import { ReceiptAudioLoader } from "$lib/insights/receipt-audio-loader";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";
  import { dayKeyOf, formatDayShort, shiftDayKey } from "$lib/overview/overview-format";
  import { setPendingTimelineFocus } from "$lib/timeline/pending-focus";
  import { JournalData } from "$lib/journal/journal-data.svelte";
  import { buildActivityRecord, frameMarks, subjectsForActivity } from "$lib/journal/journal-record";
  import JournalToolStrip from "$lib/journal/JournalToolStrip.svelte";
  import JournalLede from "$lib/journal/JournalLede.svelte";
  import JournalRiver from "$lib/journal/JournalRiver.svelte";
  import JournalInspector from "$lib/journal/JournalInspector.svelte";

  const data = new JournalData();

  // ── The shell: inspector collapses below 1000px, measured not assumed ──
  let paneWidth = $state(1100);
  const wide = $derived(paneWidth >= 1000);
  let inspectorPinned = $state(true);
  const inspectorOpen = $derived(wide && inspectorPinned);

  const todayKey = dayKeyOf(new Date());
  const isToday = $derived(data.dayKey === todayKey);
  const dayLabel = $derived(formatDayShort(data.dayKey));

  // ── The day model (pure, from journal-day.ts) ──────────────────────────
  const range = $derived(data.window);
  const rangeActivities = $derived(
    data.activities.filter((a) => a.startedAtMs < range.endMs && a.endedAtMs >= range.startMs),
  );
  const model = $derived(
    buildJournalDay({
      activities: data.activities,
      frames: data.frames,
      coveredUntilMs: data.ctxStatus?.coveredUntilMs ?? null,
      recording: captureControls.isRunning,
      engineAvailable: Boolean(data.ctxStatus?.engineAvailable),
      engineReason: data.ctxStatus?.reason ?? null,
      dayStartMs: range.startMs,
      dayEndMs: range.endMs,
    }),
  );
  const bands = $derived(bandRiver(buildRiver(model.slots, model.gaps)));
  const availableBands = $derived(bands.map((b) => b.label));
  const hasCards = $derived(model.slots.length > 0);

  const ledeStats = $derived(
    computeLedeStats({
      timePerApp: data.usage?.timePerApp ?? [],
      rangeActivities,
      rangeStartMs: range.startMs,
      rangeEndMs: range.endMs,
      engineOn: data.engineOn,
    }),
  );

  const showSkeleton = $derived(!data.rangeLoadedOnce);
  const showNothingCaptured = $derived(data.rangeLoadedOnce && !hasCards && !model.hasAnyCapture);
  const showBeingWritten = $derived(data.rangeLoadedOnce && !hasCards && model.hasAnyCapture);

  function relativeTime(ms: number | null | undefined): string {
    if (ms == null || !Number.isFinite(ms) || ms <= 0) return "";
    const diff = Date.now() - ms;
    if (diff < 60_000) return "just now";
    const min = Math.floor(diff / 60_000);
    if (min < 60) return `${min} min ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    return `${Math.floor(hr / 24)}d ago`;
  }

  // ── Selection: the inspector's subject, and what ⏎ opens ───────────────
  let selectedId = $state<number | null>(null);
  let receiptActivity = $state<Activity | null>(null);

  const marks = $derived(frameMarks(data.frames));
  const selectedIndex = $derived(model.slots.findIndex((s) => s.activity.id === selectedId));
  const selected = $derived(selectedIndex >= 0 ? model.slots[selectedIndex].activity : null);
  const record = $derived(selected ? buildActivityRecord(selected, marks) : null);
  const subjects = $derived(selected ? subjectsForActivity(selected.id, data.conclusions) : []);

  function select(activity: Activity): void {
    // Clicking the selected card again clears it — the inspector's empty state
    // is reachable without hunting for a close button (Overview's rule).
    selectedId = selectedId === activity.id ? null : activity.id;
  }

  function openReceipt(activity: Activity): void {
    selectedId = activity.id;
    receiptActivity = activity;
  }

  function openSelectedReceipt(): void {
    if (selected) receiptActivity = selected;
  }

  function showInTimeline(): void {
    if (record?.firstFrameId == null) return;
    setPendingTimelineFocus({ frameId: record.firstFrameId });
    void goto("/");
  }

  // ── Speakers in the inspector ──────────────────────────────────────────
  // Only hydrated when the engine cited spoken evidence for this activity: the
  // span hydration is one IPC per audio segment, and an activity with no cited
  // audio has no speaker row to fill.
  let turns = $state<TurnView[]>([]);
  let turnsLoading = $state(false);
  const audioLoader = new ReceiptAudioLoader({
    onProfiles: () => {},
    onTurns: (t) => {
      turns = t;
      turnsLoading = false;
    },
  });

  $effect(() => {
    const activity = selected;
    untrack(() => {
      turns = [];
      turnsLoading = false;
      audioLoader.reset();
      if (!activity) return;
      const cited = partitionEvidence(activity.evidence).audio;
      if (cited.length === 0) return;
      turnsLoading = true;
      void audioLoader.loadSpan(activity.startedAtMs, activity.endedAtMs, cited);
    });
  });

  // ── The one scrolling region ───────────────────────────────────────────
  let scrollEl = $state<HTMLDivElement | null>(null);
  let atLiveEdge = $state(true);

  function onScroll(): void {
    const el = scrollEl;
    if (!el) return;
    atLiveEdge = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }

  function jumpToNow(): void {
    const el = scrollEl;
    if (!el) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollTo({ top: el.scrollHeight, behavior: reduce ? "auto" : "smooth" });
  }

  function jumpToBand(band: BandLabel): void {
    scrollEl?.querySelector(`[data-band="${band}"]`)?.scrollIntoView({ block: "start" });
  }

  // ── Mount: first load, live refresh, keyboard ──────────────────────────
  onMount(() => {
    void data.reloadAll();
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    // A new card landing (or the watermark advancing) refreshes in place — no
    // skeleton reset, so the river never blanks on a worker beat.
    void listen("user_context_changed", () => void data.refresh()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      data.dispose();
    };
  });

  // WKWebView doesn't focus <button> on click, so element focus is unreliable —
  // a window capture-phase listener is the seam (the receipt uses the same one,
  // and owns the keyboard while it is open).
  $effect(() => {
    function onKey(e: KeyboardEvent): void {
      if (receiptActivity) return;
      if (e.metaKey && e.altKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        inspectorPinned = !inspectorPinned;
      } else if (e.key === "Enter" && selected && !e.metaKey) {
        e.preventDefault();
        openSelectedReceipt();
      }
    }
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  });
</script>

<div class="jr" bind:clientWidth={paneWidth}>
  <JournalToolStrip
    dayKey={data.dayKey}
    {availableBands}
    regenerating={data.digestRegenerating}
    canReRead={data.engineOn}
    {inspectorOpen}
    inspectorAvailable={wide}
    onday={(key) => data.setDay(key)}
    onstep={(days) => data.setDay(shiftDayKey(data.dayKey, days))}
    onjump={jumpToBand}
    onreread={() => void data.regenerateDigest()}
    ontoggleinspector={() => (inspectorPinned = !inspectorPinned)}
    onback={() => void goto("/overview")}
  />

  <div class="ss-body">
    <div class="ss-main main">
      <div class="jscroll" bind:this={scrollEl} onscroll={onScroll}>
        <JournalLede
          {dayLabel}
          digest={data.digest}
          loading={data.digestLoading}
          regenerating={data.digestRegenerating}
          error={data.digestError}
          writtenAgo={relativeTime(data.digest?.generatedAtMs)}
          stats={ledeStats}
          trackedLabel={humanizeHours(ledeStats.trackedMs)}
          activityCount={model.slots.length}
          usageLoaded={data.usageLoaded}
          rangeLoaded={data.rangeLoadedOnce}
        />
        <JournalRiver
          {bands}
          pending={model.pending}
          {showSkeleton}
          {hasCards}
          {showNothingCaptured}
          {showBeingWritten}
          {dayLabel}
          {isToday}
          {selectedId}
          onselect={select}
          onopen={openReceipt}
        />
      </div>

      {#if isToday && hasCards && !atLiveEdge}
        <button type="button" class="ss-nowpill float" onclick={jumpToNow}>↓ now</button>
      {/if}
    </div>

    {#if inspectorOpen}
      <JournalInspector
        activity={selected}
        {record}
        ordinal={selectedIndex + 1}
        total={model.slots.length}
        {subjects}
        {turns}
        {turnsLoading}
        onopen={openSelectedReceipt}
        ontimeline={showInTimeline}
      />
    {/if}
  </div>
</div>

{#if receiptActivity}
  <ActivityReceipt activity={receiptActivity} onClose={() => (receiptActivity = null)} />
{/if}

<style>
  .jr {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The float pill is positioned against the main pane, not the scrollport, so
     it never rides over the inspector. */
  .main {
    position: relative;
  }

  /* THE one scrolling region on this surface. */
  .jscroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 12px;
    /* AI-written titles carry unbreakable tokens (paths, URLs); without this
       they blow out the river's 1fr track and give the page an x-scroll. */
    overflow-wrap: anywhere;
  }

  .float {
    position: absolute;
    right: 14px;
    bottom: 10px;
    z-index: 8;
    cursor: pointer;
  }
  .float:hover {
    color: var(--app-accent);
  }
</style>
