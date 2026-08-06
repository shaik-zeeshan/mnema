<script lang="ts">
  // Journal — page 08. Not a third surface: a DESTINATION opened from Overview's
  // Today tile (⌃D). The titlebar grows one breadcrumb chip, the deck says where
  // you are and which keys are live, and `esc` puts you back on Overview.
  //
  // The day is a river you drive from the keyboard: ↑↓ moves the full-row accent
  // selection (the settings-row idiom), ⏎ opens that card's receipt. While the
  // receipt is open IT owns the keyboard and the deck re-labels to the
  // transport's own keys.
  //
  // Zero new backend: four reads that already ship (activities + frames + status
  // + digest) arranged through the shipping pure model (`buildJournalDay` +
  // `journal-view.ts` + `lede-stats.ts`). Rendering makes no LLM call — only
  // `↻ re-read` does.
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import { resetDeck, setDeck } from "$lib/deck.svelte";
  import { resetCrumbs, setCrumbs } from "$lib/crumb.svelte";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { shiftAnchor, windowFor } from "$lib/insights/activity-helpers";
  import { buildJournalDay } from "$lib/insights/journal-day";
  import { bandRiver, buildRiver } from "$lib/insights/journal-view";
  import { computeLedeStats } from "$lib/insights/lede-stats";
  import Lede from "$lib/journal/Lede.svelte";
  import River from "$lib/journal/River.svelte";
  import Receipt from "$lib/journal/Receipt.svelte";
  import type {
    Activity,
    AiRuntimeStatus,
    UserContextDigest,
    UserContextStatus,
  } from "$lib/types/recording";
  import type { FrameSummaryDto } from "$lib/types/app-infra";

  // ── The viewed day (local midnight bounds) ──────────────────────────────
  let anchorMs = $state<number>(Date.now());
  const range = $derived(windowFor(anchorMs, "day"));
  const atLatest = $derived(Date.now() < range.endMs);
  const dayLabel = $derived(
    new Date(range.startMs).toLocaleDateString(undefined, {
      weekday: "long",
      month: "long",
      day: "numeric",
    }),
  );
  const shortDayLabel = $derived(
    new Date(range.startMs).toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    }),
  );

  // ── Loaded data ─────────────────────────────────────────────────────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let statusLoaded = $state(false);
  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) || Boolean(ctxStatus?.engineAvailable),
  );

  let activities = $state<Activity[]>([]);
  let frames = $state<FrameSummaryDto[]>([]);
  let riverLoadedOnce = $state(false);
  let usage = $state<{ timePerApp: { activeMs: number }[] } | null>(null);
  let usageLoaded = $state(false);

  let digest = $state<UserContextDigest | null>(null);
  let digestLoading = $state(false);
  let digestRegenerating = $state(false);
  let digestError = $state<string | null>(null);

  // ── Derived model ───────────────────────────────────────────────────────
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
  const cards = $derived(model.slots.map((s) => s.activity));
  const hasCards = $derived(cards.length > 0);
  const ledeStats = $derived(
    computeLedeStats({
      timePerApp: usage?.timePerApp ?? [],
      rangeActivities,
      rangeStartMs: range.startMs,
      rangeEndMs: range.endMs,
      engineOn,
    }),
  );

  const showSkeleton = $derived(!riverLoadedOnce);
  const showNothingCaptured = $derived(riverLoadedOnce && !hasCards && !model.hasAnyCapture);
  const showBeingWritten = $derived(riverLoadedOnce && !hasCards && model.hasAnyCapture);

  // ── Selection + the receipt ─────────────────────────────────────────────
  let selectedId = $state<number | null>(null);
  let openActivity = $state<Activity | null>(null);
  const selectedIndex = $derived(cards.findIndex((a) => a.id === selectedId));

  // The day (or a worker beat) can drop the selected card — fall back to the
  // first one rather than leaving ↑↓ pointing at nothing.
  $effect(() => {
    const ids = cards.map((a) => a.id);
    untrack(() => {
      if (ids.length === 0) selectedId = null;
      else if (selectedId == null || !ids.includes(selectedId)) selectedId = ids[0];
    });
  });

  function move(delta: number): void {
    if (cards.length === 0) return;
    const next = Math.min(cards.length - 1, Math.max(0, selectedIndex + delta));
    selectedId = cards[next].id;
  }

  function onKeydown(event: KeyboardEvent): void {
    // The receipt owns the keyboard while it is open (it stops propagation in
    // the capture phase; this guard keeps the intent readable here too).
    if (openActivity) return;
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
    } else if (event.key === "Enter") {
      const a = cards[selectedIndex];
      if (!a) return;
      event.preventDefault();
      openActivity = a;
    } else if (event.key === "Escape") {
      event.preventDefault();
      void goto("/overview");
    }
  }

  // ── Loaders (gen-token guarded, mirrors the shipping Journal) ───────────
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
      const [nextActivities, nextFrames] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
          request: {
            capturedAtStart: new Date(startMs).toISOString(),
            capturedAtEnd: new Date(endMs).toISOString(),
          },
        }),
      ]);
      if (token !== rangeToken) return;
      activities = nextActivities;
      frames = nextFrames;
    } catch {
      // Best-effort: a failed read leaves the previous river standing.
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
    } catch {
      if (token === digestToken) digest = null;
    } finally {
      if (token === digestToken) digestLoading = false;
    }
  }

  // The busy flag gets its own sequence: `digestToken` is shared with
  // `loadDigest`, which the worker's `user_context_changed` beat re-fires during
  // a multi-second re-read — a token-gated reset would leave it stuck on
  // "reading…".
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
      if (!next) digestError = "Not enough activity in this day to write a read.";
    } catch (error) {
      if (token === digestToken)
        digestError = error instanceof Error ? error.message : "Couldn't write a read.";
    } finally {
      if (regen === regenSeq) digestRegenerating = false;
    }
  }

  // A day step can cost a paid model call (a fresh range misses the digest
  // cache), and a user may flick through days — so debounce the digest fetch on
  // a range change. Mount / event refresh call `loadDigest()` directly.
  const DIGEST_DEBOUNCE_MS = 500;
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
    }, DIGEST_DEBOUNCE_MS);
  }

  function stepDay(dir: -1 | 1): void {
    anchorMs = shiftAnchor(anchorMs, "day", dir);
  }

  // Re-query on a day change. The mount effect owns the first load, so skip the
  // priming run here.
  let rangePrimed = false;
  $effect(() => {
    range.startMs;
    range.endMs;
    untrack(() => {
      if (!rangePrimed) {
        rangePrimed = true;
        return;
      }
      openActivity = null;
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

  // Mount: first load + live refresh when a new card lands. `untrack` so the
  // loaders' own writes can never re-trigger this effect.
  $effect(() => {
    untrack(() => {
      void loadStatus().then(() => Promise.all([loadRange(), loadUsage(), loadDigest()]));
    });
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

  // ── Chrome: the breadcrumb chip + the deck ──────────────────────────────
  $effect(() => {
    setCrumbs([{ label: "Journal" }]);
    return resetCrumbs;
  });

  $effect(() => {
    const open = openActivity;
    if (open) {
      // The modal owns the keyboard, and the deck says so.
      setDeck({
        context: `Receipt · ${open.title} · ${new Date(open.startedAtMs).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", hour12: true })}`,
        hints: [
          { keys: "␣", label: "Play" },
          { keys: "←→", label: "Step frame" },
          { keys: "esc", label: "Close receipt", separator: true },
        ],
      });
    } else {
      const n = cards.length;
      setDeck({
        context: `Journal · ${dayLabel} · ${n} ${n === 1 ? "activity" : "activities"}`,
        // A keycap for a key that does nothing is a lie: an empty day binds
        // neither ↑↓ nor ⏎, so it advertises neither.
        hints: [
          ...(n > 0
            ? [
                { keys: "↑↓", label: "Move" },
                { keys: "⏎", label: "Open receipt" },
              ]
            : []),
          { keys: "esc", label: "Back to Overview", separator: n > 0 },
        ],
      });
    }
    return resetDeck;
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="jnl">
  <Lede
    {dayLabel}
    headline={digest?.headline ?? null}
    narrative={digest?.narrative ?? null}
    generatedAtMs={digest?.generatedAtMs ?? null}
    {digestError}
    {engineOn}
    regenerating={digestRegenerating}
    canReRead={!digestRegenerating && !(digestLoading && !digest)}
    stats={ledeStats}
    statsReady={riverLoadedOnce}
    usageReady={usageLoaded}
    activityCount={cards.length}
    {atLatest}
    onReRead={regenerateDigest}
    onStepDay={stepDay}
    onToday={() => (anchorMs = Date.now())}
  />

  <River
    {bands}
    pending={model.pending}
    {showSkeleton}
    {hasCards}
    {showNothingCaptured}
    {showBeingWritten}
    dayLabel={shortDayLabel}
    {selectedId}
    onSelect={(a) => (selectedId = a.id)}
    onOpen={(a) => (openActivity = a)}
  />

  {#if openActivity}
    <Receipt activity={openActivity} onClose={() => (openActivity = null)} />
  {/if}
</div>

<style>
  /* The surface fills the window between the title bar and the deck. `flex: 1 1
     auto` on a flex column, never `height: 100%` — WKWebView doesn't resolve a
     percentage height against a flex-stretched parent. `position: relative` so
     the receipt's scrim covers exactly this surface, not the chrome. */
  .jnl {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-wrap: anywhere;
  }
</style>
