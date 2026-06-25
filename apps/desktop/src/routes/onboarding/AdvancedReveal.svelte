<script lang="ts">
  import { untrack, type Snippet } from "svelte";
  import IconChevron from "~icons/lucide/chevron-right";

  // A disclosure toggle wrapping the existing `.reveal` animation. Holds only
  // local `open` state; the controls inside keep binding to the parent's drafts
  // so they serialize regardless of visibility. `open` seeds the initial state,
  // so a bay that remounts (via the parent's `{#key activeStep}`) can pre-open
  // this when the relevant draft diverges from its default.
  let {
    label = "Advanced",
    open = false,
    children,
  }: {
    label?: string;
    open?: boolean;
    children: Snippet;
  } = $props();

  // Seed once from the prop; the parent remounts the bay (via `{#key}`) to
  // re-evaluate `open`, so we deliberately do not track it after creation.
  let expanded = $state(untrack(() => open));
</script>

<div class="adv">
  <button
    type="button"
    class="adv__toggle"
    aria-expanded={expanded}
    onclick={() => (expanded = !expanded)}
  >
    <span class="adv__chevron" class:adv__chevron--open={expanded} aria-hidden="true">
      <IconChevron />
    </span>
    {label}
  </button>
  {#if expanded}
    <div class="adv__panel">
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .adv {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .adv__toggle {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    align-self: flex-start;
    padding: 4px 6px 4px 2px;
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    transition: color 0.12s;
  }
  .adv__toggle:hover {
    color: var(--app-text);
  }
  .adv__toggle:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
    color: var(--app-text);
  }
  .adv__chevron {
    display: inline-flex;
    color: var(--app-accent);
    transition: transform 0.18s ease;
  }
  /* Match the original bespoke 9px / thin-stroke disclosure caret. */
  .adv__chevron :global(svg) {
    width: 9px;
    height: 9px;
    stroke-width: 1.4;
  }
  .adv__chevron--open {
    transform: rotate(90deg);
  }
  .adv__panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: adv-reveal 0.22s ease-out;
  }
  @keyframes adv-reveal {
    from { opacity: 0; transform: translateY(-3px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .adv__panel { animation: none; }
    .adv__chevron { transition: none; }
  }
</style>
