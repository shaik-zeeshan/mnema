<script lang="ts">
  // Three Subjects, each with its newest conclusion and a five-dot conviction
  // meter. Conviction is the Conclusion's own `confidence` — a real stored
  // number, not a derived score.
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import { convictionDots } from "./format";
  import { subjectRows } from "./data";
  import type { Cell } from "./data";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";

  interface Props {
    conclusions: Cell<Conclusion[]>;
    context: Cell<UserContextStatus | null>;
    loaded: boolean;
    open: () => void;
  }

  let { conclusions, context, loaded, open }: Props = $props();

  const rows = $derived(subjectRows(conclusions.data ?? [], 3));
  const active = $derived(
    context.data ? `${context.data.subjectCount} active` : null,
  );
</script>

<Tile
  id="subjects"
  title="Subjects"
  kbd="⌃J"
  more={active}
  span={2}
  {open}
  openLabel="Open Insights"
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
        <span class="gicon" aria-hidden="true">
          <svg viewBox="0 0 16 16" fill="currentColor">
            <path d="M8 1.8 9.5 6 13.8 7.5 9.5 9 8 13.2 6.5 9 2.2 7.5 6.5 6z" />
          </svg>
        </span>
        <span class="grow__txt">
          <span class="grow__lbl">{row.subject}</span>
          <span class="grow__sub">{row.statement}</span>
        </span>
        <span class="grow__val">
          <span class="conv" aria-label="conviction {convictionDots(row.confidence)} of 5">
            {#each [0, 1, 2, 3, 4] as dot (dot)}
              <i class:on={dot < convictionDots(row.confidence)}></i>
            {/each}
          </span>
          <Chev />
        </span>
      </button>
    {/each}
  {/if}
</Tile>
