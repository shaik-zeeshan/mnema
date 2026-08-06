<script lang="ts">
  // Overview — the Timeline's peer surface (⌘2), as direction 04's bento.
  //
  // Nine tiles on a 4-column grid, every tile a destination that tells you its
  // key. The keycap rule is load-bearing: a tile only wears a keycap for a key
  // this page really binds. ⌃<key> focuses its tile (always real, always
  // visible); ⏎ on a focused tile opens the surface behind it. Tiles with no
  // surface behind them (This Week) still focus — they just never open.
  //
  // Data is nine reads that already shipped; this route adds no Rust and no
  // aggregation. Numbers the mockup drew that this machine cannot produce are
  // absent rather than invented (G8) — see the per-tile comments.
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";

  import { resetDeck, setDeck } from "$lib/deck.svelte";
  import { captureSession } from "$lib/session.svelte";
  import { openSettings } from "$lib/surface-windows";
  import { getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";

  import { loadOverview, type OverviewSnapshot } from "$lib/overview/data";
  import { coveredOn, formatHoursMinutes, localDayKey } from "$lib/overview/format";
  import MomentsTile from "$lib/overview/MomentsTile.svelte";
  import DigestTile from "$lib/overview/DigestTile.svelte";
  import CaptureTile from "$lib/overview/CaptureTile.svelte";
  import StorageTile from "$lib/overview/StorageTile.svelte";
  import ConversationsTile from "$lib/overview/ConversationsTile.svelte";
  import SubjectsTile from "$lib/overview/SubjectsTile.svelte";
  import WeekTile from "$lib/overview/WeekTile.svelte";
  import ContextTile from "$lib/overview/ContextTile.svelte";
  import AskTile from "$lib/overview/AskTile.svelte";

  const EMPTY = { data: null, error: null };
  let snap = $state<OverviewSnapshot>({
    coverage: EMPTY,
    moments: EMPTY,
    digest: EMPTY,
    conversations: EMPTY,
    conclusions: EMPTY,
    context: EMPTY,
    asks: EMPTY,
    facts: EMPTY,
    settings: EMPTY,
  });
  let loaded = $state(false);
  let holding = $state(false);
  // One ticking clock for the whole surface: the live elapsed readout and the
  // "as of" stamp. It ticks only while the document is actually shown — a
  // repaint the compositor never displays is what strands WebKit backing
  // stores, same reason the recording pill gates its clock.
  let nowMs = $state(Date.now());
  const now = $derived(new Date(nowMs));

  // Load once on mount. `untrack` so the loader's own state writes can never
  // re-trigger this effect (the settings-init trap).
  $effect(() => {
    untrack(() => {
      void loadOverview().then((next) => {
        snap = next;
        loaded = true;
      });
    });
    let tick: ReturnType<typeof setInterval> | null = null;
    const stop = (): void => {
      if (tick !== null) clearInterval(tick);
      tick = null;
    };
    const sync = (): void => {
      stop();
      if (document.visibilityState !== "visible") return;
      nowMs = Date.now();
      tick = setInterval(() => (nowMs = Date.now()), 1000);
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", sync);
    };
  });

  // ── destinations ───────────────────────────────────────────────────────────
  // Every one of these is a real surface this app already has. Insights hosts
  // its sub-surfaces in local state, not routes, so the four Insights tiles all
  // land on Insights rather than each on its own sub-tab.
  const openTimeline = (): void => void goto("/");
  const openInsights = (): void => void goto("/insights");
  const openCaptureSettings = (): void => void openSettings("capture");
  const openStorageSettings = (): void => void openSettings("storage");
  const openQuickAccess = (): void => void invoke("summon_quick_recall_window_command");

  // ── the grid's shortcut map ────────────────────────────────────────────────
  // ⌃<key> focuses its tile; ⏎ then opens it (Tile.svelte owns that half).
  const TILE_KEYS: Record<string, string> = {
    m: "moments",
    d: "digest",
    r: "capture",
    s: "storage",
    c: "conversations",
    j: "subjects",
    w: "week",
    k: "context",
  };

  function focusTile(id: string): void {
    const el = document.querySelector<HTMLElement>(`[data-tile="${id}"]`);
    // A hidden tile (the 800×600 floor drops three) has no box to focus.
    if (el && el.offsetParent !== null) el.focus();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.metaKey && event.key === "Enter") {
      event.preventDefault();
      openQuickAccess();
      return;
    }
    if (event.key === "Control") holding = true;
    if (!event.ctrlKey || event.metaKey || event.altKey) return;
    const id = TILE_KEYS[event.key.toLowerCase()];
    if (!id) return;
    event.preventDefault();
    focusTile(id);
  }

  function onKeyup(event: KeyboardEvent): void {
    if (event.key === "Control") holding = false;
  }

  // ── header + deck ──────────────────────────────────────────────────────────
  const todayKey = $derived(localDayKey(now));
  const coveredTodayMs = $derived(
    snap.coverage.data ? coveredOn(snap.coverage.data, todayKey) : null,
  );
  const dayLabel = $derived(
    now.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" }),
  );
  const conversationCount = $derived(snap.conversations.data?.length ?? 0);
  const asOf = $derived(
    `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`,
  );

  const platform = detectKeyboardPlatform();
  function shortcutDisplay(id: "toggleQuickRecall" | "openSettings"): string | null {
    const binding = getEffectiveGlobalShortcut(id).bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : null;
  }

  $effect(() => {
    const quickAccess = shortcutDisplay("toggleQuickRecall");
    const settings = shortcutDisplay("openSettings");
    setDeck({
      context: `Overview · ${dayLabel}`,
      hints: [
        { keys: "⌃", label: "focus a tile" },
        { keys: "⏎", label: "open tile" },
        ...(quickAccess ? [{ keys: quickAccess, label: "Quick Access", separator: true }] : []),
        ...(settings ? [{ keys: settings, label: "Settings" }] : []),
      ],
    });
    return resetDeck;
  });
