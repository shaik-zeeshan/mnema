<script lang="ts">
  // Five real captured frames, edge to edge, each with its wall-clock chip.
  // The frames come from `get_moments` + `get_frame_preview`; a frame whose
  // preview could not be produced draws its chip over empty chrome rather than
  // a stand-in picture.
  import Tile from "./Tile.svelte";
  import { formatClock } from "./format";
  import type { Cell, MomentCard } from "./data";

  interface Props {
    moments: Cell<MomentCard[]>;
    loaded: boolean;
    open: () => void;
  }

  let { moments, loaded, open }: Props = $props();
</script>

<Tile id="moments" title="Moments" span={4} media {open}>
  {#if moments.error}
    <p class="tile-empty t-meta">Moments unavailable — {moments.error}</p>
  {:else if loaded && (moments.data?.length ?? 0) === 0}
    <p class="tile-empty t-meta">No moments captured today yet.</p>
  {:else}
    <div class="strip">
      {#each moments.data ?? [] as moment (moment.frameId)}
        <i>
          {#if moment.url}
            <img src={moment.url} alt={moment.title} loading="lazy" />
          {/if}
          <span class="strip__t">{formatClock(moment.capturedAtMs)}</span>
        </i>
      {/each}
      {#if (moments.data?.length ?? 0) > 0}
        <span class="strip__k"><span class="kbd tile-k">⌃M</span></span>
      {/if}
    </div>
  {/if}
</Tile>
