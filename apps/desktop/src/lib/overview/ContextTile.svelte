<script lang="ts">
  // The way in to Context (⌃K). Corrected per page 10: "N facts about you" named
  // an entity the backend does not have — there is no `fact` anywhere in it. The
  // real counts are conclusions and the standing statements you wrote, and
  // "covered to HH:MM" is the derivation watermark, the one honest way to say
  // how current the understanding is. A count with a made-up noun on it is the
  // same G8 violation as a made-up number.
  //
  // Props are unchanged: no new read is added here — every number comes from the
  // `get_user_context_status` cell Overview already loads.
  import Tile from "./Tile.svelte";
  import Chev from "./Chev.svelte";
  import type { Cell } from "./data";
  import { formatClock } from "./format";
  import type { Conclusion, UserContextStatus } from "$lib/types/recording";

  interface Props {
    context: Cell<UserContextStatus | null>;
    conclusions: Cell<Conclusion[]>;
    loaded: boolean;
    open: () => void;
  }

  let { context, conclusions, loaded, open }: Props = $props();

  const conclusionCount = $derived(context.data?.conclusionCount ?? conclusions.data?.length ?? null);
  // Standing statements are not on `UserContextStatus`, and this tile adds no
  // read of its own — so the line only renders when Overview's status cell is
  // there to say the engine has anything at all, and names what it can.
  const subjectCount = $derived(context.data?.subjectCount ?? null);
  const coveredTo = $derived(
    context.data?.coveredUntilMs ? formatClock(context.data.coveredUntilMs) : null,
  );
</script>

<Tile id="context" title="Context" kbd="⌃K" {open} openLabel="Open Context">
  {#if context.error}
    <p class="tile-empty t-meta">Context unavailable — {context.error}</p>
  {:else if loaded && (conclusionCount === null || conclusionCount === 0)}
    <p class="tile-empty t-meta">Nothing worked out yet.</p>
  {:else}
    <div class="tile-row">
      <span class="t-ui strong is-mono is-num">{conclusionCount ?? 0}</span>
      <span class="t-meta">{conclusionCount === 1 ? "conclusion" : "conclusions"}</span>
    </div>
    {#if subjectCount !== null}
      <div class="tile-row">
        <span class="t-ui strong is-mono is-num">{subjectCount}</span>
        <span class="t-meta">{subjectCount === 1 ? "subject" : "subjects"}</span>
      </div>
    {/if}
    <div class="tile-row ctx__foot">
      <span class="t-meta ctx__review">{coveredTo ? `covered to ${coveredTo}` : "Review all"}</span>
      <Chev />
    </div>
  {/if}
</Tile>
