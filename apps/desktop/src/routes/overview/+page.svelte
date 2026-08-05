<script lang="ts">
  // Overview — Timeline's peer surface (⌘2), skinned in direction 01, BENTO
  // NATIVE. The tile grid IS the layout: one cell unit, one 16px gutter, and
  // exactly four legal footprints (1×1, 2×1, 2×2, 4×1 — a fifth is the moment
  // the grid stops being a grid). Every tile opens with the SAME 18px header row
  // on the same baseline, which is what licenses each payload below it to be
  // completely free — and to bleed past the 14px inset and be clipped by the
  // tile radius.
  //
  // All grid + tile machinery lives in `$lib/bento/bento.css` (imported once by
  // the shell layout). Nothing here forks it; this file owns only the page
  // frame, the day header, and the two column counts.
  //
  // Every tile runs on real backend data through `OverviewData` — one burst of
  // already-registered commands. A tile with nothing to show renders its
  // designed empty state; none of them ever invents a number (G8).
  import { onMount } from "svelte";
  import { OverviewData } from "$lib/overview/overview-data.svelte";
  import { capturedLabel } from "$lib/overview/overview-format";
  import MomentsTile from "$lib/overview/MomentsTile.svelte";
  import DigestTile from "$lib/overview/DigestTile.svelte";
  import CaptureTile from "$lib/overview/CaptureTile.svelte";
  import StorageTile from "$lib/overview/StorageTile.svelte";
  import ConversationsTile from "$lib/overview/ConversationsTile.svelte";
  import WeekTile from "$lib/overview/WeekTile.svelte";
  import ContextTile from "$lib/overview/ContextTile.svelte";
  import SubjectsTile from "$lib/overview/SubjectsTile.svelte";
  import AskTile from "$lib/overview/AskTile.svelte";

  const data = new OverviewData();

  onMount(() => {
    void data.load();
  });

  const today = new Date();
  const dayLabel = today.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  // The day line states only what was measured: captured time and the
  // conversation count (conversations, never minutes). No byte figure — nothing
  // measures per-day bytes, and G8 forbids the guess.
  const dayMeta = $derived.by(() => {
    const parts: string[] = [];
    const captured = capturedLabel(data.todayCoveredMs);
    if (captured) parts.push(`${captured} captured`);
    const count = data.conversations.length;
    if (count) parts.push(`${count} ${count === 1 ? "conversation" : "conversations"}`);
    return parts.join(" · ");
  });
</script>

<div class="ov scroll">
  <div class="ov__col">
    <div class="ov__day">
      <p class="t-title">{dayLabel}</p>
      {#if dayMeta}<p class="t-meta is-mono is-num">{dayMeta}</p>{/if}
    </div>

    <div class="bento ov__grid">
      <!-- R1 · 4×1 — media payload, bleeds all three edges -->
      <MomentsTile moments={data.moments} loaded={data.loaded} />

      <!-- R2 · 2×1 + 1×1 + 1×1 -->
      <DigestTile digest={data.digest} status={data.contextStatus} loaded={data.loaded} />
      <CaptureTile coveredMs={data.todayCoveredMs} />
      <StorageTile facts={data.facts} loaded={data.loaded} />

      <!-- R3 · 2×1 + 1×1 + 1×1 -->
      <ConversationsTile conversations={data.conversations} loaded={data.loaded} />
      <div class="drop-narrow"><WeekTile coverage={data.coverage} loaded={data.loaded} /></div>
      <ContextTile
        status={data.contextStatus}
        conclusions={data.conclusions}
        loaded={data.loaded}
      />

      <!-- R4 · 2×1 + 2×1 -->
      <div class="drop-narrow">
        <SubjectsTile
          conclusions={data.conclusions}
          subjectCount={data.contextStatus?.subjectCount ?? 0}
          loaded={data.loaded}
        />
      </div>
      <AskTile asks={data.asks} loaded={data.loaded} />
    </div>
  </div>
</div>

<style>
  .ov {
    flex: 1 1 auto; /* height:100% collapses under WKWebView — always flex here */
    min-height: 0;
    overflow-y: auto;
  }
  .ov__col {
    padding: var(--s-16) var(--pad-window) var(--s-48);
  }
  .ov__day {
    display: flex;
    align-items: baseline;
    gap: var(--s-12);
    height: 24px;
    margin-bottom: var(--gap-group);
  }
  .ov__day p {
    margin: 0;
  }

  /* Four columns, four row heights — the default window. Row heights are fixed
     because the grid's rhythm is the direction; a tile whose payload outgrows
     its row scrolls inside itself rather than pushing the grid around. */
  .ov__grid {
    grid-template-rows: 152px 140px 144px 130px;
  }

  /* `drop-narrow` is a pass-through wrapper so the tile inside keeps its own
     footprint. `display: contents` means it adds no box to the grid. */
  .drop-narrow {
    display: contents;
  }

  /* The 800×600 floor: two columns, and the stated drop order — This week and
     Subjects go, the captured-hours hero stays (it is this screen's one
     --t-display). */
  @media (max-width: 900px) {
    .ov__grid {
      grid-template-columns: repeat(2, 1fr);
      grid-template-rows: 150px 128px 132px 118px 104px;
    }
    .drop-narrow {
      display: none;
    }
  }
</style>
