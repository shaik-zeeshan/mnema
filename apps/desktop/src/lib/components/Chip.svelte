<!-- Scope chip (system.css §6 / design.html `.chip`): a small pill toggle used
     by the Quick Look scopes row (and later the Overview). Quiet group-fill at
     rest; accent-filled when on. An optional onremove renders a trailing ×
     (used by typed operator chips carried into the row). -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    on = false,
    disabled = false,
    onclick,
    onremove,
    title,
    children,
  }: {
    label?: string;
    on?: boolean;
    disabled?: boolean;
    onclick?: () => void;
    onremove?: () => void;
    title?: string;
    children?: Snippet;
  } = $props();
</script>

<button
  type="button"
  class="chip"
  class:chip--on={on}
  {disabled}
  aria-pressed={on}
  {title}
  {onclick}
>
  {#if children}{@render children()}{:else}{label}{/if}
  {#if onremove}
    <span
      class="chip__x"
      role="button"
      tabindex="-1"
      aria-label={`Remove ${label ?? "filter"}`}
      onclick={(e) => {
        e.stopPropagation();
        onremove();
      }}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.stopPropagation();
          onremove();
        }
      }}>×</span
    >
  {/if}
</button>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--gap-inline);
    height: var(--h-sm);
    padding: 0 var(--s-8);
    border: none;
    border-radius: var(--r-pill);
    background: var(--app-surface-raised);
    font: var(--w-medium) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-muted);
    white-space: nowrap;
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }

  .chip:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  .chip:focus-visible {
    outline: none;
    box-shadow: var(--ring, var(--app-ring));
  }

  .chip:disabled {
    opacity: var(--opacity-disabled, 0.45);
    pointer-events: none;
  }

  .chip--on,
  .chip--on:hover {
    background: var(--app-accent);
    color: var(--app-accent-contrast);
  }

  .chip__x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    font-size: var(--t-ui);
    line-height: 1;
    opacity: 0.75;
  }

  .chip__x:hover {
    opacity: 1;
    background: color-mix(in srgb, currentColor 18%, transparent);
  }
</style>
