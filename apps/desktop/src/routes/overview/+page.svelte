<script lang="ts">
  // Overview — Timeline's peer surface (⌘2), in Studio Shell's chrome.
  //
  // Four fixed pieces: the 38px title bar and the 24px status strip belong to
  // the root layout; this page owns the 30px contextual tool strip and the 256px
  // right inspector. Between them, one scrolling region holding the bento.
  //
  // The bento is kept whole (the direction's README) and re-skinned flatter:
  // a hairline under each tile header instead of a card edge, a 12px inset that
  // equals the 12px gutter, and 28px content rows. Every tile is a headline over
  // a read that already exists — selecting a row inside one fills the inspector,
  // which is what lets the tiles stay headlines.
  //
  // Round-4 decision **G8** is the rule for every number: real on this machine
  // or absent. A failed read renders a quiet reason, never a zero.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { OverviewData } from "$lib/overview/overview-data.svelte";
  import { dayKeyOf, formatDayTitle, formatSpan, shiftDayKey } from "$lib/overview/overview-format";
  import ToolStrip from "$lib/overview/ToolStrip.svelte";
  import Inspector from "$lib/overview/Inspector.svelte";
  import MomentsTile from "$lib/overview/MomentsTile.svelte";
  import DigestTile from "$lib/overview/DigestTile.svelte";
  import CaptureTile from "$lib/overview/CaptureTile.svelte";
  import StorageTile from "$lib/overview/StorageTile.svelte";
  import ConversationsTile from "$lib/overview/ConversationsTile.svelte";
  import SubjectsTile from "$lib/overview/SubjectsTile.svelte";
  import ContextTile from "$lib/overview/ContextTile.svelte";
  import WeekTile from "$lib/overview/WeekTile.svelte";
  import AskTile from "$lib/overview/AskTile.svelte";

  const data = new OverviewData();

  onMount(() => {
    void data.loadDay();
    void data.loadStanding();
  });

  // The 800×600 floor: the inspector collapses below 1000px and the grid drops
  // to two columns. Measured rather than assumed — the window is resizable and
  // there is no viewport unit that knows about the status strip.
  let paneWidth = $state(1100);
  const wide = $derived(paneWidth >= 1000);
  // Below 1000px every tile drops to the row count that actually fits. A tile
  // that shows two of three rows and says "3 today" in its header is honest; one
  // that lays out for 1100 and lets 800 slice through a row is not.
  const compact = $derived(!wide);
  let inspectorPinned = $state(true);
  const inspectorOpen = $derived(wide && inspectorPinned);

  const todayKey = dayKeyOf(new Date());
  const isToday = $derived(data.dayKey === todayKey);

  const coveredMs = $derived(
    data.coverage.status === "ok"
      ? (data.coverage.value.find((d) => d.day === data.dayKey)?.coveredMs ?? 0)
      : null,
  );

  // The tool strip's one-line summary. Each clause is dropped when its count is
  // not known — the line shortens rather than claiming a zero.
  const summary = $derived.by<string | null>(() => {
    const parts: string[] = [];
    if (coveredMs !== null && coveredMs > 0) parts.push(`${formatSpan(coveredMs)} captured`);
    if (data.conversations.status === "ok" && data.conversations.value.length > 0) {
      const n = data.conversations.value.length;
      parts.push(`${n} ${n === 1 ? "conversation" : "conversations"}`);
    }
    if (data.contextStatus.status === "ok" && data.contextStatus.value) {
      const n = data.contextStatus.value.conclusionCount;
      if (n > 0) parts.push(`${n} ${n === 1 ? "fact" : "facts"}`);
    }
    return parts.length > 0 ? parts.join(" · ") : null;
  });

  const selectedKey = $derived(data.selection?.key ?? null);
</script>

