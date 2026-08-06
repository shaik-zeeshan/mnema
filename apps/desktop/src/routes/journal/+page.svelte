<script lang="ts">
  // Journal — a destination, not a surface (direction 01, mockup 08). Opened
  // from Overview's Moments tile; the titlebar's `‹ Overview` chevron (owned by
  // +layout.svelte) is the way back.
  //
  // The page is the bento: THE READ on top as a 4×1, then one 4×1 tile per
  // time-of-day band, each opening with the same 18px header row while the
  // payload under it is the free when/spine/card river. Every figure on the page
  // resolves to a field the backend already serves (G8) — the day meta comes
  // from the same `computeLedeStats` Overview uses, the frame counts from the
  // frames whose `capturedAt` falls inside each activity, the away-gaps from
  // covered-frame gaps ≥ 5 minutes, and the live edge from the derivation
  // watermark rather than the clock. Nothing here is rounded because it looked
  // plausible, and a part with no value is omitted rather than faked.
  import { untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import IconChevDown from "~icons/lucide/chevron-down";
  import IconChevLeft from "~icons/lucide/chevron-left";
  import IconChevRight from "~icons/lucide/chevron-right";
  import { shiftAnchor } from "$lib/insights/activity-helpers";
  import { JournalData } from "$lib/journal/journal-data.svelte";
  import BandTile from "$lib/journal/BandTile.svelte";
  import ReadTile from "$lib/journal/ReadTile.svelte";
  import Receipt from "$lib/journal/Receipt.svelte";
  import type { Activity } from "$lib/types/recording";

  const data = new JournalData();

  let selected = $state<Activity | null>(null);
  let bodyEl = $state<HTMLDivElement | null>(null);
  let liveEl = $state<HTMLDivElement | null>(null);
  let liveVisible = $state(true);

  // Mount: first load, plus a live refresh whenever a card lands or the
  // watermark advances. The refresh is in place — no loading reset — so the day
  // never blanks to a skeleton on a worker beat.
  $effect(() => {
    void untrack(() => data.reloadAll());
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void data.loadStatus();
      void data.loadDay();
      void data.loadUsage();
      void data.loadDigest();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      data.cancelDigestDebounce();
    };
  });

  // Re-query on a day step. The mount run is skipped — the effect above owns the
  // first load — so the loaders never double-fire.
  let primed = false;
  $effect(() => {
    data.range.startMs;
    untrack(() => {
      if (!primed) {
        primed = true;
        return;
      }
      data.loadForNewDay();
    });
  });

  // The live-edge pill only exists when the bottom of today is off screen.
  $effect(() => {
    const el = liveEl;
    const root = bodyEl;
    if (!el || !root) return;
    const io = new IntersectionObserver(
      (entries) => (liveVisible = entries[entries.length - 1].isIntersecting),
      { root },
    );
    io.observe(el);
    return () => io.disconnect();
  });

  function jumpToNow(): void {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    liveEl?.scrollIntoView({ block: "end", behavior: reduce ? "auto" : "smooth" });
  }
</script>

