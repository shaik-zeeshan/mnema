<script lang="ts">
  // Landed-frame match outlines: a Quick Recall screen hit opened the Timeline,
  // so the words it matched are outlined on that one frame (D4) — solid for a
  // keyword match, dashed when the hit was found by meaning. The page mounts this
  // only while the exact preview of the landed frame is painted; dismissing the
  // landing chip unmounts it. Same boxes Quick Look draws (`.rc-box`).
  import { invoke } from "@tauri-apps/api/core";
  import type { MatchBox } from "$lib/types/app-infra";

  let {
    frameId,
    query,
    byMeaning,
    rect,
  }: {
    frameId: number;
    query: string;
    byMeaning: boolean;
    /** The painted image rect inside the stage (the page's `renderedImageRect`). */
    rect: { left: number; top: number; width: number; height: number };
  } = $props();

  let boxes = $state<MatchBox[]>([]);

  $effect(() => {
    const request = { frameId, query, byMeaning };
    let live = true;
    boxes = [];
    invoke<MatchBox[]>("get_frame_match_boxes", { request })
      .then((next) => {
        if (live) boxes = next;
      })
      // Best-effort: no outline beats an error over the frame.
      .catch(() => {});
    return () => {
      live = false;
    };
  });
</script>

{#if boxes.length > 0 && rect.width > 0 && rect.height > 0}
  <div
    class="rc-boxes"
    aria-hidden="true"
    style:left="{rect.left}px"
    style:top="{rect.top}px"
    style:width="{rect.width}px"
    style:height="{rect.height}px"
  >
    {#each boxes as box, i (i)}
      <span
        class="rc-box"
        data-kind={box.kind}
        style:left="{box.x * 100}%"
        style:top="{box.y * 100}%"
        style:width="{box.w * 100}%"
        style:height="{box.h * 100}%"
      ></span>
    {/each}
  </div>
{/if}

<style>
  .rc-boxes {
    position: absolute;
    pointer-events: none;
  }
  .rc-box {
    position: absolute;
    border-radius: 3px;
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    box-shadow: inset 0 0 0 1.5px var(--app-accent);
  }
  .rc-box[data-kind="meaning"] {
    background: color-mix(in srgb, var(--app-fg) 7%, transparent);
    box-shadow: none;
    border: 1px dashed var(--app-text-subtle);
  }
</style>
