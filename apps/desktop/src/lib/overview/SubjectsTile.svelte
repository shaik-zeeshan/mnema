<script lang="ts">
  // The way in to the Subjects destination (page 09). Two strongest views with
  // their conviction as a WHOLE PERCENT — the same quantity the destination
  // shows, in the same format, so nothing is recomputed differently one click
  // deeper. The five-dot meter is gone with it: a dot ladder is a second
  // encoding of a number the row already prints.
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import type { Cell } from "./data";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";

  interface Props {
    conclusions: Cell<Conclusion[]>;
    context: Cell<UserContextStatus | null>;
    loaded: boolean;
    open: () => void;
  }

  let { conclusions, context, loaded, open }: Props = $props();

  // Strongest first — the destination's own ordering (top confidence per
  // subject), truncated to two.
  const rows = $derived.by<Conclusion[]>(() => {
    const bySubject = new Map<string, Conclusion>();
    for (const c of conclusions.data ?? []) {
      const key = c.subject.toLocaleLowerCase();
      const seen = bySubject.get(key);
      if (!seen || c.confidence > seen.confidence) bySubject.set(key, c);
    }
    return [...bySubject.values()].sort((a, b) => b.confidence - a.confidence).slice(0, 2);
  });

  const active = $derived(context.data ? `${context.data.subjectCount} active` : null);
  const pct = (confidence: number): number =>
    Math.round(Math.max(0, Math.min(1, confidence)) * 100);
</script>

<Tile
  id="subjects"
  title="Subjects"
  kbd="⌃J"
  more={active}
  span={2}
  {open}
  openLabel="Open Subjects"
>
  {#if conclusions.error}
    <p class="tile-empty t-meta">Subjects unavailable — {conclusions.error}</p>
  {:else if loaded && rows.length === 0}
    <p class="tile-empty t-meta">
      Nothing distilled yet — Subjects appear once Mnema has watched a few days.
    </p>
  {:else}
    {#each rows as row (row.id)}
      <button class="grow" type="button" onclick={open}>
        <span class="grow__txt">
          <span class="grow__lbl">{row.subject}</span>
          <span class="grow__sub">{row.statement}</span>
        </span>
        <span class="grow__val">
          <span class="t-meta is-mono is-num subjects__pct">{pct(row.confidence)}%</span>
          <Chev />
        </span>
      </button>
    {/each}
    {#if rows.length > 0}
      <div class="tile-row subjects__all">
        <span class="t-meta subjects__link">All subjects</span>
        <span class="kbd subjects__k">⏎</span>
      </div>
    {/if}
  {/if}
</Tile>

<style>
  .subjects__pct {
    color: var(--app-text-muted);
  }
  .subjects__all {
    margin-top: auto;
  }
  .subjects__link {
    color: var(--app-accent);
  }
  .subjects__k {
    margin-left: auto;
  }
</style>
