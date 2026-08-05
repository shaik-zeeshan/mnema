<script lang="ts">
  // What Mnema has written down about you. `conclusionCount` is a stored count,
  // and the newest belief is the newest row of the list already read for the
  // Subjects tile — no third query.
  //
  // The mockup's "3 pending" header note is absent: there is no pending-review
  // count in `UserContextStatus`, and G8 says a number that isn't real on this
  // machine does not ship.
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";
  import type { LoadState } from "./overview-data.svelte";
  import TileShell from "./TileShell.svelte";

  interface Props {
    conclusions: LoadState<Conclusion[]>;
    status: LoadState<UserContextStatus | null>;
  }

  let { conclusions, status }: Props = $props();

  const count = $derived(status.status === "ok" ? (status.value?.conclusionCount ?? null) : null);

  const newest = $derived.by<Conclusion | null>(() => {
    if (conclusions.status !== "ok") return null;
    let best: Conclusion | null = null;
    for (const c of conclusions.value) if (!best || c.formedAtMs > best.formedAtMs) best = c;
    return best;
  });

  const quiet = $derived(
    status.status === "failed" && conclusions.status === "failed"
      ? "Couldn't read your context."
      : count === 0
        ? "Nothing written down about you yet."
        : null,
  );
</script>

<TileShell label="Context" {quiet}>
  {#if count !== null && count > 0}
    <div class="ss-trow">
      <span class="t-ui strong is-num">{count.toLocaleString()}</span>
      <span class="t-ui">{count === 1 ? "fact about you" : "facts about you"}</span>
    </div>
  {/if}
  {#if newest}
    <div class="ss-trow newest">
      <span class="t-meta">Newest: “{newest.statement}”</span>
    </div>
  {/if}
</TileShell>

<style>
  .strong {
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }

  .newest {
    align-items: flex-start;
    overflow: hidden;
  }

  .newest .t-meta {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
