<script lang="ts">
  // Frame 6 of the mockup: every non-happy path the drawer can reach, drawn as a
  // real panel instead of a bare <p>. Loading gets a skeleton in the SHAPE of the
  // content (three gutter+prose rows), not a spinner; the rest get a headline, a
  // sentence of plain-language explanation, the actions that lead out, and the
  // provenance footnote the app already parses.
  import type { Snippet } from "svelte";

  interface Props {
    /** `skeleton` draws the loading shape; anything else draws the notice. */
    kind?: "notice" | "skeleton";
    title?: string;
    body?: string;
    /** Mono provenance line, e.g. `skipReason silent · audioPeak 0.004`. */
    footnote?: string;
    actions?: Snippet;
  }

  let { kind = "notice", title = "", body = "", footnote = "", actions }: Props = $props();
</script>

{#if kind === "skeleton"}
  <div class="skel" aria-busy="true" aria-label="Loading transcript" role="status">
    {#each [3, 2, 3] as lineCount, row (row)}
      <div class="skel__row">
        <span class="skel__gutter"></span>
        <div class="skel__lines">
          {#each Array.from({ length: lineCount }) as _, line (line)}
            <span style="animation-delay: {line * 0.1}s"></span>
          {/each}
        </div>
      </div>
    {/each}
  </div>
{:else}
  <div class="notice">
    <h3>{title}</h3>
    {#if body}<p>{body}</p>{/if}
    {#if actions}
      <div class="notice__actions">{@render actions()}</div>
    {/if}
    {#if footnote}<span class="notice__mono">{footnote}</span>{/if}
  </div>
{/if}

<style>
  .notice {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
  }

  /* TACTILE: a state panel is prose on the drawer surface — the type ramp does
     the work, so no box, no tint, no glyph. */
  .notice h3 {
    margin: 0;
    font-size: var(--t-ui);
    font-weight: var(--w-medium);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }

  .notice p {
    margin: 0;
    max-width: 52ch;
    font-size: var(--t-meta);
    line-height: 1.55;
    color: var(--app-text-muted);
  }

  .notice__actions {
    display: flex;
    justify-content: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 4px;
  }

  .notice__mono {
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-variant-numeric: tabular-nums;
    letter-spacing: var(--ls-label);
    color: var(--app-text-subtle);
  }

  /* ── loading: the shape of the content ──────────────────────────────────── */
  .skel {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
    padding: 16px 24px;
  }

  .skel__row {
    display: grid;
    grid-template-columns: 110px minmax(0, 42ch);
    gap: 16px;
    margin-bottom: 16px;
  }

  .skel__gutter {
    justify-self: end;
    width: 70%;
  }

  .skel__lines {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .skel__lines span:nth-child(2) {
    width: 92%;
  }

  .skel__lines span:nth-child(3) {
    width: 64%;
  }

  .skel span {
    display: block;
    height: 9px;
    border-radius: 3px;
    background: var(--app-surface-hover);
    animation: drawer-skel-pulse 1.4s ease-in-out infinite;
  }

  @keyframes drawer-skel-pulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 0.9;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .skel span {
      animation: none;
    }
  }
</style>
