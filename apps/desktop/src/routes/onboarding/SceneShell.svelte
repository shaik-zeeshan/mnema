<script lang="ts">
  import type { Snippet } from "svelte";

  // Presentational shell for an onboarding "subsystem bay". It carries no
  // business state: the parent keeps every draft, derivation, and Tauri
  // subscription. The ambient backdrop (dimmed grid + halo) is rendered once by
  // the parent's `.stage` and persists across steps — deliberately *outside*
  // this component so it never becomes a transform/overflow ancestor of the
  // content (which would clip the non-portaled privacy combobox; see the Shield
  // bay). This shell only lays out the header and a scrollable content slot.
  let {
    index,
    eyebrow,
    title,
    subtitle,
    comboboxOpen = false,
    status,
    children,
  }: {
    index: string;
    eyebrow: string;
    title: string;
    subtitle: string;
    comboboxOpen?: boolean;
    status?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="scene" aria-labelledby="scene-title">
  <header class="scene__head">
    <div class="scene__meta">
      <span class="scene__index">{index}</span>
      <span class="scene__eyebrow">{eyebrow}</span>
      {#if status}
        <span class="scene__status">{@render status()}</span>
      {/if}
    </div>
    <h2 id="scene-title" class="scene__title">{title}</h2>
    <p class="scene__subtitle">{subtitle}</p>
  </header>

  <div class="scene__body" class:scene__body--combobox-open={comboboxOpen}>
    {@render children()}
  </div>
</section>

<style>
  .scene {
    position: relative;
    z-index: 1;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 22px 6px;
  }

  .scene__head {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .scene__meta {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .scene__index {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 30px;
    height: 20px;
    padding: 0 7px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    border-radius: 3px;
    color: var(--app-accent);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    font-variant-numeric: tabular-nums;
  }
  .scene__eyebrow {
    color: var(--app-text-subtle);
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.22em;
    text-transform: uppercase;
  }
  .scene__status {
    margin-left: auto;
  }
  .scene__title {
    margin: 0;
    color: var(--app-text-strong);
    font-size: 22px;
    font-weight: 700;
    line-height: 1.05;
    letter-spacing: -0.012em;
  }
  .scene__subtitle {
    margin: 0;
    max-width: 54ch;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.4;
    letter-spacing: 0.01em;
  }

  /* Content area owns the scroll so the framed stage and its backdrop stay
     fixed while a tall bay (e.g. Mind) scrolls internally. */
  .scene__body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    /* overflow:auto clips BOTH axes. Full-width controls paint past the content
       edge there — a slider thumb pinned to a track extreme (its outer half +
       ring) and focused radio/chip outlines — so they get sliced on the left
       and right. Reserve a horizontal gutter and pull it back with a matching
       negative margin: the content keeps the same width and alignment, but the
       clip box widens enough to hold those rings. */
    padding: 0 10px 12px;
    margin-inline: -10px;
  }
  /* Shield bay only: the app-picker dropdown is absolutely positioned and does
     not portal, so when it opens we drop the scroll clip and lift the content
     above the footer. The bay's content is short, so visible overflow is safe. */
  .scene__body--combobox-open {
    overflow: visible;
    position: relative;
    z-index: 20;
  }

  @media (max-width: 600px) {
    .scene {
      padding: 16px 16px 4px;
      gap: 14px;
    }
    .scene__title {
      font-size: 23px;
    }
  }
</style>