<div class="ov" bind:clientWidth={paneWidth}>
  <ToolStrip
    dayKey={data.dayKey}
    {summary}
    inspectorOpen={inspectorOpen}
    inspectorAvailable={wide}
    onday={(key) => data.setDay(key)}
    onstep={(days) => data.setDay(shiftDayKey(data.dayKey, days))}
    ontoggleinspector={() => (inspectorPinned = !inspectorPinned)}
  />

  <div class="ss-body">
    <div class="ss-main">
      <div class="pane">
        <header class="head">
          <h1 class="t-title">{formatDayTitle(data.dayKey)}</h1>
          {#if !isToday}<span class="t-meta">not today</span>{/if}
        </header>

        <div class="ss-tiles tiles" class:tiles--narrow={!wide}>
          <MomentsTile
            moments={data.moments}
            {selectedKey}
            onselect={(s) => data.select(s)}
          />
          <DigestTile digest={data.digest} {compact} onopen={() => void goto("/journal")} />
          <CaptureTile coverage={data.coverage} dayKey={data.dayKey} {isToday} {compact} />
          {#if wide}
            <StorageTile />
          {/if}
          <ConversationsTile
            conversations={data.conversations}
            {selectedKey}
            {compact}
            onselect={(s) => data.select(s)}
          />
          <SubjectsTile
            conclusions={data.conclusions}
            status={data.contextStatus}
            {selectedKey}
            {compact}
            onselect={(s) => data.select(s)}
            onopen={() => void goto("/subjects")}
          />
          {#if wide}
            <ContextTile authored={data.authored} onopen={() => void goto("/context")} />
          {/if}
          <WeekTile
            coverage={data.coverage}
            anchorKey={data.dayKey}
            {selectedKey}
            {compact}
            onselect={(s) => data.select(s)}
            onpickday={(key) => data.setDay(key)}
          />
          <AskTile asks={data.asks} {selectedKey} {compact} onselect={(s) => data.select(s)} />
        </div>
      </div>
    </div>

    {#if inspectorOpen}
      <Inspector selection={data.selection} />
    {/if}
  </div>
</div>

<style>
  .ov {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The scrolling region. `--grid-inset` equals `--grid-gutter` by the kit's
     grid rule: a container's inset is its gutter, at 12. */
  .pane {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--grid-gutter);
    padding: var(--grid-inset);
    overflow-y: auto;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: var(--s-10);
    height: 22px;
    flex: 0 0 auto;
  }

  .head h1 {
    margin: 0;
  }

  /* Wide: 4 columns, the media strip on its own row, three flexible rows under
     it. The row MINIMUM is the honest part: the grid fills the pane when there
     is room, and when the window is too short the rows hold their floor and the
     pane scrolls — rather than every tile silently cropping its last row
     (`.ss-tile__b` is `overflow: hidden`). */
  .tiles {
    flex: 1 1 auto;
    min-height: 0;
    grid-template-rows: 84px repeat(3, minmax(132px, 1fr));
  }

  /* Tiles are dense lists, not cards: 2px between rows, not the kit's 4. */
  .tiles :global(.ss-tile__b) {
    gap: 1px;
  }

  /* The 800×600 floor. Two columns, five rows — the same density the mockup
     draws. Two tiles are dropped rather than squeezed: Storage (the status strip
     already carries the per-day, monthly and free figures at the window's bottom
     edge) and Context (its count is in the tool strip's summary line). This Week
     stays where the mockup dropped it — G11 makes it a shipping tile, and
     nothing else on this surface carries the week. */
  .tiles--narrow {
    grid-template-columns: repeat(2, 1fr);
    grid-template-rows: 64px 88px minmax(84px, 1fr) minmax(84px, 1fr) 44px;
  }

  /* A tile that spans 3 or 4 columns can only span 2 here. */
  .tiles--narrow :global(.ss-tile--3),
  .tiles--narrow :global(.ss-tile--4) {
    grid-column: span 2;
  }

  /* Two-wide list tiles that pair up at the floor instead of taking a row each. */
  .tiles--narrow :global(.ov-half) {
    grid-column: span 1;
  }
</style>