<div class="jn">
  <!-- The day header: the stepper, and the day's real numbers. A figure with no
       value (no usage read, no focus on any activity, no category) is left out
       rather than shown as a zero. -->
  <div class="dbar">
    <span class="step">
      <button
        type="button"
        class="btn btn--icon btn--sm nav"
        aria-label="Previous day"
        onclick={() => (data.anchorMs = shiftAnchor(data.anchorMs, "day", -1))}
      ><IconChevLeft /></button>
      <span class="step__d">{data.dayLabel}</span>
      <button
        type="button"
        class="btn btn--icon btn--sm nav"
        aria-label="Next day"
        disabled={data.atLatest}
        onclick={() => (data.anchorMs = shiftAnchor(data.anchorMs, "day", 1))}
      ><IconChevRight /></button>
    </span>
    {#if !data.atLatest}
      <button type="button" class="btn btn--ghost btn--sm today" onclick={() => (data.anchorMs = Date.now())}>
        Today
      </button>
    {/if}

    <span class="stats">
      {#if data.usageLoaded && data.ledeStats.trackedMs > 0}
        <span class="lstat"><b>{data.trackedLabel}</b><span>tracked</span></span>
      {/if}
      {#if data.dayLoaded && data.ledeStats.deepPct !== null}
        <span class="lsep"></span>
        <span class="lstat"><b>{data.ledeStats.deepPct}%</b><span>deep focus</span></span>
      {/if}
      {#if data.dayLoaded && data.ledeStats.topCategory}
        <span class="lsep"></span>
        <span class="lstat">
          <i style="background:var({data.ledeStats.topCategory.colorVar})"></i>
          <span>{data.ledeStats.topCategory.label}</span>
        </span>
      {/if}
      {#if data.dayLoaded && data.model.slots.length > 0}
        <span class="lsep"></span>
        <span class="lstat">
          <b>{data.model.slots.length}</b>
          <span>{data.model.slots.length === 1 ? "activity" : "activities"}</span>
        </span>
      {/if}
    </span>
  </div>

  <div class="jbody scroll" bind:this={bodyEl}>
    <div class="bento grid">
      <ReadTile
        digest={data.digest}
        dayLabel={data.dayLabel}
        engineOn={data.engineOn}
        statusLoaded={data.statusLoaded}
        loading={data.digestLoading}
        regenerating={data.digestRegenerating}
        error={data.digestError}
        onReread={() => void data.regenerateDigest()}
      />

      {#if data.hasCards}
        {#each data.bands as band (band.label + band.startMs)}
          <BandTile {band} pending={data.model.pending} onOpen={(a) => (selected = a)} />
        {/each}
      {:else if !data.dayLoaded}
        <div class="tile tile--w4 tile--static">
          <div class="tile__h"><span class="t-label">The day</span></div>
          <div class="pay sk-rows">
            <div class="sk" style="width:46%"></div>
            <div class="sk" style="width:88%"></div>
            <div class="sk" style="width:64%"></div>
          </div>
        </div>
      {:else if data.showBeingWritten}
        <div class="tile tile--w4 tile--static empty">
          <div class="tile__h">
            <span class="t-label">Journal</span><span class="tile__more">no cards yet</span>
          </div>
          <div class="pay panel">
            <span class="glyph">◇</span>
            <span class="t-ui strong">Your day is being written</span>
            <span class="t-meta narrow">
              Capture is landing. The first journal card appears once the first half-hour window
              has been summarized.
            </span>
          </div>
        </div>
      {:else if data.showNothingCaptured}
        <div class="tile tile--w4 tile--static empty">
          <div class="tile__h">
            <span class="t-label">Journal</span><span class="tile__more">no capture</span>
          </div>
          <div class="pay panel">
            <span class="glyph">◇</span>
            <span class="t-ui strong">Nothing captured on {data.dayLabel}</span>
            <span class="t-meta narrow">
              There's no capture on this day, so there's no journal to show. Days with any
              recording at all show whatever was captured.
            </span>
          </div>
        </div>
      {/if}
    </div>
    <div class="live" bind:this={liveEl} aria-hidden="true"></div>
  </div>

  {#if data.atLatest && !liveVisible}
    <button type="button" class="btn btn--sm jnow" onclick={jumpToNow}>
      <IconChevDown />now
    </button>
  {/if}
</div>

{#if selected}
  <Receipt activity={selected} onClose={() => (selected = null)} />
{/if}

<style>
  .jn {
    position: relative;
    flex: 1 1 auto; /* height:100% collapses under WKWebView — always flex here */
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The destination sub-bar: stepper left, the day's real numbers right. */
  .dbar {
    flex: 0 0 40px;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: 0 var(--pad-window);
    background: var(--app-surface);
    box-shadow: 0 var(--hairline) 0 var(--app-border);
  }
  .step {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .nav {
    border-color: transparent;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
  }
  .nav:disabled {
    opacity: var(--opacity-disabled);
    cursor: default;
  }
  .nav :global(svg) {
    width: 12px;
    height: 12px;
  }
  .step__d {
    min-width: 108px;
    text-align: center;
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .today {
    cursor: pointer;
  }
  .stats {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--s-8);
  }
  .lstat {
    display: inline-flex;
    align-items: baseline;
    gap: var(--s-4);
  }
  .lstat b {
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }
  .lstat span {
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-sans);
    color: var(--app-text-subtle);
  }
  .lstat i {
    align-self: center;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .lsep {
    width: var(--hairline);
    height: 14px;
    background: var(--app-border-strong);
  }

  .jbody {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
  /* Bands are as tall as their rows — the row count is the content, so the grid
     rhythm here is the 16px gutter, not a fixed row height. */
  .grid {
    grid-auto-rows: min-content;
  }
  .live {
    height: 0;
  }

  .sk-rows {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
  }
  .sk {
    height: 11px;
    border-radius: 3px;
    background: var(--app-surface-hover);
  }

  .empty {
    min-height: 168px;
  }
  .panel {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-6);
    text-align: center;
  }
  .glyph {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-faint);
  }
  .strong {
    color: var(--app-text-strong);
  }
  .narrow {
    max-width: 44ch;
  }

  /* The live edge is opt-in: every day opens at the top. */
  .jnow {
    position: absolute;
    left: 50%;
    bottom: 18px;
    z-index: 30;
    transform: translateX(-50%);
    box-shadow: var(--app-shadow-popover, 0 8px 24px rgba(0, 0, 0, 0.32));
    cursor: pointer;
  }
  .jnow :global(svg) {
    width: 12px;
    height: 12px;
  }
</style>
