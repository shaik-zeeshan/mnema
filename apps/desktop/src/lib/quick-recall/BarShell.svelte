<!-- Quick Recall bar shell (recall.html `.rc`). The window IS this bar: a
     frameless ~800px panel whose height follows its content — 60 idle (the bar
     row alone), ≤ `cap` with a body (420 for results/answers; Quick Look passes
     ~570). The shell measures its own border box and asks Rust to resize the
     window (`set_quick_recall_frame`), which keeps the top edge anchored so the
     bar grows downward. Nothing inside may size itself off the viewport (vh,
     height: 100%): the viewport is the window this shell is sizing.

     Slots: `bar` = the 58px field row; `body` = everything below it (omit it
     for the idle bar — the hairline goes with it); `foot` = the 32px key row.
     The body is a flex column capped by `cap`: a child with
     `flex: 1 1 auto; min-height: 0; overflow-y: auto` becomes the scroller,
     anything else scrolls the body as a whole. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let {
    bar,
    body,
    foot,
    cap = 420,
  }: {
    bar: Snippet;
    body?: Snippet;
    foot?: Snippet;
    cap?: number;
  } = $props();

  let shell = $state<HTMLElement | null>(null);

  $effect(() => {
    const el = shell;
    if (!el) return;
    let requested = 0;
    const observer = new ResizeObserver(() => {
      const height = Math.ceil(el.getBoundingClientRect().height);
      if (height === requested) return;
      requested = height;
      invoke("set_quick_recall_frame", { height }).catch(() => {
        // Outside Tauri (or a window torn down mid-resize) there is no frame to set.
      });
    });
    observer.observe(el);
    return () => observer.disconnect();
  });
</script>

<section
  bind:this={shell}
  class="rc"
  aria-label="Quick Recall"
  style:--cap="{cap}px"
>
  <div class="rc-bar">{@render bar()}</div>
  {#if body}
    <div class="rc-body">{@render body()}</div>
    {#if foot}
      <footer class="rc-foot">{@render foot()}</footer>
    {/if}
  {/if}
</section>

<style>
  /* The panel's own edge; the window's native shadow and 16px content-view
     clip (windows.rs) frame it. Opaque: a transparent window cannot blur the
     desktop behind it the way the mockup's backdrop-filter draws. */
  .rc {
    display: flex;
    flex-direction: column;
    max-height: var(--cap);
    box-sizing: border-box;
    overflow: hidden;
    border: 1px solid var(--app-border-strong);
    border-radius: var(--r-xl);
    background: var(--app-bg);
    color: var(--app-text);
  }

  .rc-bar {
    flex: none;
    height: 58px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px 0 13px;
  }

  .rc-body {
    flex: 0 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
    padding: 8px 8px 10px;
    border-top: 1px solid var(--mx-hairline);
    scroll-padding: 8px;
  }

  .rc-foot {
    flex: none;
    height: 32px;
    display: flex;
    align-items: center;
    gap: var(--s-4);
    padding: 0 14px;
    border-top: 1px solid var(--mx-hairline);
    background: color-mix(in srgb, var(--app-surface) 55%, transparent);
    font: 400 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
    white-space: nowrap;
  }
</style>
