<script lang="ts">
  // The day's read, and the whole of Open Threads v1.
  //
  // Round-4 decision **G11**: "Open Threads v1 = digest prose only — surface the
  // digest's existing 'one open thread…' sentence where the mockup draws the
  // tile. No entity, no table, no extraction pipeline." So this tile renders the
  // narrative the daily digest already wrote and nothing else: no parsing for
  // thread sentences, no per-thread rows, no resolve affordance.
  //
  // The read is `get_latest_user_context_digest`, which is read-only by
  // construction — a tile mounting must never start a paid generation.
  import type { UserContextDigest } from "$lib/types/recording";
  import type { LoadState } from "./overview-data.svelte";
  import { formatClock } from "./overview-format";
  import TileShell from "./TileShell.svelte";

  interface Props {
    digest: LoadState<UserContextDigest | null>;
    /** The 800px floor: the narrative alone, clamped with a visible ellipsis.
     *  The headline is the line that goes — the open-thread sentence G11 ships
     *  lives in the narrative, so the narrative is what keeps the room. */
    compact?: boolean;
    /** The tile is the Journal's door (page 08): the header opener replaces the
     *  digest stamp — the stamp lives in the Journal's own lede. */
    onopen?: () => void;
  }

  let { digest, compact = false, onopen }: Props = $props();

  const value = $derived(digest.status === "ok" ? digest.value : null);

  const quiet = $derived(
    digest.status === "failed"
      ? "Couldn't read the daily digest."
      : digest.status === "loading"
        ? null
        : value === null
          ? "No daily read yet — Mnema writes one once a day has enough activity."
          : null,
  );

  // No stamp, no time in the header — never "digest · NaN:NaN" (G8).
  const stamp = $derived(value ? formatClock(value.generatedAtMs) : null);
  const more = $derived.by(() => {
    if (onopen) return "Open Journal";
    return stamp === null ? undefined : compact ? stamp : `digest · ${stamp}`;
  });
</script>

<TileShell label="Today" {more} {onopen} span="ss-tile--2" {quiet}>
  {#if value}
    <div class="prose">
      {#if value.headline && !compact}<p class="head">{value.headline}</p>{/if}
      <p class="body" class:clamp={compact}>{value.narrative}</p>
    </div>
  {/if}
</TileShell>

<style>
  .prose {
    min-height: 0;
    overflow: hidden;
  }

  .head {
    margin: 0 0 3px;
    font: var(--w-medium) var(--t-ui) / 1.35 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  /* 13px rather than the 14px `--t-read`: the tile is a headline, and the grid
     already spends its one --t-display on the captured-hours hero. */
  .body {
    margin: 0;
    font: var(--w-regular) 13px / 1.5 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
  }

  /* Truncation with an ellipsis, never a silent crop — the reader can see the
     sentence continues. */
  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