</script>

<svelte:window onkeydown={onKeydown} onkeyup={onKeyup} onblur={() => (holding = false)} />

<div class="ov" class:ov--holding={holding}>
  <div class="ov__hd">
    <p class="t-title ov__day">{dayLabel}</p>
    <p class="t-meta is-mono is-num ov__meta">
      {#if snap.coverage.data}{formatHoursMinutes(coveredTodayMs ?? 0)} captured{/if}
      {#if conversationCount > 0}
        · {conversationCount}
        {conversationCount === 1 ? "conversation" : "conversations"}
      {/if}
      <span class="ov__asof">· updated {asOf}</span>
    </p>
    <span class="hint ov__hold">
      <span class="kbd">⌃</span><span>hold for tile shortcuts</span>
    </span>
  </div>

  <div class="tiles">
    <MomentsTile moments={snap.moments} {loaded} open={openTimeline} />
    <DigestTile digest={snap.digest} {loaded} open={openInsights} />
    <CaptureTile
      {coveredTodayMs}
      coverageError={snap.coverage.error}
      session={captureSession.value}
      {nowMs}
      open={openCaptureSettings}
    />
    <StorageTile facts={snap.facts} settings={snap.settings} {loaded} open={openStorageSettings} />
    <ConversationsTile conversations={snap.conversations} {loaded} open={openTimeline} />
    <SubjectsTile
      conclusions={snap.conclusions}
      context={snap.context}
      {loaded}
      open={openInsights}
    />
    <WeekTile coverage={snap.coverage} {now} {loaded} />
    <ContextTile
      context={snap.context}
      conclusions={snap.conclusions}
      {loaded}
      open={openInsights}
    />
    <AskTile asks={snap.asks} {loaded} open={openQuickAccess} />
  </div>
</div>

<style>
  /* The surface fills the window between the title bar and the deck; the grid
     fills what's left of it. Nothing here scrolls — the bento is the screen. */
  .ov {
    flex: 1 1 auto;
    min-height: 0;
    padding: var(--s-16);
    display: flex;
    flex-direction: column;
    /* The 800×600 floor is a container query, not a media query: the surface
       responds to the space it is given, not to the display. */
    container-type: inline-size;
    container-name: ov;
  }

  .ov__hd {
    display: flex;
    align-items: baseline;
    gap: var(--s-12);
    height: 30px;
    flex: 0 0 30px;
  }
  .ov__day,
  .ov__meta {
    margin: 0;
  }
  .ov__hold {
    margin-left: auto;
  }

  /* ── the bento ──────────────────────────────────────────────────────────
     Direction 04's tile block (07-components.html), scoped to this surface —
     only the Overview draws tiles, so these never became global. */
  .ov :global(.tiles) {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--s-12);
    flex: 1 1 auto;
    min-height: 0;
    /* The mockup's four row heights; the last one absorbs whatever the window
       gives beyond them so the bento always fills the surface. */
    grid-template-rows: 128px 128px 168px minmax(112px, 1fr);
  }

  .ov :global(.tile) {
    background: var(--app-surface);
    border-radius: 10px;
    padding: var(--s-12);
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    min-width: 0;
    overflow: hidden;
    position: relative;
    outline: none;
  }
  .ov :global(.tile--2) {
    grid-column: span 2;
  }
  .ov :global(.tile--4) {
    grid-column: span 4;
  }
  .ov :global(.tile--media) {
    padding: 0;
    gap: 0;
  }
  .ov :global(.tile:focus-visible) {
    box-shadow: 0 0 0 2px var(--app-accent);
  }

  .ov :global(.tile-h) {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    margin-bottom: 2px;
  }
  .ov :global(.tile-h .more) {
    margin-left: auto;
  }
  .ov :global(.tile-k) {
    opacity: 0.62;
    transition: opacity 100ms ease;
  }
  .ov :global(.tile-k--far) {
    margin-left: auto;
  }
  /* Hold ⌃ and the whole shortcut map raises contrast. */
  .ov--holding :global(.tile-k) {
    opacity: 1;
  }

  .ov :global(.tile-row) {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    min-height: 18px;
  }
  .ov :global(.tile-empty) {
    margin: 0;
    align-self: flex-start;
  }
  /* The media tile has no padding of its own, so its empty state supplies it. */
  .ov :global(.tile--media .tile-empty) {
    padding: var(--s-12);
  }

  /* Rows inside a tile: thumb / label+sub / value + chevron. */
  .ov :global(.grow) {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 5px 0;
    min-width: 0;
    width: 100%;
    background: none;
    border: 0;
    border-radius: var(--r-sm);
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: default;
  }
  .ov :global(.grow:hover) {
    background: var(--app-surface-hover);
  }
  .ov :global(.grow:focus-visible) {
    box-shadow: 0 0 0 2px var(--app-accent);
    outline: none;
  }
  .ov :global(.grow__txt) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .ov :global(.grow__lbl) {
    font: var(--w-regular) var(--t-ui) / 1.25 var(--app-font-sans);
    color: var(--app-text-strong);
    letter-spacing: var(--ls-ui);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov :global(.grow__sub) {
    font: var(--w-regular) var(--t-meta) / 1.3 var(--app-font-sans);
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov :global(.grow__val) {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--gap-inline);
    flex: 0 0 auto;
  }
  .ov :global(.chev) {
    display: inline-flex;
    color: var(--app-text-faint);
  }
  .ov :global(.chev svg) {
    width: 11px;
    height: 11px;
  }

  .ov :global(.conv) {
    display: inline-flex;
    gap: 2px;
  }
  .ov :global(.conv i) {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--app-text-faint);
  }
  .ov :global(.conv i.on) {
    background: var(--app-accent);
  }

  .ov :global(.gicon) {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .ov :global(.gicon svg) {
    width: 11px;
    height: 11px;
  }

  /* Moments strip — real frames, edge to edge. */
  .ov :global(.strip) {
    display: flex;
    gap: var(--s-4);
    height: 100%;
    position: relative;
  }
  .ov :global(.strip > i) {
    position: relative;
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    display: block;
    background: var(--app-surface-subtle);
  }
  .ov :global(.strip > i:first-child) {
    border-radius: 10px 0 0 10px;
  }
  .ov :global(.strip > i:last-child) {
    border-radius: 0 10px 10px 0;
  }
  .ov :global(.strip img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .ov :global(.strip__t) {
    position: absolute;
    left: 6px;
    bottom: 5px;
    z-index: 2;
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: #fff;
    padding: 3px 5px;
    border-radius: var(--r-sm);
    background: rgba(10, 12, 16, 0.6);
    backdrop-filter: blur(6px);
  }
  .ov :global(.strip__k) {
    position: absolute;
    right: 6px;
    bottom: 5px;
    z-index: 2;
  }

  /* Usage bar — the settings ladder's bar, reused. */
  .ov :global(.ladder__bar) {
    position: relative;
    height: 6px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    overflow: hidden;
  }
  .ov :global(.ladder__bar i) {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    background: var(--app-accent);
    opacity: 0.75;
  }
  .ov :global(.storage__bar) {
    margin-top: auto;
  }
  .ov :global(.storage__foot) {
    color: var(--app-text-subtle);
  }
  .ov :global(.storage__foot-row) {
    margin-top: auto;
  }

  /* Digest prose. Clamped rather than clipped: a tile is a fixed box, and a
     sentence sheared mid-descender reads as a bug. */
  .ov :global(.digest) {
    margin: 0;
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
  }

  /* Capture. */
  .ov :global(.capture__hero) {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
  }
  .ov :global(.capture__state) {
    margin-top: 2px;
  }
  .ov :global(.capture__elapsed) {
    margin-left: auto;
  }
  .ov :global(.rdot) {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-record);
    flex: 0 0 auto;
  }
  .ov :global(.rdot--paused) {
    background: var(--app-warn);
  }
  .ov :global(.rdot--off) {
    background: var(--app-text-faint);
  }
  .ov :global(.srcs) {
    display: inline-flex;
    gap: 5px;
  }
  .ov :global(.src) {
    width: 13px;
    height: 13px;
  }
  .ov :global(.src--screen) {
    color: var(--app-source-screen);
  }
  .ov :global(.src--mic) {
    color: var(--app-source-mic);
  }
  .ov :global(.src--sys) {
    color: var(--app-source-sysaudio);
  }

  /* Conversations. */
  .ov :global(.wavethumb) {
    width: 44px;
    height: 26px;
    border-radius: 4px;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }
  .ov :global(.wavethumb svg) {
    width: 34px;
    height: 20px;
  }

  /* This week. */
  .ov :global(.wk) {
    display: flex;
    align-items: flex-end;
    gap: 5px;
    height: 40px;
  }
  .ov :global(.wk .spark) {
    flex: 1;
    height: 100%;
    display: flex;
    align-items: flex-end;
  }
  .ov :global(.wk .spark i) {
    display: block;
    width: 100%;
    background: var(--app-accent);
    opacity: 0.4;
    border-radius: 1.5px;
  }
  .ov :global(.wk .spark--today i) {
    opacity: 1;
  }
  .ov :global(.wkl) {
    display: flex;
    gap: 5px;
    margin-top: 4px;
  }
  .ov :global(.wkl span) {
    flex: 1;
    text-align: center;
    color: var(--app-text-faint);
  }
  .ov :global(.wkl span.wkl--today) {
    color: var(--app-accent);
  }
  .ov :global(.wk__foot) {
    margin-top: auto;
  }
  .ov :global(.wk__busiest) {
    margin-left: auto;
  }

  /* Context. */
  .ov :global(.ctx__newest) {
    align-items: flex-start;
    min-height: 0;
  }
  .ov :global(.ctx__newest span) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .ov :global(.ctx__foot) {
    margin-top: auto;
  }
  .ov :global(.ctx__review) {
    color: var(--app-text-subtle);
  }
  .ov :global(.ctx__foot .chev) {
    margin-left: auto;
  }

  /* Ask launcher — the field that starts the next question. */
  .ov :global(.asklaunch) {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    height: var(--h-md);
    padding: 0 var(--s-8);
    margin-top: auto;
    border: 0;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    cursor: default;
    text-align: left;
    width: 100%;
  }
  .ov :global(.asklaunch:hover) {
    background: var(--app-surface-hover);
  }
  .ov :global(.asklaunch:focus-visible) {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--app-border-strong),
      0 0 0 2px var(--app-accent);
    outline: none;
  }
  .ov :global(.asklaunch svg) {
    width: 14px;
    height: 14px;
    color: var(--app-text-subtle);
    flex: 0 0 auto;
  }
  .ov :global(.asklaunch__ph) {
    color: var(--app-text-subtle);
  }
  .ov :global(.asklaunch__k) {
    margin-left: auto;
  }
  .ov :global(.ask__row) {
    padding-top: 0;
  }

  /* ── 800 × 600 floor ────────────────────────────────────────────────────
     Two columns. Storage, This Week and the Ask tile drop; the 6:42 hero
     stays (the screen's one display-size number) and Capture drops its
     sources row instead. */
  @container ov (width < 900px) {
    .ov :global(.tiles) {
      grid-template-columns: repeat(2, 1fr);
      grid-template-rows: 110px 92px 108px minmax(96px, 1fr);
    }
    .ov :global(.tile--4) {
      grid-column: span 2;
    }
    .ov :global(.digest) {
      -webkit-line-clamp: 2;
      line-clamp: 2;
    }
    /* The strip drops to three frames, so the third one owns the right edge. */
    .ov :global([data-tile="moments"] .strip > i:nth-child(3)) {
      border-radius: 0 10px 10px 0;
    }
    .ov :global([data-tile="storage"]),
    .ov :global([data-tile="week"]),
    .ov :global([data-tile="ask"]) {
      display: none;
    }
    .ov :global([data-tile="conversations"]),
    .ov :global([data-tile="subjects"]) {
      grid-column: span 1;
    }
    /* Two rows, no thumb, no sub-line: the narrow tile is a list, not a card. */
    .ov :global([data-tile="conversations"] .grow:nth-of-type(3)),
    .ov :global([data-tile="subjects"] .grow:nth-of-type(3)),
    .ov :global([data-tile="conversations"] .grow__sub),
    .ov :global([data-tile="conversations"] .wavethumb),
    .ov :global([data-tile="subjects"] .grow__sub),
    .ov :global([data-tile="subjects"] .gicon),
    .ov :global(.capture__sources),
    .ov :global([data-tile="moments"] .strip > i:nth-child(n + 4)),
    .ov__asof,
    .ov__hold {
      display: none;
    }
  }
</style>
